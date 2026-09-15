use dji4g_at_protocol::{Apn, PdpContextId, VerifiedUsbNetProfile};
use dji4g_domain::{
    AfterStateHash, DeviceEpoch, DnsProfile, ErrorCode, OperationOutcome, RollbackOutcome,
};
use dji4g_windows_platform::{
    AdapterProof, DispatchResult, FakeRepairBackend, HotspotProof, RepairAction, RepairBackend,
    RepairError, RepairObservation, TargetProof, WindowsRepairExecutor,
};
use std::net::{IpAddr, Ipv4Addr};

fn apn(value: &str) -> Apn {
    Apn::try_from(value).expect("fixture APN")
}

fn ready_backend() -> FakeRepairBackend {
    FakeRepairBackend::ready(RepairObservation::fixture(
        DeviceEpoch(4),
        11,
        TargetProof::dji_gen1([0x11; 32]),
        AdapterProof::fixture([0x22; 32]),
    ))
}

#[test]
fn refresh_dhcp_revalidates_and_applies_with_one_native_mutation() {
    let executor = WindowsRepairExecutor::new(ready_backend());
    let plan = executor
        .prepare(RepairAction::RefreshDhcp)
        .expect("prepare");

    let result = executor.execute(&plan);

    assert!(matches!(
        result.outcome(),
        OperationOutcome::Applied {
            after_state_hash: AfterStateHash(_)
        }
    ));
    assert_eq!(executor.backend().mutation_count(), 1);
}

#[test]
fn stale_before_state_fails_closed_without_dispatch() {
    let mut backend = ready_backend();
    let mut executor = WindowsRepairExecutor::new(backend.clone());
    let plan = executor
        .prepare(RepairAction::RestartAdapter)
        .expect("prepare");

    // The before-state hash covers only action-relevant fields; an unrelated observation change
    // (here a bumped revision) must NOT fail a wanted repair...
    backend.mutate_observation(|observation| observation.revision += 1);
    executor.replace_backend(backend);
    let unrelated = executor.execute(&plan);
    assert!(
        matches!(unrelated.outcome(), OperationOutcome::Applied { .. }),
        "unrelated state churn must not fail an action-relevant revalidation"
    );
    assert_eq!(executor.backend().mutation_count(), 1);

    // ...while a change to the adapter the action restarts must fail closed.
    let mut changed = ready_backend();
    let mut executor = WindowsRepairExecutor::new(changed.clone());
    let plan = executor
        .prepare(RepairAction::RestartAdapter)
        .expect("prepare");
    changed.mutate_observation(|observation| observation.adapter_up = false);
    executor.replace_backend(changed);
    let result = executor.execute(&plan);

    assert!(matches!(
        result.outcome(),
        OperationOutcome::Failed {
            code: ErrorCode::EvidenceExpired,
            ..
        }
    ));
    assert_eq!(executor.backend().mutation_count(), 0);
}

#[test]
fn timeout_or_removal_is_outcome_unknown_and_never_retried() {
    let mut backend = ready_backend();
    backend.set_dispatch_result(DispatchResult::Unknown(ErrorCode::Timeout));
    let executor = WindowsRepairExecutor::new(backend);
    let plan = executor
        .prepare(RepairAction::RestartModule)
        .expect("prepare");

    let result = executor.execute(&plan);

    assert!(matches!(
        result.outcome(),
        OperationOutcome::OutcomeUnknown {
            code: ErrorCode::Timeout
        }
    ));
    assert_eq!(executor.backend().mutation_count(), 1);
}

#[test]
fn typed_writes_and_apn_inactive_guard_are_enforced() {
    let mut backend = ready_backend();
    backend.set_contexts(&[(1, "IP", "old.example", false)]);
    let executor = WindowsRepairExecutor::new(backend);
    let cid = PdpContextId::try_from(1).unwrap();
    let plan = executor
        .prepare(RepairAction::SetApn {
            cid,
            apn: apn("new.example"),
        })
        .expect("inactive existing CID");
    let result = executor.execute(&plan);
    assert!(matches!(result.outcome(), OperationOutcome::Applied { .. }));
    assert_eq!(executor.backend().last_typed_write(), Some("SetApn"));

    let mut active_backend = ready_backend();
    active_backend.set_contexts(&[(1, "IP", "old.example", true)]);
    let active_executor = WindowsRepairExecutor::new(active_backend);
    assert!(matches!(
        active_executor.prepare(RepairAction::SetApn {
            cid,
            apn: apn("new.example")
        }),
        Err(RepairError::PdpContextActive)
    ));
}

#[test]
fn apn_write_rejects_ipv6_and_dual_stack_contexts_before_dispatch() {
    for pdp_type in ["IPV6", "IPV4V6"] {
        let mut backend = ready_backend();
        backend.set_contexts(&[(1, pdp_type, "old.example", false)]);
        let executor = WindowsRepairExecutor::new(backend);
        let cid = PdpContextId::try_from(1).unwrap();
        assert!(matches!(
            executor.prepare(RepairAction::SetApn {
                cid,
                apn: apn("new.example")
            }),
            Err(RepairError::PdpContextIncomplete)
        ));
        assert_eq!(executor.backend().mutation_count(), 0);
    }
}

#[test]
fn usb_profile_is_closed_to_zero_or_one_and_readback_mismatch_fails() {
    let mut backend = ready_backend();
    backend.set_usb_profile(Some(VerifiedUsbNetProfile::DjiNdis));
    backend.set_after_usb_profile(Some(VerifiedUsbNetProfile::DjiNdis));
    let executor = WindowsRepairExecutor::new(backend);
    let plan = executor
        .prepare(RepairAction::SetUsbNetProfile {
            profile: VerifiedUsbNetProfile::Ecm,
        })
        .expect("prepare");
    let result = executor.execute(&plan);
    assert!(matches!(result.outcome(), OperationOutcome::Failed { .. }));
    assert_eq!(executor.backend().mutation_count(), 1);
}

#[test]
fn uac_cancel_is_failed_with_zero_writes() {
    let mut backend = ready_backend();
    backend.set_dispatch_result(DispatchResult::Rejected(ErrorCode::OperationCancelled));
    let executor = WindowsRepairExecutor::new(backend);
    let plan = executor
        .prepare(RepairAction::RestartAdapter)
        .expect("prepare");

    let result = executor.execute(&plan);

    assert!(matches!(
        result.outcome(),
        OperationOutcome::Failed {
            code: ErrorCode::OperationCancelled,
            ..
        }
    ));
    assert_eq!(executor.backend().mutation_count(), 0);
}

#[test]
fn target_drift_is_rejected_before_any_action_boundary_call() {
    let mut backend = ready_backend();
    let mut executor = WindowsRepairExecutor::new(backend.clone());
    let plan = executor
        .prepare(RepairAction::RefreshDhcp)
        .expect("prepare");
    backend.mutate_observation(|observation| {
        observation.target = TargetProof::dji_gen1([0x99; 32]);
    });
    executor.replace_backend(backend);

    let result = executor.execute(&plan);

    assert!(matches!(result.outcome(), OperationOutcome::Failed { .. }));
    assert_eq!(executor.backend().mutation_count(), 0);
}

#[test]
fn after_scan_identity_drift_never_reports_applied() {
    let mut backend = ready_backend();
    let executor = WindowsRepairExecutor::new(backend.clone());
    let plan = executor
        .prepare(RepairAction::RefreshDhcp)
        .expect("prepare");
    let mut after = backend.observe().expect("before observation");
    after.target = TargetProof::dji_gen1([0x9a; 32]);
    backend.set_after_observation(after);

    let result = executor.execute(&plan);

    assert!(matches!(
        result.outcome(),
        OperationOutcome::OutcomeUnknown {
            code: ErrorCode::DeviceIdentityChanged
        }
    ));
    assert_eq!(executor.backend().mutation_count(), 1);
}

#[test]
fn modeled_dns_change_requires_exact_readback_and_preserves_old_state_in_before_hash() {
    let mut backend = ready_backend();
    let requested = DnsProfile::Static {
        servers: vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))],
    };
    let executor = WindowsRepairExecutor::new(backend.clone());
    let plan = executor
        .prepare(RepairAction::ApplyDnsProfile {
            profile: requested.clone(),
        })
        .expect("automatic DNS state is a valid before state");
    let applied = executor.execute(&plan);
    assert!(matches!(
        applied.outcome(),
        OperationOutcome::Applied { .. }
    ));
    assert_eq!(executor.backend().mutation_count(), 1);

    backend = ready_backend();
    backend.set_after_dns_profile(Some(DnsProfile::Automatic));
    let mismatch_executor = WindowsRepairExecutor::new(backend);
    let plan = mismatch_executor
        .prepare(RepairAction::ApplyDnsProfile { profile: requested })
        .expect("prepare");
    let result = mismatch_executor.execute(&plan);
    assert!(matches!(result.outcome(), OperationOutcome::Failed { .. }));
    assert_eq!(mismatch_executor.backend().mutation_count(), 1);
}

#[test]
fn hotspot_toggle_requires_fresh_exact_source_profile_and_capability_proof() {
    let mut unsupported = ready_backend();
    unsupported.set_hotspot(None, false);
    let executor = WindowsRepairExecutor::new(unsupported);
    assert!(matches!(
        executor.prepare(RepairAction::ToggleHotspot { enabled: true }),
        Err(RepairError::HotspotUnavailable)
    ));
    assert_eq!(executor.backend().mutation_count(), 0);

    let mut backend = ready_backend();
    backend.set_hotspot(Some(HotspotProof::fixture([0x55; 32], [0x66; 32])), false);
    let executor = WindowsRepairExecutor::new(backend);
    let plan = executor
        .prepare(RepairAction::ToggleHotspot { enabled: true })
        .expect("fresh supported hotspot");
    let result = executor.execute(&plan);
    assert!(matches!(result.outcome(), OperationOutcome::Applied { .. }));
    assert_eq!(executor.backend().last_typed_write(), Some("ToggleHotspot"));
}

#[test]
fn physical_reenumeration_requires_one_exact_root_and_after_scan_proof() {
    let mut backend = ready_backend();
    backend.mutate_observation(|observation| {
        observation.reenumerated = false;
    });
    let executor = WindowsRepairExecutor::new(backend);
    let plan = executor
        .prepare(RepairAction::ReenumerateDevice)
        .expect("exact root is present");
    let result = executor.execute(&plan);
    assert!(matches!(result.outcome(), OperationOutcome::Applied { .. }));

    let mut ambiguous = ready_backend();
    ambiguous.mutate_observation(|observation| observation.target_count = 2);
    let executor = WindowsRepairExecutor::new(ambiguous);
    assert!(matches!(
        executor.prepare(RepairAction::ReenumerateDevice),
        Err(RepairError::TargetAmbiguous)
    ));
    assert_eq!(executor.backend().mutation_count(), 0);
}

#[test]
fn adapter_partial_failure_keeps_best_effort_rollback_evidence() {
    let mut backend = ready_backend();
    backend.set_dispatch_result(DispatchResult::Failed {
        code: ErrorCode::PermissionDenied,
        rollback: RollbackOutcome::Failed {
            code: ErrorCode::RollbackFailed,
        },
    });
    let executor = WindowsRepairExecutor::new(backend);
    let plan = executor
        .prepare(RepairAction::RestartAdapter)
        .expect("prepare");
    let result = executor.execute(&plan);
    assert!(matches!(
        result.outcome(),
        OperationOutcome::Failed {
            code: ErrorCode::PermissionDenied,
            rollback: RollbackOutcome::Failed {
                code: ErrorCode::RollbackFailed
            }
        }
    ));
}
