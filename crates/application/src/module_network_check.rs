use crate::{
    AdapterObservationDto, AdapterStateDto, BackendEvent, CheckResult, DiagnosticCheckId as Id,
    DiagnosticCheckState, DiagnosticSet, ProbeObservationDto, RefreshCycleId,
};
use dji4g_domain::{
    DeviceEpoch, ModuleNetworkEvidence, ModuleNetworkVerdict, NetworkEvidenceState as S,
    classify_module_network,
};
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModuleNetworkCheckPhase {
    Queued,
    Running,
    Finished,
    Stale,
}

/// One explicit check only. Periodic refreshes cannot replace this report.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleNetworkCheckSnapshot {
    pub request_id: u64,
    pub epoch: DeviceEpoch,
    pub phase: ModuleNetworkCheckPhase,
    pub started_at: Option<SystemTime>,
    pub finished_at: Option<SystemTime>,
    pub allow_probe_once: bool,
    pub diagnostics: DiagnosticSet,
    pub evidence: ModuleNetworkEvidence,
    pub verdict: ModuleNetworkVerdict,
    pub adapter: Option<AdapterObservationDto>,
    pub probe: Option<ProbeObservationDto>,
    pub after_operation: Option<u64>,
    pub operation_outcome: Option<dji4g_domain::OperationOutcome>,
    pub dhcp_attempted: bool,
    pub(crate) cycle: Option<RefreshCycleId>,
}

impl ModuleNetworkCheckSnapshot {
    pub fn queued(request_id: u64, epoch: DeviceEpoch, allow_probe_once: bool) -> Self {
        Self {
            request_id,
            epoch,
            phase: ModuleNetworkCheckPhase::Queued,
            started_at: None,
            finished_at: None,
            allow_probe_once,
            diagnostics: DiagnosticSet::new(epoch),
            evidence: ModuleNetworkEvidence::default(),
            verdict: ModuleNetworkVerdict::Inconclusive,
            adapter: None,
            probe: None,
            after_operation: None,
            operation_outcome: None,
            dhcp_attempted: false,
            cycle: None,
        }
    }
    #[must_use]
    pub fn active(&self) -> bool {
        matches!(
            self.phase,
            ModuleNetworkCheckPhase::Queued | ModuleNetworkCheckPhase::Running
        )
    }
    #[must_use]
    pub fn fresh(&self, epoch: DeviceEpoch, now: SystemTime) -> bool {
        self.phase == ModuleNetworkCheckPhase::Finished
            && self.epoch == epoch
            && self.finished_at.is_some_and(|at| {
                now.duration_since(at)
                    .is_ok_and(|age| age <= Duration::from_secs(30))
            })
    }
    pub(crate) fn invalidate(&mut self) {
        self.phase = ModuleNetworkCheckPhase::Stale;
        self.verdict = ModuleNetworkVerdict::Inconclusive;
    }
    pub(crate) fn observe(
        &mut self,
        event: &BackendEvent,
        diagnostics: &DiagnosticSet,
        epoch: DeviceEpoch,
        now: SystemTime,
    ) {
        if self.phase != ModuleNetworkCheckPhase::Running {
            return;
        }
        let event_cycle = match event {
            BackendEvent::RefreshStarted { cycle, .. }
            | BackendEvent::InventoryFinished { cycle, .. }
            | BackendEvent::AtFinished { cycle, .. }
            | BackendEvent::AdapterFinished { cycle, .. }
            | BackendEvent::ProbeFinished { cycle, .. }
            | BackendEvent::RefreshFinished { cycle, .. } => Some(*cycle),
            _ => None,
        };
        if event_cycle != self.cycle {
            return;
        }
        // A fresh inventory is the sole authority allowed to adopt the initial target epoch.
        if let BackendEvent::InventoryFinished {
            result: CheckResult::Passed { value, .. },
            ..
        } = event
        {
            if self.epoch != epoch && self.epoch != DeviceEpoch(0) {
                self.invalidate();
                return;
            }
            self.epoch = epoch;
            self.evidence.device = match value.presence {
                dji4g_domain::DevicePresence::Supported(_) => S::Passed,
                dji4g_domain::DevicePresence::NotDetected => S::Failed,
                _ => S::Unavailable,
            };
        }
        if self.epoch != epoch {
            self.invalidate();
            return;
        }
        self.diagnostics = diagnostics.clone();
        self.evidence.adapter = step_state(&diagnostics.get(Id::WindowsAdapter).state);
        if let BackendEvent::AdapterFinished {
            result: CheckResult::Passed { value, .. },
            ..
        } = event
        {
            if value.epoch != epoch {
                self.invalidate();
                return;
            }
            self.evidence.address_route = if value.state == AdapterStateDto::UsableAddressAndRoute {
                S::Passed
            } else {
                S::Failed
            };
            self.evidence.link = value.details.as_ref().map_or(S::Unavailable, |d| {
                if d.link_up { S::Passed } else { S::Failed }
            });
            self.adapter = Some(value.clone());
        }
        if let BackendEvent::ProbeFinished {
            result: CheckResult::Passed { value, .. },
            ..
        } = event
        {
            if value.epoch != epoch
                || self
                    .adapter
                    .as_ref()
                    .is_none_or(|a| a.binding.adapter_id != value.adapter_id)
            {
                self.invalidate();
                return;
            }
            self.probe = Some(value.clone());
        }
        self.evidence.gateway = step_state(&diagnostics.get(Id::BoundGateway).state);
        self.evidence.public = step_state(&diagnostics.get(Id::BoundPublic).state);
        self.evidence.dns = step_state(&diagnostics.get(Id::BoundDns).state);
        if matches!(event, BackendEvent::RefreshFinished { .. }) {
            self.phase = ModuleNetworkCheckPhase::Finished;
            self.finished_at = Some(now);
            self.verdict = classify_module_network(&self.evidence);
        }
    }
}

#[must_use]
pub fn step_state(state: &DiagnosticCheckState) -> S {
    match state {
        DiagnosticCheckState::Passed => S::Passed,
        DiagnosticCheckState::Failed { .. } => S::Failed,
        DiagnosticCheckState::Unavailable { .. } => S::Unavailable,
        DiagnosticCheckState::Running { .. } => S::Running,
        DiagnosticCheckState::Expired => S::Stale,
        DiagnosticCheckState::Unexecuted { .. } => S::NotRun,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkRepairKind {
    RenewDhcp,
    RestartAdapter,
    AutomaticDns,
}
impl NetworkRepairKind {
    #[must_use]
    pub fn request(self) -> crate::ControlledRepairRequest {
        match self {
            Self::RenewDhcp => crate::ControlledRepairRequest::RefreshDhcp,
            Self::RestartAdapter => crate::ControlledRepairRequest::RestartAdapter,
            Self::AutomaticDns => crate::ControlledRepairRequest::ApplyDnsProfile {
                profile: dji4g_domain::DnsProfile::Automatic,
            },
        }
    }
    #[must_use]
    pub fn readiness_key(self) -> crate::ActionReadinessKey {
        match self {
            Self::RenewDhcp => crate::ActionReadinessKey::RenewDhcp,
            Self::RestartAdapter => crate::ActionReadinessKey::RestartAdapter,
            Self::AutomaticDns => crate::ActionReadinessKey::ApplyDnsProfile,
        }
    }
}
impl ModuleNetworkCheckSnapshot {
    /// Suggestions require positive configuration evidence, never a missing observation.
    #[must_use]
    pub fn recommended_repairs(&self) -> Vec<NetworkRepairKind> {
        if self.phase != ModuleNetworkCheckPhase::Finished {
            return Vec::new();
        }
        let Some(details) = self.adapter.as_ref().and_then(|a| a.details.as_ref()) else {
            return Vec::new();
        };
        match self.verdict {
            ModuleNetworkVerdict::AddressRouteIssue if details.link_up && details.dhcp_v4 => {
                vec![if self.dhcp_attempted {
                    NetworkRepairKind::RestartAdapter
                } else {
                    NetworkRepairKind::RenewDhcp
                }]
            }
            ModuleNetworkVerdict::DnsIssue if details.dns_automatic == Some(false) => {
                vec![NetworkRepairKind::AutomaticDns]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dji4g_domain::{AdapterBinding, StableDeviceIdentity};
    fn report() -> ModuleNetworkCheckSnapshot {
        let mut c = ModuleNetworkCheckSnapshot::queued(1, DeviceEpoch(1), false);
        c.phase = ModuleNetworkCheckPhase::Finished;
        c.verdict = ModuleNetworkVerdict::AddressRouteIssue;
        c.adapter = Some(AdapterObservationDto {
            details: Some(crate::AdapterNetworkDetails {
                link_up: true,
                dhcp_v4: true,
                dns_automatic: Some(false),
            }),
            epoch: DeviceEpoch(1),
            binding: AdapterBinding {
                target: StableDeviceIdentity {
                    container_id: "fixture".into(),
                    device_instance_id: "fixture".into(),
                    vid: 0x2ca3,
                    pid: 0x4006,
                },
                adapter_id: "{fixture}".into(),
            },
            state: AdapterStateDto::NoUsableAddressOrRoute,
            addresses: vec![],
            gateways: vec![],
            dns_servers: vec![],
            ipv4: false,
            ipv6: false,
            rx_bytes: None,
            tx_bytes: None,
        });
        c
    }
    #[test]
    fn suggestions_require_verified_dhcp_or_nonautomatic_dns() {
        let mut c = report();
        assert_eq!(c.recommended_repairs(), vec![NetworkRepairKind::RenewDhcp]);
        c.adapter
            .as_mut()
            .unwrap()
            .details
            .as_mut()
            .unwrap()
            .dhcp_v4 = false;
        assert!(c.recommended_repairs().is_empty());
        c.verdict = ModuleNetworkVerdict::DnsIssue;
        assert_eq!(
            c.recommended_repairs(),
            vec![NetworkRepairKind::AutomaticDns]
        );
        c.adapter
            .as_mut()
            .unwrap()
            .details
            .as_mut()
            .unwrap()
            .dns_automatic = Some(true);
        assert!(c.recommended_repairs().is_empty());
        c.adapter
            .as_mut()
            .unwrap()
            .details
            .as_mut()
            .unwrap()
            .dns_automatic = None;
        assert!(c.recommended_repairs().is_empty());
        c.adapter.as_mut().unwrap().details = None;
        assert!(c.recommended_repairs().is_empty());
    }
    #[test]
    fn a_different_cycle_cannot_finish_or_populate_the_report() {
        let mut c = report();
        c.phase = ModuleNetworkCheckPhase::Running;
        c.cycle = Some(RefreshCycleId(5));
        c.observe(
            &BackendEvent::RefreshFinished {
                cycle: RefreshCycleId(4),
                epoch: DeviceEpoch(1),
            },
            &DiagnosticSet::new(DeviceEpoch(1)),
            DeviceEpoch(1),
            SystemTime::UNIX_EPOCH,
        );
        assert_eq!(c.phase, ModuleNetworkCheckPhase::Running);
        assert!(c.finished_at.is_none());
    }
}
