use std::io::{Read, Write as _};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use super::{CheckResult, CheckStatus, Section};
use crate::config::EndpointConfig;

pub fn check_endpoints(configs: &[EndpointConfig]) -> Vec<CheckResult> {
    if configs.is_empty() {
        return vec![];
    }

    let tls_config = Arc::new(build_tls_config());

    configs
        .iter()
        .map(|c| check_one_endpoint(c, &tls_config))
        .collect()
}

fn build_tls_config() -> ClientConfig {
    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth()
}

struct ParsedUrl<'a> {
    https: bool,
    host: &'a str,
    port: u16,
    path: &'a str,
}

fn parse_url(url: &str) -> Result<ParsedUrl<'_>, String> {
    let (https, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err("unsupported scheme (use http:// or https://)".into());
    };

    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
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
        path,
    })
}

fn check_one_endpoint(config: &EndpointConfig, tls_config: &Arc<ClientConfig>) -> CheckResult {
    let name = config.name.clone();

    let parsed = match parse_url(&config.url) {
        Ok(p) => p,
        Err(e) => {
            return CheckResult {
                section: Section::Endpoints,
                name,
                status: CheckStatus::Skipped,
                summary: e,
                ..Default::default()
            };
        }
    };

    match do_http_get(&parsed, tls_config) {
        Ok((status_code, elapsed)) => {
            let elapsed_ms = elapsed.as_millis();

            let check_status = if let Some(expected) = config.expect_status {
                if status_code == expected {
                    CheckStatus::Ok
                } else {
                    CheckStatus::Critical
                }
            } else if (200..400).contains(&status_code) {
                CheckStatus::Ok
            } else if (400..500).contains(&status_code) {
                CheckStatus::Warning
            } else {
                CheckStatus::Critical
            };

            CheckResult {
                section: Section::Endpoints,
                name,
                status: check_status,
                summary: format!("{} ({}ms)", status_code, elapsed_ms),
                ..Default::default()
            }
        }
        Err(e) => CheckResult {
            section: Section::Endpoints,
            name,
            status: CheckStatus::Critical,
            summary: e,
            ..Default::default()
        },
    }
}

fn do_http_get(parsed: &ParsedUrl<'_>, tls_config: &Arc<ClientConfig>) -> Result<(u16, Duration), String> {
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
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        parsed.path, parsed.host
    );

    let start = Instant::now();

    if parsed.https {
        let server_name = ServerName::try_from(parsed.host.to_string())
            .map_err(|e| format!("invalid hostname: {}", e))?;

        let conn = ClientConnection::new(tls_config.clone(), server_name)
            .map_err(|e| format!("TLS error: {}", e))?;

        let mut tls = StreamOwned::new(conn, tcp);
        tls.write_all(request.as_bytes())
            .map_err(|e| format!("write failed: {}", e))?;

        let status = read_status_code(&mut tls)?;
        Ok((status, start.elapsed()))
    } else {
        let mut tcp = tcp;
        tcp.write_all(request.as_bytes())
            .map_err(|e| format!("write failed: {}", e))?;

        let status = read_status_code(&mut tcp)?;
        Ok((status, start.elapsed()))
    }
}

fn read_status_code(reader: &mut impl Read) -> Result<u16, String> {
    let mut buf = [0u8; 512];
    let n = reader.read(&mut buf).map_err(|e| format!("read failed: {}", e))?;

    if n < 12 {
        return Err("response too short".into());
    }

    // Parse "HTTP/1.x NNN ..."
    let response = std::str::from_utf8(&buf[..n.min(32)])
        .map_err(|_| "invalid response".to_string())?;

    let status_str = response
        .split_whitespace()
        .nth(1)
        .ok_or("no status code in response")?;

    status_str
        .parse::<u16>()
        .map_err(|_| format!("invalid status code: {}", status_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url_http() {
        let p = parse_url("http://localhost:8080/health").unwrap();
        assert!(!p.https);
        assert_eq!(p.host, "localhost");
        assert_eq!(p.port, 8080);
        assert_eq!(p.path, "/health");
    }

    #[test]
    fn test_parse_url_https_default_port() {
        let p = parse_url("https://example.com").unwrap();
        assert!(p.https);
        assert_eq!(p.host, "example.com");
        assert_eq!(p.port, 443);
        assert_eq!(p.path, "/");
    }

    #[test]
    fn test_parse_url_http_default_port() {
        let p = parse_url("http://10.0.0.1/api").unwrap();
        assert!(!p.https);
        assert_eq!(p.host, "10.0.0.1");
        assert_eq!(p.port, 80);
        assert_eq!(p.path, "/api");
    }

    #[test]
    fn test_parse_url_invalid_scheme() {
        assert!(parse_url("ftp://host").is_err());
    }
}
