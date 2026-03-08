use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Default, Clone)]
pub struct Config {
    #[serde(default)]
    pub backup: Vec<BackupConfig>,
    #[serde(default)]
    pub certificate: Vec<CertificateConfig>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum BackupConfig {
    #[serde(rename = "file")]
    File {
        name: String,
        path: String,
        pattern: String,
        max_age: String,
    },
    #[serde(rename = "restic")]
    Restic {
        name: String,
        repo: String,
        max_age: String,
        #[serde(default)]
        password_file: Option<String>,
    },
    #[serde(rename = "zfs")]
    Zfs {
        name: String,
        dataset: String,
        max_age: String,
    },
}

impl BackupConfig {
    pub fn name(&self) -> &str {
        match self {
            BackupConfig::File { name, .. } => name,
            BackupConfig::Restic { name, .. } => name,
            BackupConfig::Zfs { name, .. } => name,
        }
    }

    pub fn max_age_str(&self) -> &str {
        match self {
            BackupConfig::File { max_age, .. } => max_age,
            BackupConfig::Restic { max_age, .. } => max_age,
            BackupConfig::Zfs { max_age, .. } => max_age,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CertificateConfig {
    pub endpoint: String,
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("alertpaca")
        .join("config.toml")
}

pub fn load_config(path: Option<&str>) -> Result<Config> {
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => config_path(),
    };

    if !path.exists() {
        return Ok(Config::default());
    }

    let contents = std::fs::read_to_string(&path)?;
    let config: Config = toml::from_str(&contents)?;
    Ok(config)
}

/// Parse a duration string like "24h", "7d", "1w", "30m" into seconds.
pub fn parse_duration_secs(s: &str) -> Result<i64> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("empty duration string");
    }

    let (num_str, suffix) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().map_err(|_| anyhow::anyhow!("invalid duration: {}", s))?;

    let multiplier = match suffix {
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        _ => anyhow::bail!("unknown duration suffix: {}", suffix),
    };

    Ok(num * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration_secs("24h").unwrap(), 86400);
        assert_eq!(parse_duration_secs("7d").unwrap(), 604800);
        assert_eq!(parse_duration_secs("1w").unwrap(), 604800);
        assert_eq!(parse_duration_secs("30m").unwrap(), 1800);
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration_secs("").is_err());
        assert!(parse_duration_secs("abc").is_err());
        assert!(parse_duration_secs("24x").is_err());
    }
}
