//! Read-only computer-network evidence, independent of the DJI device epoch.

use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpFamily {
    V4,
    V6,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostAdapter {
    pub guid: String,
    pub luid: u64,
    pub alias: String,
    pub up: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostDefaultRoute {
    pub family: IpFamily,
    pub luid: u64,
    pub metric: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostProxyMode {
    Disabled,
    Manual,
    AutoConfig,
    AutoDetect,
    Mixed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyClient {
    ClashVergeRev,
    Other,
    None,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyBinding {
    pub client: ProxyClient,
    pub version: Option<String>,
    pub interface_alias: String,
    /// True only when a supported client has one unambiguous durable source.
    pub repairable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostNetworkObservation {
    pub adapters: Vec<HostAdapter>,
    pub default_routes: Vec<HostDefaultRoute>,
    pub system_proxy: HostProxyMode,
    pub binding: Option<ProxyBinding>,
    pub proxy_inspection_complete: bool,
    pub proxy_error_code: Option<String>,
    pub observed_at: SystemTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostNetworkFinding {
    NoKnownProblem,
    MissingBoundInterface,
    BoundInterfaceDown,
    BindingAmbiguous,
    EvidenceIncomplete,
}

/// A missing interface is certain only after a complete native inventory.  An unplugged DJI
/// module is irrelevant to this classifier: users can diagnose their computer without one.
#[must_use]
pub fn classify_host_network(observation: &HostNetworkObservation) -> HostNetworkFinding {
    let Some(binding) = &observation.binding else {
        return if observation.proxy_inspection_complete {
            HostNetworkFinding::NoKnownProblem
        } else {
            HostNetworkFinding::EvidenceIncomplete
        };
    };
    if binding.interface_alias.trim().is_empty() {
        return HostNetworkFinding::BindingAmbiguous;
    }
    let matching = observation
        .adapters
        .iter()
        .filter(|adapter| adapter.alias.eq_ignore_ascii_case(&binding.interface_alias))
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [] => HostNetworkFinding::MissingBoundInterface,
        [adapter] if !adapter.up => HostNetworkFinding::BoundInterfaceDown,
        [_] => HostNetworkFinding::NoKnownProblem,
        _ => HostNetworkFinding::BindingAmbiguous,
    }
}

#[must_use]
pub fn host_observation_is_fresh(observed_at: SystemTime, now: SystemTime) -> bool {
    now.duration_since(observed_at)
        .is_ok_and(|age| age <= Duration::from_secs(30))
}
