pub mod backups;
pub mod certificates;
pub mod ports;
pub mod services;
pub mod system;

use serde::Serialize;

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
}

impl Section {
    pub fn label(&self) -> &'static str {
        match self {
            Section::System => "SYSTEM",
            Section::Services => "SERVICES",
            Section::Backups => "BACKUPS",
            Section::Certificates => "CERTIFICATES",
            Section::Ports => "PORTS",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub section: Section,
    pub name: String,
    pub status: CheckStatus,
    pub summary: String,
}

pub fn run_all_checks(config: &Config) -> Vec<CheckResult> {
    let mut results = Vec::new();

    results.extend(system::check_system(config));
    results.extend(services::check_services());
    results.extend(backups::check_backups(&config.backup));
    results.extend(certificates::check_certificates(&config.certificate));
    results.extend(ports::check_ports());

    results
}
