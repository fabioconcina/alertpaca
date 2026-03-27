pub mod backups;
pub mod certificates;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod cron;
pub mod dns;
pub mod endpoints;
pub mod ntp;
pub mod ports;
pub mod services;
pub mod system;
pub mod updates;

use serde::Serialize;

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum CheckStatus {
    Ok,
    Warning,
    Critical,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Section {
    System,
    Services,
    Backups,
    Certificates,
    Ports,
    Ntp,
    Endpoints,
    Dns,
    Updates,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Cron,
}

impl Section {
    pub fn label(&self) -> &'static str {
        match self {
            Section::System => "SYSTEM",
            Section::Services => "SERVICES",
            Section::Backups => "BACKUPS",
            Section::Certificates => "CERTIFICATES",
            Section::Ports => "PORTS",
            Section::Ntp => "NTP",
            Section::Endpoints => "ENDPOINTS",
            Section::Dns => "DNS",
            Section::Updates => "UPDATES",
            Section::Cron => "CRON",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub section: Section,
    pub name: String,
    pub status: CheckStatus,
    pub summary: String,
    /// Minimum status level that triggers a push notification.
    /// Defaults to Warning (i.e. notify on Warning and Critical).
    #[serde(skip)]
    pub notify_minimum: CheckStatus,
}

impl Default for CheckResult {
    fn default() -> Self {
        Self {
            section: Section::System,
            name: String::new(),
            status: CheckStatus::Ok,
            summary: String::new(),
            notify_minimum: CheckStatus::Warning,
        }
    }
}

pub fn run_all_checks(config: &Config) -> Vec<CheckResult> {
    let mut results = Vec::new();

    results.extend(system::check_system(config));
    results.extend(services::check_services(&config.systemd));
    results.extend(backups::check_backups(&config.backup));
    results.extend(certificates::check_certificates(&config.certificate));
    results.extend(ports::check_ports());
    results.extend(ntp::check_ntp(&config.ntp));
    results.extend(endpoints::check_endpoints(&config.endpoint));
    results.extend(dns::check_dns(&config.dns));
    results.extend(updates::check_updates());
    results.extend(cron::check_cron(&config.cron));

    results
}
