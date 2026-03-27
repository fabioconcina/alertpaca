use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("alertpaca")
}

fn ensure_data_dir() -> Result<PathBuf> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Atomic write: write to .tmp then rename.
fn atomic_write(path: &Path, contents: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

// --- Disk history ---

#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct DiskHistory {
    /// mount_point -> vec of (unix_timestamp, used_bytes)
    pub(crate) disks: HashMap<String, Vec<(i64, u64)>>,
}

impl DiskHistory {
    pub(crate) fn load() -> Self {
        let path = data_dir().join("history.json");
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub(crate) fn save(&self) -> Result<()> {
        let dir = ensure_data_dir()?;
        let path = dir.join("history.json");
        let json = serde_json::to_string_pretty(self)?;
        atomic_write(&path, &json)
    }

    /// Record a data point and prune old entries (keep last 168 = 1 week at hourly).
    pub(crate) fn record(&mut self, mount: &str, timestamp: i64, used_bytes: u64) {
        let entries = self.disks.entry(mount.to_string()).or_default();
        entries.push((timestamp, used_bytes));
        // Keep last 168 entries
        if entries.len() > 168 {
            let drain = entries.len() - 168;
            entries.drain(..drain);
        }
    }

    /// Predict days until full using linear regression.
    /// Returns Some(days) if trending up, None otherwise.
    pub(crate) fn predict_days_until_full(&self, mount: &str, total_bytes: u64) -> Option<f64> {
        let entries = self.disks.get(mount)?;
        if entries.len() < 2 {
            return None;
        }

        let first = entries.first()?;
        let last = entries.last()?;

        let dt = (last.0 - first.0) as f64;
        if dt <= 0.0 {
            return None;
        }

        let du = last.1 as f64 - first.1 as f64;
        if du <= 0.0 {
            // Not trending up
            return None;
        }

        let bytes_per_sec = du / dt;
        let remaining = total_bytes as f64 - last.1 as f64;
        if remaining <= 0.0 {
            return Some(0.0);
        }

        let secs = remaining / bytes_per_sec;
        Some(secs / 86400.0)
    }
}

// --- Port state ---

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub(crate) struct Listener {
    pub(crate) addr: String,
    pub(crate) port: u16,
    pub(crate) process: String,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct PortState {
    pub(crate) listeners: Vec<Listener>,
    pub(crate) timestamp: i64,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl PortState {
    pub(crate) fn load() -> Option<Self> {
        let path = data_dir().join("ports.json");
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    pub(crate) fn save(&self) -> Result<()> {
        let dir = ensure_data_dir()?;
        let path = dir.join("ports.json");
        let json = serde_json::to_string_pretty(self)?;
        atomic_write(&path, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disk_prediction_trending_up() {
        let mut history = DiskHistory::default();
        // 1 hour apart, 1GB growth, 10GB total
        history.record("/", 0, 5_000_000_000);
        history.record("/", 3600, 6_000_000_000);

        let days = history.predict_days_until_full("/", 10_000_000_000);
        assert!(days.is_some());
        let d = days.unwrap();
        // 4GB remaining at 1GB/hour = 4 hours = 0.167 days
        assert!((d - 0.167).abs() < 0.01);
    }

    #[test]
    fn test_disk_prediction_trending_down() {
        let mut history = DiskHistory::default();
        history.record("/", 0, 6_000_000_000);
        history.record("/", 3600, 5_000_000_000);

        let days = history.predict_days_until_full("/", 10_000_000_000);
        assert!(days.is_none()); // Not trending up
    }
}
