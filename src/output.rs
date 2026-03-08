use std::io::{self, Write};

use crate::checks::{CheckResult, CheckStatus, Section};

/// Write results as pretty-printed JSON to stdout.
pub fn write_json(results: &[CheckResult]) -> io::Result<()> {
    let json = serde_json::to_string_pretty(results)
        .map_err(io::Error::other)?;
    let mut out = io::stdout().lock();
    writeln!(out, "{json}")
}

/// Write results as a plain-text table to stdout.
pub fn write_table(results: &[CheckResult]) -> io::Result<()> {
    let mut out = io::stdout().lock();
    let mut current_section: Option<Section> = None;

    for result in results {
        if current_section != Some(result.section) {
            if current_section.is_some() {
                writeln!(out)?;
            }
            writeln!(out, " {}", result.section.label())?;
            current_section = Some(result.section);
        }

        let icon = match result.status {
            CheckStatus::Ok => "✓",
            CheckStatus::Warning => "⚠",
            CheckStatus::Critical => "✗",
            CheckStatus::Skipped => "—",
        };

        writeln!(out, " {icon} {:<16} {}", result.name, result.summary)?;
    }

    Ok(())
}

/// Return the appropriate exit code for a set of results.
pub fn exit_code(results: &[CheckResult]) -> i32 {
    let has_issue = results
        .iter()
        .any(|r| matches!(r.status, CheckStatus::Warning | CheckStatus::Critical));
    if has_issue {
        crate::exitcode::HEALTH_ISSUE
    } else {
        crate::exitcode::SUCCESS
    }
}
