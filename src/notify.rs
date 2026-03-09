use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::checks::{CheckResult, CheckStatus};
use crate::config::NotifyConfig;

#[derive(Debug, Serialize, Deserialize, Default)]
struct LastStatus {
    checks: HashMap<String, String>,
}

fn data_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"))
        .join("alertpaca")
}

fn status_label(s: CheckStatus) -> &'static str {
    match s {
        CheckStatus::Ok => "Ok",
        CheckStatus::Warning => "Warning",
        CheckStatus::Critical => "Critical",
        CheckStatus::Skipped => "Skipped",
    }
}

fn is_problem(s: CheckStatus) -> bool {
    matches!(s, CheckStatus::Warning | CheckStatus::Critical)
}

/// Check results against previous state, send notifications for changes, update state.
pub fn notify(config: &NotifyConfig, results: &[CheckResult]) {
    let path = data_dir().join("last_status.json");

    let previous: LastStatus = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let mut messages: Vec<String> = Vec::new();

    for result in results {
        if result.status == CheckStatus::Skipped {
            continue;
        }

        let key = format!("{}:{}", result.section.label(), result.name);
        let current = status_label(result.status);

        if let Some(prev) = previous.checks.get(&key) {
            if prev == current {
                continue; // no change
            }

            if is_problem(result.status) {
                let icon = if result.status == CheckStatus::Critical {
                    "🔴"
                } else {
                    "🟡"
                };
                messages.push(format!(
                    "{} {} — {} (was {})",
                    icon, key, result.summary, prev
                ));
            } else if result.status == CheckStatus::Ok && (prev == "Warning" || prev == "Critical")
            {
                messages.push(format!("🟢 {} — {} (recovered)", key, result.summary));
            }
        } else if is_problem(result.status) {
            // First run with a problem
            let icon = if result.status == CheckStatus::Critical {
                "🔴"
            } else {
                "🟡"
            };
            messages.push(format!("{} {} — {}", icon, key, result.summary));
        }
    }

    // Save current state
    let mut current = LastStatus::default();
    for result in results {
        if result.status != CheckStatus::Skipped {
            let key = format!("{}:{}", result.section.label(), result.name);
            current
                .checks
                .insert(key, status_label(result.status).to_string());
        }
    }
    if let Ok(json) = serde_json::to_string_pretty(&current) {
        let _ = fs::create_dir_all(data_dir());
        let tmp = path.with_extension("tmp");
        let _ = fs::write(&tmp, &json).and_then(|_| fs::rename(&tmp, &path));
    }

    if messages.is_empty() {
        return;
    }

    let body = messages.join("\n");
    let _ = send_ntfy(&config.url, &body);
}

fn send_ntfy(url: &str, body: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or("invalid ntfy URL")?;
    let is_https = url.starts_with("https://");

    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };

    let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
        (h, p.parse::<u16>().unwrap_or(if is_https { 443 } else { 80 }))
    } else {
        (authority, if is_https { 443 } else { 80 })
    };

    let addr = format!("{}:{}", host, port);
    let timeout = Duration::from_secs(10);

    let socket_addr = addr
        .to_socket_addrs()
        .map_err(|e| format!("resolve: {}", e))?
        .next()
        .ok_or("no address")?;

    let tcp = TcpStream::connect_timeout(&socket_addr, timeout)
        .map_err(|e| format!("connect: {}", e))?;
    tcp.set_write_timeout(Some(timeout)).ok();
    tcp.set_read_timeout(Some(timeout)).ok();

    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
        path,
        host,
        body.len(),
        body
    );

    if is_https {
        use rustls::pki_types::ServerName;
        use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
        use std::sync::Arc;

        let mut root_store = RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls_config = Arc::new(
            ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth(),
        );
        let server_name = ServerName::try_from(host.to_string())
            .map_err(|e| format!("hostname: {}", e))?;
        let conn = ClientConnection::new(tls_config, server_name)
            .map_err(|e| format!("TLS: {}", e))?;
        let mut tls = StreamOwned::new(conn, tcp);
        tls.write_all(request.as_bytes())
            .map_err(|e| format!("write: {}", e))?;
    } else {
        let mut tcp = tcp;
        tcp.write_all(request.as_bytes())
            .map_err(|e| format!("write: {}", e))?;
    }

    Ok(())
}
