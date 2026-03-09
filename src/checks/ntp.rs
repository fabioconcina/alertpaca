use std::net::UdpSocket;
use std::time::Duration;

use super::{CheckResult, CheckStatus, Section};
use crate::config::NtpConfig;

/// NTP epoch is 1900-01-01, Unix epoch is 1970-01-01.
/// Difference in seconds: 70 years (with 17 leap years).
const NTP_UNIX_OFFSET: u64 = 2_208_988_800;

pub fn check_ntp(config: &Option<NtpConfig>) -> Vec<CheckResult> {
    let server = config
        .as_ref()
        .map(|c| c.server.as_str())
        .unwrap_or("pool.ntp.org");

    let warn_ms = config.as_ref().and_then(|c| c.warn_ms).unwrap_or(100);
    let critical_ms = config.as_ref().and_then(|c| c.critical_ms).unwrap_or(1000);

    match query_ntp_offset(server) {
        Ok(offset_ms) => {
            let abs = offset_ms.unsigned_abs();
            let status = if abs >= critical_ms {
                CheckStatus::Critical
            } else if abs >= warn_ms {
                CheckStatus::Warning
            } else {
                CheckStatus::Ok
            };

            let summary = if abs < 1000 {
                format!("{:+}ms vs {}", offset_ms, server)
            } else {
                format!("{:+.1}s vs {}", offset_ms as f64 / 1000.0, server)
            };

            vec![CheckResult {
                section: Section::Ntp,
                name: "clock".into(),
                status,
                summary,
            }]
        }
        Err(e) => vec![CheckResult {
            section: Section::Ntp,
            name: "clock".into(),
            status: CheckStatus::Skipped,
            summary: e,
        }],
    }
}

fn query_ntp_offset(server: &str) -> Result<i64, String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind: {}", e))?;
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("timeout: {}", e))?;

    socket
        .connect(format!("{}:123", server))
        .map_err(|e| format!("connect {}: {}", server, e))?;

    // Build NTP v4 client request (mode 3)
    let mut packet = [0u8; 48];
    packet[0] = 0x23; // LI=0, VN=4, Mode=3

    let t1 = unix_now_ms();

    socket
        .send(&packet)
        .map_err(|e| format!("send: {}", e))?;

    let mut resp = [0u8; 48];
    let n = socket
        .recv(&mut resp)
        .map_err(|e| format!("recv: {}", e))?;

    let t4 = unix_now_ms();

    if n < 48 {
        return Err("short NTP response".into());
    }

    // Extract transmit timestamp (bytes 40..47)
    let secs = u32::from_be_bytes([resp[40], resp[41], resp[42], resp[43]]) as u64;
    let frac = u32::from_be_bytes([resp[44], resp[45], resp[46], resp[47]]) as u64;

    if secs == 0 {
        return Err("server returned zero timestamp".into());
    }

    let ntp_ms = ((secs - NTP_UNIX_OFFSET) * 1000) + (frac * 1000 / 0x1_0000_0000);

    // Simple offset: server_time - client_midpoint
    let midpoint = (t1 + t4) / 2;
    let offset = ntp_ms as i64 - midpoint as i64;

    Ok(offset)
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
