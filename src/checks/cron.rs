use chrono::{Datelike, NaiveDateTime, Timelike};

#[cfg(target_os = "linux")]
use chrono::Local;
#[cfg(target_os = "linux")]
use std::process::Command;

#[cfg(target_os = "linux")]
use super::{CheckResult, CheckStatus, Section};
#[cfg(not(target_os = "linux"))]
use super::CheckResult;
#[cfg(target_os = "linux")]
use crate::config::CronConfig;

/// A parsed cron job: schedule + command string + source file.
struct CronJob {
    schedule: CronSchedule,
    command: String,
    source: String,
}

/// Parsed five-field cron schedule. Each field is a sorted vec of valid values.
struct CronSchedule {
    minutes: Vec<u8>,  // 0-59
    hours: Vec<u8>,    // 0-23
    doms: Vec<u8>,     // 1-31
    months: Vec<u8>,   // 1-12
    dows: Vec<u8>,     // 0-6 (0=Sunday)
}

impl CronSchedule {
    /// Check if a given datetime matches this schedule.
    fn matches(&self, dt: &NaiveDateTime) -> bool {
        let minute = dt.minute() as u8;
        let hour = dt.hour() as u8;
        let dom = dt.day() as u8;
        let month = dt.month() as u8;
        let dow = dt.weekday().num_days_from_sunday() as u8;

        self.minutes.contains(&minute)
            && self.hours.contains(&hour)
            && self.doms.contains(&dom)
            && self.months.contains(&month)
            && self.dows.contains(&dow)
    }
}

/// Parse a single cron field (e.g., "*/5", "1,3,5", "1-10", "*").
fn parse_field(field: &str, min: u8, max: u8) -> Option<Vec<u8>> {
    let mut values = Vec::new();

    for part in field.split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("*/") {
            let step: u8 = rest.parse().ok()?;
            if step == 0 {
                return None;
            }
            let mut v = min;
            while v <= max {
                values.push(v);
                v = v.checked_add(step)?;
            }
        } else if part == "*" {
            values.extend(min..=max);
        } else if part.contains('-') {
            let parts: Vec<&str> = part.splitn(2, '-').collect();
            let start: u8 = parts[0].parse().ok()?;
            let end: u8 = parts[1].parse().ok()?;
            if start > max || end > max || start > end {
                return None;
            }
            values.extend(start..=end);
        } else {
            let v: u8 = part.parse().ok()?;
            if v < min || v > max {
                return None;
            }
            values.push(v);
        }
    }

    values.sort_unstable();
    values.dedup();
    Some(values)
}

/// Map common cron shorthands to their five-field equivalents.
fn expand_shorthand(s: &str) -> Option<&'static str> {
    match s {
        "@yearly" | "@annually" => Some("0 0 1 1 *"),
        "@monthly" => Some("0 0 1 * *"),
        "@weekly" => Some("0 0 * * 0"),
        "@daily" | "@midnight" => Some("0 0 * * *"),
        "@hourly" => Some("0 * * * *"),
        _ => None,
    }
}

/// Parse a five-field cron schedule string.
fn parse_schedule(spec: &str) -> Option<CronSchedule> {
    let fields: Vec<&str> = spec.split_whitespace().collect();
    if fields.len() < 5 {
        return None;
    }

    Some(CronSchedule {
        minutes: parse_field(fields[0], 0, 59)?,
        hours: parse_field(fields[1], 0, 23)?,
        doms: parse_field(fields[2], 1, 31)?,
        months: parse_field(fields[3], 1, 12)?,
        dows: parse_field(fields[4], 0, 6)?,
    })
}

/// Parse a crontab line into a CronJob. Handles standard 5-field and shorthand formats.
/// Returns None for comments, blank lines, env assignments, and @reboot.
fn parse_crontab_line(line: &str, is_system: bool, source: &str) -> Option<CronJob> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    // Skip environment variable assignments (KEY=value)
    if trimmed.contains('=') && !trimmed.starts_with('@') && !trimmed.starts_with('*') {
        // Heuristic: if the first token contains '=' it's probably an env var
        if let Some(first) = trimmed.split_whitespace().next()
            && first.contains('=')
        {
            return None;
        }
    }
    // Skip @reboot — no periodic schedule
    if trimmed.starts_with("@reboot") {
        return None;
    }

    // Handle shorthand (@daily, @hourly, etc.)
    if trimmed.starts_with('@') {
        let parts: Vec<&str> = trimmed.splitn(2, |c: char| c.is_whitespace()).collect();
        let expanded = expand_shorthand(parts[0])?;
        let command = parts.get(1).unwrap_or(&"").trim().to_string();
        if command.is_empty() {
            return None;
        }
        let schedule = parse_schedule(expanded)?;
        let source = source.to_string();
        return Some(CronJob { schedule, command, source });
    }

    // Standard 5-field format
    // For system crontabs there's a user field after the 5 schedule fields
    let min_fields = if is_system { 7 } else { 6 };
    let parts: Vec<&str> = trimmed.splitn(min_fields, |c: char| c.is_whitespace()).collect();
    if parts.len() < min_fields {
        return None;
    }

    let spec = format!("{} {} {} {} {}", parts[0], parts[1], parts[2], parts[3], parts[4]);
    let schedule = parse_schedule(&spec)?;

    // Last element contains the command (everything after schedule + optional user field)
    let command = parts[min_fields - 1].trim().to_string();
    if command.is_empty() {
        return None;
    }

    let source = source.to_string();
    Some(CronJob { schedule, command, source })
}

/// Collect jobs from the current user's crontab.
#[cfg(target_os = "linux")]
fn collect_user_crontab() -> Vec<CronJob> {
    let output = match Command::new("crontab").arg("-l").output() {
        Ok(o) => o,
        Err(_) => return vec![],
    };

    // Exit code 1 with "no crontab for" means no crontab — not an error
    if !output.status.success() {
        return vec![];
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| parse_crontab_line(line, false, "user crontab"))
        .collect()
}

/// Collect jobs from system crontab files.
#[cfg(target_os = "linux")]
fn collect_system_crontabs() -> Vec<CronJob> {
    let mut jobs = Vec::new();

    // /etc/crontab (system format with user field)
    if let Ok(contents) = std::fs::read_to_string("/etc/crontab") {
        for line in contents.lines() {
            if let Some(job) = parse_crontab_line(line, true, "/etc/crontab") {
                jobs.push(job);
            }
        }
    }

    // /etc/cron.d/* (system format)
    if let Ok(entries) = std::fs::read_dir("/etc/cron.d") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let source = path.to_string_lossy().to_string();
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    for line in contents.lines() {
                        if let Some(job) = parse_crontab_line(line, true, &source) {
                            jobs.push(job);
                        }
                    }
                }
            }
        }
    }

    jobs
}

/// Extract a short label from a command for display purposes.
fn command_label(cmd: &str) -> String {
    // Strip shell redirections, env vars, and take the meaningful part
    let cleaned = cmd
        .split("&&")
        .next()
        .unwrap_or(cmd)
        .split('|')
        .next()
        .unwrap_or(cmd)
        .trim();

    // Take the basename of the first path-like token
    let first_token = cleaned.split_whitespace().next().unwrap_or(cleaned);
    let label = first_token.rsplit('/').next().unwrap_or(first_token);

    if label.len() > 40 {
        format!("{}…", &label[..39])
    } else {
        label.to_string()
    }
}

/// Extract a search hint from the command to match against journal logs.
/// The cron daemon typically logs the command as-is or a truncated version.
#[cfg(target_os = "linux")]
fn journal_search_hint(cmd: &str) -> String {
    // Use the first meaningful path or command token
    let trimmed = cmd.trim();
    if trimmed.len() > 60 {
        trimmed[..60].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Check journald for evidence that a cron command ran within the lookback window.
#[cfg(target_os = "linux")]
fn check_journal(hint: &str, lookback_minutes: u64) -> bool {
    let since = format!("{} minutes ago", lookback_minutes);

    // Try journalctl first
    if let Ok(output) = Command::new("journalctl")
        .args(["-t", "CRON", "--since", &since, "--no-pager", "-q", "-o", "cat"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Cron logs typically contain CMD (command) entries
            // Match on a meaningful substring of the command
            let search = hint.split_whitespace().next().unwrap_or(hint);
            return stdout.lines().any(|line| line.contains(search));
        }
    }

    // Fallback: check /var/log/syslog
    if let Ok(contents) = std::fs::read_to_string("/var/log/syslog") {
        let search = hint.split_whitespace().next().unwrap_or(hint);
        // Simple heuristic: look for recent CRON lines containing our command
        // (without strict timestamp parsing — syslog format varies)
        return contents
            .lines()
            .rev() // Most recent entries last
            .take(10000) // Don't read the entire file
            .any(|line| line.contains("CRON") && line.contains(search));
    }

    // Cannot verify — assume ok rather than false alarm
    false
}

/// Determine if a cron schedule should have fired at least once in the lookback window.
/// Walks backward from `now` minute-by-minute.
#[cfg(target_os = "linux")]
fn should_have_run(schedule: &CronSchedule, lookback_minutes: u64) -> bool {
    let now = Local::now().naive_local();
    // Replace seconds with 0 for clean minute boundaries
    let now = now
        .date()
        .and_hms_opt(now.hour(), now.minute(), 0)
        .unwrap_or(now);

    for i in 0..lookback_minutes {
        let check_time = now - chrono::Duration::minutes(i as i64);
        if schedule.matches(&check_time) {
            return true;
        }
    }

    false
}

/// Default lookback window: 25 hours (covers daily jobs with some slack).
#[cfg(target_os = "linux")]
const DEFAULT_LOOKBACK_MINUTES: u64 = 1500;

#[cfg(target_os = "linux")]
pub fn check_cron(config: &Option<CronConfig>) -> Vec<CheckResult> {
    let mut all_jobs = collect_user_crontab();
    all_jobs.extend(collect_system_crontabs());

    if all_jobs.is_empty() {
        return vec![];
    }

    let ignore = config.as_ref().map(|c| &c.ignore[..]).unwrap_or(&[]);

    // Filter out ignored commands
    let jobs: Vec<&CronJob> = all_jobs
        .iter()
        .filter(|j| !ignore.iter().any(|pat| j.command.contains(pat.as_str())))
        .collect();

    if jobs.is_empty() {
        return vec![];
    }

    let lookback = DEFAULT_LOOKBACK_MINUTES;
    let mut results = Vec::new();
    let mut ok_count = 0u32;

    for job in &jobs {
        if !should_have_run(&job.schedule, lookback) {
            ok_count += 1;
            continue;
        }

        let hint = journal_search_hint(&job.command);
        if check_journal(&hint, lookback) {
            ok_count += 1;
        } else {
            let label = command_label(&job.command);
            results.push(CheckResult {
                section: Section::Cron,
                name: "cron".into(),
                status: CheckStatus::Warning,
                summary: format!("{} — no evidence of run ({})", label, job.source),
            });
        }
    }

    if ok_count > 0 || results.is_empty() {
        results.push(CheckResult {
            section: Section::Cron,
            name: "cron".into(),
            status: CheckStatus::Ok,
            summary: format!(
                "{} job{} ok",
                ok_count,
                if ok_count == 1 { "" } else { "s" }
            ),
        });
    }

    results
}

#[cfg(not(target_os = "linux"))]
pub fn check_cron(_config: &Option<crate::config::CronConfig>) -> Vec<CheckResult> {
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_field_star() {
        let vals = parse_field("*", 0, 59).unwrap();
        assert_eq!(vals.len(), 60);
        assert_eq!(vals[0], 0);
        assert_eq!(vals[59], 59);
    }

    #[test]
    fn test_parse_field_step() {
        let vals = parse_field("*/15", 0, 59).unwrap();
        assert_eq!(vals, vec![0, 15, 30, 45]);
    }

    #[test]
    fn test_parse_field_range() {
        let vals = parse_field("1-5", 0, 59).unwrap();
        assert_eq!(vals, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_parse_field_list() {
        let vals = parse_field("1,3,5", 0, 59).unwrap();
        assert_eq!(vals, vec![1, 3, 5]);
    }

    #[test]
    fn test_parse_field_single() {
        let vals = parse_field("30", 0, 59).unwrap();
        assert_eq!(vals, vec![30]);
    }

    #[test]
    fn test_parse_field_invalid() {
        assert!(parse_field("60", 0, 59).is_none());
        assert!(parse_field("*/0", 0, 59).is_none());
        assert!(parse_field("abc", 0, 59).is_none());
    }

    #[test]
    fn test_parse_schedule() {
        let sched = parse_schedule("0 2 * * *").unwrap();
        assert_eq!(sched.minutes, vec![0]);
        assert_eq!(sched.hours, vec![2]);
        assert_eq!(sched.doms.len(), 31);
        assert_eq!(sched.months.len(), 12);
        assert_eq!(sched.dows.len(), 7);
    }

    #[test]
    fn test_parse_schedule_complex() {
        let sched = parse_schedule("*/5 9-17 * * 1-5").unwrap();
        assert_eq!(sched.minutes, vec![0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55]);
        assert_eq!(sched.hours, vec![9, 10, 11, 12, 13, 14, 15, 16, 17]);
        assert_eq!(sched.dows, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_parse_crontab_line_standard() {
        let job = parse_crontab_line("0 2 * * * /usr/local/bin/backup.sh", false, "test").unwrap();
        assert_eq!(job.command, "/usr/local/bin/backup.sh");
        assert_eq!(job.schedule.minutes, vec![0]);
        assert_eq!(job.schedule.hours, vec![2]);
    }

    #[test]
    fn test_parse_crontab_line_shorthand() {
        let job = parse_crontab_line("@daily /usr/local/bin/cleanup.sh", false, "test").unwrap();
        assert_eq!(job.command, "/usr/local/bin/cleanup.sh");
        assert_eq!(job.schedule.minutes, vec![0]);
        assert_eq!(job.schedule.hours, vec![0]);
    }

    #[test]
    fn test_parse_crontab_line_system_format() {
        let job =
            parse_crontab_line("0 2 * * * root /usr/local/bin/backup.sh", true, "test").unwrap();
        assert!(job.command.contains("/usr/local/bin/backup.sh"));
    }

    #[test]
    fn test_parse_crontab_line_skip_comment() {
        assert!(parse_crontab_line("# this is a comment", false, "test").is_none());
    }

    #[test]
    fn test_parse_crontab_line_skip_empty() {
        assert!(parse_crontab_line("", false, "test").is_none());
        assert!(parse_crontab_line("   ", false, "test").is_none());
    }

    #[test]
    fn test_parse_crontab_line_skip_env_var() {
        assert!(parse_crontab_line("SHELL=/bin/bash", false, "test").is_none());
        assert!(parse_crontab_line("PATH=/usr/bin:/bin", false, "test").is_none());
    }

    #[test]
    fn test_parse_crontab_line_skip_reboot() {
        assert!(parse_crontab_line("@reboot /usr/local/bin/startup.sh", false, "test").is_none());
    }

    #[test]
    fn test_expand_shorthand() {
        assert_eq!(expand_shorthand("@daily"), Some("0 0 * * *"));
        assert_eq!(expand_shorthand("@hourly"), Some("0 * * * *"));
        assert_eq!(expand_shorthand("@weekly"), Some("0 0 * * 0"));
        assert_eq!(expand_shorthand("@monthly"), Some("0 0 1 * *"));
        assert_eq!(expand_shorthand("@yearly"), Some("0 0 1 1 *"));
        assert_eq!(expand_shorthand("@annually"), Some("0 0 1 1 *"));
        assert_eq!(expand_shorthand("@reboot"), None);
        assert_eq!(expand_shorthand("invalid"), None);
    }

    #[test]
    fn test_command_label() {
        assert_eq!(command_label("/usr/local/bin/backup.sh"), "backup.sh");
        assert_eq!(
            command_label("/usr/local/bin/backup.sh && touch /tmp/ok"),
            "backup.sh"
        );
        assert_eq!(
            command_label("/usr/local/bin/backup.sh | logger"),
            "backup.sh"
        );
        assert_eq!(command_label("backup.sh"), "backup.sh");
    }

    #[test]
    fn test_schedule_matches() {
        // 0 2 * * * — should match 02:00 on any day
        let sched = parse_schedule("0 2 * * *").unwrap();
        let dt = NaiveDateTime::parse_from_str("2026-03-13 02:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert!(sched.matches(&dt));

        let dt2 = NaiveDateTime::parse_from_str("2026-03-13 03:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert!(!sched.matches(&dt2));
    }

    #[test]
    fn test_daily_schedule_matches_within_window() {
        // A daily job at 02:00 should match at least one minute in a 25h window
        let sched = parse_schedule("0 2 * * *").unwrap();
        // Check that 02:00 on any recent day matches
        let dt = NaiveDateTime::parse_from_str("2026-03-13 02:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert!(sched.matches(&dt));
        // And 02:01 does not
        let dt2 = NaiveDateTime::parse_from_str("2026-03-13 02:01:00", "%Y-%m-%d %H:%M:%S").unwrap();
        assert!(!sched.matches(&dt2));
    }
}
