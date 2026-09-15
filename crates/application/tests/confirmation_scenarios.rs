use std::time::{Duration, SystemTime};

use dji4g_application::{
    ActionKindTag, ActionRequest, ConfirmationInvalidationReason, DeviceEpoch,
    EpochInvalidationReason, FailureCode, LanguageCode, PLAN_LIFETIME, PortError,
    PreparedActionState, StableCode, UiCommand,
};
use dji4g_domain::ErrorCode;

const NOW: SystemTime = SystemTime::UNIX_EPOCH;

#[test]
fn prepare_only_publishes_redacted_summary_and_does_not_bump_evidence_revision() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let before = controller.snapshot();
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();
    let after = controller.snapshot();

    assert_eq!(after.app.revision, before.app.revision);
    let prepared = after.prepared_action.expect("prepared action");
    assert_eq!(prepared.id, id);
    assert_eq!(prepared.action, ActionKindTag::RestartModule);
    assert!(!format!("{prepared:?}").contains("container"));
    assert!(!format!("{prepared:?}").contains("hash"));
}

#[test]
fn prepared_action_does_not_self_invalidate_when_only_publication_changes() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();
    let publication_before = controller.snapshot().publication_revision;
    controller.set_language(LanguageCode::EnUs);
    let snapshot = controller.snapshot();
    assert!(snapshot.prepared_action.as_ref().is_some_and(
        |plan| plan.id == id && matches!(plan.state, PreparedActionState::AwaitingConfirmation)
    ));
    assert!(snapshot.publication_revision > publication_before);
    controller.confirm_action(id).unwrap();
    assert_eq!(controller.executor_call_count(), 1);
}

#[test]
fn expiry_invalidates_plan_without_calling_executor() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();
    controller.advance_time(Duration::from_secs(31));
    let stale = controller.snapshot();
    assert_eq!(stale.app.freshness, dji4g_domain::Freshness::Stale);
    assert_eq!(
        stale.app.availability,
        dji4g_domain::Availability::Detecting
    );
    let result = controller.confirm_action(id);
    assert_eq!(
        result,
        Err(dji4g_application::ConfirmError::Invalidated(
            ConfirmationInvalidationReason::Expired
        ))
    );
    assert_eq!(controller.executor_call_count(), 0);
}

#[test]
fn evidence_revision_change_invalidates_plan_before_execution() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();
    controller.accept_evidence_change();
    let result = controller.confirm_action(id);
    assert_eq!(
        result,
        Err(dji4g_application::ConfirmError::Invalidated(
            ConfirmationInvalidationReason::SnapshotChanged
        ))
    );
    assert_eq!(controller.executor_call_count(), 0);
}

#[test]
fn duplicate_confirmation_consumes_plan_once() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();
    let first = controller.confirm_action(id).unwrap();
    let second = controller.confirm_action(id);
    assert_eq!(first, dji4g_application::ConfirmResult::Executed);
    assert_eq!(
        second,
        Err(dji4g_application::ConfirmError::AlreadyConsumed)
    );
    assert_eq!(controller.executor_call_count(), 1);
}

#[test]
fn uac_cancellation_is_failed_without_retry() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    controller.executor_returns(Err(PortError {
        code: FailureCode::new(
            ErrorCode::OperationCancelled,
            StableCode::try_from_static("operation:uac_cancelled").unwrap(),
        ),
        os_code: None,
    }));
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();
    controller.confirm_action(id).unwrap();
    assert_eq!(controller.executor_call_count(), 1);
    assert!(matches!(
        controller.snapshot().operation.unwrap().state,
        dji4g_application::OperationState::Finished {
            outcome: dji4g_domain::OperationOutcome::Failed {
                code: ErrorCode::OperationCancelled,
                ..
            },
            ..
        }
    ));
}

#[test]
fn removal_after_prepare_invalidates_and_does_not_execute() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();
    controller.invalidate_epoch(DeviceEpoch(2), EpochInvalidationReason::PhysicalRemoval);
    assert_eq!(
        controller.confirm_action(id),
        Err(dji4g_application::ConfirmError::Invalidated(
            ConfirmationInvalidationReason::DeviceRemoved
        ))
    );
    assert_eq!(controller.executor_call_count(), 0);
}

#[test]
fn toggle_hotspot_revalidation_cannot_change_module_availability() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let before = controller.snapshot().app.availability;
    let id = controller
        .prepare_action(ActionRequest::ToggleHotspot { enabled: true })
        .unwrap();
    controller.confirm_action(id).unwrap();
    assert_eq!(controller.snapshot().app.availability, before);
}

#[test]
fn prepare_rejects_missing_or_expired_prerequisites_without_a_plan() {
    let mut missing = dji4g_application::Controller::new(
        dji4g_application::ReducerState::new(NOW),
        std::sync::Arc::new(dji4g_application::FakeActionExecutor::new()),
        std::sync::Arc::new(dji4g_application::FakeClock::new(NOW)),
    );
    assert!(matches!(
        missing.prepare_action(ActionRequest::RestartModule),
        Err(dji4g_application::PrepareError::Prerequisite(_))
            | Err(dji4g_application::PrepareError::MissingTarget)
    ));
    assert!(missing.snapshot().prepared_action.is_none());

    let mut expired = dji4g_application::Controller::for_test(NOW);
    expired.advance_time(Duration::from_secs(31));
    assert!(matches!(
        expired.prepare_action(ActionRequest::RestartModule),
        Err(dji4g_application::PrepareError::Prerequisite(_))
    ));
    assert!(expired.snapshot().prepared_action.is_none());
}

#[test]
fn applied_without_readback_is_outcome_unknown() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();
    controller.confirm_action(id).unwrap();
    assert!(matches!(
        controller.snapshot().operation.unwrap().state,
        dji4g_application::OperationState::Finished {
            outcome: dji4g_domain::OperationOutcome::OutcomeUnknown {
                code: ErrorCode::VerificationFailed
            },
            ..
        }
    ));
}

/// The automatic monitoring cadence yields to a live confirmation, because every scan bumps
/// `evidence_revision` and would otherwise tear down the plan the user is about to confirm.  The
/// window must stay bounded by the plan's own lifetime so an abandoned plan can never stop
/// monitoring.
#[test]
fn interaction_in_flight_tracks_only_a_live_confirmation_window() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    assert!(
        !controller.interaction_in_flight(NOW),
        "an idle controller must not suppress monitoring"
    );

    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();
    assert!(
        controller.interaction_in_flight(NOW),
        "a plan awaiting confirmation suppresses the automatic cadence"
    );
    assert!(
        controller.interaction_in_flight(NOW + PLAN_LIFETIME),
        "still inside the plan lifetime"
    );
    assert!(
        !controller.interaction_in_flight(NOW + PLAN_LIFETIME + Duration::from_secs(1)),
        "an abandoned plan must never suppress monitoring forever"
    );

    controller.cancel_action(id).unwrap();
    assert!(
        !controller.interaction_in_flight(NOW),
        "a cancelled plan must not suppress monitoring"
    );
}

#[test]
fn interaction_in_flight_ends_when_the_operation_finishes() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();
    controller.confirm_action(id).unwrap();
    assert_eq!(controller.executor_call_count(), 1);
    assert!(
        !controller.interaction_in_flight(NOW),
        "a finished operation only awaits dismissal and must not suppress monitoring"
    );
}

#[test]
fn cancel_removes_the_plan_so_the_modal_can_close() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();
    controller.cancel_action(id).unwrap();
    assert!(
        controller.snapshot().prepared_action.is_none(),
        "cancelling must remove the plan outright so the confirmation modal closes"
    );
    // A second cancel is a no-op (the plan is already gone), never a stuck state.
    assert!(controller.cancel_action(id).is_err());
    assert!(controller.snapshot().prepared_action.is_none());
}

/// The runner path (`begin_confirmation`) must never execute synchronously on the caller: the
/// operation starts in `Running`, the executor runs on its own thread, and polling collects the
/// terminal result.
#[test]
fn background_confirmation_publishes_running_then_finished() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    let id = controller
        .prepare_action(ActionRequest::RestartModule)
        .unwrap();

    controller.begin_confirmation(id).unwrap();
    assert_eq!(
        controller
            .snapshot()
            .operation
            .as_ref()
            .map(|operation| &operation.state),
        Some(&dji4g_application::OperationState::Running {
            phase: dji4g_application::OperationPhase::Executing,
        }),
        "the operation must be Running while the background executor works"
    );

    let mut deadline = 0;
    while controller.poll_operation_completion().is_none() {
        deadline += 1;
        assert!(
            deadline < 1000,
            "the fake executor must report promptly on its own thread"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(controller.executor_call_count(), 1);
    assert!(matches!(
        controller.snapshot().operation.unwrap().state,
        dji4g_application::OperationState::Finished { .. }
    ));
}

/// A confirm dispatched through `handle_command` (the runner's command path) starts a background
/// operation instead of blocking the caller; the runner's own poll then finishes it.
#[test]
fn handle_command_confirm_starts_a_pollable_background_operation() {
    let mut controller = dji4g_application::Controller::for_test(NOW);
    controller
        .handle_command(UiCommand::PrepareAction {
            request: ActionRequest::RestartAdapter,
        })
        .expect("prepare accepted");
    let id = controller.snapshot().prepared_action.unwrap().id;
    controller
        .handle_command(UiCommand::ConfirmAction { id })
        .expect("confirm accepted");

    assert!(
        matches!(
            controller
                .snapshot()
                .operation
                .as_ref()
                .map(|operation| &operation.state),
            Some(&dji4g_application::OperationState::Running { .. })
        ),
        "the confirm must return immediately with the operation running"
    );
    let mut deadline = 0;
    while controller.poll_operation_completion().is_none() {
        deadline += 1;
        assert!(deadline < 1000, "fake executor must finish promptly");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(matches!(
        controller.snapshot().operation.unwrap().state,
        dji4g_application::OperationState::Finished { .. }
    ));
}
