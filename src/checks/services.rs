use std::io::{Read, Write};
use std::process::Command;

use super::{CheckResult, CheckStatus, Section};
use crate::config::SystemdConfig;

pub fn check_services(systemd_config: &Option<SystemdConfig>) -> Vec<CheckResult> {
    let mut results = Vec::new();
    results.extend(check_systemd(systemd_config));
    results.extend(check_docker());
    results
}

fn check_systemd(config: &Option<SystemdConfig>) -> Vec<CheckResult> {
    let output = match Command::new("systemctl")
        .args(["--no-pager", "--plain", "list-units", "--state=failed", "--no-legend"])
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            return vec![CheckResult {
                section: Section::Services,
                name: "systemd".into(),
                status: CheckStatus::Skipped,
                summary: "systemctl not available".into(),
                ..Default::default()
            }];
        }
    };

    let ignore = config.as_ref().map(|c| &c.ignore[..]).unwrap_or(&[]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let failed: Vec<&str> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split_whitespace().next().unwrap_or("unknown"))
        .filter(|unit| !ignore.iter().any(|i| i == unit))
        .collect();

    if failed.is_empty() {
        vec![CheckResult {
            section: Section::Services,
            name: "systemd".into(),
            status: CheckStatus::Ok,
            summary: "all units ok".into(),
            ..Default::default()
        }]
    } else {
        let names: Vec<String> = failed.iter().take(3).map(|s| s.to_string()).collect();
        let mut summary = format!("{} failed: {}", failed.len(), names.join(", "));
        if failed.len() > 3 {
            summary.push_str(&format!(" (+{})", failed.len() - 3));
        }
        vec![CheckResult {
            section: Section::Services,
            name: "systemd".into(),
            status: CheckStatus::Critical,
            summary,
            ..Default::default()
        }]
    }
}

fn check_docker() -> Vec<CheckResult> {
    // Check if Docker socket exists
    let socket_path = "/var/run/docker.sock";
    if !std::path::Path::new(socket_path).exists() {
        return vec![];
    }

    // Connect to Docker socket and query containers
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;

        let mut stream = match UnixStream::connect(socket_path) {
            Ok(s) => s,
            Err(_) => {
                return vec![CheckResult {
                    section: Section::Services,
                    name: "docker".into(),
                    status: CheckStatus::Skipped,
                    summary: "cannot connect to Docker socket".into(),
                    ..Default::default()
                }];
            }
        };

        let request = "GET /containers/json?all=true HTTP/1.0\r\nHost: localhost\r\n\r\n";
        if stream.write_all(request.as_bytes()).is_err() {
            return vec![CheckResult {
                section: Section::Services,
                name: "docker".into(),
                status: CheckStatus::Skipped,
                summary: "failed to query Docker".into(),
                ..Default::default()
            }];
        }

        let mut response = String::new();
        if stream.read_to_string(&mut response).is_err() {
            return vec![CheckResult {
                section: Section::Services,
                name: "docker".into(),
                status: CheckStatus::Skipped,
                summary: "failed to read Docker response".into(),
                ..Default::default()
            }];
        }

        // Parse HTTP response — find the JSON body after the blank line
        let body = match response.split("\r\n\r\n").nth(1) {
            Some(b) => b,
            None => {
                return vec![CheckResult {
                    section: Section::Services,
                    name: "docker".into(),
                    status: CheckStatus::Skipped,
                    summary: "unexpected Docker response".into(),
                    ..Default::default()
                }];
            }
        };

        let containers: Vec<serde_json::Value> = match serde_json::from_str(body) {
            Ok(c) => c,
            Err(_) => {
                return vec![CheckResult {
                    section: Section::Services,
                    name: "docker".into(),
                    status: CheckStatus::Skipped,
                    summary: "failed to parse Docker response".into(),
                    ..Default::default()
                }];
            }
        };

        parse_docker_containers(&containers)
    }

    #[cfg(not(unix))]
    {
        vec![]
    }
}

fn parse_docker_containers(containers: &[serde_json::Value]) -> Vec<CheckResult> {
    let mut results = Vec::new();
    let mut healthy_count = 0u32;
    let mut problem_count = 0u32;

    for container in containers {
        let state = container["State"].as_str().unwrap_or("");
        let status = container["Status"].as_str().unwrap_or("");
        let names = container["Names"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .trim_start_matches('/');

        if state == "restarting" {
            problem_count += 1;
            results.push(CheckResult {
                section: Section::Services,
                name: "docker".into(),
                status: CheckStatus::Warning,
                summary: format!("{} restarting", names),
                ..Default::default()
            });
        } else if status.contains("unhealthy") {
            problem_count += 1;
            results.push(CheckResult {
                section: Section::Services,
                name: "docker".into(),
                status: CheckStatus::Warning,
                summary: format!("{} unhealthy", names),
                ..Default::default()
            });
        } else if state == "running" {
            healthy_count += 1;
        }
    }

    if healthy_count > 0 || (problem_count == 0 && !containers.is_empty()) {
        results.push(CheckResult {
            section: Section::Services,
            name: "docker".into(),
            status: CheckStatus::Ok,
            summary: format!(
                "{} container{} healthy",
                healthy_count,
                if healthy_count == 1 { "" } else { "s" }
            ),
            ..Default::default()
        });
    }

    if containers.is_empty() {
        results.push(CheckResult {
            section: Section::Services,
            name: "docker".into(),
            status: CheckStatus::Ok,
            summary: "no containers".into(),
            ..Default::default()
        });
    }

    results
}
