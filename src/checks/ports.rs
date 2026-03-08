use super::{CheckResult, CheckStatus, Section};

#[cfg(target_os = "linux")]
use crate::state::{Listener, PortState};

pub fn check_ports() -> Vec<CheckResult> {
    #[cfg(target_os = "linux")]
    {
        check_ports_linux()
    }

    #[cfg(not(target_os = "linux"))]
    {
        vec![CheckResult {
            section: Section::Ports,
            name: "listeners".into(),
            status: CheckStatus::Skipped,
            summary: "port tracking requires Linux".into(),
        }]
    }
}

#[cfg(target_os = "linux")]
fn check_ports_linux() -> Vec<CheckResult> {
    let current = match gather_listeners() {
        Ok(l) => l,
        Err(e) => {
            return vec![CheckResult {
                section: Section::Ports,
                name: "listeners".into(),
                status: CheckStatus::Skipped,
                summary: format!("failed to read ports: {}", e),
            }];
        }
    };

    let previous = PortState::load();
    let now = chrono::Utc::now().timestamp();

    // Save current state
    let new_state = PortState {
        listeners: current.clone(),
        timestamp: now,
    };
    let _ = new_state.save();

    match previous {
        None => {
            vec![CheckResult {
                section: Section::Ports,
                name: "listeners".into(),
                status: CheckStatus::Ok,
                summary: format!("baseline recorded ({} listeners)", current.len()),
            }]
        }
        Some(prev) => diff_ports(&prev.listeners, &current),
    }
}

#[cfg(target_os = "linux")]
fn diff_ports(previous: &[Listener], current: &[Listener]) -> Vec<CheckResult> {
    use std::collections::HashSet;

    let prev_set: HashSet<(String, u16)> = previous
        .iter()
        .map(|l| (l.addr.clone(), l.port))
        .collect();
    let curr_set: HashSet<(String, u16)> = current
        .iter()
        .map(|l| (l.addr.clone(), l.port))
        .collect();

    let new_listeners: Vec<_> = curr_set.difference(&prev_set).collect();
    let missing_listeners: Vec<_> = prev_set.difference(&curr_set).collect();

    let mut results = Vec::new();

    if new_listeners.is_empty() && missing_listeners.is_empty() {
        results.push(CheckResult {
            section: Section::Ports,
            name: "listeners".into(),
            status: CheckStatus::Ok,
            summary: "no changes since last check".into(),
        });
        return results;
    }

    for (addr, port) in &new_listeners {
        results.push(CheckResult {
            section: Section::Ports,
            name: "listeners".into(),
            status: CheckStatus::Ok,
            summary: format!("new: {}:{}", addr, port),
        });
    }

    for (addr, port) in &missing_listeners {
        results.push(CheckResult {
            section: Section::Ports,
            name: "listeners".into(),
            status: CheckStatus::Warning,
            summary: format!("missing: {}:{}", addr, port),
        });
    }

    results
}

#[cfg(target_os = "linux")]
fn gather_listeners() -> Result<Vec<Listener>, String> {
    let mut listeners = Vec::new();

    for path in &["/proc/net/tcp", "/proc/net/tcp6"] {
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for line in contents.lines().skip(1) {
            if let Some(listener) = parse_proc_net_line(line) {
                listeners.push(listener);
            }
        }
    }

    Ok(listeners)
}

#[cfg(target_os = "linux")]
fn parse_proc_net_line(line: &str) -> Option<Listener> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 4 {
        return None;
    }

    // State field (index 3): 0A = LISTEN
    let state = fields[3];
    if state != "0A" {
        return None;
    }

    // Local address field (index 1): ADDR:PORT in hex
    let local = fields[1];
    let (addr_hex, port_hex) = local.rsplit_once(':')?;

    let port = u16::from_str_radix(port_hex, 16).ok()?;
    let addr = parse_hex_addr(addr_hex);

    Some(Listener {
        addr,
        port,
        process: String::new(),
    })
}

#[cfg(target_os = "linux")]
fn parse_hex_addr(hex: &str) -> String {
    match hex.len() {
        8 => {
            // IPv4: stored as little-endian 32-bit
            let bytes = u32::from_str_radix(hex, 16).unwrap_or(0).to_be_bytes();
            format!("{}.{}.{}.{}", bytes[3], bytes[2], bytes[1], bytes[0])
        }
        32 => {
            // IPv6: stored as 4 little-endian 32-bit words
            let mut parts = Vec::new();
            for i in 0..4 {
                let word = &hex[i * 8..(i + 1) * 8];
                let val = u32::from_str_radix(word, 16).unwrap_or(0);
                let bytes = val.to_be_bytes();
                parts.push(format!(
                    "{:02x}{:02x}:{:02x}{:02x}",
                    bytes[3], bytes[2], bytes[1], bytes[0]
                ));
            }
            parts.join(":")
        }
        _ => hex.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_hex_addr_ipv4() {
        assert_eq!(parse_hex_addr("0100007F"), "127.0.0.1");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_proc_line_listen() {
        let line = "   0: 0100007F:0035 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 12345 1 0000000000000000 100 0 0 10 0";
        let listener = parse_proc_net_line(line).unwrap();
        assert_eq!(listener.port, 53);
        assert_eq!(listener.addr, "127.0.0.1");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_proc_line_not_listen() {
        let line = "   1: 0100007F:0035 0100007F:9C40 01 00000000:00000000 00:00000000 00000000     0        0 12345 1 0000000000000000 100 0 0 10 0";
        assert!(parse_proc_net_line(line).is_none());
    }
}
