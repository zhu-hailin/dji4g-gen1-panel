//! Sending consumes the visible decision; terminal results cannot arm it again.
use super::*;
use crate::localization::TextKey;
use dji4g_application::{Controller, ControllerSnapshot};
use std::sync::Mutex;

const RECIPIENT: &str = "+12025550123";
const BODY: &str = "Send regression fixture";

#[derive(Default)]
struct SendSink {
    attempts: Mutex<Vec<(String, String)>>,
    refreshes: Mutex<usize>,
    failure: Option<UiSendError>,
}

impl UiCommandSink for SendSink {
    fn try_send(&self, command: UiCommand) -> Result<(), UiSendError> {
        match command {
            UiCommand::SmsSend { recipient, body } => {
                self.attempts.lock().unwrap().push((recipient, body));
                if let Some(error) = &self.failure {
                    return Err(error.clone());
                }
            }
            UiCommand::SmsRefresh => *self.refreshes.lock().unwrap() += 1,
            _ => {}
        }
        Ok(())
    }
}

struct SendUi {
    context: egui::Context,
    state: SmsComposeState,
    snapshot: ControllerSnapshot,
    sink: SendSink,
    language: Language,
}

impl SendUi {
    fn new(language: Language, failure: Option<UiSendError>) -> Self {
        let context = egui::Context::default();
        crate::ui::initialize_visuals(&context);
        context.style_mut(|style| style.animation_time = 0.0);
        let draft = Draft {
            recipient: RECIPIENT.into(),
            body: BODY.into(),
        };
        Self {
            context,
            state: SmsComposeState {
                open: true,
                editor_focus: Some(EditorFocus::Body),
                confirmation: Some(draft.clone()),
                draft,
                ..Default::default()
            },
            snapshot: Controller::for_test(std::time::SystemTime::UNIX_EPOCH).snapshot(),
            sink: SendSink {
                failure,
                ..Default::default()
            },
            language,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>, full_page: bool) -> egui::FullOutput {
        let context = self.context.clone();
        context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    if full_page {
                        crate::ui::sms::render(
                            ui,
                            &self.snapshot,
                            &[],
                            self.language,
                            &self.sink,
                            &mut self.state,
                        );
                    } else {
                        render(
                            ui,
                            self.language,
                            &mut self.state,
                            &self.snapshot,
                            &self.sink,
                        );
                    }
                });
            },
        )
    }

    fn settle(&mut self, full_page: bool) -> egui::FullOutput {
        self.frame(vec![], full_page);
        self.frame(vec![], full_page);
        self.frame(vec![], full_page)
    }

    fn click(&mut self, point: egui::Pos2, full_page: bool) -> egui::FullOutput {
        self.frame(pointer(point, true), full_page);
        self.frame(pointer(point, false), full_page)
    }

    fn enter(&mut self, full_page: bool) {
        for pressed in [true, false] {
            self.frame(
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: Some(egui::Key::Enter),
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                full_page,
            );
        }
    }

    fn confirm_by_click(&mut self) -> egui::Pos2 {
        let output = self.settle(false);
        let window = self
            .context
            .read_response(egui::Id::new("sms-compose-confirmation"))
            .expect("send review must be visible")
            .rect;
        let label = t(self.language, TextKey::ComposeConfirmAction);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, &label, Some(window)))
            .expect("localized confirm button must be inside the review")
            .center();
        self.click(point, false);
        point
    }

    fn assert_closed(&self, output: &egui::FullOutput) {
        assert!(!self.state.open, "editor reopened in {:?}", self.language);
        assert!(self.state.editor_focus.is_none());
        assert!(self.state.confirmation.is_none());
        for key in [TextKey::ComposeTitle, TextKey::ComposeConfirmTitle] {
            assert!(
                !has_text(output, &t(self.language, key)),
                "send modal remained visible in {:?}",
                self.language
            );
        }
    }

    fn assert_single_attempt(&self) {
        assert_eq!(
            self.sink.attempts.lock().unwrap().as_slice(),
            &[(RECIPIENT.to_owned(), BODY.to_owned())]
        );
    }
}

fn pointer(point: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(point),
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

fn text_rect(shape: &egui::Shape, label: &str, bounds: Option<egui::Rect>) -> Option<egui::Rect> {
    match shape {
        egui::Shape::Text(text) if text.galley.text() == label => {
            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
            bounds
                .is_none_or(|bounds| bounds.contains_rect(rect))
                .then_some(rect)
        }
        egui::Shape::Vec(shapes) => shapes
            .iter()
            .find_map(|shape| text_rect(shape, label, bounds)),
        _ => None,
    }
}

fn has_text(output: &egui::FullOutput, label: &str) -> bool {
    output
        .shapes
        .iter()
        .any(|shape| text_rect(&shape.shape, label, None).is_some())
}

fn send_snapshot(phase: SmsSendPhase, result: Option<SmsSendResult>) -> SmsSendSnapshot {
    SmsSendSnapshot {
        request_id: 8,
        phase,
        result,
        failure: None,
    }
}

fn refresh_frame(
    context: &egui::Context,
    ui: &mut SendUi,
    label: &str,
    events: Vec<egui::Event>,
) -> egui::Response {
    let mut response = None;
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 700.0),
            )),
            events,
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |panel| {
                response = Some(crate::ui::sms::refresh_button(
                    panel,
                    ui.language,
                    &ui.snapshot,
                    &ui.sink,
                    &mut ui.state,
                    label,
                ));
            });
        },
    );
    response.expect("the refresh control must be drawn")
}

#[test]
fn actual_refresh_buttons_respect_local_send_and_decision_gates_before_snapshot_acknowledgment() {
    for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
        for key in [TextKey::SmsRefreshList, TextKey::SmsRefreshMessages] {
            for gate in [
                "idle",
                "editor",
                "confirmation",
                "replacement",
                "storage",
                "pending",
            ] {
                let mut ui = SendUi::new(language, None);
                match gate {
                    "idle" => {
                        ui.state.open = false;
                        ui.state.confirmation = None;
                    }
                    "editor" => ui.state.confirmation = None,
                    "confirmation" => {}
                    "replacement" => {
                        ui.state.open = false;
                        ui.state.confirmation = None;
                        ui.state.replacement_draft = Some(Draft {
                            recipient: "10010".into(),
                            body: "CXYE".into(),
                        });
                    }
                    "storage" => {
                        ui.state.open = false;
                        ui.state.confirmation = None;
                        ui.state.storage_confirmation = Some((
                            None,
                            ui.snapshot.sim_epoch,
                            dji4g_domain::SmsStorageId("SM".into()),
                        ));
                    }
                    "pending" => {
                        ui.confirm_by_click();
                        assert!(ui.state.pending.is_some());
                        ui.assert_single_attempt();
                    }
                    _ => unreachable!(),
                }
                assert!(!ui.snapshot.serial_work_busy);
                assert!(!ui.snapshot.sms_refresh_pending);
                assert!(ui.snapshot.sms_send.is_none());
                let draft = ui.state.draft.clone();
                // Render the actual control without a modal scrim, so a blocked pointer cannot
                // make an enabled refresh button look safe. The controller snapshot stays idle.
                let context = egui::Context::default();
                crate::ui::initialize_visuals(&context);
                let label = t(language, key);
                let mut response = refresh_frame(&context, &mut ui, &label, vec![]);
                for _ in 0..2 {
                    response = refresh_frame(&context, &mut ui, &label, vec![]);
                }
                assert_eq!(
                    response.enabled(),
                    gate == "idle",
                    "{language:?}/{key:?}/{gate}"
                );
                let point = response.rect.center();
                refresh_frame(&context, &mut ui, &label, pointer(point, true));
                let clicked = refresh_frame(&context, &mut ui, &label, pointer(point, false));
                assert_eq!(
                    clicked.clicked(),
                    gate == "idle",
                    "{language:?}/{key:?}/{gate}"
                );
                assert_eq!(
                    *ui.sink.refreshes.lock().unwrap(),
                    usize::from(gate == "idle"),
                    "{language:?}/{key:?}/{gate}"
                );
                assert!(ui.state.draft == draft);
                assert_eq!(ui.state.pending.is_some(), gate == "pending");
            }
        }
    }
}

#[test]
fn full_page_refresh_click_cannot_queue_a_read_before_the_send_snapshot_arrives() {
    for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
        let mut ui = SendUi::new(language, None);
        ui.confirm_by_click();
        assert!(ui.state.pending.is_some());
        assert!(!ui.snapshot.serial_work_busy);
        assert!(ui.snapshot.sms_send.is_none());
        let output = ui.settle(true);
        let label = t(language, TextKey::SmsRefreshList);
        let point = output
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, &label, None))
            .expect("the refresh entry remains visible during a pending send")
            .center();
        ui.click(point, true);
        ui.assert_single_attempt();
        assert_eq!(*ui.sink.refreshes.lock().unwrap(), 0);
        assert!(ui.state.pending.is_some());
        assert!(!ui.state.open);
    }
}

#[test]
fn real_confirmation_click_closes_the_modal_and_cannot_repeat_across_send_results() {
    for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
        for result in [
            SmsSendResult::Submitted,
            SmsSendResult::Failed,
            SmsSendResult::OutcomeUnknown,
        ] {
            let mut ui = SendUi::new(language, None);
            let draft = ui.state.draft.clone();
            // A previous success must not be presented as this newly queued request's result.
            let mut previous =
                send_snapshot(SmsSendPhase::Finished, Some(SmsSendResult::Submitted));
            previous.request_id = 7;
            ui.snapshot.sms_send = Some(previous);
            let old_confirm_position = ui.confirm_by_click();
            assert!(ui.state.pending.is_some());
            assert!(ui.state.draft == draft);
            let output = ui.settle(false);
            ui.assert_closed(&output);
            assert!(
                has_text(&output, &t(language, TextKey::ComposeQueued))
                    || has_text(&output, &t(language, TextKey::ComposeStageQueued))
            );
            assert!(!has_text(
                &output,
                &t(language, TextKey::ComposeSubmittedUnknown)
            ));
            ui.click(old_confirm_position, false);
            ui.enter(false);
            ui.assert_single_attempt();

            for phase in [
                SmsSendPhase::Queued,
                SmsSendPhase::Preparing,
                SmsSendPhase::Submitting,
                SmsSendPhase::WaitingForResult,
            ] {
                ui.snapshot.sms_send = Some(send_snapshot(phase, None));
                ui.snapshot.serial_work_busy = true;
                let output = ui.settle(false);
                ui.assert_closed(&output);
                assert!(ui.state.pending.is_some());
                assert!(ui.state.draft == draft);
                ui.state.open_editor();
                assert!(!ui.state.open, "an active send cannot reopen its editor");
                ui.click(old_confirm_position, false);
                ui.enter(false);
                ui.assert_single_attempt();
            }

            ui.snapshot.sms_send = Some(send_snapshot(SmsSendPhase::Finished, Some(result)));
            ui.snapshot.serial_work_busy = false;
            let output = ui.settle(false);
            ui.assert_closed(&output);
            assert!(ui.state.pending.is_none());
            if result == SmsSendResult::Submitted {
                assert!(ui.state.draft == Draft::default());
            } else {
                assert!(ui.state.draft == draft);
            }
            ui.click(old_confirm_position, false);
            ui.enter(false);
            ui.state.confirm(&ui.sink, ui.snapshot.sms_send.as_ref());
            ui.assert_single_attempt();
            let output = ui.settle(false);
            ui.assert_closed(&output);
            if result != SmsSendResult::Submitted {
                ui.state.open_editor();
                assert!(ui.state.open, "retained draft can be explicitly reopened");
                assert!(
                    ui.state.confirmation.is_none(),
                    "reopening does not confirm a resend"
                );
                ui.assert_single_attempt();
            }
        }
    }
}

#[test]
fn local_queue_failure_keeps_the_editor_and_draft_visible_in_each_language() {
    for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
        for failure in [UiSendError::QueueFull, UiSendError::Closed] {
            let mut ui = SendUi::new(language, Some(failure.clone()));
            let draft = ui.state.draft.clone();
            ui.confirm_by_click();
            let output = ui.settle(false);
            assert!(ui.state.open);
            assert!(ui.state.draft == draft);
            assert!(ui.state.pending.is_none());
            assert!(ui.state.confirmation.is_none());
            assert!(has_text(&output, &t(language, TextKey::ComposeTitle)));
            assert!(has_text(&output, &enqueue_error(failure, language)));
            assert!(has_text(&output, BODY), "retained draft must be visible");
            ui.assert_single_attempt();
        }
    }
}

#[test]
fn asynchronous_backend_rejection_preserves_a_closed_unconfirmed_draft() {
    for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
        let mut ui = SendUi::new(language, None);
        let draft = ui.state.draft.clone();
        let mut previous = send_snapshot(SmsSendPhase::Finished, Some(SmsSendResult::Submitted));
        previous.request_id = 7;
        ui.snapshot.sms_send = Some(previous);
        let old_confirm_position = ui.confirm_by_click();
        ui.snapshot.command_state.sms_send_rejected_seq += 1;
        let output = ui.settle(false);
        ui.assert_closed(&output);
        assert!(ui.state.pending.is_none());
        assert!(ui.state.draft == draft);
        assert!(has_text(&output, &t(language, TextKey::ComposeBackendBusy)));
        assert!(!has_text(
            &output,
            &t(language, TextKey::ComposeSubmittedUnknown)
        ));
        ui.click(old_confirm_position, false);
        ui.enter(false);
        ui.state.confirm(&ui.sink, None);
        ui.assert_single_attempt();
    }
}

#[test]
fn new_sms_action_does_not_open_the_editor_during_a_pending_send_or_serial_work() {
    for pending_send in [false, true] {
        let mut ui = SendUi::new(Language::ZhCn, None);
        if pending_send {
            ui.confirm_by_click();
        } else {
            ui.state.open = false;
            ui.state.confirmation = None;
            ui.state.editor_focus = None;
            ui.snapshot.serial_work_busy = true;
        }
        let output = ui.settle(true);
        let label = t(
            ui.language,
            if ui.state.has_draft() {
                TextKey::ComposeResumeDraft
            } else {
                TextKey::ComposeTitle
            },
        );
        let action = output
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, &label, None))
            .expect("the New SMS control remains visible")
            .center();
        let clicked = ui.click(action, true);
        assert!(!ui.state.open);
        assert!(ui.state.confirmation.is_none());
        assert!(
            !clicked.platform_output.events.iter().any(|event| {
                matches!(event, egui::output::OutputEvent::Clicked(info) if info.label.as_deref() == Some(label.as_str()))
            }),
            "the New SMS button must be disabled, not merely ignore its click"
        );
        assert_eq!(
            ui.sink.attempts.lock().unwrap().len(),
            usize::from(pending_send)
        );

        let balance_entry = ui.context.data(|data| {
            data.get_temp::<egui::Rect>(egui::Id::new("balance-entry-rect"))
                .expect("balance menu entry remains available")
        });
        ui.click(balance_entry.center(), true);
        ui.settle(true);
        let template = ui.context.data(|data| {
            data.get_temp::<egui::Rect>(egui::Id::new(("balance-template-rect", "10010")))
                .expect("balance template remains visible in the menu")
        });
        let clicked = ui.click(template.center(), true);
        let template_label = format!(
            "{} · CXYE → 10010",
            t(ui.language, TextKey::CarrierChinaUnicom)
        );
        assert!(
            !clicked.platform_output.events.iter().any(|event| {
                matches!(event, egui::output::OutputEvent::Clicked(info) if info.label.as_deref() == Some(template_label.as_str()))
            }),
            "balance templates must also be disabled before the send snapshot arrives"
        );
        assert!(!ui.state.open);
        assert!(ui.state.confirmation.is_none());
        assert_eq!(
            ui.sink.attempts.lock().unwrap().len(),
            usize::from(pending_send)
        );
    }

    let mut state = SmsComposeState {
        module_read_only: true,
        ..Default::default()
    };
    state.open_editor();
    assert!(
        !state.open,
        "a read-only module cannot open the send editor"
    );
    assert!(state.editor_focus.is_none());
}
