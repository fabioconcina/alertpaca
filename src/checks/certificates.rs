use std::io::Read as _;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use super::{CheckResult, CheckStatus, Section, tls_client_config};
use crate::config::CertificateConfig;

pub(crate) fn check_certificates(configs: &[CertificateConfig]) -> Vec<CheckResult> {
    if configs.is_empty() {
        return vec![];
    }

    let tls_config = tls_client_config();

    configs
        .iter()
        .map(|c| check_one_cert(c, &tls_config))
        .collect()
}

fn check_one_cert(config: &CertificateConfig, tls_config: &Arc<ClientConfig>) -> CheckResult {
    let name = config.endpoint.clone();
    let (host, port) = parse_endpoint(&config.endpoint);

    match check_cert_expiry(host, port, tls_config) {
        Ok(days) => {
            let status = if days < 7 {
                CheckStatus::Critical
            } else if days < 30 {
                CheckStatus::Warning
            } else {
                CheckStatus::Ok
            };

            CheckResult {
                section: Section::Certificates,
                name,
                status,
                summary: format!("expires in {}d", days),
                ..Default::default()
            }
        }
        Err(e) => CheckResult {
            section: Section::Certificates,
            name,
            status: CheckStatus::Skipped,
            summary: e,
            ..Default::default()
        },
    }
}

fn parse_endpoint(endpoint: &str) -> (&str, u16) {
    if let Some((host, port_str)) = endpoint.rsplit_once(':')
        && let Ok(port) = port_str.parse::<u16>()
    {
        return (host, port);
    }
    (endpoint, 443)
}

fn check_cert_expiry(host: &str, port: u16, tls_config: &Arc<ClientConfig>) -> Result<i64, String> {
    let addr = format!("{}:{}", host, port);
    let tcp = TcpStream::connect_timeout(
        &addr.parse().map_err(|e| format!("invalid address: {}", e))?,
        Duration::from_secs(10),
    )
    .map_err(|e| format!("connect failed: {}", e))?;

    tcp.set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| format!("set timeout: {}", e))?;

    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| format!("invalid hostname: {}", e))?;

    let conn = ClientConnection::new(tls_config.clone(), server_name)
        .map_err(|e| format!("TLS error: {}", e))?;

    let mut tls = StreamOwned::new(conn, tcp);

    // Force handshake by attempting a read
    let mut buf = [0u8; 0];
    let _ = tls.read(&mut buf);

    // Extract the peer certificate
    let certs = tls
        .conn
        .peer_certificates()
        .ok_or("no peer certificates")?;

    let leaf = certs.first().ok_or("empty certificate chain")?;

    // Parse X.509 to get notAfter
    let (_, cert) = x509_parser::parse_x509_certificate(leaf.as_ref())
        .map_err(|e| format!("x509 parse error: {}", e))?;

    let not_after = cert.validity().not_after.timestamp();
    let now = chrono::Utc::now().timestamp();
    let days = (not_after - now) / 86400;

    Ok(days)
}
