//! Pollable host-network task state.  The platform owns all file paths and secrets.

use std::time::SystemTime;

use dji4g_domain::{HostNetworkFinding, HostNetworkObservation};

use crate::{PortError, PortFuture};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HostNetworkPhase {
    #[default]
    NotChecked,
    Checking,
    Ready,
    Preparing,
    AwaitingConfirmation,
    Applying,
    AwaitingRestart,
    Restoring,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyRepairPreview {
    pub plan_id: u64,
    pub interface_alias: String,
    pub expires_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyRepairResult {
    pub backup_id: u64,
    pub changed: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HostNetworkSnapshot {
    pub phase: HostNetworkPhase,
    pub observation: Option<HostNetworkObservation>,
    pub finding: Option<HostNetworkFinding>,
    pub finding_id: Option<u64>,
    pub preview: Option<ProxyRepairPreview>,
    pub result: Option<ProxyRepairResult>,
    /// Stable, non-sensitive error code; never file contents or a subscription URL.
    pub error_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostNetworkRequest {
    Inspect,
    Prepare(u64),
    Apply(u64),
    Restore(u64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HostNetworkOutcome {
    Inspected(HostNetworkObservation),
    Prepared(ProxyRepairPreview),
    Applied(ProxyRepairResult),
    Restored,
}

pub trait HostNetworkPort: Send + Sync {
    fn inspect(&self) -> PortFuture<'_, Result<HostNetworkObservation, PortError>>;
    fn prepare_repair(
        &self,
        finding_id: u64,
    ) -> PortFuture<'_, Result<ProxyRepairPreview, PortError>>;
    fn apply_repair(&self, plan_id: u64) -> PortFuture<'_, Result<ProxyRepairResult, PortError>>;
    fn restore_repair(&self, backup_id: u64) -> PortFuture<'_, Result<(), PortError>>;
}
