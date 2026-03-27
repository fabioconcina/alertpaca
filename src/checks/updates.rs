use super::{CheckResult, CheckStatus, Section};

#[cfg(target_os = "linux")]
pub fn check_updates() -> Vec<CheckResult> {
    match query_apt_updates() {
        Ok((total, security)) => {
            if total == 0 {
                vec![CheckResult {
                    section: Section::Updates,
                    name: "packages".into(),
                    status: CheckStatus::Ok,
                    summary: "system is up to date".into(),
                    ..Default::default()
                }]
            } else {
                let auto_updates = is_unattended_upgrades_active();
                let status = if security > 0 && !auto_updates {
                    CheckStatus::Warning
                } else {
                    CheckStatus::Ok
                };

                let summary = if security > 0 {
                    if auto_updates {
                        format!("{} upgradable ({} security, auto)", total, security)
                    } else {
                        format!("{} upgradable ({} security)", total, security)
                    }
                } else {
                    format!("{} upgradable", total)
                };

                vec![CheckResult {
                    section: Section::Updates,
                    name: "packages".into(),
                    status,
                    summary,
                    ..Default::default()
                }]
            }
        }
        Err(e) => vec![CheckResult {
            section: Section::Updates,
            name: "packages".into(),
            status: CheckStatus::Skipped,
            summary: e,
            ..Default::default()
        }],
    }
}

#[cfg(not(target_os = "linux"))]
pub fn check_updates() -> Vec<CheckResult> {
    vec![CheckResult {
        section: Section::Updates,
        name: "packages".into(),
        status: CheckStatus::Skipped,
        summary: "apt not available (Linux only)".into(),
        ..Default::default()
    }]
}

#[cfg(target_os = "linux")]
fn query_apt_updates() -> Result<(usize, usize), String> {
    use std::process::Command;
    let output = Command::new("apt")
        .args(["list", "--upgradable"])
        .env("LANG", "C")
        .output()
        .map_err(|e| format!("apt: {}", e))?;

    if !output.status.success() {
        return Err("apt list failed".into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut total = 0;
    let mut security = 0;

    for line in stdout.lines() {
        // Skip the "Listing..." header line
        if line.starts_with("Listing") || line.is_empty() {
            continue;
        }
        total += 1;
        // Security updates typically contain "-security" in the origin
        if line.contains("-security") {
            security += 1;
        }
    }

    Ok((total, security))
}

#[cfg(target_os = "linux")]
fn is_unattended_upgrades_active() -> bool {
    use std::process::Command;
    Command::new("systemctl")
        .args(["is-active", "--quiet", "unattended-upgrades"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
