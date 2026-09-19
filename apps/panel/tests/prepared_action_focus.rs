//! Confirmation travels through the system's own MessageBox (native dialog), and the box is
//! presented *immediately by the button click* — never by a later snapshot — so a prepare
//! started anywhere (the overview hotspot button, a repairs button) is confirmed in front of
//! the user without the panel ever changing pages.  These tests pin that behaviour with a
//! recording backend: the page stays put, the confirm/result boxes are presented exactly once,
//! and the user's answer flows back through the ordinary command sink after the controller
//! publishes the plan the click prepared.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

use dji4g_application::{
    ActionKindTag, ActionPlanId, ControllerSnapshot, DeviceEpoch, DiagnosticSet, DisruptionLevel,
    PreparedActionSnapshot, PreparedActionState, RiskLevel, UiCommand,
};
use dji4g_domain::{ActionKind, Availability, Freshness, HotspotStatus};
use dji4g_panel::app::{PanelApp, PanelCommandSink, PanelInputs};
use dji4g_panel::native_dialog::NativeDialogBackend;
use eframe::egui;

const NOW: SystemTime = SystemTime::UNIX_EPOCH;

struct RecordingSink {
    commands: std::sync::Mutex<Vec<dji4g_application::UiCommand>>,
    dropped: AtomicUsize,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            commands: std::sync::Mutex::new(Vec::new()),
            dropped: AtomicUsize::new(0),
        }
    }

    fn commands(&self) -> Vec<dji4g_application::UiCommand> {
        self.commands.lock().expect("sink command lock").clone()
    }
}

impl dji4g_panel::app::UiCommandSink for RecordingSink {
    fn try_send(
        &self,
        command: dji4g_application::UiCommand,
    ) -> Result<(), dji4g_application::UiSendError> {
        self.commands
            .lock()
            .expect("sink command lock")
            .push(command);
        Ok(())
    }
}

impl Drop for RecordingSink {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

/// Recording dialog backend: never shows a real box, records the requests, and answers the
/// confirm box with the configured verdict.  The owner parameter (the panel `HWND`) is ignored.
#[derive(Clone)]
struct FakeDialog {
    calls: Arc<Mutex<Vec<String>>>,
    answer: Arc<AtomicBool>,
}

impl FakeDialog {
    fn new(answer: bool) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            answer: Arc::new(AtomicBool::new(answer)),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("dialog lock").clone()
    }

    /// Wait (bounded) for the dialog worker to record a call.
    fn wait_for_calls(&self) -> Vec<String> {
        let mut deadline = 0;
        loop {
            let calls = self.calls();
            if !calls.is_empty() {
                return calls;
            }
            deadline += 1;
            assert!(deadline < 1000, "dialog worker must report promptly");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl NativeDialogBackend for FakeDialog {
    fn confirm(&self, _owner: Option<isize>, title: &str, message: &str) -> bool {
        self.calls
            .lock()
            .expect("dialog lock")
            .push(format!("confirm:{title}:{message}"));
        self.answer.load(Ordering::SeqCst)
    }

    fn inform(&self, _owner: Option<isize>, title: &str, message: &str) {
        self.calls
            .lock()
            .expect("dialog lock")
            .push(format!("inform:{title}:{message}"));
    }
}

/// Wait (bounded) for a command matching `predicate` to reach the sink, then return it.
fn wait_for_command(
    sink: &Arc<RecordingSink>,
    predicate: impl Fn(&UiCommand) -> bool,
) -> UiCommand {
    let mut deadline = 0;
    loop {
        if let Some(command) = sink
            .commands()
            .into_iter()
            .find(|command| predicate(command))
        {
            return command;
        }
        deadline += 1;
        assert!(deadline < 1000, "the expected command must reach the sink");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Wait (bounded) until the dialog backend has recorded exactly `expected` calls.
fn wait_for_call_count(dialog: &FakeDialog, expected: usize) {
    let mut deadline = 0;
    while dialog.calls().len() < expected {
        deadline += 1;
        assert!(deadline < 1000, "the dialog backend must record the call");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(dialog.calls().len(), expected);
}

fn base_snapshot() -> ControllerSnapshot {
    ControllerSnapshot {
        publication_revision: 0,
        app: Arc::new(dji4g_application::AppSnapshot {
            revision: 0,
            observed_at: NOW,
            freshness: Freshness::Fresh,
            availability: Availability::Available,
            hotspot: HotspotStatus::Off,
            device: None,
            cellular: None,
            network: None,
            active_operation: None,
            issues: Vec::new(),
        }),
        diagnostics: DiagnosticSet::new(DeviceEpoch(1)),
        prepared_action: None,
        operation: None,
        settings: Default::default(),
        command_state: Default::default(),
        action_readiness: Vec::new(),
        feedback: None,
        sim_epoch: 0,
        feature_status: None,
        adapter_metrics: None,
        timeline: Default::default(),
        sms_inbox: Default::default(),
        sms_messages: Vec::new(),
        sms_send: None,
        sms_refresh_pending: false,
        sms_inbox_failure: None,
        device_tools: Default::default(),
    }
}

/// A snapshot carrying a prepared hotspot-enable plan still awaiting confirmation, for the
/// action the tests click.  `expires_at` is far in the future so the plan never expires while
/// the (fake) box is open.
fn prepared_snapshot(id: u128) -> ControllerSnapshot {
    let mut snapshot = base_snapshot();
    snapshot.publication_revision = 1;
    snapshot.prepared_action = Some(PreparedActionSnapshot {
        id: ActionPlanId::from_u128(id),
        action: ActionKindTag::ToggleHotspot { enabled: true },
        target_profile: dji4g_domain::DeviceProfile::DJI_GEN1,
        based_on_revision: 0,
        expires_at: SystemTime::now() + Duration::from_secs(300),
        disruption: DisruptionLevel::Brief,
        risk: RiskLevel::Low,
        requires_elevation: false,
        state: PreparedActionState::AwaitingConfirmation,
    });
    snapshot
}

fn harness() -> (
    PanelApp,
    dji4g_application::sync::watch::Sender<Arc<ControllerSnapshot>>,
    Arc<RecordingSink>,
) {
    let (snapshot_tx, snapshot_rx) =
        dji4g_application::sync::watch::channel(Arc::new(base_snapshot()));
    let sink = Arc::new(RecordingSink::new());
    let app = PanelApp::headless(PanelInputs::new(snapshot_rx, sink.clone(), None, None));
    (app, snapshot_tx, sink)
}

#[test]
fn a_click_opens_its_confirm_box_immediately_and_yes_confirms_the_published_plan() {
    let (mut app, snapshot_tx, sink) = harness();
    let dialog = FakeDialog::new(true);
    app.set_dialog_backend(Arc::new(dialog.clone()));
    assert_eq!(app.current_page(), dji4g_panel::app::Page::Overview);

    // The click dispatches the prepare and presents the box from the click itself — the plan is
    // not published yet, which proves the box does not wait for the controller round-trip.
    app.prepare_action_now(ActionKind::ToggleHotspot { enabled: true });

    let calls = dialog.wait_for_calls();
    assert_eq!(calls.len(), 1, "exactly one confirm box per click");
    assert!(calls[0].starts_with("confirm:"));
    assert!(
        !sink
            .commands()
            .iter()
            .any(|command| matches!(command, dji4g_application::UiCommand::ConfirmAction { .. })),
        "the box appears before any plan is published"
    );
    assert_eq!(
        app.current_page(),
        dji4g_panel::app::Page::Overview,
        "the native confirm box overlays any page, so a click must not navigate"
    );

    // Now the controller publishes the prepared plan for the same action...
    snapshot_tx
        .send(Arc::new(prepared_snapshot(1)))
        .expect("publish prepared snapshot");

    // ...and the Yes answer travels back through the ordinary command sink as ConfirmAction.
    let confirmed = wait_for_command(&sink, |command| {
        matches!(command, UiCommand::ConfirmAction { .. })
    });
    assert!(matches!(
        confirmed,
        UiCommand::ConfirmAction { id } if id == ActionPlanId::from_u128(1)
    ));
    assert_eq!(dialog.calls().len(), 1, "the box is presented exactly once");
}

#[test]
fn answering_no_cancels_the_published_plan() {
    let (mut app, snapshot_tx, sink) = harness();
    let dialog = FakeDialog::new(false);
    app.set_dialog_backend(Arc::new(dialog.clone()));

    app.prepare_action_now(ActionKind::ToggleHotspot { enabled: true });
    dialog.wait_for_calls();

    snapshot_tx
        .send(Arc::new(prepared_snapshot(1)))
        .expect("publish");
    let cancelled = wait_for_command(&sink, |command| {
        matches!(command, UiCommand::CancelAction { .. })
    });
    assert!(matches!(
        cancelled,
        UiCommand::CancelAction { id } if id == ActionPlanId::from_u128(1)
    ));
}

#[test]
fn a_repair_click_confirms_the_controlled_request() {
    let (mut app, snapshot_tx, sink) = harness();
    let dialog = FakeDialog::new(true);
    app.set_dialog_backend(Arc::new(dialog.clone()));

    app.prepare_repair_now(dji4g_application::ControlledRepairRequest::RefreshDhcp);
    dialog.wait_for_calls();
    assert!(matches!(
        sink.commands().as_slice(),
        [UiCommand::PrepareRepair { .. }]
    ));

    let mut prepared = base_snapshot();
    prepared.prepared_action = Some(PreparedActionSnapshot {
        id: ActionPlanId::from_u128(5),
        action: ActionKindTag::RenewDhcp,
        target_profile: dji4g_domain::DeviceProfile::DJI_GEN1,
        based_on_revision: 0,
        expires_at: SystemTime::now() + Duration::from_secs(300),
        disruption: DisruptionLevel::Brief,
        risk: RiskLevel::Low,
        requires_elevation: true,
        state: PreparedActionState::AwaitingConfirmation,
    });
    snapshot_tx.send(Arc::new(prepared)).expect("publish");
    let confirmed = wait_for_command(&sink, |command| {
        matches!(command, UiCommand::ConfirmAction { .. })
    });
    assert!(matches!(
        confirmed,
        UiCommand::ConfirmAction { id } if id == ActionPlanId::from_u128(5)
    ));
}

#[test]
fn a_second_click_while_a_confirm_box_is_pending_is_ignored_and_the_slot_reopens() {
    let (mut app, snapshot_tx, sink) = harness();
    let dialog = FakeDialog::new(true);
    app.set_dialog_backend(Arc::new(dialog.clone()));

    // First click dispatches the prepare and presents the box; the plan is not published yet, so
    // the worker still owns the dialog slot.
    app.prepare_action_now(ActionKind::ToggleHotspot { enabled: true });
    wait_for_command(&sink, |command| {
        matches!(command, UiCommand::PrepareAction { .. })
    });
    dialog.wait_for_calls();

    // A second click while the box is pending is ignored: exactly one prepare and one box.
    app.prepare_action_now(ActionKind::ToggleHotspot { enabled: true });
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(dialog.calls().len(), 1, "one box per click");
    assert_eq!(
        sink.commands()
            .iter()
            .filter(|command| matches!(command, UiCommand::PrepareAction { .. }))
            .count(),
        1,
        "the pending click must not dispatch a second prepare"
    );

    // Publishing the plan lets the answer turn into ConfirmAction, and the dialog slot reopens.
    snapshot_tx
        .send(Arc::new(prepared_snapshot(11)))
        .expect("publish");
    wait_for_command(&sink, |command| {
        matches!(command, UiCommand::ConfirmAction { .. })
    });

    // The controller would consume the confirmed plan; mirror that (drop it from the snapshot)
    // so a later click's worker cannot match the now-stale plan 11 still in the fake stream.
    snapshot_tx
        .send(Arc::new(base_snapshot()))
        .expect("publish");

    // The slot is free again: a fresh click is honoured.  Retry until the release lands so the
    // assertion cannot race with the worker clearing the busy flag right after the send.
    let mut retries = 0;
    while sink
        .commands()
        .iter()
        .filter(|command| matches!(command, UiCommand::PrepareAction { .. }))
        .count()
        < 2
    {
        app.prepare_action_now(ActionKind::ToggleHotspot { enabled: true });
        std::thread::sleep(Duration::from_millis(5));
        retries += 1;
        assert!(
            retries < 400,
            "the dialog slot must reopen after the box is done"
        );
    }
    wait_for_call_count(&dialog, 2);
    snapshot_tx
        .send(Arc::new(prepared_snapshot(12)))
        .expect("publish");
    let confirmed = wait_for_command(&sink, |command| {
        matches!(
            command,
            UiCommand::ConfirmAction { id } if *id == ActionPlanId::from_u128(12)
        )
    });
    assert!(matches!(
        confirmed,
        UiCommand::ConfirmAction { id } if id == ActionPlanId::from_u128(12)
    ));
}

#[test]
fn publishing_a_prepared_action_alone_never_opens_a_box() {
    let (mut app, snapshot_tx, sink) = harness();
    let dialog = FakeDialog::new(false);
    app.set_dialog_backend(Arc::new(dialog.clone()));
    let ctx = egui::Context::default();

    snapshot_tx
        .send(Arc::new(prepared_snapshot(1)))
        .expect("publish");
    app.receive_latest_nonblocking(&ctx);
    app.drive_native_dialogs();
    std::thread::sleep(Duration::from_millis(20));

    assert!(
        dialog.calls().is_empty(),
        "a prepared snapshot alone must never open a confirm box; only the click does"
    );
    assert!(
        sink.commands().is_empty(),
        "and it must not emit a confirm/cancel command"
    );
}

#[test]
fn without_a_dialog_backend_a_click_still_dispatches_the_prepare() {
    let (app, _snapshot_tx, sink) = harness();
    app.prepare_action_now(ActionKind::ToggleHotspot { enabled: true });
    assert!(matches!(
        sink.commands().as_slice(),
        [UiCommand::PrepareAction { .. }]
    ));
}

#[test]
fn a_finished_operation_opens_one_native_result_box_that_dismisses() {
    let (mut app, snapshot_tx, sink) = harness();
    let dialog = FakeDialog::new(false);
    app.set_dialog_backend(Arc::new(dialog.clone()));
    let ctx = egui::Context::default();

    let mut finished = base_snapshot();
    finished.operation = Some(dji4g_application::OperationUiSnapshot {
        operation_id: 7,
        action: ActionKindTag::RestartAdapter,
        started_at: NOW,
        state: dji4g_application::OperationState::Finished {
            outcome: dji4g_domain::OperationOutcome::Applied {
                after_state_hash: dji4g_domain::AfterStateHash([0_u8; 32]),
            },
            finished_at: NOW,
        },
    });
    snapshot_tx.send(Arc::new(finished)).expect("publish");
    app.receive_latest_nonblocking(&ctx);
    app.drive_native_dialogs();

    let calls = dialog.wait_for_calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].starts_with("inform:"),
        "result box must be informational"
    );
    assert_eq!(
        app.current_page(),
        dji4g_panel::app::Page::Overview,
        "the result box must not navigate either"
    );

    wait_for_command(&sink, |command| {
        matches!(
            command,
            UiCommand::DismissOperation { operation_id } if *operation_id == 7
        )
    });
    app.drive_native_dialogs();
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(
        dialog.calls().len(),
        1,
        "the result box is presented exactly once"
    );
}

#[test]
fn snapshots_without_any_dialog_state_never_move_the_page_or_open_dialogs() {
    let (mut app, snapshot_tx, sink) = harness();
    let dialog = FakeDialog::new(false);
    app.set_dialog_backend(Arc::new(dialog.clone()));
    let ctx = egui::Context::default();

    let mut next = base_snapshot();
    next.publication_revision = 5;
    snapshot_tx.send(Arc::new(next)).expect("publish");
    app.receive_latest_nonblocking(&ctx);
    app.drive_native_dialogs();

    assert_eq!(app.current_page(), dji4g_panel::app::Page::Overview);
    std::thread::sleep(Duration::from_millis(20));
    assert!(dialog.calls().is_empty(), "no state, no dialog");
    assert!(sink.commands().is_empty());
}

#[test]
fn a_rejected_command_surfaces_exactly_one_toast() {
    let (mut app, snapshot_tx, _sink) = harness();
    let ctx = egui::Context::default();
    assert!(app.toast_text().is_none());
    assert_eq!(app.last_feedback_seq(), 0);

    let rejected = |seq, stable| {
        let mut snapshot = base_snapshot();
        snapshot.feedback = Some(dji4g_application::UiFeedback {
            seq,
            code: dji4g_application::FailureCode::new(
                dji4g_domain::ErrorCode::Internal,
                dji4g_application::StableCode::try_from_static(stable).unwrap(),
            ),
        });
        snapshot
    };

    snapshot_tx
        .send(Arc::new(rejected(1, "app:busy")))
        .expect("publish");
    app.receive_latest_nonblocking(&ctx);
    assert!(
        app.toast_text()
            .is_some_and(|text| text.contains("已有操作正在执行")),
        "the busy rejection must surface as a user-visible toast"
    );
    assert_eq!(app.last_feedback_seq(), 1);

    // Re-applying the same rejection must not toast again: the seq guard is the dedupe key.
    snapshot_tx
        .send(Arc::new(rejected(1, "app:busy")))
        .expect("publish");
    app.receive_latest_nonblocking(&ctx);
    assert_eq!(app.last_feedback_seq(), 1);

    // A genuinely new rejection (higher seq) surfaces once more with its own message.
    snapshot_tx
        .send(Arc::new(rejected(2, "app:confirm_rejected")))
        .expect("publish");
    app.receive_latest_nonblocking(&ctx);
    assert_eq!(app.last_feedback_seq(), 2);
    assert!(
        app.toast_text()
            .is_some_and(|text| text.contains("确认未生效")),
        "a new rejection must replace the toast with its own message"
    );
}
