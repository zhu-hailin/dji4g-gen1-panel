//! UI-owned SMS drafts. Never logged or serialized; confirmation freezes the exact payload.
use crate::app::UiCommandSink;
use crate::localization::Language;
use dji4g_application::{SmsSendPhase, SmsSendResult, SmsSendSnapshot, UiCommand, UiSendError};
use eframe::egui::{self, Ui};

#[cfg(test)]
#[path = "sms_compose_send_tests.rs"]
mod send_flow_tests;

/// One catalog string in the language this page was rendered with.
fn t(language: crate::localization::Language, key: crate::localization::TextKey) -> String {
    crate::localization::LocalizedText::new(language, key).text
}

#[derive(Clone, Default, PartialEq, Eq)]
struct Draft {
    recipient: String,
    body: String,
}

struct Pending {
    draft: Draft,
    after_id: Option<u64>,
    request_id: Option<u64>,
    after_send_rejected: u64,
    context_changed: bool,
}

/// Content-free result of starting a reply; existing drafts require an explicit decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReplyStartResult {
    Opened,
    ReplacementConfirmation,
    InvalidRecipient,
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditorFocus {
    Recipient,
    Body,
}

#[derive(Default)]
pub(crate) struct SmsComposeState {
    pub open: bool,
    editor_focus: Option<EditorFocus>,
    draft: Draft,
    confirmation: Option<Draft>,
    pending: Option<Pending>,
    pub outgoing: bool,
    pub search: String,
    pub inbox_filter: super::InboxFilter,
    pub refresh_error: Option<String>,
    pub storage_confirmation: Option<(
        Option<dji4g_domain::DeviceEpoch>,
        u64,
        dji4g_domain::SmsStorageId,
    )>,
    pub auto_refresh_paused: bool,
    pub read_cancel_requested: bool,
    last_refresh_attempt: Option<std::time::Instant>,
    last_visible: Option<std::time::Instant>,
    pub selected: Option<[u8; 32]>,
    pub serial_busy: bool,
    pub module_read_only: bool,
    replacement_draft: Option<Draft>,
    pub error: Option<String>,
    last_send_rejected: u64,
    context: Option<(Option<dji4g_domain::DeviceEpoch>, u64)>,
    /// The interface language of the last frame, so notices built outside `render` follow it too.
    language: Language,
}

impl SmsComposeState {
    /// Observe snapshot updates, including while another page is visible. Durable, send-specific
    /// rejection evidence survives a feedback notice being replaced before the next UI frame.
    pub(crate) fn observe_snapshot(&mut self, controller: &dji4g_application::ControllerSnapshot) {
        self.observe_context((
            controller.app.device.as_ref().map(|device| device.epoch),
            controller.sim_epoch,
        ));
        self.synchronize(controller.sms_send.as_ref());
        self.observe_send_rejection(controller.command_state.sms_send_rejected_seq);
        self.serial_busy = controller.serial_work_busy;
        self.module_read_only = super::read_only_module(controller);
    }

    pub(super) fn open_editor(&mut self) {
        if !self.can_open_editor() {
            return;
        }
        if !self.open {
            self.editor_focus = Some(EditorFocus::Recipient);
        }
        self.open = true;
    }

    pub(in crate::ui) fn can_open_editor(&self) -> bool {
        !self.module_read_only && !self.serial_busy && self.pending.is_none()
    }

    pub(super) fn has_draft(&self) -> bool {
        !self.draft.recipient.is_empty() || !self.draft.body.is_empty()
    }

    pub(super) fn begin_reply(&mut self, recipient: &str) -> ReplyStartResult {
        self.begin_draft(Draft {
            recipient: recipient.to_owned(),
            body: String::new(),
        })
    }

    /// A service template is only a draft. It takes the same replacement and send confirmations
    /// as a reply, and never queues a device command when the menu item is selected.
    pub(crate) fn begin_balance_query(&mut self, recipient: &str, body: &str) -> ReplyStartResult {
        self.begin_draft(Draft {
            recipient: recipient.to_owned(),
            body: body.to_owned(),
        })
    }

    fn begin_draft(&mut self, draft: Draft) -> ReplyStartResult {
        if self.module_read_only
            || self.serial_busy
            || self.pending.is_some()
            || self.confirmation.is_some()
            || self.replacement_draft.is_some()
        {
            self.error = Some(t(
                self.language,
                crate::localization::TextKey::ComposeBusyDraftKept,
            ));
            return ReplyStartResult::Busy;
        }
        if dji4g_at_protocol::validate_sms_recipient(&draft.recipient).is_err() {
            self.error = Some(t(
                self.language,
                crate::localization::TextKey::ComposeUnsupportedSender,
            ));
            return ReplyStartResult::InvalidRecipient;
        }
        self.error = None;
        if !self.draft.recipient.is_empty() || !self.draft.body.is_empty() {
            self.replacement_draft = Some(draft);
            ReplyStartResult::ReplacementConfirmation
        } else {
            self.draft = draft;
            self.open = true;
            self.editor_focus = Some(EditorFocus::Body);
            ReplyStartResult::Opened
        }
    }

    fn resolve_reply(&mut self, replace: bool) {
        let Some(draft) = self.replacement_draft.take() else {
            return;
        };
        if replace
            && !self.module_read_only
            && !self.serial_busy
            && self.pending.is_none()
            && self.confirmation.is_none()
        {
            self.draft = draft;
            self.open = true;
            self.editor_focus = Some(EditorFocus::Body);
        }
    }

    pub(crate) fn search_shortcuts_allowed(&self) -> bool {
        !self.open
            && self.confirmation.is_none()
            && self.replacement_draft.is_none()
            && self.storage_confirmation.is_none()
            && self.pending.is_none()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn review_reply_replace(&mut self) {
        self.review_editor();
        self.begin_reply("+8613900000000");
    }
    /// Debug-only visual fixture. This only opens a confirmation; it never dispatches a command.
    #[cfg(debug_assertions)]
    pub(crate) fn review_confirmation(&mut self) {
        self.draft = Draft {
            recipient: "+12025550123".into(),
            body: t(
                Language::ZhCn,
                crate::localization::TextKey::ComposeDemoDraft,
            ),
        };
        self.confirmation = Some(self.draft.clone());
        self.open = true;
    }

    #[cfg(debug_assertions)]
    pub(crate) fn review_editor(&mut self) {
        self.review_confirmation();
        self.confirmation = None;
        self.editor_focus = Some(EditorFocus::Recipient);
    }

    /// Simulated, UI-only queue failure for visual checks; it never dispatches a send.
    #[cfg(debug_assertions)]
    pub(crate) fn review_queue_error(&mut self) {
        self.review_editor();
        self.error = Some(enqueue_error(UiSendError::QueueFull, self.language));
    }

    pub(super) fn request_refresh(&mut self, now: std::time::Instant, sink: &dyn UiCommandSink) {
        self.auto_refresh_paused = false;
        self.read_cancel_requested = false;
        self.last_refresh_attempt = Some(now);
        self.refresh_error = sink
            .try_send(UiCommand::SmsRefresh)
            .err()
            .map(|e| enqueue_error(e, self.language));
    }

    pub(super) fn auto_refresh(
        &mut self,
        now: std::time::Instant,
        available: bool,
        busy: bool,
        query_pending: bool,
        sink: &dyn UiCommandSink,
    ) {
        // Entering the page refreshes immediately; ordinary render frames never enqueue duplicates.
        if self.last_visible.is_some_and(|seen| {
            now.saturating_duration_since(seen) > std::time::Duration::from_secs(2)
        }) {
            self.last_refresh_attempt = None;
        }
        self.last_visible = Some(now);
        // Background reads share the module's serial channel. Keep that channel free while the
        // user edits or reviews a draft, so synchronization cannot disable the focused fields.
        if self.module_read_only
            || self.auto_refresh_paused
            || self.open
            || self.confirmation.is_some()
            || self.replacement_draft.is_some()
            || self.storage_confirmation.is_some()
            || !available
            || busy
            || query_pending
            || self.pending.is_some()
        {
            return;
        }
        if self.last_refresh_attempt.is_none_or(|at| {
            now.saturating_duration_since(at) >= std::time::Duration::from_secs(15)
        }) {
            self.request_refresh(now, sink);
        }
    }

    fn ready(&self) -> bool {
        dji4g_at_protocol::build_ucs2_submit(&self.draft.recipient, &self.draft.body).is_ok()
    }

    fn edited(&mut self) {
        self.confirmation = None;
        self.error = None;
    }

    fn confirm(&mut self, sink: &dyn UiCommandSink, snapshot: Option<&SmsSendSnapshot>) {
        if self.pending.is_some() || snapshot.is_some_and(|s| s.phase != SmsSendPhase::Finished) {
            return;
        }
        let Some(frozen) = self.confirmation.take() else {
            return;
        };
        if frozen != self.draft || !self.ready() {
            return;
        }
        match sink.try_send(UiCommand::SmsSend {
            recipient: frozen.recipient.clone(),
            body: frozen.body.clone(),
        }) {
            Ok(()) => {
                self.error = None;
                // The accepted request owns this draft until its result arrives. Return to the
                // inbox immediately instead of reopening the same form behind the confirmation.
                self.open = false;
                self.editor_focus = None;
                self.pending = Some(Pending {
                    draft: frozen,
                    after_id: snapshot.map(|s| s.request_id),
                    request_id: None,
                    after_send_rejected: self.last_send_rejected,
                    context_changed: false,
                });
            }
            Err(error) => {
                self.error = Some(enqueue_error(error, self.language));
                self.open = true;
                self.editor_focus = Some(EditorFocus::Body);
            }
        }
    }

    pub fn synchronize(&mut self, snapshot: Option<&SmsSendSnapshot>) {
        let (Some(pending), Some(snapshot)) = (&mut self.pending, snapshot) else {
            return;
        };
        if pending.request_id.is_none()
            && pending.after_id.is_none_or(|id| snapshot.request_id > id)
        {
            pending.request_id = Some(snapshot.request_id);
        }
        if pending.request_id != Some(snapshot.request_id)
            || snapshot.phase != SmsSendPhase::Finished
        {
            return;
        }
        if snapshot.result == Some(SmsSendResult::Submitted) && !pending.context_changed {
            // The next idle inbox frame should read the operator's reply without waiting for the
            // ordinary cadence. Auto-refresh still yields to an open draft and serial work.
            self.last_refresh_attempt = None;
        }
        if snapshot.result == Some(SmsSendResult::Submitted)
            && self.draft == pending.draft
            && !pending.context_changed
        {
            self.draft = Draft::default();
            self.open = false;
        }
        self.pending = None;
    }

    fn observe_context(&mut self, context: (Option<dji4g_domain::DeviceEpoch>, u64)) {
        if self
            .storage_confirmation
            .as_ref()
            .is_some_and(|(epoch, sim, _)| (*epoch, *sim) != context)
        {
            self.storage_confirmation = None;
        }
        if self.context.is_some_and(|old| old != context) {
            self.confirmation = None;
            self.replacement_draft = None;
            self.selected = None;
            self.last_refresh_attempt = None;
            if let Some(pending) = &mut self.pending {
                pending.context_changed = true;
            }
        }
        self.context = Some(context);
    }

    fn observe_send_rejection(&mut self, seq: u64) {
        if self.pending.as_ref().is_some_and(|pending| {
            pending.request_id.is_none() && seq > pending.after_send_rejected
        }) {
            self.pending = None;
            self.error = Some(t(
                self.language,
                crate::localization::TextKey::ComposeBackendBusy,
            ));
        }
        self.last_send_rejected = self.last_send_rejected.max(seq);
    }
}

pub(super) fn enqueue_error(error: UiSendError, language: Language) -> String {
    match error {
        UiSendError::QueueFull => t(language, crate::localization::TextKey::ComposeQueueFull),
        UiSendError::Closed => t(language, crate::localization::TextKey::ComposeChannelClosed),
    }
}

pub(super) fn render(
    ui: &mut Ui,
    language: Language,
    state: &mut SmsComposeState,
    controller: &dji4g_application::ControllerSnapshot,
    sink: &dyn UiCommandSink,
) {
    state.language = language;
    let snapshot = controller.sms_send.as_ref();
    state.observe_snapshot(controller);
    // Before the new request has a snapshot, the controller may still show the previous result.
    // Only expose a snapshot bound to this send; otherwise show its pending state below.
    let visible_snapshot = snapshot.filter(|snapshot| {
        (state.error.is_none() || snapshot.phase != SmsSendPhase::Finished)
            && state
                .pending
                .as_ref()
                .is_none_or(|pending| pending.request_id == Some(snapshot.request_id))
    });
    let sending =
        state.pending.is_some() || snapshot.is_some_and(|s| s.phase != SmsSendPhase::Finished);
    let busy = state.serial_busy || state.module_read_only || sending;
    if let Some(snapshot) = visible_snapshot {
        egui::Frame::none()
            .fill(crate::ui::scale::surface_sunken())
            .rounding(10.0)
            .inner_margin(14.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let text = match snapshot.phase {
                    SmsSendPhase::Queued => phase_name(snapshot.phase, language),
                    SmsSendPhase::Preparing => phase_name(snapshot.phase, language),
                    SmsSendPhase::Submitting => phase_name(snapshot.phase, language),
                    SmsSendPhase::WaitingForResult => phase_name(snapshot.phase, language),
                    SmsSendPhase::Finished => match snapshot.result {
                        Some(SmsSendResult::Submitted) => t(
                            language,
                            crate::localization::TextKey::ComposeSubmittedUnknown,
                        ),
                        Some(SmsSendResult::Failed) => t(
                            language,
                            crate::localization::TextKey::ComposeFailedDraftKept,
                        ),
                        _ => t(
                            language,
                            crate::localization::TextKey::ComposeUnknownMaybeSent,
                        ),
                    },
                };
                ui.horizontal_wrapped(|ui| {
                    let tone = send_result_tone(snapshot);
                    ui.label(
                        egui::RichText::new(format!("{} {text}", tone.marker()))
                            .color(tone.color())
                            .strong(),
                    );
                    if let Some(failure) = &snapshot.failure {
                        if let Some(code) = failure.cms_code {
                            ui.label(crate::ui::meta_text(format!("CMS {code}")));
                        }
                        if let Some(code) = failure.cme_code {
                            ui.label(crate::ui::meta_text(format!("CME {code}")));
                        }
                    }
                });
                if let Some(failure) = &snapshot.failure {
                    // The failure explanation used to sit behind a disclosure triangle inside this
                    // notice.  It is now a flat always-open block, and the detail keeps the quiet
                    // tiers so the notice does not grow into a second card.
                    ui.label(crate::ui::section_heading(t(
                        language,
                        crate::localization::TextKey::ComposeFailureHeading,
                    )));
                    crate::ui::wrapped_label(
                        ui,
                        crate::ui::detail_text(crate::localization::format_positional(
                            language,
                            crate::localization::TextKey::ComposeFailureStage,
                            &[&phase_name(failure.stage, language), &failure.code],
                        )),
                    );
                    crate::ui::wrapped_label(
                        ui,
                        crate::ui::detail_text(failure_advice(&failure.code, language)),
                    );
                    if let Some(code) = failure.cms_code {
                        ui.label(crate::ui::meta_text(
                            crate::localization::format_positional(
                                language,
                                crate::localization::TextKey::ComposeCmsError,
                                &[&code.to_string()],
                            ),
                        ));
                    }
                    if let Some(code) = failure.cme_code {
                        ui.label(crate::ui::meta_text(
                            crate::localization::format_positional(
                                language,
                                crate::localization::TextKey::ComposeCmeError,
                                &[&code.to_string()],
                            ),
                        ));
                    }
                    if let Some(code) = failure.os_code {
                        ui.label(crate::ui::meta_text(
                            crate::localization::format_positional(
                                language,
                                crate::localization::TextKey::ComposeSystemError,
                                &[&code.to_string()],
                            ),
                        ));
                    }
                    crate::ui::wrapped_label(
                        ui,
                        crate::ui::detail_text(submission_notice(
                            snapshot.result,
                            failure,
                            language,
                        )),
                    );
                }
            });
    } else if state.pending.is_some() {
        ui.label(t(
            language,
            if state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.request_id.is_some())
            {
                crate::localization::TextKey::ComposeSendingWait
            } else {
                crate::localization::TextKey::ComposeQueued
            },
        ));
    }
    if !state.open
        && let Some(error) = &state.error
    {
        ui.colored_label(super::StatusTone::Negative.color(), error);
    }
    // A reply whose original draft still needs a decision takes the screen on its own: showing the
    // editor underneath a second dialog would stack two dimmed cards and two scrims.
    if state.open && state.confirmation.is_none() && state.replacement_draft.is_none() {
        // The form is the only element allowed to grow, so it carries the bound: the reserve covers
        // the card's padding, its title block, the hairline and the decision row.
        let fields_max_height = (ui.ctx().screen_rect().height() - 340.0).clamp(140.0, 400.0);
        let can_advance = state.ready() && !busy;
        let outcome = crate::ui::modal::show(
            ui.ctx(),
            &crate::ui::modal::Dialog::new(
                "sms-compose-window",
                &t(language, crate::localization::TextKey::ComposeTitle),
            )
            .description(&t(language, crate::localization::TextKey::ComposeIntro))
            .width(500.0),
            |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                egui::ScrollArea::vertical()
                    .id_salt("sms-compose-fields")
                    .max_height(fields_max_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(t(
                                language,
                                crate::localization::TextKey::ComposeRecipient,
                            ))
                            .strong(),
                        );
                        // 5pt of vertical padding puts this single-line box exactly on the shared
                        // `CONTROL_H` (28pt): a field taller than every other control was the one
                        // thing that still made the compose card read as uneven.
                        let recipient = ui.add_enabled(
                            !busy,
                            egui::TextEdit::singleline(&mut state.draft.recipient)
                                .id(egui::Id::new("sms-compose-recipient"))
                                .hint_text(t(
                                    language,
                                    crate::localization::TextKey::ComposeRecipientHint,
                                ))
                                .desired_width(f32::INFINITY)
                                .margin(egui::vec2(12.0, 5.0)),
                        );
                        ui.label(crate::ui::meta_text(t(
                            language,
                            crate::localization::TextKey::ComposeRecipientNote,
                        )));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(t(
                                    language,
                                    crate::localization::TextKey::ComposeBodyLabel,
                                ))
                                .strong(),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(crate::ui::meta_text(
                                        crate::localization::format_positional(
                                            language,
                                            crate::localization::TextKey::ComposeLength,
                                            &[&state.draft.body.chars().count().to_string()],
                                        ),
                                    ));
                                },
                            );
                        });
                        let body = ui.add_enabled(
                            !busy,
                            egui::TextEdit::multiline(&mut state.draft.body)
                                .id(egui::Id::new("sms-compose-body"))
                                .hint_text(t(
                                    language,
                                    crate::localization::TextKey::ComposeBodyHint,
                                ))
                                .desired_width(f32::INFINITY)
                                .desired_rows(4)
                                .margin(egui::vec2(12.0, 12.0)),
                        );
                        if recipient.changed() || body.changed() {
                            state.edited();
                        }
                        if !busy {
                            match state.editor_focus.take() {
                                Some(EditorFocus::Recipient) => recipient.request_focus(),
                                Some(EditorFocus::Body) => body.request_focus(),
                                None => {}
                            }
                        }
                        ui.add_space(10.0);
                        egui::Frame::none()
                            .fill(crate::ui::scale::surface_sunken())
                            .rounding(8.0)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.label(crate::ui::meta_text(t(
                                    language,
                                    crate::localization::TextKey::ComposeLimits,
                                )));
                                ui.label(crate::ui::meta_text(t(
                                    language,
                                    crate::localization::TextKey::ComposeCostNote,
                                )));
                            });
                    });
                if let Some(error) = &state.error {
                    crate::ui::wrapped_label(
                        ui,
                        egui::RichText::new(error).color(super::StatusTone::Negative.color()),
                    );
                }
                ui.label(crate::ui::meta_text(if sending {
                    t(language, crate::localization::TextKey::ComposeSendingWait)
                } else if busy {
                    t(language, crate::localization::TextKey::ComposeModuleBusy)
                } else if state.error.is_some() {
                    t(language, crate::localization::TextKey::ComposeDraftKept)
                } else if !state.ready() {
                    t(language, crate::localization::TextKey::ComposeNeedInput)
                } else {
                    t(language, crate::localization::TextKey::ComposeDraftReady)
                }));
            },
            &[
                crate::ui::modal::DialogAction::primary(&t(
                    language,
                    crate::localization::TextKey::ComposeNextConfirm,
                ))
                .enabled(can_advance),
                crate::ui::modal::DialogAction::cancel(&t(
                    language,
                    crate::localization::TextKey::ButtonCancel,
                )),
            ],
        );
        if outcome.chosen() == Some(0) {
            state.confirmation = Some(state.draft.clone());
        } else if outcome.dismissed() {
            state.open = false;
            state.editor_focus = None;
        }
    }
    if let Some(frozen) = state.confirmation.clone() {
        // The review text is the only element allowed to grow, so it is the one that carries the
        // bound: the reserve covers the card's padding, its title block, the hairline and the
        // decision row. That keeps the card inside the viewport on a short window, and both
        // decisions on screen no matter how long the message is.
        let review_max_height = (ui.ctx().screen_rect().height() - 280.0).clamp(64.0, 260.0);
        let outcome = crate::ui::modal::show(
            ui.ctx(),
            &crate::ui::modal::Dialog::new(
                "sms-compose-confirmation",
                &t(language, crate::localization::TextKey::ComposeConfirmTitle),
            )
            .description(&t(
                language,
                crate::localization::TextKey::ComposeConfirmIntro,
            ))
            .width(460.0),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("sms-confirmation-content")
                    .max_height(review_max_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(&frozen.recipient).strong());
                        ui.separator();
                        ui.add(egui::Label::new(&frozen.body).wrap());
                    });
                ui.add(
                    egui::Label::new(t(
                        language,
                        crate::localization::TextKey::ComposeConfirmNote,
                    ))
                    .wrap(),
                );
            },
            &[
                crate::ui::modal::DialogAction::primary(&t(
                    language,
                    crate::localization::TextKey::ComposeConfirmAction,
                ))
                .enabled(!busy),
                crate::ui::modal::DialogAction::cancel(&t(
                    language,
                    crate::localization::TextKey::ButtonCancel,
                )),
            ],
        );
        if outcome.chosen() == Some(0) {
            state.confirm(sink, snapshot);
            ui.ctx().request_repaint();
        } else if outcome.dismissed() {
            state.confirmation = None;
        }
    }
    if state.replacement_draft.is_some() {
        let outcome = crate::ui::modal::show(
            ui.ctx(),
            &crate::ui::modal::Dialog::new(
                "sms-compose-keep-draft",
                &t(
                    language,
                    crate::localization::TextKey::ComposeKeepDraftTitle,
                ),
            )
            .description(&t(
                language,
                crate::localization::TextKey::ComposeKeepDraftBody,
            )),
            |_ui| {},
            &[
                crate::ui::modal::DialogAction::destructive(&t(
                    language,
                    crate::localization::TextKey::ComposeReplaceDraft,
                ))
                .enabled(!busy),
                crate::ui::modal::DialogAction::cancel(&t(
                    language,
                    crate::localization::TextKey::ComposeKeepDraft,
                )),
            ],
        );
        if outcome.chosen() == Some(0) {
            state.resolve_reply(true);
        } else if outcome.dismissed() {
            state.resolve_reply(false);
        }
    }
}

fn send_result_tone(snapshot: &SmsSendSnapshot) -> super::StatusTone {
    if snapshot.phase != SmsSendPhase::Finished {
        return super::StatusTone::Progress;
    }
    match snapshot.result {
        Some(SmsSendResult::Submitted) => super::StatusTone::Positive,
        Some(SmsSendResult::Failed) => super::StatusTone::Negative,
        Some(SmsSendResult::OutcomeUnknown) | None => super::StatusTone::Caution,
    }
}

fn submission_notice(
    result: Option<SmsSendResult>,
    failure: &dji4g_application::SmsFailureDetail,
    language: Language,
) -> String {
    if result == Some(SmsSendResult::Failed) && failure.code == "sms:module_rejected" {
        t(language, crate::localization::TextKey::ComposeRejected)
    } else if failure.submission_possible {
        t(language, crate::localization::TextKey::ComposeMaybeSent)
    } else {
        t(language, crate::localization::TextKey::ComposeNotSubmitted)
    }
}

fn phase_name(phase: SmsSendPhase, language: Language) -> String {
    match phase {
        SmsSendPhase::Queued => t(language, crate::localization::TextKey::ComposeStageQueued),
        SmsSendPhase::Preparing => t(
            language,
            crate::localization::TextKey::ComposeStagePreparing,
        ),
        SmsSendPhase::Submitting => t(
            language,
            crate::localization::TextKey::ComposeStageSubmitting,
        ),
        SmsSendPhase::WaitingForResult => {
            t(language, crate::localization::TextKey::ComposeStageAwaiting)
        }
        SmsSendPhase::Finished => t(language, crate::localization::TextKey::ComposeStageDone),
    }
}

fn failure_advice(code: &str, language: Language) -> String {
    match code {
        "sms:port_busy" => return t(language, crate::localization::TextKey::ComposeSerialBusy),
        "sms:port_open_failed" => {
            return t(
                language,
                crate::localization::TextKey::ComposeSerialOpenFailed,
            );
        }
        "sms:cleanup_timeout" => {
            return t(
                language,
                crate::localization::TextKey::ComposeSerialCloseTimeout,
            );
        }
        "sms:no_device" => return t(language, crate::localization::TextKey::ComposeNoDevice),
        "sms:device_changed" => {
            return t(
                language,
                crate::localization::TextKey::ComposeContextChanged,
            );
        }
        "sms:module_rejected" => {
            return t(
                language,
                crate::localization::TextKey::ComposeModuleRejected,
            );
        }
        "sms:transport_failed" => {
            return t(language, crate::localization::TextKey::ComposeSerialFailed);
        }
        "sms:timeout" => return t(language, crate::localization::TextKey::ComposeTimeout),
        "sms:missing_reference" => {
            return t(language, crate::localization::TextKey::ComposeNoReference);
        }
        "sms:unexpected_final_code" => {
            return t(language, crate::localization::TextKey::ComposeUnexpectedEnd);
        }
        _ => {}
    }
    if code.contains("lease") || code.contains("busy") {
        t(
            language,
            crate::localization::TextKey::ComposeSerialBusyShort,
        )
    } else if code.contains("open") || code.contains("access") {
        t(language, crate::localization::TextKey::ComposePortBusyShort)
    } else if code.contains("timeout") || code.contains("deadline") {
        t(language, crate::localization::TextKey::ComposeTimeoutShort)
    } else if code.contains("invalid") {
        t(
            language,
            crate::localization::TextKey::ComposeValidationFailed,
        )
    } else if code.contains("disconnect") || code.contains("closed") || code.contains("removed") {
        t(language, crate::localization::TextKey::ComposeDeviceLost)
    } else {
        t(language, crate::localization::TextKey::ComposeGenericAdvice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replying_only_prefills_the_original_recipient() {
        let mut state = SmsComposeState::default();
        assert_eq!(
            state.begin_reply("+8613800138000"),
            ReplyStartResult::Opened
        );
        assert_eq!(state.draft.recipient, "+8613800138000");
        assert!(state.draft.body.is_empty());
        assert!(state.open);
        assert!(state.confirmation.is_none());
        assert!(state.pending.is_none());
    }
    #[test]
    fn existing_draft_is_kept_until_explicit_replacement() {
        let mut state = ready();
        state.confirmation = None;
        let original = state.draft.clone();
        assert_eq!(
            state.begin_reply("+8613900000000"),
            ReplyStartResult::ReplacementConfirmation
        );
        assert!(state.draft == original);
        state.resolve_reply(false);
        assert!(state.draft == original);
        state.begin_reply("+8613900000000");
        state.resolve_reply(true);
        assert_eq!(state.draft.recipient, "+8613900000000");
        assert!(state.draft.body.is_empty());
    }

    #[test]
    fn balance_templates_only_prefill_and_require_explicit_draft_replacement() {
        for (recipient, body) in [("10086", "CXYE"), ("10010", "CXYE"), ("10001", "102")] {
            let mut empty = SmsComposeState::default();
            assert_eq!(
                empty.begin_balance_query(recipient, body),
                ReplyStartResult::Opened
            );
            assert_eq!(empty.draft.recipient, recipient);
            assert_eq!(empty.draft.body, body);
            assert!(empty.confirmation.is_none() && empty.pending.is_none());
            assert!(empty.open);

            let mut state = ready();
            state.confirmation = None;
            let original = state.draft.clone();
            assert_eq!(
                state.begin_balance_query(recipient, body),
                ReplyStartResult::ReplacementConfirmation
            );
            assert!(state.draft == original);
            state.resolve_reply(false);
            assert!(state.draft == original);
            assert_eq!(
                state.begin_balance_query(recipient, body),
                ReplyStartResult::ReplacementConfirmation
            );
            state.resolve_reply(true);
            assert_eq!(state.draft.recipient, recipient);
            assert_eq!(state.draft.body, body);
            assert!(state.confirmation.is_none() && state.pending.is_none());
        }
    }

    #[test]
    fn busy_frozen_and_context_changed_balance_templates_keep_the_existing_draft() {
        for guard in ["serial", "read only", "confirmation", "replacement"] {
            let mut state = ready();
            state.confirmation = None;
            let original = state.draft.clone();
            match guard {
                "serial" => state.serial_busy = true,
                "read only" => state.module_read_only = true,
                "confirmation" => state.confirmation = Some(original.clone()),
                _ => {
                    state.replacement_draft = Some(Draft {
                        recipient: "10086".into(),
                        body: "CXYE".into(),
                    })
                }
            }
            assert_eq!(
                state.begin_balance_query("10010", "CXYE"),
                ReplyStartResult::Busy
            );
            assert!(state.draft == original);
        }
        let mut state = ready();
        state.confirmation = None;
        state.observe_context((Some(dji4g_domain::DeviceEpoch(1)), 0));
        let original = state.draft.clone();
        assert_eq!(
            state.begin_balance_query("10086", "CXYE"),
            ReplyStartResult::ReplacementConfirmation
        );
        state.observe_context((Some(dji4g_domain::DeviceEpoch(1)), 1));
        state.resolve_reply(true);
        assert!(state.draft == original);
        assert!(state.replacement_draft.is_none());
    }

    #[test]
    fn local_inbox_changes_and_search_guards_never_modify_a_draft() {
        let mut state = ready();
        let original = state.draft.clone();
        state.search = "local query".into();
        state.inbox_filter = super::super::InboxFilter::Unread;
        state.outgoing = true;
        assert!(!state.search_shortcuts_allowed());
        state.confirmation = None;
        assert!(state.search_shortcuts_allowed());
        state.open_editor();
        assert!(!state.search_shortcuts_allowed());
        state.open = false;
        state.storage_confirmation = Some((None, 0, dji4g_domain::SmsStorageId("SM".into())));
        assert!(!state.search_shortcuts_allowed());
        state.storage_confirmation = None;
        state.begin_balance_query("10086", "CXYE");
        assert!(!state.search_shortcuts_allowed());
        assert!(state.draft == original);
    }
    #[test]
    fn invalid_sender_and_busy_or_frozen_send_cannot_replace_draft() {
        let mut state = ready();
        let original = state.draft.clone();
        assert_eq!(state.begin_reply("+8613900000000"), ReplyStartResult::Busy);
        assert!(state.draft == original);
        assert!(state.confirmation.is_some());
        state.confirmation = None;
        assert_eq!(
            state.begin_reply("BANK"),
            ReplyStartResult::InvalidRecipient
        );
        assert!(state.replacement_draft.is_none());
        state.serial_busy = true;
        assert_eq!(state.begin_reply("+8613900000000"), ReplyStartResult::Busy);
        assert!(state.replacement_draft.is_none());
        assert!(state.draft == original);
    }

    #[test]
    fn unrecognized_service_and_national_numbers_are_not_normalized_into_reply_recipients() {
        for recipient in [
            "1008611",
            "10690000",
            "13800138000",
            "BANK",
            " +8613800138000",
        ] {
            let mut state = SmsComposeState::default();
            assert_eq!(
                state.begin_reply(recipient),
                ReplyStartResult::InvalidRecipient
            );
            assert!(state.draft.recipient.is_empty());
            assert!(state.replacement_draft.is_none());
            assert!(!state.open);
        }
    }

    #[test]
    fn device_or_sim_change_clears_waiting_reply_replacement_and_preserves_draft() {
        for next in [
            (Some(dji4g_domain::DeviceEpoch(2)), 0),
            (Some(dji4g_domain::DeviceEpoch(1)), 1),
        ] {
            let mut state = ready();
            state.confirmation = None;
            state.observe_context((Some(dji4g_domain::DeviceEpoch(1)), 0));
            let original = state.draft.clone();
            assert_eq!(
                state.begin_reply("+8613900000000"),
                ReplyStartResult::ReplacementConfirmation
            );
            state.observe_context(next);
            assert!(state.replacement_draft.is_none());
            state.resolve_reply(true);
            assert!(state.draft == original);
            assert!(state.confirmation.is_none());
        }
    }

    #[test]
    fn replaced_reply_sends_only_the_exact_frozen_recipient_and_body() {
        struct Capture(std::sync::Mutex<Vec<(String, String)>>);
        impl UiCommandSink for Capture {
            fn try_send(&self, command: UiCommand) -> Result<(), UiSendError> {
                if let UiCommand::SmsSend { recipient, body } = command {
                    self.0.lock().unwrap().push((recipient, body));
                }
                Ok(())
            }
        }
        let sink = Capture(std::sync::Mutex::new(Vec::new()));
        for changed_field in [None, Some("recipient"), Some("body")] {
            let mut state = ready();
            state.confirmation = None;
            assert_eq!(
                state.begin_reply("+8613900000000"),
                ReplyStartResult::ReplacementConfirmation
            );
            state.resolve_reply(true);
            state.draft.body = "明确确认的回复".into();
            state.confirmation = Some(state.draft.clone());
            match changed_field {
                Some("recipient") => state.draft.recipient = "+8613700000000".into(),
                Some("body") => state.draft.body.push('新'),
                _ => {}
            }
            state.confirm(&sink, None);
            if changed_field.is_none() {
                assert_eq!(state.begin_reply("+8613600000000"), ReplyStartResult::Busy);
            } else {
                assert!(state.pending.is_none());
            }
        }
        assert_eq!(
            *sink.0.lock().unwrap(),
            vec![("+8613900000000".to_owned(), "明确确认的回复".to_owned())]
        );
    }
    #[test]
    fn sending_and_three_results_have_distinct_tones() {
        assert_eq!(
            send_result_tone(&done(1, SmsSendResult::Submitted)),
            super::super::StatusTone::Positive
        );
        assert_eq!(
            send_result_tone(&done(1, SmsSendResult::Failed)),
            super::super::StatusTone::Negative
        );
        assert_eq!(
            send_result_tone(&done(1, SmsSendResult::OutcomeUnknown)),
            super::super::StatusTone::Caution
        );
        let mut sending = done(1, SmsSendResult::Submitted);
        sending.phase = SmsSendPhase::WaitingForResult;
        sending.result = None;
        assert_eq!(
            send_result_tone(&sending),
            super::super::StatusTone::Progress
        );
    }
    #[test]
    fn explicit_module_rejection_overrides_body_write_uncertainty_in_notice() {
        let failure = dji4g_application::SmsFailureDetail::new(
            SmsSendPhase::WaitingForResult,
            "sms:module_rejected",
            true,
        );
        assert!(
            submission_notice(Some(SmsSendResult::Failed), &failure, Language::ZhCn)
                .contains("明确拒绝")
        );
        assert!(failure.submission_possible);
        assert!(
            submission_notice(
                Some(SmsSendResult::OutcomeUnknown),
                &failure,
                Language::ZhCn
            )
            .contains("可能已经提交")
        );
    }
    struct Sink(Result<(), UiSendError>);
    impl UiCommandSink for Sink {
        fn try_send(&self, _: UiCommand) -> Result<(), UiSendError> {
            self.0.clone()
        }
    }
    fn ready() -> SmsComposeState {
        let mut state = SmsComposeState::default();
        state.draft = Draft {
            recipient: "+8613800138000".into(),
            body: "测试".into(),
        };
        state.confirmation = Some(state.draft.clone());
        state
    }
    #[test]
    fn edit_invalidates_confirmation() {
        let mut s = ready();
        s.draft.body.push('新');
        s.edited();
        assert!(s.confirmation.is_none());
    }
    #[test]
    fn queue_errors_preserve_draft_and_unlock() {
        for error in [UiSendError::QueueFull, UiSendError::Closed] {
            let mut s = ready();
            let draft = s.draft.clone();
            s.confirm(&Sink(Err(error)), None);
            assert!(s.draft == draft);
            assert!(s.pending.is_none());
            assert!(s.error.is_some());
        }
    }
    #[test]
    fn acceptance_locks_and_preserves_draft() {
        let mut s = ready();
        s.confirm(&Sink(Ok(())), None);
        assert!(s.pending.is_some());
        assert_eq!(s.draft.body, "测试");
    }
    #[test]
    fn shared_validation_rejects_invalid_payload() {
        let mut s = ready();
        s.draft.recipient = "123".into();
        assert!(!s.ready());
        s = ready();
        s.draft.body = "😀".into();
        assert!(!s.ready());
    }
    fn done(id: u64, result: SmsSendResult) -> SmsSendSnapshot {
        SmsSendSnapshot {
            request_id: id,
            phase: SmsSendPhase::Finished,
            result: Some(result),
            failure: None,
        }
    }
    #[test]
    fn only_new_matching_success_clears_original_draft() {
        let old = done(8, SmsSendResult::Submitted);
        let mut s = ready();
        s.confirm(&Sink(Ok(())), Some(&old));
        s.synchronize(Some(&old));
        assert!(s.pending.is_some());
        assert_eq!(s.draft.body, "测试");
        s.synchronize(Some(&done(9, SmsSendResult::Submitted)));
        assert!(s.pending.is_none());
        assert!(s.draft.body.is_empty());
    }
    #[test]
    fn edits_during_send_survive_success() {
        let mut s = ready();
        s.confirm(&Sink(Ok(())), None);
        s.draft.body = "下一条".into();
        s.synchronize(Some(&done(1, SmsSendResult::Submitted)));
        assert_eq!(s.draft.body, "下一条");
    }
    #[test]
    fn failure_and_unknown_preserve_original_draft() {
        for result in [SmsSendResult::Failed, SmsSendResult::OutcomeUnknown] {
            let mut s = ready();
            s.confirm(&Sink(Ok(())), None);
            s.synchronize(Some(&done(1, result)));
            assert_eq!(s.draft.body, "测试");
            assert!(s.pending.is_none());
        }
    }
    #[test]
    fn duplicate_confirmation_does_not_enqueue_twice() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counting(AtomicUsize);
        impl UiCommandSink for Counting {
            fn try_send(&self, _: UiCommand) -> Result<(), UiSendError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
        let sink = Counting(AtomicUsize::new(0));
        let mut s = ready();
        s.confirm(&sink, None);
        s.confirmation = Some(s.draft.clone());
        s.confirm(&sink, None);
        assert_eq!(sink.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn newer_send_rejection_unlocks_and_keeps_draft() {
        let mut s = ready();
        s.observe_send_rejection(4);
        s.confirm(&Sink(Ok(())), None);
        s.observe_send_rejection(4);
        assert!(s.pending.is_some());
        s.observe_send_rejection(5);
        assert!(s.pending.is_none());
        assert_eq!(s.draft.body, "测试");
        assert!(s.error.is_some());
    }

    #[test]
    fn unrelated_rejection_does_not_unlock_bound_send() {
        let mut s = ready();
        s.confirm(&Sink(Ok(())), None);
        let active = SmsSendSnapshot {
            request_id: 1,
            phase: SmsSendPhase::Preparing,
            result: None,
            failure: None,
        };
        s.synchronize(Some(&active));
        s.observe_send_rejection(1);
        assert!(s.pending.is_some());
    }

    #[test]
    fn stale_device_success_does_not_clear_draft() {
        let mut s = ready();
        s.observe_context((Some(dji4g_domain::DeviceEpoch(1)), 0));
        s.confirm(&Sink(Ok(())), None);
        s.observe_context((Some(dji4g_domain::DeviceEpoch(2)), 0));
        s.synchronize(Some(&done(1, SmsSendResult::Submitted)));
        assert_eq!(s.draft.body, "测试");
        assert!(s.pending.is_none());
    }

    #[test]
    fn exact_frozen_payload_is_sent_and_unannounced_edit_is_rejected() {
        use std::sync::Mutex;
        struct Capture(Mutex<Vec<(String, String)>>);
        impl UiCommandSink for Capture {
            fn try_send(&self, command: UiCommand) -> Result<(), UiSendError> {
                if let UiCommand::SmsSend { recipient, body } = command {
                    self.0.lock().unwrap().push((recipient, body));
                }
                Ok(())
            }
        }
        let sink = Capture(Mutex::new(Vec::new()));
        let mut s = ready();
        s.confirm(&sink, None);
        assert_eq!(
            sink.0.lock().unwrap().as_slice(),
            &[("+8613800138000".to_owned(), "测试".to_owned())]
        );
        let mut changed = ready();
        changed.draft.body.push('新');
        changed.confirm(&sink, None);
        assert_eq!(sink.0.lock().unwrap().len(), 1);
    }
    #[test]
    fn automatic_refresh_is_bounded_and_keeps_draft_selection() {
        struct RefreshSink(std::sync::atomic::AtomicUsize);
        impl UiCommandSink for RefreshSink {
            fn try_send(&self, command: UiCommand) -> Result<(), UiSendError> {
                assert!(matches!(command, UiCommand::SmsRefresh));
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }
        let sink = RefreshSink(std::sync::atomic::AtomicUsize::new(0));
        let now = std::time::Instant::now();
        let mut state = ready();
        state.confirmation = None;
        state.selected = Some([7; 32]);
        let draft = state.draft.clone();
        state.auto_refresh(now, false, false, false, &sink);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        state.auto_refresh(now, true, false, false, &sink);
        state.auto_refresh(now, true, false, false, &sink);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        let later = now + std::time::Duration::from_secs(16);
        state.auto_refresh(later, true, true, false, &sink);
        state.auto_refresh(later, true, false, true, &sink);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        state.auto_refresh(later, true, false, false, &sink);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(state.draft == draft);
        assert_eq!(state.selected, Some([7; 32]));
    }

    #[test]
    fn a_confirmed_submission_refreshes_once_when_the_inbox_becomes_idle() {
        struct RefreshSink(std::sync::atomic::AtomicUsize);
        impl UiCommandSink for RefreshSink {
            fn try_send(&self, command: UiCommand) -> Result<(), UiSendError> {
                assert!(matches!(command, UiCommand::SmsRefresh));
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }
        let sink = RefreshSink(std::sync::atomic::AtomicUsize::new(0));
        let now = std::time::Instant::now();
        let mut state = ready();
        state.last_refresh_attempt = Some(now);
        state.confirm(&Sink(Ok(())), None);
        state.auto_refresh(now, true, false, false, &sink);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        state.synchronize(Some(&done(1, SmsSendResult::Submitted)));
        assert!(state.last_refresh_attempt.is_none());
        state.auto_refresh(now, true, true, false, &sink);
        state.auto_refresh(now, true, false, true, &sink);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        state.auto_refresh(now, true, false, false, &sink);
        state.auto_refresh(now, true, false, false, &sink);
        assert_eq!(sink.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn automatic_refresh_waits_for_editor_and_confirmations_then_resumes_once() {
        struct RefreshSink(std::sync::atomic::AtomicUsize);
        impl UiCommandSink for RefreshSink {
            fn try_send(&self, command: UiCommand) -> Result<(), UiSendError> {
                assert!(matches!(command, UiCommand::SmsRefresh));
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }

        for dialog in [
            "editor",
            "send confirmation",
            "replace draft",
            "storage location",
        ] {
            let sink = RefreshSink(std::sync::atomic::AtomicUsize::new(0));
            let now = std::time::Instant::now();
            let mut state = ready();
            let draft = state.draft.clone();
            state.confirmation = None;
            match dialog {
                "editor" => state.open_editor(),
                "send confirmation" => state.confirmation = Some(state.draft.clone()),
                "replace draft" => {
                    state.replacement_draft = Some(Draft {
                        recipient: "+8613900000000".into(),
                        body: String::new(),
                    })
                }
                _ => {
                    state.storage_confirmation =
                        Some((None, 0, dji4g_domain::SmsStorageId("SM".into())))
                }
            }
            state.auto_refresh(now, true, false, false, &sink);
            let later = now + std::time::Duration::from_secs(16);
            state.auto_refresh(later, true, false, false, &sink);
            assert_eq!(
                sink.0.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "{dialog}"
            );
            assert!(state.draft == draft);

            state.open = false;
            state.confirmation = None;
            state.replacement_draft = None;
            state.storage_confirmation = None;
            state.auto_refresh(later, true, false, false, &sink);
            state.auto_refresh(later, true, false, false, &sink);
            assert_eq!(
                sink.0.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "{dialog}"
            );
            assert!(state.draft == draft);
        }
    }

    #[test]
    fn editor_focus_starts_on_recipient_or_reply_body_and_does_not_repeat() {
        let snapshot =
            dji4g_application::Controller::for_test(std::time::SystemTime::UNIX_EPOCH).snapshot();
        let context = egui::Context::default();
        let mut state = SmsComposeState::default();
        state.open_editor();
        let render_frame = |state: &mut SmsComposeState| {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 700.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        render(ui, Language::ZhCn, state, &snapshot, &Sink(Ok(())));
                    });
                },
            );
        };
        render_frame(&mut state);
        assert_eq!(
            context.memory(|memory| memory.focused()),
            Some(egui::Id::new("sms-compose-recipient"))
        );
        context.memory_mut(|memory| memory.request_focus(egui::Id::new("sms-compose-body")));
        render_frame(&mut state);
        assert_eq!(
            context.memory(|memory| memory.focused()),
            Some(egui::Id::new("sms-compose-body"))
        );

        state.open = false;
        assert_eq!(
            state.begin_reply("+8613800138000"),
            ReplyStartResult::Opened
        );
        render_frame(&mut state);
        assert_eq!(
            context.memory(|memory| memory.focused()),
            Some(egui::Id::new("sms-compose-body"))
        );
        assert!(state.editor_focus.is_none());
    }

    #[test]
    fn send_rejection_survives_feedback_clearing_before_any_ui_observation() {
        let mut controller =
            dji4g_application::Controller::for_test(std::time::SystemTime::UNIX_EPOCH);
        let mut state = ready();
        state.observe_snapshot(&controller.snapshot());
        let draft = state.draft.clone();
        state.confirm(&Sink(Ok(())), None);
        assert!(state.pending.is_some());
        controller.set_sms_refresh_pending(true);
        assert!(
            controller
                .handle_command(UiCommand::SmsSend {
                    recipient: draft.recipient.clone(),
                    body: draft.body.clone(),
                })
                .is_err()
        );
        // The watch receiver coalesces both updates: the UI never sees the feedback notice.
        controller
            .handle_command(UiCommand::ClearToolHistory)
            .unwrap();
        let snapshot = controller.snapshot();
        assert!(snapshot.feedback.is_none());
        assert!(snapshot.sms_send.is_none());
        assert_eq!(snapshot.command_state.sms_send_rejected_seq, 1);
        state.observe_snapshot(&snapshot);
        assert!(state.pending.is_none());
        assert!(state.draft == draft);
        assert!(
            state
                .error
                .as_ref()
                .is_some_and(|error| error.contains("未提交"))
        );
        state.observe_snapshot(&snapshot);
        assert!(state.pending.is_none());
        assert!(state.draft == draft);
    }

    #[test]
    fn a_read_rejection_cannot_unlock_an_unbound_send() {
        let mut controller =
            dji4g_application::Controller::for_test(std::time::SystemTime::UNIX_EPOCH);
        let mut state = ready();
        state.observe_snapshot(&controller.snapshot());
        state.confirm(&Sink(Ok(())), None);
        controller.set_sms_refresh_pending(true);
        assert!(
            controller
                .handle_command(UiCommand::SmsReadStorage {
                    storage: dji4g_domain::SmsStorageId("ME".into())
                })
                .is_err()
        );
        let snapshot = controller.snapshot();
        assert_eq!(
            snapshot.feedback.as_ref().unwrap().code.stable.as_str(),
            "sms:busy"
        );
        assert_eq!(snapshot.command_state.sms_send_rejected_seq, 0);
        state.observe_snapshot(&snapshot);
        assert!(state.pending.is_some());
        assert!(state.error.is_none());
    }

    #[test]
    fn accepted_send_binds_before_a_later_send_rejection_is_observed() {
        let mut controller =
            dji4g_application::Controller::for_test(std::time::SystemTime::UNIX_EPOCH);
        let mut state = ready();
        state.observe_snapshot(&controller.snapshot());
        let draft = state.draft.clone();
        state.confirm(&Sink(Ok(())), None);
        let send = || UiCommand::SmsSend {
            recipient: draft.recipient.clone(),
            body: draft.body.clone(),
        };
        controller.handle_command(send()).unwrap();
        assert!(controller.handle_command(send()).is_err());
        controller
            .handle_command(UiCommand::ClearToolHistory)
            .unwrap();
        let snapshot = controller.snapshot();
        assert!(snapshot.feedback.is_none());
        assert_eq!(snapshot.command_state.sms_send_rejected_seq, 1);
        state.observe_snapshot(&snapshot);
        assert_eq!(state.pending.as_ref().unwrap().request_id, Some(1));
        assert!(state.error.is_none());
        assert!(state.draft == draft);
    }

    #[test]
    fn hidden_context_change_invalidates_storage_confirmation_and_keeps_draft() {
        let mut snapshot =
            dji4g_application::Controller::for_test(std::time::SystemTime::UNIX_EPOCH).snapshot();
        let mut state = ready();
        state.observe_snapshot(&snapshot);
        let draft = state.draft.clone();
        state.storage_confirmation = Some((
            snapshot.app.device.as_ref().map(|device| device.epoch),
            snapshot.sim_epoch,
            dji4g_domain::SmsStorageId("SM".into()),
        ));
        snapshot.sim_epoch += 1;
        state.observe_snapshot(&snapshot);
        assert!(state.storage_confirmation.is_none());
        assert!(state.confirmation.is_none());
        assert!(state.draft == draft);
    }

    #[test]
    fn confirmation_fits_the_viewport_and_both_decisions_remain_clickable() {
        struct RecordingSink(std::sync::Mutex<Vec<UiCommand>>);
        impl UiCommandSink for RecordingSink {
            fn try_send(&self, command: UiCommand) -> Result<(), UiSendError> {
                self.0.lock().unwrap().push(command);
                Ok(())
            }
        }
        for size in [
            egui::vec2(552.0, 400.0),
            egui::vec2(800.0, 600.0),
            egui::vec2(1100.0, 760.0),
        ] {
            for body in [
                "测试".to_owned(),
                "确认的正文".repeat(14),
                "验\n".repeat(35),
            ] {
                for confirm in [false, true] {
                    let snapshot =
                        dji4g_application::Controller::for_test(std::time::SystemTime::UNIX_EPOCH)
                            .snapshot();
                    let context = egui::Context::default();
                    crate::ui::initialize_visuals(&context);
                    context.style_mut(|style| style.animation_time = 0.0);
                    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                    let mut state = ready();
                    state.open_editor();
                    state.draft.body = body.clone();
                    state.confirmation = Some(state.draft.clone());
                    let draft = state.draft.clone();
                    let sink = RecordingSink(Default::default());
                    let mut action = None;
                    for tick in 0..6 {
                        let events = if tick == 3 || tick == 4 {
                            let point = action.expect("the confirmation action must be visible");
                            vec![
                                egui::Event::PointerMoved(point),
                                egui::Event::PointerButton {
                                    pos: point,
                                    button: egui::PointerButton::Primary,
                                    pressed: tick == 3,
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ]
                        } else {
                            vec![]
                        };
                        let output = context.run(
                            egui::RawInput {
                                screen_rect: Some(screen),
                                events,
                                ..Default::default()
                            },
                            |context| {
                                egui::CentralPanel::default().show(context, |ui| {
                                    render(ui, Language::ZhCn, &mut state, &snapshot, &sink);
                                });
                            },
                        );
                        if tick == 2 {
                            let window = context
                                .read_response(egui::Id::new("sms-compose-confirmation"))
                                .expect("the confirmation window must be rendered")
                                .rect;
                            assert!(screen.contains_rect(window), "{size:?}: {window:?}");
                            let cancel_label =
                                t(Language::ZhCn, crate::localization::TextKey::ButtonCancel);
                            let confirm_label = t(
                                Language::ZhCn,
                                crate::localization::TextKey::ComposeConfirmAction,
                            );
                            for label in [&cancel_label, &confirm_label] {
                                // Match the decision *inside the confirmation window*: the label
                                // 取消 also appears elsewhere on the page, and shapes are emitted in
                                // paint order, so taking the first match picked the wrong one.
                                let text = output.shapes.iter().find_map(|shape| {
                                    if let egui::Shape::Text(text) = &shape.shape
                                        && text.galley.text() == label
                                    {
                                        let rect =
                                            egui::Rect::from_min_size(text.pos, text.galley.size());
                                        window.contains_rect(rect).then_some(rect)
                                    } else {
                                        None
                                    }
                                });
                                let text = text.unwrap_or_else(|| {
                                    let seen = output
                                        .shapes
                                        .iter()
                                        .filter_map(|shape| match &shape.shape {
                                            egui::Shape::Text(text) => {
                                                Some(text.galley.text().to_owned())
                                            }
                                            _ => None,
                                        })
                                        .collect::<Vec<_>>();
                                    panic!(
                                        "{size:?}: {label:?} not inside {window:?}; \
                                         texts on screen: {seen:?}"
                                    )
                                });
                                assert!(screen.contains_rect(text));
                                assert!(window.contains_rect(text));
                                if label
                                    == if confirm {
                                        confirm_label.as_str()
                                    } else {
                                        cancel_label.as_str()
                                    }
                                {
                                    action = Some(text.center());
                                }
                            }
                        }
                    }
                    assert!(state.confirmation.is_none());
                    assert!(state.draft == draft);
                    assert_eq!(sink.0.lock().unwrap().len(), usize::from(confirm));
                    assert_eq!(state.pending.is_some(), confirm);
                }
            }
        }
    }

    #[test]
    fn queue_failure_is_visible_inside_the_editor_and_keeps_the_draft() {
        for failure in [UiSendError::QueueFull, UiSendError::Closed] {
            let snapshot =
                dji4g_application::Controller::for_test(std::time::SystemTime::UNIX_EPOCH)
                    .snapshot();
            let context = egui::Context::default();
            let mut state = ready();
            state.open_editor();
            let draft = state.draft.clone();
            state.confirm(&Sink(Err(failure.clone())), None);
            let expected = enqueue_error(failure, Language::ZhCn);
            let mut visible_error = None;
            for _ in 0..2 {
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 700.0),
                        )),
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            render(ui, Language::ZhCn, &mut state, &snapshot, &Sink(Ok(())));
                        });
                    },
                );
                visible_error = output.shapes.iter().find_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text() == expected
                    {
                        Some(text.pos + text.galley.size() * 0.5)
                    } else {
                        None
                    }
                });
            }
            let point = visible_error.expect("queue error should be drawn in the editor");
            // The editor is the panel's modal now, so its text lives in the dialog's foreground
            // layer rather than in a `egui::Window`'s middle layer.
            assert_eq!(
                context.layer_id_at(point),
                Some(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("sms-compose-window"),
                ))
            );
            assert!(state.draft == draft);
            assert!(state.open);
            assert!(state.confirmation.is_none());
            assert!(state.pending.is_none());
        }
    }
}
