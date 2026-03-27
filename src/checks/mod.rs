pub(crate) mod backups;
pub(crate) mod certificates;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) mod cron;
pub(crate) mod dns;
pub(crate) mod endpoints;
pub(crate) mod ntp;
pub(crate) mod ports;
pub(crate) mod services;
pub(crate) mod system;
pub(crate) mod updates;

use std::sync::Arc;

use rustls::{ClientConfig, RootCertStore};
use serde::Serialize;

use crate::config::Config;

/// Shared TLS client config using system root certificates.
/// Used by certificate, endpoint, and notification checks.
pub(crate) fn tls_client_config() -> Arc<ClientConfig> {
    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    Arc::new(
        ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) enum CheckStatus {
    Ok,
    Warning,
    Critical,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum Section {
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
    pub(crate) fn label(&self) -> &'static str {
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
pub(crate) struct CheckResult {
    pub(crate) section: Section,
    pub(crate) name: String,
    pub(crate) status: CheckStatus,
    pub(crate) summary: String,
    /// Minimum status level that triggers a push notification.
    /// Defaults to Warning (i.e. notify on Warning and Critical).
    #[serde(skip)]
    pub(crate) notify_minimum: CheckStatus,
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

pub(crate) fn run_all_checks(config: &Config) -> Vec<CheckResult> {
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
