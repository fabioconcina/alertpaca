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
                }]
            } else {
                let status = if security > 0 {
                    CheckStatus::Warning
                } else {
                    CheckStatus::Ok
                };

                let summary = if security > 0 {
                    format!("{} upgradable ({} security)", total, security)
                } else {
                    format!("{} upgradable", total)
                };

                vec![CheckResult {
                    section: Section::Updates,
                    name: "packages".into(),
                    status,
                    summary,
                }]
            }
        }
        Err(e) => vec![CheckResult {
            section: Section::Updates,
            name: "packages".into(),
            status: CheckStatus::Skipped,
            summary: e,
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
