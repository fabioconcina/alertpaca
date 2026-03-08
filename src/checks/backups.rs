use std::process::Command;
use std::time::SystemTime;

use super::{CheckResult, CheckStatus, Section};
use crate::config::{parse_duration_secs, BackupConfig};

pub fn check_backups(configs: &[BackupConfig]) -> Vec<CheckResult> {
    configs.iter().map(check_one_backup).collect()
}

fn check_one_backup(config: &BackupConfig) -> CheckResult {
    let name = config.name().to_string();
    let max_age_secs = match parse_duration_secs(config.max_age_str()) {
        Ok(s) => s,
        Err(e) => {
            return CheckResult {
                section: Section::Backups,
                name,
                status: CheckStatus::Skipped,
                summary: format!("invalid max_age: {}", e),
            };
        }
    };

    let age_secs = match config {
        BackupConfig::File {
            path, pattern, ..
        } => check_file_backup(path, pattern),
        BackupConfig::Restic {
            repo,
            password_file,
            ..
        } => check_restic_backup(repo, password_file.as_deref()),
        BackupConfig::Zfs { dataset, .. } => check_zfs_backup(dataset),
    };

    match age_secs {
        Ok(age) => {
            let status = if age > max_age_secs * 2 {
                CheckStatus::Critical
            } else if age > max_age_secs {
                CheckStatus::Warning
            } else {
                CheckStatus::Ok
            };

            CheckResult {
                section: Section::Backups,
                name,
                status,
                summary: format!(
                    "last backup {} ago (max: {})",
                    format_age(age),
                    config.max_age_str()
                ),
            }
        }
        Err(e) => CheckResult {
            section: Section::Backups,
            name,
            status: CheckStatus::Skipped,
            summary: e.to_string(),
        },
    }
}

fn check_file_backup(path: &str, pattern: &str) -> Result<i64, String> {
    let full_pattern = format!("{}/{}", path, pattern);
    let mut newest: Option<SystemTime> = None;

    for entry in glob::glob(&full_pattern).map_err(|e| format!("invalid pattern: {}", e))? {
        let entry = entry.map_err(|e| format!("glob error: {}", e))?;
        let meta = entry.metadata().map_err(|e| format!("stat error: {}", e))?;
        let modified = meta.modified().map_err(|e| format!("mtime error: {}", e))?;

        if newest.is_none() || modified > newest.unwrap() {
            newest = Some(modified);
        }
    }

    match newest {
        Some(time) => {
            let age = SystemTime::now()
                .duration_since(time)
                .map_err(|e| format!("time error: {}", e))?;
            Ok(age.as_secs() as i64)
        }
        None => Err("no matching files found".into()),
    }
}

fn check_restic_backup(repo: &str, password_file: Option<&str>) -> Result<i64, String> {
    let mut cmd = Command::new("restic");
    cmd.args(["-r", repo, "snapshots", "--json", "--latest", "1"]);
    if let Some(pf) = password_file {
        cmd.args(["--password-file", pf]);
    }

    let output = cmd.output().map_err(|e| format!("restic not available: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("restic error: {}", stderr.lines().next().unwrap_or("unknown")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let snapshots: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).map_err(|e| format!("parse error: {}", e))?;

    let snap = snapshots.first().ok_or("no snapshots found")?;
    let time_str = snap["time"]
        .as_str()
        .ok_or("missing time field in snapshot")?;

    let snap_time = chrono::DateTime::parse_from_rfc3339(time_str)
        .map_err(|e| format!("time parse error: {}", e))?;

    let age = chrono::Utc::now()
        .signed_duration_since(snap_time)
        .num_seconds();

    Ok(age)
}

fn check_zfs_backup(dataset: &str) -> Result<i64, String> {
    let output = Command::new("zfs")
        .args([
            "list", "-t", "snapshot", "-o", "creation", "-s", "creation",
            "-H", "-p", dataset,
        ])
        .output()
        .map_err(|e| format!("zfs not available: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("zfs error: {}", stderr.lines().next().unwrap_or("unknown")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let last_line = stdout.lines().last().ok_or("no snapshots found")?;
    let timestamp: i64 = last_line
        .trim()
        .parse()
        .map_err(|e| format!("timestamp parse error: {}", e))?;

    let now = chrono::Utc::now().timestamp();
    Ok(now - timestamp)
}

fn format_age(secs: i64) -> String {
    if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}
