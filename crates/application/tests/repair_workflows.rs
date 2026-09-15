use std::time::SystemTime;

use dji4g_application::{
    ActionKind, ConfirmError, ControlledRepairRequest, OperationState, PreparedActionState,
};
use dji4g_at_protocol::{Apn, PdpContextId, VerifiedUsbNetProfile};
use dji4g_domain::{DnsProfile, ErrorCode, OperationOutcome};
use std::net::{IpAddr, Ipv4Addr};

const NOW: SystemTime = SystemTime::UNIX_EPOCH;

fn apn(value: &str) -> Apn {
    Apn::try_from(value).expect("fixture APN")
}

#[test]
fn only_the_modeled_controlled_requests_can_cross_the_repair_boundary() {
    let requests = [
        ControlledRepairRequest::RefreshDhcp,
        ControlledRepairRequest::ApplyDnsProfile {
            profile: DnsProfile::Automatic,
        },
        ControlledRepairRequest::RestartAdapter,
        ControlledRepairRequest::ReenumerateDevice,
        ControlledRepairRequest::SetUsbNetProfile {
            profile: VerifiedUsbNetProfile::DjiNdis,
        },
        ControlledRepairRequest::SetApn {
            cid: PdpContextId::try_from(1).expect("fixture CID"),
            apn: apn("new.example"),
        },
        ControlledRepairRequest::RestartModule,
        ControlledRepairRequest::ToggleHotspot { enabled: true },
    ];

    for request in requests {
        let action = request.clone().into_action();
        assert!(ControlledRepairRequest::try_from_action(action).is_ok());
    }
    assert!(ControlledRepairRequest::try_from_action(ActionKind::Refresh).is_err());
}

#[test]
fn modeled_dns_profile_is_validated_and_round_trips_without_widening_the_action() {
    let profile = DnsProfile::Static {
        servers: vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))],
    };
    let request = ControlledRepairRequest::ApplyDnsProfile {
        profile: profile.clone(),
    };
    assert_eq!(
        request.clone().into_action(),
        ActionKind::ApplyDnsProfile { profile }
    );
    assert_eq!(
        ControlledRepairRequest::try_from_action(request.into_action()),
        Ok(ControlledRepairRequest::ApplyDnsProfile {
            profile: DnsProfile::Static {
                servers: vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))],
            },
        })
    );
    assert!(
        ControlledRepairRequest::try_from_action(ActionKind::ApplyDnsProfile {
            profile: DnsProfile::Static {
                servers: Vec::new()
            },
        })
        .is_err()
    );
}

#[test]
fn typed_apn_request_is_redacted_and_can_be_confirmed_once() {
    let request = ControlledRepairRequest::SetApn {
        cid: PdpContextId::try_from(1).expect("fixture CID"),
        apn: apn("secret.example"),
    };
    assert!(!format!("{request:?}").contains("secret.example"));

    let mut controller = dji4g_application::Controller::for_test(NOW);
    let id = controller.prepare_repair(request).expect("prepare");
    assert!(matches!(
        controller.snapshot().prepared_action.unwrap().state,
        PreparedActionState::AwaitingConfirmation
    ));
    controller.confirm_action(id).expect("confirm");
    assert_eq!(controller.executor_call_count(), 1);
}

#[test]
fn stale_confirmation_never_reaches_the_executor() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let id = controller
        .prepare_repair(ControlledRepairRequest::RestartAdapter)
        .expect("prepare");
    controller.accept_evidence_change();
    assert!(matches!(
        controller.confirm_action(id),
        Err(ConfirmError::Invalidated(_))
    ));
    assert_eq!(controller.executor_call_count(), 0);
}

#[test]
fn uac_cancel_and_transport_timeout_are_distinct_terminal_outcomes() {
    let mut cancelled = dji4g_application::Controller::for_test(NOW);
    cancelled.executor_returns(Err(dji4g_application::PortError::new(
        ErrorCode::OperationCancelled,
        "operation:uac_cancelled",
    )));
    let id = cancelled
        .prepare_repair(ControlledRepairRequest::RestartAdapter)
        .expect("prepare");
    cancelled.confirm_action(id).expect("confirm");
    assert!(matches!(
        cancelled.snapshot().operation.unwrap().state,
        OperationState::Finished {
            outcome: OperationOutcome::Failed {
                code: ErrorCode::OperationCancelled,
                ..
            },
            ..
        }
    ));

    let mut timed_out = dji4g_application::Controller::for_test(NOW);
    timed_out.executor_returns(Ok(dji4g_application::ExecutionReceipt {
        outcome: dji4g_application::ExecutionReceiptOutcome::OutcomeUnknown {
            code: ErrorCode::Timeout,
        },
        after_state_hash: None,
    }));
    let id = timed_out
        .prepare_repair(ControlledRepairRequest::RestartModule)
        .expect("prepare");
    timed_out.confirm_action(id).expect("confirm");
    assert!(matches!(
        timed_out.snapshot().operation.unwrap().state,
        OperationState::Finished {
            outcome: OperationOutcome::OutcomeUnknown {
                code: ErrorCode::Timeout
            },
            ..
        }
    ));
}

#[test]
fn typed_conversion_rejects_invalid_apn_before_a_plan_exists() {
    let action = ActionKind::EditApn {
        cid: 1,
        apn: "bad,apn".to_owned(),
    };
    assert!(ControlledRepairRequest::try_from_action(action).is_err());

    let mut controller = dji4g_application::Controller::for_test(NOW);
    assert!(
        controller
            .prepare_repair(ControlledRepairRequest::SetApn {
                cid: PdpContextId::try_from(1).expect("fixture CID"),
                apn: apn("safe.example"),
            })
            .is_ok()
    );
}
