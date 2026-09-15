use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::{
    Availability, CellularSnapshot, DeviceSnapshot, ErrorCode, HotspotStatus, NetworkSnapshot,
    OperationSnapshot,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Freshness {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum IssueSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum IssueLayer {
    Device,
    Cellular,
    Network,
    BoundProbe,
    Hotspot,
    Operation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issue {
    pub code: ErrorCode,
    pub severity: IssueSeverity,
    pub layer: IssueLayer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSnapshot {
    pub revision: u64,
    pub observed_at: SystemTime,
    pub freshness: Freshness,
    pub availability: Availability,
    pub hotspot: HotspotStatus,
    pub device: Option<DeviceSnapshot>,
    pub cellular: Option<CellularSnapshot>,
    pub network: Option<NetworkSnapshot>,
    pub active_operation: Option<OperationSnapshot>,
    pub issues: Vec<Issue>,
}
