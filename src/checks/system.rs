use std::thread;
use std::time::Duration;

use sysinfo::{Components, Disks, System};

use super::{CheckResult, CheckStatus, Section};
use crate::config::Config;
use crate::state::DiskHistory;

pub(crate) fn check_system(_config: &Config) -> Vec<CheckResult> {
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
        ..Default::default()
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
        ..Default::default()
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
            ..Default::default()
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
        ..Default::default()
    });

    // Uptime
    let uptime_secs = System::uptime();
    results.push(CheckResult {
        section: Section::System,
        name: "uptime".into(),
        status: CheckStatus::Ok,
        summary: format_uptime(uptime_secs),
        ..Default::default()
    });

    // Temperature sensors
    let components = Components::new_with_refreshed_list();
    for component in &components {
        let label = component.label().to_lowercase();
        // Only report CPU package/core temps (skip duplicates like individual cores)
        let name = if label.contains("package") || label.contains("tctl") || label.contains("tdie")
        {
            "cpu temp".to_string()
        } else if label.contains("core") || label.contains("cpu") {
            continue; // skip per-core — package covers overall CPU temp
        } else if label.contains("composite") || label.contains("nvme") {
            "disk temp".to_string()
        } else {
            continue; // skip unrecognized sensors
        };

        let Some(temp) = component.temperature() else {
            continue;
        };
        let critical = component
            .critical()
            .unwrap_or(if name.starts_with("cpu") { 95.0 } else { 70.0 });
        let warn = critical - 15.0;

        let status = if temp >= critical {
            CheckStatus::Critical
        } else if temp >= warn {
            CheckStatus::Warning
        } else {
            CheckStatus::Ok
        };

        results.push(CheckResult {
            section: Section::System,
            name,
            status,
            summary: format!("{:.0}°C", temp),
            ..Default::default()
        });
    }

    // Disks
    let now = chrono::Utc::now().timestamp();
    let mut history = DiskHistory::load();
    let disks = Disks::new_with_refreshed_list();

    for disk in disks.list() {
        let mount = disk.mount_point().to_string_lossy().to_string();

        // Skip pseudo-filesystems and boot partitions
        let fs_type = disk.file_system().to_string_lossy().to_string();
        if is_pseudo_fs(&fs_type, &mount) || is_boot_partition(&mount) {
            continue;
        }

        // Use statvfs directly: sysinfo's available_space() maps to f_bavail
        // (excludes root-reserved blocks), so total - available counts the
        // reserve as used. df reports used = (f_blocks - f_bfree) * f_frsize.
        let Some((total, used, available)) = statvfs_disk_usage(&mount) else {
            continue;
        };
        if total == 0 {
            continue;
        }
        let usable = used + available;
        let pct = if usable > 0 {
            (used as f64 / usable as f64) * 100.0
        } else {
            0.0
        };

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
            ..Default::default()
        });
    }

    // Save disk history (best-effort)
    let _ = history.save();

    results
}

/// (total, used, available) in bytes from POSIX statvfs.
/// Uses f_bfree (true free) for `used` so root-reserved blocks aren't counted as used,
/// and f_bavail for `available` so the percentage reflects what users can actually fill.
// Casts are needed for portability: statvfs fields are c_ulong (u32 on 32-bit, u64 on 64-bit).
#[allow(clippy::unnecessary_cast)]
fn statvfs_disk_usage(mount: &str) -> Option<(u64, u64, u64)> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    let path = CString::new(mount).ok()?;
    let mut s = MaybeUninit::<libc::statvfs>::uninit();
    let r = unsafe { libc::statvfs(path.as_ptr(), s.as_mut_ptr()) };
    if r != 0 {
        return None;
    }
    let s = unsafe { s.assume_init() };
    let frsize = s.f_frsize as u64;
    let blocks = s.f_blocks as u64;
    let bfree = s.f_bfree as u64;
    let bavail = s.f_bavail as u64;
    Some((
        blocks * frsize,
        blocks.saturating_sub(bfree) * frsize,
        bavail * frsize,
    ))
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

fn is_boot_partition(mount: &str) -> bool {
    mount == "/boot" || mount.starts_with("/boot/")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(2048), "2K");
        assert_eq!(format_bytes(1024 * 1024), "1M");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0G");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024 * 1024), "3.0T");
    }

    #[test]
    fn test_format_uptime() {
        assert_eq!(format_uptime(300), "5m");
        assert_eq!(format_uptime(3661), "1h 1m");
        assert_eq!(format_uptime(90061), "1d 1h");
    }

    #[test]
    fn test_format_days() {
        assert_eq!(format_days(0.5), "12h");
        assert_eq!(format_days(3.0), "3d");
        assert_eq!(format_days(60.0), "2mo");
        assert_eq!(format_days(400.0), "1y");
    }

    #[test]
    fn test_is_pseudo_fs() {
        assert!(is_pseudo_fs("tmpfs", "/tmp"));
        assert!(is_pseudo_fs("ext4", "/dev/shm"));
        assert!(is_pseudo_fs("proc", "/proc"));
        assert!(!is_pseudo_fs("ext4", "/"));
        assert!(!is_pseudo_fs("ext4", "/home"));
    }
}
