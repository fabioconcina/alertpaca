use std::io::{Read, Write as _};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use serde_json::Value;

use super::{CheckResult, CheckStatus, Section, tls_client_config};
use crate::config::SyncthingConfig;

pub(crate) fn check_syncthing(configs: &[SyncthingConfig]) -> Vec<CheckResult> {
    if configs.is_empty() {
        return vec![];
    }

    let tls_config = tls_client_config();

    configs
        .iter()
        .flat_map(|c| check_one(c, &tls_config))
        .collect()
}

/// Each instance yields a "peers" result and a "folders" result. If the API is
/// unreachable (e.g. the container is down), only a single Critical peers result
/// is returned since nothing else can be queried.
fn check_one(config: &SyncthingConfig, tls_config: &Arc<ClientConfig>) -> Vec<CheckResult> {
    let peers_name = format!("{} peers", config.name);
    let folders_name = format!("{} folders", config.name);

    let connections = match get_json(
        &config.url,
        "/rest/system/connections",
        &config.api_key,
        tls_config,
    ) {
        Ok(v) => v,
        Err(e) => {
            // API unreachable covers the container-down / GUI-down case.
            return vec![CheckResult {
                section: Section::Syncthing,
                name: peers_name,
                status: CheckStatus::Critical,
                summary: format!("API unreachable: {}", e),
                ..Default::default()
            }];
        }
    };

    let mut results = vec![check_peers(peers_name, &connections)];

    let folders_result = match get_json(
        &config.url,
        "/rest/config/folders",
        &config.api_key,
        tls_config,
    ) {
        Ok(v) => check_folders(&folders_name, &v, config, tls_config),
        Err(e) => CheckResult {
            section: Section::Syncthing,
            name: folders_name,
            status: CheckStatus::Critical,
            summary: format!("folder list unreachable: {}", e),
            ..Default::default()
        },
    };
    results.push(folders_result);

    results
}

fn check_peers(name: String, connections: &Value) -> CheckResult {
    let conns = connections.get("connections").and_then(|c| c.as_object());
    let Some(conns) = conns else {
        return CheckResult {
            section: Section::Syncthing,
            name,
            status: CheckStatus::Warning,
            summary: "unexpected connections response".into(),
            ..Default::default()
        };
    };

    let total = conns.len();
    let disconnected: Vec<String> = conns
        .iter()
        .filter(|(_, v)| !v.get("connected").and_then(Value::as_bool).unwrap_or(false))
        .map(|(id, _)| short_id(id))
        .collect();

    if total == 0 {
        return CheckResult {
            section: Section::Syncthing,
            name,
            status: CheckStatus::Warning,
            summary: "no peer devices configured".into(),
            ..Default::default()
        };
    }

    if disconnected.is_empty() {
        CheckResult {
            section: Section::Syncthing,
            name,
            status: CheckStatus::Ok,
            summary: format!("{}/{} connected", total, total),
            ..Default::default()
        }
    } else {
        CheckResult {
            section: Section::Syncthing,
            name,
            status: CheckStatus::Warning,
            summary: format!(
                "{}/{} disconnected: {}",
                disconnected.len(),
                total,
                disconnected.join(", ")
            ),
            ..Default::default()
        }
    }
}

fn check_folders(
    name: &str,
    folders: &Value,
    config: &SyncthingConfig,
    tls_config: &Arc<ClientConfig>,
) -> CheckResult {
    let Some(list) = folders.as_array() else {
        return CheckResult {
            section: Section::Syncthing,
            name: name.to_string(),
            status: CheckStatus::Warning,
            summary: "unexpected folders response".into(),
            ..Default::default()
        };
    };

    let mut errored: Vec<String> = Vec::new();
    let mut paused: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for folder in list {
        let id = folder.get("id").and_then(Value::as_str).unwrap_or("?");
        let label = folder.get("label").and_then(Value::as_str).unwrap_or(id);

        if folder.get("paused").and_then(Value::as_bool).unwrap_or(false) {
            paused.push(label.to_string());
            continue;
        }

        checked += 1;
        let path = format!("/rest/db/status?folder={}", id);
        match get_json(&config.url, &path, &config.api_key, tls_config) {
            Ok(status) => {
                let state = status.get("state").and_then(Value::as_str).unwrap_or("");
                let errors = status.get("errors").and_then(Value::as_i64).unwrap_or(0);
                let pull_errors = status
                    .get("pullErrors")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                if state == "error" || errors > 0 || pull_errors > 0 {
                    let detail = if state == "error" {
                        "error state".to_string()
                    } else {
                        format!("{} errors", errors + pull_errors)
                    };
                    errored.push(format!("{} ({})", label, detail));
                }
            }
            Err(e) => errored.push(format!("{} (status unreachable: {})", label, e)),
        }
    }

    if !errored.is_empty() {
        CheckResult {
            section: Section::Syncthing,
            name: name.to_string(),
            status: CheckStatus::Critical,
            summary: errored.join(", "),
            ..Default::default()
        }
    } else if !paused.is_empty() {
        CheckResult {
            section: Section::Syncthing,
            name: name.to_string(),
            status: CheckStatus::Warning,
            summary: format!("paused: {}", paused.join(", ")),
            ..Default::default()
        }
    } else {
        CheckResult {
            section: Section::Syncthing,
            name: name.to_string(),
            status: CheckStatus::Ok,
            summary: format!("{} in sync", checked),
            ..Default::default()
        }
    }
}

/// Syncthing device IDs are long; the first block is enough to identify a peer.
fn short_id(id: &str) -> String {
    id.split('-').next().unwrap_or(id).to_string()
}

struct ParsedUrl<'a> {
    https: bool,
    host: &'a str,
    port: u16,
    base_path: &'a str,
}

fn parse_url(url: &str) -> Result<ParsedUrl<'_>, String> {
    let (https, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err("unsupported scheme (use http:// or https://)".into());
    };

    let (authority, base_path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].trim_end_matches('/')),
        None => (rest, ""),
    };

    let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
        let port = p.parse::<u16>().map_err(|_| format!("invalid port: {}", p))?;
        (h, port)
    } else {
        (authority, if https { 443 } else { 80 })
    };

    if host.is_empty() {
        return Err("empty host".into());
    }

    Ok(ParsedUrl {
        https,
        host,
        port,
        base_path,
    })
}

/// GET a Syncthing REST path with the API key and parse the JSON body.
fn get_json(
    base_url: &str,
    path: &str,
    api_key: &str,
    tls_config: &Arc<ClientConfig>,
) -> Result<Value, String> {
    let parsed = parse_url(base_url)?;
    let full_path = format!("{}{}", parsed.base_path, path);
    let (status, body) = do_http_get(&parsed, &full_path, api_key, tls_config)?;

    if status != 200 {
        return Err(format!("HTTP {}", status));
    }

    serde_json::from_str(&body).map_err(|e| format!("bad JSON: {}", e))
}

fn do_http_get(
    parsed: &ParsedUrl<'_>,
    path: &str,
    api_key: &str,
    tls_config: &Arc<ClientConfig>,
) -> Result<(u16, String), String> {
    let addr = format!("{}:{}", parsed.host, parsed.port);
    let timeout = Duration::from_secs(10);

    let socket_addr = addr
        .to_socket_addrs()
        .map_err(|e| format!("resolve failed: {}", e))?
        .next()
        .ok_or_else(|| format!("no addresses for {}", parsed.host))?;

    let tcp = TcpStream::connect_timeout(&socket_addr, timeout)
        .map_err(|e| format!("connect failed: {}", e))?;
    tcp.set_read_timeout(Some(timeout))
        .map_err(|e| format!("set timeout: {}", e))?;
    tcp.set_write_timeout(Some(timeout))
        .map_err(|e| format!("set timeout: {}", e))?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nX-API-Key: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        path, parsed.host, api_key
    );

    if parsed.https {
        let server_name = ServerName::try_from(parsed.host.to_string())
            .map_err(|e| format!("invalid hostname: {}", e))?;
        let conn = ClientConnection::new(tls_config.clone(), server_name)
            .map_err(|e| format!("TLS error: {}", e))?;
        let mut tls = StreamOwned::new(conn, tcp);
        tls.write_all(request.as_bytes())
            .map_err(|e| format!("write failed: {}", e))?;
        read_response(&mut tls)
    } else {
        let mut tcp = tcp;
        tcp.write_all(request.as_bytes())
            .map_err(|e| format!("write failed: {}", e))?;
        read_response(&mut tcp)
    }
}

/// Read a full HTTP/1.1 response (delimited by Connection: close), returning the
/// status code and the decoded body.
fn read_response(reader: &mut impl Read) -> Result<(u16, String), String> {
    let mut raw = Vec::new();
    reader
        .read_to_end(&mut raw)
        .map_err(|e| format!("read failed: {}", e))?;

    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no header/body separator")?;
    let head = &raw[..split];
    let body = &raw[split + 4..];

    let head_str = std::str::from_utf8(head).map_err(|_| "invalid headers".to_string())?;
    let mut lines = head_str.split("\r\n");
    let status_line = lines.next().ok_or("empty response")?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or("no status code")?
        .parse::<u16>()
        .map_err(|_| "invalid status code".to_string())?;

    let chunked = lines.any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });

    let body = if chunked {
        dechunk(body)?
    } else {
        body.to_vec()
    };

    let body = String::from_utf8(body).map_err(|_| "non-UTF8 body".to_string())?;
    Ok((status, body))
}

/// Minimal HTTP chunked-transfer decoder (safety net; Syncthing normally sends
/// Content-Length so the body is read close-delimited).
fn dechunk(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let nl = data
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("malformed chunk size")?;
        let size_str = std::str::from_utf8(&data[..nl]).map_err(|_| "bad chunk size")?;
        let size = usize::from_str_radix(size_str.trim().split(';').next().unwrap_or(""), 16)
            .map_err(|_| "bad chunk size")?;
        data = &data[nl + 2..];
        if size == 0 {
            break;
        }
        if data.len() < size {
            return Err("truncated chunk".into());
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size..];
        if data.starts_with(b"\r\n") {
            data = &data[2..];
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_url_base_path_stripped() {
        let p = parse_url("http://localhost:8384/").unwrap();
        assert!(!p.https);
        assert_eq!(p.host, "localhost");
        assert_eq!(p.port, 8384);
        assert_eq!(p.base_path, "");
    }

    #[test]
    fn test_short_id() {
        assert_eq!(short_id("KZYKRZO-CSOASYW-P5J7UW5"), "KZYKRZO");
        assert_eq!(short_id("plain"), "plain");
    }

    #[test]
    fn test_peers_all_connected() {
        let v = json!({"connections": {
            "AAA-BBB": {"connected": true},
            "CCC-DDD": {"connected": true}
        }});
        let r = check_peers("t".into(), &v);
        assert_eq!(r.status, CheckStatus::Ok);
        assert_eq!(r.summary, "2/2 connected");
    }

    #[test]
    fn test_peers_one_disconnected() {
        let v = json!({"connections": {
            "AAA-BBB": {"connected": true},
            "CCC-DDD": {"connected": false}
        }});
        let r = check_peers("t".into(), &v);
        assert_eq!(r.status, CheckStatus::Warning);
        assert!(r.summary.contains("CCC"));
    }

    #[test]
    fn test_dechunk() {
        let raw = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(dechunk(raw).unwrap(), b"Wikipedia");
    }
}
