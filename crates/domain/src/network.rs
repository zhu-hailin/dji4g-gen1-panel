use serde::{Deserialize, Serialize};

use crate::StableDeviceIdentity;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterBinding {
    pub target: StableDeviceIdentity,
    pub adapter_id: String,
}

impl AdapterBinding {
    #[must_use]
    pub fn is_valid_for(&self, target: &StableDeviceIdentity) -> bool {
        !self.adapter_id.trim().is_empty() && self.target == *target && target.is_supported()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundEvidence<T> {
    pub binding: AdapterBinding,
    pub value: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AdapterState {
    UsableAddressAndRoute,
    NoUsableAddressOrRoute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BoundPublicStatus {
    Succeeded,
    Failed { consecutive_cycles: u8 },
    Incomplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BoundDnsStatus {
    Succeeded,
    Failed,
    Incomplete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProtocolCoverage {
    AllRequiredFamilies,
    SingleFamilyOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AtControlAvailability {
    Available,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DefaultRouteOwner {
    TargetAdapter,
    VpnOrTun,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum GlobalConnectivity {
    Online,
    Offline,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSnapshot {
    pub adapter_id: String,
    pub addresses: Vec<String>,
    pub gateways: Vec<String>,
    pub dns_servers: Vec<String>,
    pub adapter_state: AdapterState,
    pub bound_public: BoundPublicStatus,
    pub bound_dns: BoundDnsStatus,
    pub protocol_coverage: ProtocolCoverage,
    pub system_default_route: DefaultRouteOwner,
    /// Measured interface throughput since the previous sample; None = no baseline yet.
    #[serde(default)]
    pub down_bytes_per_sec: Option<u64>,
    #[serde(default)]
    pub up_bytes_per_sec: Option<u64>,
}
