use dji4g_application::{
    AtObservation, BackendEvent, CheckResult, DeviceEpoch, DiagnosticCheckId, DiagnosticCheckState,
    ReducerState, RefreshCycleId, UnexecutedReason, reduce_state,
};
use dji4g_domain::{
    AtControlAvailability, AttachState, Availability, BoundDnsStatus, BoundPublicStatus,
    CellularSnapshot, FeatureStatus, RegistrationState, SimState, UnavailableReason,
};
use std::time::SystemTime;

const NOW: SystemTime = SystemTime::UNIX_EPOCH;

#[test]
fn settings_event_disables_probe_and_late_success_cannot_restore_green() {
    let state = ReducerState::test_ready(NOW);
    let mut settings = state.snapshot().settings.clone();
    settings.active_probe = false;
    let state = reduce_state(&state, BackendEvent::SettingsChanged { settings }, NOW);
    let state = reduce_state(
        &state,
        BackendEvent::ProbeFinished {
            cycle: RefreshCycleId(1),
            epoch: DeviceEpoch(1),
            result: CheckResult::Passed {
                observed_at: NOW,
                value: dji4g_application::ProbeObservationDto {
                    route_choices: Vec::new(),
                    epoch: DeviceEpoch(1),
                    adapter_id: "{adapter}".into(),
                    gateway: dji4g_application::ProbeStageDto::Passed,
                    public: dji4g_application::ProbeStageDto::Passed,
                    dns: dji4g_application::ProbeStageDto::Passed,
                    protocol_coverage: Some(dji4g_domain::ProtocolCoverage::AllRequiredFamilies),
                    system_route: None,
                },
            },
        },
        NOW,
    );
    assert_ne!(state.snapshot().app.availability, Availability::Available);
    assert_eq!(
        state.snapshot().app.network.as_ref().unwrap().bound_public,
        BoundPublicStatus::Incomplete
    );
    assert_eq!(
        state
            .snapshot()
            .diagnostics
            .iter()
            .find(|check| check.id == DiagnosticCheckId::BoundPublic)
            .unwrap()
            .state,
        DiagnosticCheckState::Unexecuted {
            reason: UnexecutedReason::DisabledBySetting
        }
    );
}

#[test]
fn cellular_success_requires_sim_registration_and_attachment_and_recovers_after_missing() {
    let ready = || {
        let mut value = cellular(SimState::Ready);
        value.registration = RegistrationState::RegisteredHome;
        value.attached = AttachState::Attached;
        value
    };
    let mut state = observe(
        &ReducerState::test_ready(NOW),
        Some(cellular(SimState::Missing)),
    );
    state = observe(&state, Some(ready()));
    assert_eq!(state.snapshot().app.availability, Availability::Available);
    assert_eq!(
        state
            .snapshot()
            .diagnostics
            .iter()
            .find(|c| c.id == DiagnosticCheckId::Cellular)
            .unwrap()
            .state,
        DiagnosticCheckState::Passed
    );
    for observation in [
        None,
        Some(cellular(SimState::Unknown)),
        Some(cellular(SimState::Ready)),
    ] {
        let unavailable = observe(&state, observation);
        assert_ne!(
            unavailable
                .snapshot()
                .diagnostics
                .iter()
                .find(|c| c.id == DiagnosticCheckId::Cellular)
                .unwrap()
                .state,
            DiagnosticCheckState::Passed
        );
    }
    for sim in [
        SimState::PinRequired,
        SimState::PukRequired,
        SimState::Rejected,
    ] {
        let blocked = observe(&state, Some(cellular(sim)));
        assert_eq!(
            blocked.snapshot().app.availability,
            Availability::Unavailable(UnavailableReason::CellularRejected)
        );
    }
    let mut denied = ready();
    denied.registration = RegistrationState::Denied;
    assert_eq!(
        observe(&state, Some(denied)).snapshot().app.availability,
        Availability::Unavailable(UnavailableReason::CellularRejected)
    );
}

fn cellular(sim: SimState) -> CellularSnapshot {
    CellularSnapshot {
        sim,
        registration: RegistrationState::NotRegistered,
        attached: AttachState::Detached,
        carrier: None,
        radio_access_technology: None,
        signal_rssi_dbm: None,
        apn: None,
        pdp_address: None,
        pdp_state: None,
        firmware: None,
        serving_cell: None,
        sim_identity: None,
        numbers: None,
        temperature_celsius: None,
        temperature_status: FeatureStatus::NotProbed,
    }
}

fn observe(state: &ReducerState, cellular: Option<CellularSnapshot>) -> ReducerState {
    reduce_state(
        state,
        BackendEvent::AtFinished {
            cycle: RefreshCycleId(1),
            epoch: DeviceEpoch(1),
            result: CheckResult::Passed {
                value: AtObservation {
                    availability: AtControlAvailability::Available,
                    cellular,
                },
                observed_at: NOW,
            },
        },
        NOW,
    )
}

#[test]
fn working_at_transport_does_not_claim_missing_sim_is_ready() {
    let state = observe(
        &ReducerState::test_ready(NOW),
        Some(cellular(SimState::Missing)),
    );
    let snapshot = state.snapshot();
    assert_eq!(
        snapshot
            .diagnostics
            .iter()
            .find(|c| c.id == DiagnosticCheckId::AtControl)
            .unwrap()
            .state,
        DiagnosticCheckState::Passed
    );
    assert!(matches!(
        snapshot
            .diagnostics
            .iter()
            .find(|c| c.id == DiagnosticCheckId::Cellular)
            .unwrap()
            .state,
        DiagnosticCheckState::Failed { .. } | DiagnosticCheckState::Unavailable { .. }
    ));
    assert_eq!(
        snapshot.app.availability,
        Availability::Unavailable(UnavailableReason::CellularRejected)
    );
}

#[test]
fn disabling_active_probe_immediately_retracts_previous_success() {
    let mut state = ReducerState::test_ready(NOW);
    assert_eq!(state.snapshot().app.availability, Availability::Available);
    state.set_active_probe(false);
    let snapshot = state.snapshot();
    assert_ne!(snapshot.app.availability, Availability::Available);
    assert!(!snapshot.settings.active_probe);
    for id in [
        DiagnosticCheckId::BoundGateway,
        DiagnosticCheckId::BoundPublic,
        DiagnosticCheckId::BoundDns,
    ] {
        assert_eq!(
            snapshot
                .diagnostics
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .state,
            DiagnosticCheckState::Unexecuted {
                reason: UnexecutedReason::DisabledBySetting
            }
        );
    }
    let network = snapshot.app.network.as_ref().unwrap();
    assert_eq!(network.bound_public, BoundPublicStatus::Incomplete);
    assert_eq!(network.bound_dns, BoundDnsStatus::Incomplete);
    state.set_active_probe(true);
    assert_ne!(state.snapshot().app.availability, Availability::Available);
}
