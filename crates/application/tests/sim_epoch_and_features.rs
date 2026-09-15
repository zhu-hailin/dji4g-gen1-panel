//! SIM-session epoch tracking, module feature capability statuses, and sensitive-Debug guards
//! (research document §4.3 SIM epoch, §8.1 FeatureStatus, §8.2 privacy).

use std::time::SystemTime;

use dji4g_application::{
    ActionRequest, AtObservation, BackendEvent, CheckResult, ConfirmError,
    ConfirmationInvalidationReason, Controller, DeviceEpoch, EpochInvalidationReason, FeatureKey,
    PreparedActionState, ReducerState, RefreshCycleId, reduce_state,
};
use dji4g_domain::{
    AtControlAvailability, AttachState, CellularSnapshot, FeatureStatus, NumberLookup, PhoneNumber,
    RegistrationState, SimIdentity, SimState,
};

const NOW: SystemTime = SystemTime::UNIX_EPOCH;

fn fp(value: u8) -> [u8; 8] {
    [value; 8]
}

fn identity(fingerprint: [u8; 8]) -> SimIdentity {
    SimIdentity {
        iccid_masked: "8986****0123".to_owned(),
        fingerprint,
    }
}

fn cellular(
    firmware: Option<&str>,
    sim_identity: Option<SimIdentity>,
    numbers: Option<NumberLookup>,
) -> CellularSnapshot {
    CellularSnapshot {
        sim: SimState::Ready,
        registration: RegistrationState::RegisteredHome,
        attached: AttachState::Attached,
        carrier: Some("test".to_owned()),
        radio_access_technology: Some("LTE".to_owned()),
        signal_rssi_dbm: Some(-70),
        apn: Some("cmnet".to_owned()),
        pdp_address: Some("10.0.0.2".to_owned()),
        pdp_state: Some("active".to_owned()),
        firmware: firmware.map(str::to_owned),
        serving_cell: None,
        sim_identity,
        numbers,
        temperature_celsius: None,
        temperature_status: FeatureStatus::NotProbed,
    }
}

fn at_observation(cellular: Option<CellularSnapshot>) -> AtObservation {
    AtObservation {
        availability: AtControlAvailability::Available,
        cellular,
    }
}

fn at_finished(cycle: u64, epoch: DeviceEpoch, observation: AtObservation) -> BackendEvent {
    BackendEvent::AtFinished {
        cycle: RefreshCycleId(cycle),
        epoch,
        result: CheckResult::Passed {
            value: observation,
            observed_at: NOW,
        },
    }
}

/// A card change proven by a fingerprint difference, with the USB device epoch unchanged, must
/// advance the SIM-session epoch.
#[test]
fn sim_epoch_advances_when_fingerprint_changes_within_same_device_epoch() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        at_finished(
            1,
            epoch,
            at_observation(Some(cellular(Some("EC200A"), Some(identity(fp(1))), None))),
        ),
        NOW,
    );
    assert_eq!(
        state.snapshot().sim_epoch,
        0,
        "first identity only anchors the fingerprint"
    );

    state = reduce_state(
        &state,
        at_finished(
            2,
            epoch,
            at_observation(Some(cellular(Some("EC200A"), Some(identity(fp(2))), None))),
        ),
        NOW,
    );
    assert_eq!(state.snapshot().sim_epoch, 1);
}

/// The same fingerprint across cycles must never advance the SIM-session epoch.
#[test]
fn sim_epoch_stays_when_fingerprint_is_unchanged() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        at_finished(
            1,
            epoch,
            at_observation(Some(cellular(Some("EC200A"), Some(identity(fp(1))), None))),
        ),
        NOW,
    );
    state = reduce_state(
        &state,
        at_finished(
            2,
            epoch,
            at_observation(Some(cellular(Some("EC200A"), Some(identity(fp(1))), None))),
        ),
        NOW,
    );
    assert_eq!(state.snapshot().sim_epoch, 0);
}

/// An unreadable ICCID (`sim_identity == None`) must never advance the epoch, and must keep the
/// previously anchored fingerprint so a later change is still detected (conservative §4.3
/// semantics: continuity cannot be confirmed, so old-card data is not assumed current).
#[test]
fn sim_epoch_stays_and_fingerprint_is_retained_when_identity_goes_unreadable() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        at_finished(
            1,
            epoch,
            at_observation(Some(cellular(Some("EC200A"), Some(identity(fp(1))), None))),
        ),
        NOW,
    );
    state = reduce_state(
        &state,
        at_finished(
            2,
            epoch,
            at_observation(Some(cellular(Some("EC200A"), None, None))),
        ),
        NOW,
    );
    assert_eq!(state.snapshot().sim_epoch, 0);

    // The retained fingerprint still detects the swap once the identity becomes readable again.
    state = reduce_state(
        &state,
        at_finished(
            3,
            epoch,
            at_observation(Some(cellular(Some("EC200A"), Some(identity(fp(2))), None))),
        ),
        NOW,
    );
    assert_eq!(state.snapshot().sim_epoch, 1);
}

/// A card swapped during a physical unplug/replug is still detected: the fingerprint survives the
/// device-epoch invalidation.
#[test]
fn sim_fingerprint_survives_device_epoch_invalidation() {
    let mut state = ReducerState::new(NOW);
    state = reduce_state(
        &state,
        at_finished(
            1,
            DeviceEpoch(1),
            at_observation(Some(cellular(Some("EC200A"), Some(identity(fp(1))), None))),
        ),
        NOW,
    );
    state = reduce_state(
        &state,
        BackendEvent::EpochInvalidated {
            next_epoch: DeviceEpoch(2),
            reason: EpochInvalidationReason::PhysicalRemoval,
        },
        NOW,
    );
    assert_eq!(state.snapshot().sim_epoch, 0);

    state = reduce_state(
        &state,
        at_finished(
            2,
            DeviceEpoch(2),
            at_observation(Some(cellular(Some("EC200A"), Some(identity(fp(2))), None))),
        ),
        NOW,
    );
    assert_eq!(state.snapshot().sim_epoch, 1);
}

/// A SIM change detected by the controller must invalidate a prepared confirmation plan, so the
/// user cannot confirm a repair against a card that is no longer the one the plan was prepared
/// for.
#[test]
fn sim_change_invalidates_a_prepared_plan() {
    let mut controller = Controller::for_test(NOW);
    let epoch = DeviceEpoch(1);
    controller.apply_backend_event(at_finished(
        1,
        epoch,
        at_observation(Some(cellular(Some("EC200A"), Some(identity(fp(1))), None))),
    ));
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();

    controller.apply_backend_event(at_finished(
        2,
        epoch,
        at_observation(Some(cellular(Some("EC200A"), Some(identity(fp(2))), None))),
    ));

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.sim_epoch, 1);
    assert!(snapshot.prepared_action.as_ref().is_some_and(|plan| {
        plan.id == id
            && matches!(
                plan.state,
                PreparedActionState::Invalidated {
                    reason: ConfirmationInvalidationReason::SnapshotChanged
                }
            )
    }));
    assert_eq!(
        controller.confirm_action(id),
        Err(ConfirmError::Invalidated(
            ConfirmationInvalidationReason::SnapshotChanged
        ))
    );
    assert_eq!(controller.executor_call_count(), 0);
}

/// A fresh feature verdict must start `NotProbed`, and an `Empty` result must be recorded as
/// `Empty` — never promoted to `UnsupportedConfirmed` by this layer.
#[test]
fn feature_verdicts_start_not_probed_and_empty_is_never_promoted() {
    let mut controller = Controller::for_test(NOW);
    controller.apply_backend_event(at_finished(
        1,
        DeviceEpoch(1),
        at_observation(Some(cellular(Some("EC200A"), None, None))),
    ));

    assert_eq!(
        controller.snapshot().feature(FeatureKey::Cnum),
        FeatureStatus::NotProbed
    );
    assert_eq!(
        controller.snapshot().feature(FeatureKey::Qccid),
        FeatureStatus::NotProbed
    );

    controller.set_feature_status(FeatureKey::Cnum, FeatureStatus::Empty);
    assert_eq!(
        controller.snapshot().feature(FeatureKey::Cnum),
        FeatureStatus::Empty
    );
    assert_eq!(
        controller.snapshot().feature(FeatureKey::Qccid),
        FeatureStatus::NotProbed
    );

    // Interpretable evidence may still be recorded as UnsupportedConfirmed by the collector.
    controller.set_feature_status(FeatureKey::Cnum, FeatureStatus::UnsupportedConfirmed);
    assert_eq!(
        controller.snapshot().feature(FeatureKey::Cnum),
        FeatureStatus::UnsupportedConfirmed
    );
}

/// A firmware string change must invalidate the module capability cache (§8.1): every cached
/// verdict returns to `NotProbed` until the collector probes again.
#[test]
fn firmware_change_clears_cached_feature_verdicts() {
    let mut controller = Controller::for_test(NOW);
    let epoch = DeviceEpoch(1);
    controller.apply_backend_event(at_finished(
        1,
        epoch,
        at_observation(Some(cellular(Some("EC200A.0.0.1"), None, None))),
    ));
    controller.set_feature_status(FeatureKey::Cnum, FeatureStatus::Supported);
    assert_eq!(
        controller.snapshot().feature(FeatureKey::Cnum),
        FeatureStatus::Supported
    );

    controller.apply_backend_event(at_finished(
        2,
        epoch,
        at_observation(Some(cellular(Some("EC200A.0.0.2"), None, None))),
    ));
    assert_eq!(
        controller.snapshot().feature(FeatureKey::Cnum),
        FeatureStatus::NotProbed
    );
    assert_eq!(
        controller.snapshot().feature(FeatureKey::Qccid),
        FeatureStatus::NotProbed
    );

    // A fresh verdict for the new firmware is recorded normally.
    controller.set_feature_status(FeatureKey::Cnum, FeatureStatus::Supported);
    assert_eq!(
        controller.snapshot().feature(FeatureKey::Cnum),
        FeatureStatus::Supported
    );
}

/// Device removal drops the anchored capability context entirely.
#[test]
fn device_epoch_invalidation_drops_cached_feature_verdicts() {
    let mut controller = Controller::for_test(NOW);
    controller.apply_backend_event(at_finished(
        1,
        DeviceEpoch(1),
        at_observation(Some(cellular(Some("EC200A"), None, None))),
    ));
    controller.set_feature_status(FeatureKey::Cnum, FeatureStatus::Supported);
    assert_eq!(
        controller.snapshot().feature(FeatureKey::Cnum),
        FeatureStatus::Supported
    );

    controller.apply_backend_event(BackendEvent::EpochInvalidated {
        next_epoch: DeviceEpoch(2),
        reason: EpochInvalidationReason::PhysicalRemoval,
    });
    assert_eq!(
        controller.snapshot().feature(FeatureKey::Cnum),
        FeatureStatus::NotProbed
    );
}

/// Sensitive debugging guard (§8.2): neither the snapshot nor the AT observation Debug output may
/// leak a plaintext phone number or a full ICCID.
#[test]
fn snapshot_and_at_observation_debug_never_leak_plaintext_numbers_or_full_iccid() {
    let mut controller = Controller::for_test(NOW);
    let numbers = NumberLookup::Reported(vec![PhoneNumber::new("+12025550123", 145)]);
    controller.apply_backend_event(at_finished(
        1,
        DeviceEpoch(1),
        at_observation(Some(cellular(
            Some("EC200A"),
            Some(SimIdentity {
                iccid_masked: "8986****0123".to_owned(),
                fingerprint: fp(9),
            }),
            Some(numbers),
        ))),
    ));

    let snapshot = controller.snapshot();
    let debug = format!("{snapshot:?}");
    assert!(
        !debug.contains("+12025550123"),
        "plaintext phone number leaked into Debug"
    );
    assert!(
        !debug.contains("8986012312345678901"),
        "a full ICCID leaked into Debug (only the masked form may travel)"
    );

    let observation = at_observation(Some(cellular(
        Some("EC200A"),
        None,
        Some(NumberLookup::Reported(vec![PhoneNumber::new(
            "+8613800138000",
            145,
        )])),
    )));
    let at_debug = format!("{observation:?}");
    assert!(
        !at_debug.contains("+8613800138000"),
        "plaintext phone number leaked into the AT observation Debug output"
    );
}
