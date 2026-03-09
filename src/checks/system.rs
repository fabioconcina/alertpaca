use std::thread;
use std::time::Duration;

use sysinfo::{Disks, System};

use super::{CheckResult, CheckStatus, Section};
use crate::config::Config;
use crate::state::DiskHistory;

pub fn check_system(_config: &Config) -> Vec<CheckResult> {
    let mut results = Vec::new();
    let mut sys = System::new();

    // CPU — need two refreshes with a delay for accurate measurement
    sys.refresh_cpu_usage();
    thread::sleep(Duration::from_millis(200));
    sys.refresh_cpu_usage();

    let cpu_usage = sys.global_cpu_usage();
    let cpu_count = sys.cpus().len();
    let cpu_status = if cpu_usage > 95.0 {
        CheckStatus::Critical
    } else if cpu_usage > 80.0 {
        CheckStatus::Warning
    } else {
        CheckStatus::Ok
    };
    results.push(CheckResult {
        section: Section::System,
        name: "cpu".into(),
        status: cpu_status,
        summary: format!("{:.0}% usage ({} cores)", cpu_usage, cpu_count),
    });

    // Memory
    sys.refresh_memory();
    let total_mem = sys.total_memory();
    let used_mem = sys.used_memory();
    let mem_pct = if total_mem > 0 {
        (used_mem as f64 / total_mem as f64) * 100.0
    } else {
        0.0
    };
    let mem_status = if mem_pct > 95.0 {
        CheckStatus::Critical
    } else if mem_pct > 80.0 {
        CheckStatus::Warning
    } else {
        CheckStatus::Ok
    };
    results.push(CheckResult {
        section: Section::System,
        name: "memory".into(),
        status: mem_status,
        summary: format!(
            "{} / {} ({:.0}%)",
            format_bytes(used_mem),
            format_bytes(total_mem),
            mem_pct
        ),
    });

    // Swap
    let total_swap = sys.total_swap();
    let used_swap = sys.used_swap();
    if total_swap > 0 {
        let swap_pct = (used_swap as f64 / total_swap as f64) * 100.0;
        let swap_status = if swap_pct > 90.0 {
            CheckStatus::Critical
        } else if swap_pct > 50.0 {
            CheckStatus::Warning
        } else {
            CheckStatus::Ok
        };
        results.push(CheckResult {
            section: Section::System,
            name: "swap".into(),
            status: swap_status,
            summary: format!(
                "{} / {} ({:.0}%)",
                format_bytes(used_swap),
                format_bytes(total_swap),
                swap_pct
            ),
        });
    }

    // Load average
    let load = System::load_average();
    let load_1 = load.one;
    let load_status = if cpu_count > 0 {
        if load_1 > (cpu_count * 2) as f64 {
            CheckStatus::Critical
        } else if load_1 > cpu_count as f64 {
            CheckStatus::Warning
        } else {
            CheckStatus::Ok
        }
    } else {
        CheckStatus::Ok
    };
    results.push(CheckResult {
        section: Section::System,
        name: "load".into(),
        status: load_status,
        summary: format!("{:.2} ({} cores)", load_1, cpu_count),
    });

    // Uptime
    let uptime_secs = System::uptime();
    results.push(CheckResult {
        section: Section::System,
        name: "uptime".into(),
        status: CheckStatus::Ok,
        summary: format_uptime(uptime_secs),
    });

    // Disks
    let now = chrono::Utc::now().timestamp();
    let mut history = DiskHistory::load();
    let disks = Disks::new_with_refreshed_list();

    for disk in disks.list() {
        let mount = disk.mount_point().to_string_lossy().to_string();

        // Skip pseudo-filesystems
        let fs_type = disk.file_system().to_string_lossy().to_string();
        if is_pseudo_fs(&fs_type, &mount) {
            continue;
        }

        let total = disk.total_space();
        let available = disk.available_space();
        if total == 0 {
            continue;
        }
        let used = total.saturating_sub(available);
        let pct = (used as f64 / total as f64) * 100.0;

        let disk_status = if pct > 90.0 {
            CheckStatus::Critical
        } else if pct > 80.0 {
            CheckStatus::Warning
        } else {
            CheckStatus::Ok
        };

        // Record for fill prediction
        history.record(&mount, now, used);
        let prediction = history.predict_days_until_full(&mount, total);

        let mut summary = format!("{:.0}% used ({} / {})", pct, format_bytes(used), format_bytes(total));
        if let Some(days) = prediction
            && days < 365.0
        {
            summary.push_str(&format!(" — ~{} until full", format_days(days)));
        }

        results.push(CheckResult {
            section: Section::System,
            name: format!("disk {}", mount),
            status: disk_status,
            summary,
        });
    }

    // Save disk history (best-effort)
    let _ = history.save();

    results
}

fn is_pseudo_fs(fs_type: &str, mount: &str) -> bool {
    let pseudo_types = [
        "tmpfs", "devtmpfs", "sysfs", "proc", "devpts", "cgroup", "cgroup2",
        "pstore", "debugfs", "securityfs", "configfs", "fusectl", "mqueue",
        "hugetlbfs", "binfmt_misc", "autofs", "efivarfs", "tracefs",
        "bpf", "nsfs", "overlay",
    ];
    if pseudo_types.contains(&fs_type) {
        return true;
    }
    let pseudo_mounts = ["/dev", "/sys", "/proc", "/run"];
    if pseudo_mounts.iter().any(|&m| mount.starts_with(m) && mount != "/run/media") {
        return true;
    }
    false
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if bytes >= TB {
        format!("{:.1}T", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1}G", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0}M", bytes as f64 / MB as f64)
    } else {
        format!("{:.0}K", bytes as f64 / KB as f64)
    }
}

fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;

    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

fn format_days(days: f64) -> String {
    if days < 1.0 {
        let hours = (days * 24.0).round() as u64;
        format!("{}h", hours)
    } else if days < 30.0 {
        format!("{}d", days.round() as u64)
    } else if days < 365.0 {
        let months = (days / 30.0).round() as u64;
        format!("{}mo", months)
    } else {
        let years = (days / 365.0).round() as u64;
        format!("{}y", years)
    }
}
