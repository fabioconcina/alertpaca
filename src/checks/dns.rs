use std::net::UdpSocket;
use std::time::{Duration, Instant};

use super::{CheckResult, CheckStatus, Section};
use crate::config::DnsConfig;

pub fn check_dns(configs: &[DnsConfig]) -> Vec<CheckResult> {
    if configs.is_empty() {
        return vec![];
    }

    configs.iter().map(check_one_dns).collect()
}

fn check_one_dns(config: &DnsConfig) -> CheckResult {
    let server = config.server.as_deref().unwrap_or("127.0.0.1");
    let name = config.name.clone();

    match resolve(&config.domain, server) {
        Ok((resolved, elapsed)) => {
            let ms = elapsed.as_millis();
            let status = if ms > 1000 {
                CheckStatus::Warning
            } else {
                CheckStatus::Ok
            };
            CheckResult {
                section: Section::Dns,
                name,
                status,
                summary: format!("{} ({}ms via {})", resolved, ms, server),
            }
        }
        Err(e) => CheckResult {
            section: Section::Dns,
            name,
            status: CheckStatus::Critical,
            summary: format!("{} via {}", e, server),
        },
    }
}

/// Build a minimal DNS A query packet for the given domain.
fn build_query(domain: &str) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(64);

    // Header: ID=0xABCD, flags=0x0100 (standard query, RD=1)
    // QDCOUNT=1, ANCOUNT=0, NSCOUNT=0, ARCOUNT=0
    pkt.extend_from_slice(&[0xAB, 0xCD, 0x01, 0x00]);
    pkt.extend_from_slice(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

    // Question: encode domain labels
    for label in domain.split('.') {
        let len = label.len();
        if len == 0 || len > 63 {
            continue;
        }
        pkt.push(len as u8);
        pkt.extend_from_slice(label.as_bytes());
    }
    pkt.push(0); // root label

    // QTYPE=A (1), QCLASS=IN (1)
    pkt.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);

    pkt
}

/// Parse the first A record from a DNS response.
fn parse_response(buf: &[u8], n: usize) -> Result<String, String> {
    if n < 12 {
        return Err("response too short".into());
    }

    let rcode = buf[3] & 0x0F;
    if rcode != 0 {
        return Err(match rcode {
            1 => "FORMERR".into(),
            2 => "SERVFAIL".into(),
            3 => "NXDOMAIN".into(),
            5 => "REFUSED".into(),
            _ => format!("RCODE {}", rcode),
        });
    }

    let ancount = u16::from_be_bytes([buf[4 + 2], buf[5 + 2]]);
    if ancount == 0 {
        return Err("no answers".into());
    }

    // Skip question section
    let mut pos = 12;
    // Skip QNAME
    while pos < n {
        let len = buf[pos] as usize;
        if len == 0 {
            pos += 1;
            break;
        }
        if len >= 0xC0 {
            pos += 2;
            break;
        }
        pos += 1 + len;
    }
    pos += 4; // skip QTYPE + QCLASS

    // Parse answer records, find first A record
    for _ in 0..ancount {
        if pos >= n {
            break;
        }
        // Skip NAME (may be compressed)
        if pos < n && buf[pos] >= 0xC0 {
            pos += 2;
        } else {
            while pos < n {
                let len = buf[pos] as usize;
                if len == 0 {
                    pos += 1;
                    break;
                }
                pos += 1 + len;
            }
        }

        if pos + 10 > n {
            break;
        }

        let rtype = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
        let rdlength = u16::from_be_bytes([buf[pos + 8], buf[pos + 9]]) as usize;
        pos += 10;

        if rtype == 1 && rdlength == 4 && pos + 4 <= n {
            return Ok(format!("{}.{}.{}.{}", buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]));
        }

        pos += rdlength;
    }

    Err("no A record found".into())
}

fn resolve(domain: &str, server: &str) -> Result<(String, Duration), String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind: {}", e))?;
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("timeout: {}", e))?;

    let dest = if server.contains(':') {
        server.to_string()
    } else {
        format!("{}:53", server)
    };

    socket
        .connect(&dest)
        .map_err(|e| format!("connect: {}", e))?;

    let query = build_query(domain);
    let start = Instant::now();

    socket
        .send(&query)
        .map_err(|e| format!("send: {}", e))?;

    let mut buf = [0u8; 512];
    let n = socket.recv(&mut buf).map_err(|e| format!("recv: {}", e))?;

    let elapsed = start.elapsed();
    let ip = parse_response(&buf, n)?;

    Ok((ip, elapsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_query() {
        let pkt = build_query("example.com");
        // Header is 12 bytes
        assert_eq!(pkt[0..2], [0xAB, 0xCD]); // ID
        assert_eq!(pkt[4..6], [0x00, 0x01]); // QDCOUNT=1
        // First label: 7, "example"
        assert_eq!(pkt[12], 7);
        assert_eq!(&pkt[13..20], b"example");
        // Second label: 3, "com"
        assert_eq!(pkt[20], 3);
        assert_eq!(&pkt[21..24], b"com");
        // Root label
        assert_eq!(pkt[24], 0);
    }

    #[test]
    fn test_parse_response_nxdomain() {
        let mut buf = [0u8; 64];
        buf[3] = 3; // RCODE=NXDOMAIN
        assert_eq!(parse_response(&buf, 64).unwrap_err(), "NXDOMAIN");
    }
}
