//! SMS page: aggregate inbox state from the snapshot plus the stored-message list.
//!
//! `ControllerSnapshot` carries the merged message view (`sms_messages`, a read-only copy whose
//! `SmsMessage` redacts sender/body in `Debug`/`Serialize`); this page only projects it for
//! display, masks senders by default, and never logs or exports message content.
//!
//! Layout: inbox/outgoing filters share a responsive list/detail view below a compact stats strip;
//! the New SMS action opens a UI-owned draft with an explicit frozen confirmation. Outgoing records are local
//! bookkeeping — their `index` is a locally assigned transaction id, not a module storage index,
//! so their rows never issue module commands (no `SmsRead`, no `SmsDelete`) and only present the
//! submission status.

use std::time::{Duration, Instant};
#[path = "sms_compose.rs"]
mod compose;
pub(crate) use compose::{ReplyStartResult, SmsComposeState};

use dji4g_application::{ControllerSnapshot, UiCommand};
use dji4g_domain::{
    FeatureStatus, SmsDeleteItemResult, SmsDirection, SmsDisplayMessage, SmsEncoding,
    SmsFragmentKey, SmsMessage, SmsStatus,
};
use eframe::egui::{self, RichText, Ui};

use super::{StatusTone, meta_text, scale, sms_layout, wrapped_label};
use crate::app::UiCommandSink;
use crate::localization::{
    Language, LocalizedText, TextArgs, TextKey, feature_status_note, format_text_in,
};

/// One catalog string in the language this page was rendered with.
fn t(language: crate::localization::Language, key: crate::localization::TextKey) -> String {
    crate::localization::LocalizedText::new(language, key).text
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum InboxFilter {
    #[default]
    All,
    Unread,
    Incomplete,
}

impl InboxFilter {
    fn label(self) -> TextKey {
        match self {
            Self::All => TextKey::SmsFilterAll,
            Self::Unread => TextKey::SmsUnread,
            Self::Incomplete => TextKey::SmsFilterIncomplete,
        }
    }

    fn matches(self, row: &SmsRowVm) -> bool {
        match self {
            Self::All => true,
            Self::Unread => row.unread == Some(true),
            Self::Incomplete => row.status == SmsStatus::Incomplete,
        }
    }
}

fn filtered_rows<'a>(rows: &'a [SmsRowVm], state: &SmsComposeState) -> Vec<&'a SmsRowVm> {
    let query = state.search.trim().to_lowercase();
    rows.iter()
        .filter(|row| {
            (row.direction == SmsDirection::Outgoing) == state.outgoing
                && (state.outgoing || state.inbox_filter.matches(row))
                && (query.is_empty()
                    || row.sender_full.to_lowercase().contains(&query)
                    || row.body.to_lowercase().contains(&query))
        })
        .collect()
}

fn clear_hidden_selection(rows: &[&SmsRowVm], state: &mut SmsComposeState) {
    if state
        .selected
        .is_some_and(|key| !rows.iter().any(|row| row.stable_id == key))
    {
        state.selected = None;
    }
}

/// One projected stored message. Sender and body are carried verbatim only inside this UI-local
/// value; the list projection never feeds a log, a diagnostic export, or any serialized document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmsRowVm {
    pub index: u32,
    pub stable_id: [u8; 32],
    pub fragments: Vec<SmsFragmentKey>,
    pub delete_allowed: bool,
    /// `Some(true)` unread, `Some(false)` read, `None` while the read state is unknown.
    pub unread: Option<bool>,
    pub sender_masked: String,
    /// Full sender, only revealed after an explicit row click.
    pub sender_full: String,
    pub body: String,
    pub timestamp: Option<String>,
    pub encoding: SmsEncoding,
    /// `(sequence, total)` of a long message, when the PDU carried a concatenation header.
    pub multipart: Option<(u8, u8)>,
    pub status: SmsStatus,
    /// Outgoing rows are local submission records, not module-stored mail.
    pub direction: SmsDirection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmsVm {
    pub title: LocalizedText,
    pub intro: LocalizedText,
    pub status: LocalizedText,
    pub message_count: usize,
    pub unread_count: usize,
    pub capacity: Option<(u32, u32)>,
    pub capacity_text: Option<LocalizedText>,
    pub has_incomplete: bool,
    /// Local-cache evictions reported by the store; non-zero means the local view is
    /// incomplete even though the module may still hold the messages.
    pub evicted: u32,
    pub incomplete_warning: LocalizedText,
    pub empty_text: LocalizedText,
    pub list_pending_text: LocalizedText,
    pub rows: Vec<SmsRowVm>,
}

/// Classification text for the inbox-wide [`FeatureStatus`]: a never-probed store reads
/// 「尚未查询」, a completed read reads 「已读取」, and every classified failure keeps its
/// precise note so 「没有数据」 and 「查询失败」 are never confused.
#[must_use]
pub fn sms_status_text(status: FeatureStatus, language: Language) -> LocalizedText {
    match status {
        FeatureStatus::NotProbed => LocalizedText::new(language, TextKey::SmsStatusNotQueried),
        FeatureStatus::Supported | FeatureStatus::Empty => {
            LocalizedText::new(language, TextKey::SmsStatusRead)
        }
        classified => feature_status_note(classified).map_or_else(
            || LocalizedText::new(language, TextKey::SmsStatusNotQueried),
            |key| LocalizedText::new(language, key),
        ),
    }
}

/// Encoding display vocabulary: GSM7/UCS2 are protocol names and stay verbatim; anything the
/// codec does not model is the localized 「其他」.
#[must_use]
pub fn sms_encoding_text(encoding: SmsEncoding, language: Language) -> String {
    match encoding {
        SmsEncoding::Gsm7 => "GSM7".to_owned(),
        SmsEncoding::Ucs2 => "UCS2".to_owned(),
        SmsEncoding::Other => LocalizedText::new(language, TextKey::SmsEncodingOther).text,
    }
}

/// One presentation tag on a row's preview line. `tone` stays neutral unless the message carries
/// a real problem (an incomplete long message), so warning colour keeps its meaning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmsRowTag {
    pub text: String,
    pub tone: StatusTone,
}

/// Read-state presentation for the row's leading dot: a filled progress dot only for unread mail,
/// a hollow neutral one for read or unknown state.
#[must_use]
pub fn row_read_state(unread: Option<bool>) -> (StatusTone, &'static str, TextKey) {
    match unread {
        Some(true) => (StatusTone::Progress, "●", TextKey::SmsUnread),
        Some(false) => (StatusTone::Neutral, "○", TextKey::SmsRead),
        None => (StatusTone::Neutral, "○", TextKey::ValueUnknown),
    }
}

/// Submission status of one outgoing record: the only thing the module proved about it.
/// `Received`/`Incomplete` are inbox vocabulary and are never produced for outgoing records;
/// the impossible combination stays honestly 「未知」.
#[must_use]
pub fn outgoing_state(status: SmsStatus) -> (StatusTone, TextKey) {
    match status {
        SmsStatus::Submitted => (StatusTone::Positive, TextKey::SmsOutgoingSubmitted),
        SmsStatus::Failed => (StatusTone::Negative, TextKey::SmsOutgoingFailed),
        SmsStatus::OutcomeUnknown | SmsStatus::Received | SmsStatus::Incomplete => {
            (StatusTone::Caution, TextKey::SmsOutgoingUnknown)
        }
    }
}

#[cfg(test)]
#[test]
fn an_unknown_submission_is_a_caution_not_a_neutral_receipt() {
    assert_eq!(
        outgoing_state(SmsStatus::OutcomeUnknown).0,
        StatusTone::Caution
    );
}

/// Right-aligned vocabulary of one row, ordered left to right: encoding, fragment count, then the
/// incomplete warning. Only the warning carries a warning tone. Outgoing rows carry none of it:
/// the send receipt reports no encoding, concatenation headers never apply, and the status label
/// in the row's first line already says the rest.
#[must_use]
pub fn row_tags(row: &SmsRowVm, language: Language) -> Vec<SmsRowTag> {
    if row.direction == SmsDirection::Outgoing {
        return Vec::new();
    }
    let mut tags = vec![SmsRowTag {
        text: sms_encoding_text(row.encoding, language),
        tone: StatusTone::Neutral,
    }];
    if let Some((_, total)) = row.multipart {
        tags.push(SmsRowTag {
            text: crate::localization::format_positional(
                language,
                crate::localization::TextKey::SmsFragmentsRead,
                &[&row.fragments.len().to_string(), &total.to_string()],
            ),
            tone: StatusTone::Neutral,
        });
    }
    if row.status == SmsStatus::Incomplete {
        tags.push(SmsRowTag {
            text: LocalizedText::new(language, TextKey::SmsIncompleteTag).text,
            tone: StatusTone::Caution,
        });
    }
    tags
}

/// Project one stored message. The sender defaults to its masked form; the full sender and body
/// are placed in the VM so an explicit row click can reveal/copy them in place.
#[must_use]
pub fn sms_row_vm(message: &SmsMessage) -> SmsRowVm {
    let fragments = if message.direction == SmsDirection::Incoming {
        vec![message.fragment_key()]
    } else {
        Vec::new()
    };
    let display = SmsDisplayMessage {
        message: message.clone(),
        fragments: fragments.clone(),
        delete_allowed: message.direction == SmsDirection::Incoming,
    };
    SmsRowVm {
        index: message.index,
        stable_id: display.stable_id(),
        fragments,
        delete_allowed: display.delete_allowed,
        unread: message.read.map(|read| !read),
        sender_masked: message.sender_masked(),
        sender_full: message.sender().to_owned(),
        body: message.body().to_owned(),
        timestamp: message.service_centre_timestamp.clone(),
        encoding: message.encoding,
        multipart: message
            .multipart
            .map(|multipart| (multipart.sequence, multipart.total)),
        status: message.status,
        direction: message.direction,
    }
}

fn display_row_vm(message: &SmsDisplayMessage) -> SmsRowVm {
    let mut row = sms_row_vm(&message.message);
    row.stable_id = message.stable_id();
    row.fragments = message.fragments.clone();
    row.delete_allowed = message.delete_allowed;
    row
}

#[must_use]
pub fn sms_vm(snapshot: &ControllerSnapshot, messages: &[SmsMessage], language: Language) -> SmsVm {
    let summary = &snapshot.sms_inbox;
    let mut status = sms_status_text(summary.status, language);
    if let Some(error) = &snapshot.sms_inbox_failure {
        let reason = match error.code.stable().as_str() {
            "sms:port_busy" => t(language, crate::localization::TextKey::SmsErrPortBusy),
            "sms:port_open_failed" | "sms:permission_denied" => {
                t(language, crate::localization::TextKey::SmsErrPortAccess)
            }
            "sms:unsupported" => t(language, crate::localization::TextKey::SmsErrNoAtPort),
            "sms:pdu_mode_required" | "sms:pdu_confirm_failed" => {
                t(language, crate::localization::TextKey::SmsErrPduMode)
            }
            "sms:verification_failed" => t(
                language,
                crate::localization::TextKey::SmsErrResponseInvalid,
            ),
            "sms:timeout" | "app:stage_timeout" => {
                t(language, crate::localization::TextKey::SmsErrTimeout)
            }
            "sms:no_device" => t(language, crate::localization::TextKey::SmsErrNoDevice),
            "sms:device_removed" => t(language, crate::localization::TextKey::SmsErrDeviceGone),
            "sms:sim_identity_required" => {
                t(language, crate::localization::TextKey::SmsErrSimUnknown)
            }
            "sms:sim_changed" => t(language, crate::localization::TextKey::SmsErrSimChanged),
            "sms:sim_identity_unverified" => {
                t(language, crate::localization::TextKey::SmsErrSimIdentity)
            }
            "sms:read_cancelled" => t(language, crate::localization::TextKey::SmsStopped),
            "sms:context_changed" => {
                t(language, crate::localization::TextKey::SmsErrContextChanged)
            }
            "sms:storage_restore_unknown" => t(
                language,
                crate::localization::TextKey::SmsErrRestoreUnconfirmed,
            ),
            "sms:storage_unsupported" | "sms:storage_selection_unavailable" => t(
                language,
                crate::localization::TextKey::SmsErrUnsupportedLocation,
            ),
            "sms:list_too_large" => t(language, crate::localization::TextKey::SmsErrLimit),
            _ => t(language, crate::localization::TextKey::SmsErrQueryFailed),
        };
        status.text = crate::localization::format_positional(
            language,
            crate::localization::TextKey::SmsFailureWithCode,
            &[&reason, error.code.stable().as_str()],
        );
        if let Some(code) = error.os_code {
            status
                .text
                .push_str(&crate::localization::format_positional(
                    language,
                    crate::localization::TextKey::SmsSystemError,
                    &[&code.to_string()],
                ));
        }
    }
    SmsVm {
        title: LocalizedText::new(language, TextKey::SmsTitle),
        intro: LocalizedText::new(language, TextKey::SmsIntro),
        status,
        message_count: summary.message_count,
        unread_count: summary.unread_count,
        capacity: summary.capacity,
        capacity_text: summary.capacity.map(|(used, total)| {
            format_text_in(
                language,
                TextKey::SmsCapacityUsed,
                &TextArgs::used_total(used, total),
            )
        }),
        has_incomplete: summary.has_incomplete,
        evicted: summary.evicted,
        incomplete_warning: LocalizedText::new(language, TextKey::SmsIncompleteWarning),
        empty_text: LocalizedText::new(language, TextKey::SmsEmpty),
        list_pending_text: LocalizedText::new(language, TextKey::SmsListPending),
        rows: messages.iter().map(sms_row_vm).collect(),
    }
}

/// How long a delete confirmation stays armed. Sending uses a persistent confirmation window.
pub(crate) const CONFIRM_ARM_WINDOW: Duration = Duration::from_secs(3);

/// The height the workspace really received this frame. Recorded in egui's temp memory so the
/// layout tests can observe the rendered geometry instead of re-deriving it from the arithmetic.
fn workspace_height_id() -> egui::Id {
    egui::Id::new("sms-workspace-height")
}

/// The height the message list really received inside its column, after the search field.
fn list_viewport_id() -> egui::Id {
    egui::Id::new("sms-list-viewport")
}

fn read_only_module(snapshot: &ControllerSnapshot) -> bool {
    snapshot.app.device.as_ref().is_some_and(|device| {
        device.identity.profile().is_some() && !device.identity.allows_controlled_actions()
    })
}

pub(crate) fn render(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    messages: &[SmsDisplayMessage],
    language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    let mut vm = sms_vm(snapshot, &[], language);
    vm.rows = messages.iter().map(display_row_vm).collect();
    ui.spacing_mut().item_spacing.y = 8.0;
    state.serial_busy = snapshot.serial_work_busy;
    state.module_read_only = read_only_module(snapshot);
    if state.module_read_only {
        ui.label(meta_text(t(language, TextKey::ReadOnlyModuleReason)));
        state.storage_confirmation = None;
    }
    ui.horizontal_wrapped(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new(t(language, crate::localization::TextKey::SmsPageTitle))
                    .size(scale::TITLE)
                    .color(scale::ink()),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let details_label = if state.refresh_error.is_some() || state.auto_refresh_paused {
                t(
                    language,
                    crate::localization::TextKey::SmsReadDetailsAttention,
                )
            } else {
                t(language, crate::localization::TextKey::SmsReadDetails)
            };
            ui.menu_button(details_label, |ui| {
                ui.set_max_width(420.0);
                render_history_controls(ui, language, snapshot, sink, state);
                render_inbox_notes(ui, language, &vm, snapshot, state);
            });
            super::sms_balance::render_entry(ui, language, state);
            if ui
                .add_enabled(
                    state.can_open_editor(),
                    super::theme::primary_button(t(
                        language,
                        if state.has_draft() {
                            crate::localization::TextKey::ComposeResumeDraft
                        } else {
                            crate::localization::TextKey::ComposeTitle
                        },
                    )),
                )
                .clicked()
            {
                state.open_editor();
            }
            refresh_button(
                ui,
                language,
                snapshot,
                sink,
                state,
                &t(language, crate::localization::TextKey::SmsRefreshList),
            );
        });
    });
    super::sms_freshness::render(ui, snapshot, language, state);
    if snapshot.sms_refresh_pending {
        render_history_controls(ui, language, snapshot, sink, state);
    }
    render_storage_confirmation(ui, language, snapshot, sink, state);
    ui.add_space(4.0);
    compose::render(ui, language, state, snapshot, sink);
    if let Some(deletion) = &snapshot.sms_delete {
        render_delete_result(ui, language, deletion);
    }
    let busy = snapshot.serial_work_busy
        || snapshot
            .sms_send
            .as_ref()
            .is_some_and(|send| send.is_active())
        || snapshot.operation.as_ref().is_some_and(|operation| {
            matches!(
                operation.state,
                dji4g_application::OperationState::Running { .. }
            )
        });
    state.auto_refresh(
        Instant::now(),
        snapshot.app.device.is_some() && !state.module_read_only,
        busy,
        snapshot.sms_refresh_pending,
        sink,
    );
    render_inbox(ui, &vm, snapshot, language, sink, state);
}

fn refresh(sink: &dyn UiCommandSink, state: &mut SmsComposeState) {
    state.request_refresh(Instant::now(), sink);
}

fn refresh_button(
    ui: &mut Ui,
    language: Language,
    snapshot: &ControllerSnapshot,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
    label: &str,
) -> egui::Response {
    let response = ui
        .add_enabled(
            !read_only_module(snapshot)
                && !snapshot.sms_refresh_pending
                && !snapshot.serial_work_busy
                && state.search_shortcuts_allowed(),
            egui::Button::new(label),
        )
        .on_disabled_hover_text(t(
            language,
            if read_only_module(snapshot) {
                TextKey::ReadOnlyModuleReason
            } else {
                TextKey::SmsBusyWait
            },
        ));
    if response.clicked() {
        refresh(sink, state);
    }
    response
}

fn render_history_controls(
    ui: &mut Ui,
    language: Language,
    snapshot: &ControllerSnapshot,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    use dji4g_domain::SmsReadPhase;
    let context = (
        snapshot.app.device.as_ref().map(|device| device.epoch),
        snapshot.sim_epoch,
    );
    if snapshot.sms_refresh_pending {
        ui.horizontal_wrapped(|ui| {
            crate::ui::components::loading_spinner(ui);
            let label = match snapshot.sms_read_phase {
                None => t(language, crate::localization::TextKey::SmsPhaseWaiting),
                Some(SmsReadPhase::Verifying) => {
                    t(language, crate::localization::TextKey::SmsPhaseConfirming)
                }
                Some(SmsReadPhase::ReadingStorage) => t(
                    language,
                    crate::localization::TextKey::SmsPhaseQueryingStorage,
                ),
                Some(SmsReadPhase::SwitchingStorage) => {
                    t(language, crate::localization::TextKey::SmsPhaseSelecting)
                }
                Some(SmsReadPhase::Listing) => {
                    t(language, crate::localization::TextKey::SmsPhaseReading)
                }
                Some(SmsReadPhase::Decoding) => {
                    t(language, crate::localization::TextKey::SmsPhaseOrganizing)
                }
                Some(SmsReadPhase::RestoringStorage) => {
                    t(language, crate::localization::TextKey::SmsPhaseRestoring)
                }
                Some(SmsReadPhase::Complete) => {
                    t(language, crate::localization::TextKey::SmsPhaseReleasing)
                }
            };
            ui.label(crate::localization::format_positional(
                language,
                crate::localization::TextKey::SmsProgressRecords,
                &[&label, &snapshot.sms_read_progress.to_string()],
            ));
            if ui
                .add_enabled(
                    !state.read_cancel_requested,
                    egui::Button::new(t(language, crate::localization::TextKey::SmsStopReading)),
                )
                .clicked()
            {
                match sink.try_send(UiCommand::SmsCancelRead) {
                    Ok(()) => {
                        state.auto_refresh_paused = true;
                        state.read_cancel_requested = true;
                        state.refresh_error =
                            Some(t(language, crate::localization::TextKey::SmsStopRequested));
                    }
                    Err(error) => {
                        state.refresh_error = Some(compose::enqueue_error(error, language))
                    }
                }
            }
        });
    }
    if let Some(report) = snapshot
        .sms_read_report
        .as_ref()
        .filter(|_| !snapshot.sms_refresh_pending)
    {
        let location = report.storage.as_ref().map_or(
            t(
                language,
                crate::localization::TextKey::SmsStorageUnconfirmed,
            ),
            |s| match s.0.as_str() {
                "SM" => t(language, crate::localization::TextKey::SmsStorageSim),
                "ME" => t(language, crate::localization::TextKey::SmsStorageModule),
                "MT" => t(language, crate::localization::TextKey::SmsStorageModuleArea),
                _ => t(
                    language,
                    crate::localization::TextKey::SmsStorageCurrentArea,
                ),
            },
        );
        egui::CollapsingHeader::new(crate::localization::format_positional(
            language,
            crate::localization::TextKey::SmsRecordsRead,
            &[&report.raw_records.to_string()],
        ))
        .id_salt("sms-history-help")
        .show(ui, |ui| {
            ui.label(crate::localization::format_positional(
                language,
                crate::localization::TextKey::SmsLastRead,
                &[
                    &location,
                    &report.decoded_records.to_string(),
                    &report.skipped_records.to_string(),
                ],
            ));
            ui.label(t(language, crate::localization::TextKey::SmsRefreshNote));
            ui.label(t(
                language,
                crate::localization::TextKey::SmsOtherLocationsNote,
            ));
            ui.horizontal_wrapped(|ui| {
                for (token, label) in [
                    ("SM", t(language, crate::localization::TextKey::SmsReadSim)),
                    (
                        "ME",
                        t(language, crate::localization::TextKey::SmsReadModule),
                    ),
                ] {
                    let supported = report.supported_storages.iter().any(|s| s.0 == token);
                    let reason = if read_only_module(snapshot) {
                        t(language, TextKey::ReadOnlyModuleReason)
                    } else if snapshot.serial_work_busy {
                        t(language, crate::localization::TextKey::SmsTaskBusy)
                    } else if !supported {
                        t(
                            language,
                            crate::localization::TextKey::SmsLocationUnsupported,
                        )
                    } else {
                        t(language, crate::localization::TextKey::SmsWillConfirm)
                    };
                    if ui
                        .add_enabled(
                            supported && !snapshot.serial_work_busy && !read_only_module(snapshot),
                            egui::Button::new(label),
                        )
                        .on_hover_text(reason)
                        .clicked()
                    {
                        state.storage_confirmation = Some((
                            context.0,
                            context.1,
                            dji4g_domain::SmsStorageId(token.into()),
                        ));
                        ui.close_menu();
                    }
                }
            });
        });
    }
}

fn render_storage_confirmation(
    ui: &mut Ui,
    language: Language,
    snapshot: &ControllerSnapshot,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    // This window belongs to the page, rather than the details menu that opens it. It must
    // remain usable after clicking outside that menu, and clear if the device or SIM changes.
    let context = (
        snapshot.app.device.as_ref().map(|device| device.epoch),
        snapshot.sim_epoch,
    );
    if state
        .storage_confirmation
        .as_ref()
        .is_some_and(|(epoch, sim, _)| (*epoch, *sim) != context)
    {
        state.storage_confirmation = None;
    }
    if let Some((_, _, storage)) = state.storage_confirmation.clone() {
        egui::Window::new(t(
            language,
            crate::localization::TextKey::SmsReadOtherLocation,
        ))
        .collapsible(false)
        .resizable(false)
        .default_width(370.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ui.ctx(), |ui| {
            let storage_label = if storage.0 == "SM" {
                t(language, crate::localization::TextKey::SmsStorageSim)
            } else {
                t(language, crate::localization::TextKey::SmsStorageModule)
            };
            ui.label(crate::localization::format_positional(
                language,
                crate::localization::TextKey::SmsReadOtherBody,
                &[&storage_label],
            ));
            if let Some(error) = &state.refresh_error {
                wrapped_label(ui, RichText::new(error).color(StatusTone::Negative.color()));
            }
            ui.horizontal(|ui| {
                if ui
                    .button(t(language, crate::localization::TextKey::ButtonCancel))
                    .clicked()
                {
                    state.storage_confirmation = None;
                }
                if ui
                    .add_enabled(
                        !snapshot.serial_work_busy,
                        egui::Button::new(t(
                            language,
                            crate::localization::TextKey::SmsConfirmRead,
                        )),
                    )
                    .clicked()
                {
                    match sink.try_send(UiCommand::SmsReadStorage {
                        storage: storage.clone(),
                    }) {
                        Ok(()) => {
                            state.auto_refresh_paused = true;
                            state.read_cancel_requested = false;
                            state.refresh_error = None;
                            state.storage_confirmation = None;
                        }
                        Err(error) => {
                            state.refresh_error = Some(compose::enqueue_error(error, language))
                        }
                    }
                }
            });
        });
    }
}

fn badge(ui: &mut Ui, text: impl Into<String>, color: egui::Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.09))
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(8.0, 4.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).size(12.0).color(color));
        });
}

fn empty_panel(ui: &mut Ui, title: &str, note: &str, height: f32) {
    let height = if height.is_finite() {
        height.max(0.0)
    } else {
        0.0
    };
    ui.vertical_centered(|ui| {
        ui.add_space((height * 0.20).clamp(8.0, 96.0));
        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - 68.0) / 2.0).max(0.0));
            egui::Frame::none()
                .fill(scale::surface_sunken())
                .rounding(18.0)
                .inner_margin(20.0)
                .show(ui, |ui| {
                    ui.label(
                        super::icons::text(ui.ctx(), super::icons::MAIL, 28.0)
                            .color(scale::DOWNLOAD),
                    );
                });
        });
        ui.add_space(16.0);
        ui.label(RichText::new(title).size(18.0).strong().color(scale::ink()));
        ui.add_space(4.0);
        wrapped_label(ui, meta_text(note));
    });
}

fn render_inbox_notes(
    ui: &mut Ui,
    language: Language,
    vm: &SmsVm,
    snapshot: &ControllerSnapshot,
    state: &SmsComposeState,
) {
    // Query evidence has its own quiet status strip; sending results remain independent.
    ui.horizontal_wrapped(|ui| {
        if snapshot.sms_refresh_pending {
            crate::ui::components::loading_spinner(ui);
            ui.label(meta_text(t(
                language,
                crate::localization::TextKey::SmsSyncing,
            )));
        } else {
            ui.label(meta_text(&vm.status.text));
        }
        if vm.unread_count > 0 {
            badge(
                ui,
                crate::localization::format_positional(
                    language,
                    crate::localization::TextKey::SmsUnreadCount,
                    &[&vm.unread_count.to_string()],
                ),
                scale::DOWNLOAD,
            );
        }
        if let Some((used, total)) = vm.capacity {
            ui.label(meta_text(crate::localization::format_positional(
                language,
                crate::localization::TextKey::SmsStorageUsage,
                &[&used.to_string(), &total.to_string()],
            )));
        }
    });
    if state.auto_refresh_paused {
        ui.label(meta_text(t(
            language,
            crate::localization::TextKey::SmsAutoSyncPaused,
        )));
    }
    // A sync failure is real information, but it used to print a full paragraph above the list
    // and eat the space the messages needed. It stays one click away instead.
    if let Some(error) = &state.refresh_error {
        egui::CollapsingHeader::new(
            RichText::new(t(language, crate::localization::TextKey::SmsSyncProblem))
                .size(13.0)
                .color(StatusTone::Caution.color()),
        )
        .id_salt("sms-refresh-error")
        .default_open(false)
        .show(ui, |ui| {
            wrapped_label(ui, RichText::new(error).color(StatusTone::Caution.color()));
        });
    }
    if vm.has_incomplete || vm.evicted > 0 {
        egui::CollapsingHeader::new(t(language, crate::localization::TextKey::SmsSyncHistory))
            .id_salt("sms-sync-notes")
            .default_open(false)
            .show(ui, |ui| {
                if vm.has_incomplete {
                    wrapped_label(
                        ui,
                        RichText::new(&vm.incomplete_warning.text)
                            .color(StatusTone::Caution.color()),
                    );
                }
                if vm.evicted > 0 {
                    wrapped_label(
                        ui,
                        meta_text(crate::localization::format_positional(
                            language,
                            crate::localization::TextKey::SmsCacheTrimmed,
                            &[&vm.evicted.to_string()],
                        )),
                    );
                }
            });
    }
}

fn search_id() -> egui::Id {
    egui::Id::new("sms-message-search")
}

fn render_search(ui: &mut Ui, language: Language, state: &mut SmsComposeState) {
    let delete_armed = state.selected.is_some_and(|key| {
        ui.data(|data| data.get_temp::<Instant>(egui::Id::new(("sms-row-delete-armed", key))))
            .is_some_and(|at| at.elapsed() < CONFIRM_ARM_WINDOW)
    });
    // Decisions and the editor own their keys. Do not use wants_keyboard_input here: that would
    // also block Esc while the search box itself is focused.
    if state.search_shortcuts_allowed() && !delete_armed {
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::F)) {
            ui.memory_mut(|memory| memory.request_focus(search_id()));
        }
        // egui surrenders focus at the start of an Esc frame, before this widget renders.
        if ui.memory(|memory| {
            memory.has_focus(search_id()) || memory.had_focus_last_frame(search_id())
        }) && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            state.search.clear();
            ui.memory_mut(|memory| memory.request_focus(search_id()));
        }
    }
    let clear_label = t(language, TextKey::SmsClearSearch);
    let clear_width = (ui
        .painter()
        .layout_no_wrap(
            clear_label.clone(),
            egui::TextStyle::Button.resolve(ui.style()),
            ui.visuals().text_color(),
        )
        .size()
        .x
        + 2.0 * ui.spacing().button_padding.x)
        .max(scale::BUTTON_MIN_W);
    ui.horizontal(|ui| {
        let search_width =
            (ui.available_width() - clear_width - ui.spacing().item_spacing.x).max(40.0);
        let search = ui.add_sized(
            egui::vec2(search_width, scale::CONTROL_H),
            egui::TextEdit::singleline(&mut state.search)
                .id(search_id())
                .hint_text(t(language, TextKey::SmsSearchHint))
                .desired_width(f32::INFINITY)
                .margin(egui::vec2(12.0, 5.0)),
        );
        let clear = super::components::action_button(
            ui,
            &clear_label,
            super::components::ButtonKind::Outlined,
            !state.search.is_empty(),
            None,
        );
        if clear.clicked() {
            state.search.clear();
            ui.memory_mut(|memory| memory.request_focus(search_id()));
        }
        #[cfg(test)]
        let clear_interaction = (
            clear.id,
            clear.interact_rect,
            ui.clip_rect(),
            clear.hovered(),
            clear.is_pointer_button_down_on(),
            clear.clicked(),
        );
        #[cfg(test)]
        ui.data_mut(|data| {
            data.insert_temp(egui::Id::new("sms-search-rect"), search.rect);
            data.insert_temp(egui::Id::new("sms-clear-search-rect"), clear.rect);
            data.insert_temp(
                egui::Id::new("sms-clear-search-interaction"),
                clear_interaction,
            );
        });
        let _ = search;
    });
}

fn render_inbox(
    ui: &mut Ui,
    vm: &SmsVm,
    snapshot: &ControllerSnapshot,
    language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    ui.add_space(4.0);
    egui::Frame::none()
        .fill(scale::surface())
        .rounding(14.0)
        // The sheet holds the tabs, the search field, the list and the footer note, so it needs the
        // same card padding every other card in the panel uses; without it those controls sit
        // flush against the rounded edge.
        .inner_margin(egui::Margin::same(scale::CARD_PAD))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let incoming = vm
                .rows
                .iter()
                .filter(|r| r.direction != SmsDirection::Outgoing)
                .count();
            let outgoing = vm.rows.len() - incoming;
            let narrow_controls = ui.available_width() < 400.0;
            ui.horizontal_wrapped(|ui| {
                if super::components::segmented_control(
                    ui,
                    egui::Id::new("sms-direction"),
                    &mut state.outgoing,
                    &[
                        super::components::TabItem::new(
                            false,
                            crate::localization::format_positional(
                                language,
                                crate::localization::TextKey::SmsTabInbox,
                                &[&incoming.to_string()],
                            ),
                        ),
                        super::components::TabItem::new(
                            true,
                            crate::localization::format_positional(
                                language,
                                crate::localization::TextKey::SmsTabOutgoing,
                                &[&outgoing.to_string()],
                            ),
                        ),
                    ],
                ) {
                    state.selected = None;
                }
                if !state.outgoing {
                    if narrow_controls {
                        ui.end_row();
                    }
                    let response = egui::ComboBox::from_id_salt("sms-inbox-filter")
                        .width(148.0)
                        .selected_text(t(language, state.inbox_filter.label()))
                        .show_ui(ui, |ui| {
                            for choice in [
                                InboxFilter::All,
                                InboxFilter::Unread,
                                InboxFilter::Incomplete,
                            ] {
                                let response = ui.selectable_value(
                                    &mut state.inbox_filter,
                                    choice,
                                    t(language, choice.label()),
                                );
                                #[cfg(test)]
                                ui.data_mut(|data| {
                                    data.insert_temp(
                                        egui::Id::new(("sms-filter-choice", choice as u8)),
                                        response.rect,
                                    );
                                });
                                let _ = response;
                            }
                        })
                        .response;
                    #[cfg(test)]
                    ui.data_mut(|data| {
                        data.insert_temp(egui::Id::new("sms-filter-rect"), response.rect)
                    });
                    let _ = response;
                } else {
                    #[cfg(test)]
                    ui.data_mut(|data| data.remove::<egui::Rect>(egui::Id::new("sms-filter-rect")));
                }
            });
            ui.add_space(8.0);
            render_search(ui, language, state);
            ui.add_space(10.0);
            let rows = filtered_rows(&vm.rows, state);
            clear_hidden_selection(&rows, state);
            // Everything the frame has left, minus the footer note, is the workspace. One
            // subtraction, from the container's real height — no screen-coordinate estimate and
            // no fixed cap, so a taller window really does show more messages.
            let workspace_height = (ui.available_height() - sms_layout::FOOTER_RESERVE).max(0.0);
            let layout = sms_layout::workspace_layout(
                ui.available_width(),
                workspace_height,
                sms_layout::COLUMN_GAP,
            );
            ui.data_mut(|data| {
                data.insert_temp(workspace_height_id(), layout.body_height);
            });
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), layout.body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    if layout.wide {
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(layout.list_width, layout.body_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    list_panel(ui, &rows, snapshot, language, sink, state);
                                },
                            );
                            let (divider, _) = ui.allocate_exact_size(
                                egui::vec2(1.0, layout.body_height),
                                egui::Sense::hover(),
                            );
                            ui.painter().line_segment(
                                [divider.center_top(), divider.center_bottom()],
                                egui::Stroke::new(1.0_f32, scale::line()),
                            );
                            ui.allocate_ui_with_layout(
                                egui::vec2(layout.detail_width, layout.body_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    egui::ScrollArea::vertical()
                                        .id_salt("sms-reader")
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            render_detail(ui, &rows, language, sink, state);
                                        });
                                },
                            );
                        });
                    } else if state.selected.is_some() {
                        // Narrow layout: the reader replaces the list, the back button stays
                        // pinned above its own bounded scroll area so a long message scrolls
                        // inside the reader instead of moving the page.
                        if ui
                            .button(t(language, crate::localization::TextKey::SmsBackToList))
                            .clicked()
                        {
                            state.selected = None;
                        }
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical()
                            .id_salt("sms-reader-narrow")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                render_detail(ui, &rows, language, sink, state);
                            });
                    } else {
                        list_panel(ui, &rows, snapshot, language, sink, state);
                    }
                },
            );
            ui.add_space(14.0);
            ui.separator();
            // Truncated to one line so the reserved footer height above stays exact.
            ui.add(
                egui::Label::new(meta_text(t(
                    language,
                    crate::localization::TextKey::SmsFooterNote,
                )))
                .truncate(),
            );
        });
}

fn list_panel(
    ui: &mut Ui,
    rows: &[&SmsRowVm],
    snapshot: &ControllerSnapshot,
    language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    // The shared search row has already consumed its height, including in the narrow reader.
    // Give the list the actual remaining space, with no second search-height subtraction.
    let viewport = sms_layout::list_viewport_height(ui.available_height(), 0.0);
    ui.data_mut(|data| {
        data.insert_temp(list_viewport_id(), viewport);
    });
    if rows.is_empty() {
        let (title, note) = if !state.search.trim().is_empty() {
            (
                t(language, crate::localization::TextKey::SmsEmptySearch),
                t(language, crate::localization::TextKey::SmsEmptySearchHint),
            )
        } else if !state.outgoing && state.inbox_filter != InboxFilter::All {
            (
                t(language, TextKey::SmsEmptyFilter),
                t(language, TextKey::SmsEmptyFilterHint),
            )
        } else if state.outgoing {
            (
                t(language, crate::localization::TextKey::SmsEmptyOutgoing),
                t(language, crate::localization::TextKey::SmsEmptyOutgoingHint),
            )
        } else if snapshot.sms_refresh_pending {
            (
                t(language, crate::localization::TextKey::SmsLoading),
                t(language, crate::localization::TextKey::SmsLoadingHint),
            )
        } else {
            match snapshot.sms_inbox.status {
                FeatureStatus::NotProbed => (
                    t(language, crate::localization::TextKey::SmsInboxWaiting),
                    t(language, crate::localization::TextKey::SmsInboxWaitingHint),
                ),
                FeatureStatus::Supported | FeatureStatus::Empty => (
                    t(language, crate::localization::TextKey::SmsInboxEmpty),
                    t(language, crate::localization::TextKey::SmsInboxEmptyHint),
                ),
                _ => (
                    t(language, crate::localization::TextKey::SmsUnavailable),
                    t(language, crate::localization::TextKey::SmsUnavailableHint),
                ),
            }
        };
        empty_panel(ui, &title, &note, viewport);
        if !state.outgoing
            && state.inbox_filter == InboxFilter::All
            && state.search.is_empty()
            && !snapshot.sms_refresh_pending
        {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                refresh_button(
                    ui,
                    language,
                    snapshot,
                    sink,
                    state,
                    &t(language, crate::localization::TextKey::SmsRefreshMessages),
                );
            });
        }
    } else {
        egui::ScrollArea::vertical()
            .id_salt("sms-message-list")
            .auto_shrink([false, false])
            .max_height(viewport)
            .show(ui, |ui| {
                render_list(ui, rows, language, sink, state);
            });
    }
}

fn render_list(
    ui: &mut Ui,
    rows: &[&SmsRowVm],
    language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    for row in rows {
        let outgoing = row.direction == SmsDirection::Outgoing;
        let key = row.stable_id;
        let selected = state.selected == Some(key);
        let frame = egui::Frame::none()
            .rounding(10.0)
            .inner_margin(10.0)
            .fill(if selected {
                scale::surface_raised()
            } else {
                scale::surface_alt()
            });
        let response = frame
            .show(ui, |ui| {
                // These are labels inside one >=60px clickable message row, not small controls.
                ui.spacing_mut().interact_size.y = 20.0;
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.set_min_width(ui.available_width());
                ui.set_min_height(40.0);
                ui.horizontal_wrapped(|ui| {
                    if row.unread == Some(true) && !outgoing {
                        ui.colored_label(scale::DOWNLOAD, "●");
                    }
                    ui.label(
                        RichText::new(&row.sender_masked)
                            .size(15.0)
                            .strong()
                            .color(scale::ink()),
                    );
                    ui.label(meta_text(
                        row.timestamp
                            .as_deref()
                            .map(str::to_owned)
                            .unwrap_or_else(|| {
                                t(language, crate::localization::TextKey::SmsNoTimestamp)
                            }),
                    ));
                });
                ui.add(
                    egui::Label::new(
                        RichText::new(body_preview(&row.body))
                            .size(14.0)
                            .color(scale::secondary()),
                    )
                    .truncate(),
                );
                if outgoing {
                    let (tone, label) = outgoing_state(row.status);
                    ui.colored_label(
                        tone.color(),
                        format!("{} {}", tone.marker(), label.to_string(language)),
                    );
                }
            })
            .response
            .interact(egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.hovered() && !selected {
            ui.painter().rect_stroke(
                response.rect,
                10.0,
                egui::Stroke::new(1.0_f32, scale::line()),
            );
        }
        if response.clicked() {
            state.selected = Some(key);
            if !outgoing && !state.serial_busy && !state.module_read_only {
                state.error = sink
                    .try_send(UiCommand::SmsRead { index: row.index })
                    .err()
                    .map(|e| compose::enqueue_error(e, language));
            }
        }
        ui.add_space(6.0);
    }
}

fn render_detail(
    ui: &mut Ui,
    rows: &[&SmsRowVm],
    language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    let Some(row) = rows
        .iter()
        .find(|row| state.selected == Some(row.stable_id))
    else {
        empty_panel(
            ui,
            &t(language, crate::localization::TextKey::SmsReaderEmpty),
            &t(language, crate::localization::TextKey::SmsReaderEmptyHint),
            ui.available_height(),
        );
        return;
    };
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        badge(
            ui,
            if row.direction == SmsDirection::Incoming {
                t(language, crate::localization::TextKey::SmsKindIncoming)
            } else {
                t(language, crate::localization::TextKey::SmsKindOutgoing)
            },
            scale::DOWNLOAD,
        );
        for tag in row_tags(row, language) {
            badge(ui, tag.text, tag.tone.color());
        }
    });
    ui.add_space(6.0);
    wrapped_label(ui, RichText::new(&row.sender_full).size(18.0).strong());
    ui.label(meta_text(
        row.timestamp
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                t(
                    language,
                    crate::localization::TextKey::SmsNoTimestampFromModule,
                )
            }),
    ));
    ui.add_space(8.0);
    egui::Frame::none()
        .fill(scale::surface_alt())
        .rounding(12.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            wrapped_label(ui, RichText::new(&row.body).size(14.0).color(scale::ink()));
        });
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button(TextKey::ButtonCopy.to_string(language)).clicked() {
            ui.ctx()
                .copy_text(format!("{}\n{}", row.sender_full, row.body));
        }
        if row.direction == SmsDirection::Incoming {
            if super::components::action_button(
                ui,
                &t(language, crate::localization::TextKey::SmsReply),
                super::components::ButtonKind::Tonal,
                !state.serial_busy,
                Some(&t(language, crate::localization::TextKey::SmsTaskBusy)),
            )
            .clicked()
            {
                state.begin_reply(&row.sender_full);
            }
            render_delete_button(ui, row, language, sink, state);
        } else {
            badge(
                ui,
                outgoing_state(row.status).1.to_string(language),
                outgoing_state(row.status).0.color(),
            );
        }
    });
}
/// Per-row delete with an in-page two-click confirmation: the first click arms the button for
/// [`CONFIRM_ARM_WINDOW`] and the second dispatches. No deletion is ever sent unconfirmed, and
/// only incoming rows reach this path (an outgoing record has no module copy to delete).
fn render_delete_button(
    ui: &mut Ui,
    row: &SmsRowVm,
    language: Language,
    sink: &dyn UiCommandSink,
    state: &mut SmsComposeState,
) {
    let armed_id = egui::Id::new(("sms-row-delete-armed", row.stable_id));
    let enabled = row.delete_allowed
        && !row.fragments.is_empty()
        && !state.serial_busy
        && !state.module_read_only;
    if !enabled {
        ui.data_mut(|data| data.remove::<Instant>(armed_id));
    }
    let armed = enabled
        && ui
            .data(|data| data.get_temp::<Instant>(armed_id))
            .is_some_and(|at| at.elapsed() < CONFIRM_ARM_WINDOW);
    let label = if armed {
        crate::localization::format_positional(
            language,
            crate::localization::TextKey::SmsConfirmDeleteFragments,
            &[&row.fragments.len().to_string()],
        )
    } else if row.status == SmsStatus::Incomplete {
        crate::localization::format_positional(
            language,
            crate::localization::TextKey::SmsDeleteFragments,
            &[&row.fragments.len().to_string()],
        )
    } else {
        // English inflects the noun, so a single-fragment message reads its own arm; Chinese has
        // one form and the two arms render identically.
        crate::localization::format_positional(
            language,
            if row.fragments.len() == 1 {
                crate::localization::TextKey::SmsDeleteMessageOne
            } else {
                crate::localization::TextKey::SmsDeleteMessage
            },
            &[&row.fragments.len().to_string()],
        )
    };
    if super::components::action_button(
        ui,
        &label,
        super::components::ButtonKind::Destructive,
        enabled,
        Some(&t(
            language,
            if state.module_read_only {
                TextKey::ReadOnlyModuleReason
            } else {
                TextKey::SmsDeleteBusy
            },
        )),
    )
    .clicked()
    {
        if armed {
            ui.data_mut(|data| data.remove::<Instant>(armed_id));
            state.error = sink
                .try_send(UiCommand::SmsDelete {
                    fragments: row.fragments.clone(),
                })
                .err()
                .map(|error| compose::enqueue_error(error, language));
        } else {
            ui.data_mut(|data| data.insert_temp(armed_id, Instant::now()));
        }
    }
    if !row.delete_allowed {
        wrapped_label(
            ui,
            meta_text(t(language, crate::localization::TextKey::SmsDeleteConflict)),
        );
    }
}

fn delete_result_text(
    deletion: &dji4g_application::SmsDeleteSnapshot,
    language: Language,
) -> (StatusTone, String) {
    let deleted = deletion
        .items
        .iter()
        .filter(|item| item.result == SmsDeleteItemResult::Deleted)
        .count();
    let unknown = deletion
        .items
        .iter()
        .filter(|item| item.result == SmsDeleteItemResult::OutcomeUnknown)
        .count();
    if !deletion.finished {
        return (
            StatusTone::Progress,
            crate::localization::format_positional(
                language,
                crate::localization::TextKey::SmsDeleting,
                &[&deleted.to_string(), &deletion.total.to_string()],
            ),
        );
    }
    if deleted == deletion.total && deletion.total > 0 {
        return (
            StatusTone::Positive,
            crate::localization::format_positional(
                language,
                crate::localization::TextKey::SmsDeletedAll,
                &[&deletion.total.to_string()],
            ),
        );
    }
    if unknown > 0 {
        return (
            StatusTone::Caution,
            crate::localization::format_positional(
                language,
                crate::localization::TextKey::SmsDeleteUnknown,
                &[
                    &deleted.to_string(),
                    &deletion.total.to_string(),
                    &unknown.to_string(),
                ],
            ),
        );
    }
    if deleted > 0 {
        return (
            StatusTone::Caution,
            crate::localization::format_positional(
                language,
                crate::localization::TextKey::SmsDeletePartial,
                &[&deleted.to_string(), &deletion.total.to_string()],
            ),
        );
    }
    (
        StatusTone::Negative,
        t(language, crate::localization::TextKey::SmsDeleteNone),
    )
}

fn render_delete_result(
    ui: &mut Ui,
    language: Language,
    deletion: &dji4g_application::SmsDeleteSnapshot,
) {
    let (tone, text) = delete_result_text(deletion, language);
    wrapped_label(
        ui,
        RichText::new(format!("{} {text}", tone.marker())).color(tone.color()),
    );
    if deletion.items.len() > 1 || deletion.items.iter().any(|item| item.code.is_some()) {
        egui::CollapsingHeader::new(t(
            language,
            crate::localization::TextKey::SmsDeleteResultTitle,
        ))
        .show(ui, |ui| {
            for item in &deletion.items {
                let result = match item.result {
                    SmsDeleteItemResult::Deleted => {
                        t(language, crate::localization::TextKey::SmsDeleteConfirmed)
                    }
                    SmsDeleteItemResult::Failed => {
                        t(language, crate::localization::TextKey::SmsDeleteFailed)
                    }
                    SmsDeleteItemResult::OutcomeUnknown => t(
                        language,
                        crate::localization::TextKey::SmsDeleteUnknownShort,
                    ),
                    SmsDeleteItemResult::NotAttempted => {
                        t(language, crate::localization::TextKey::SmsDeleteNotRun)
                    }
                };
                let code = item.code.as_deref().unwrap_or("");
                wrapped_label(
                    ui,
                    meta_text(format!(
                        "{} #{} · {result} {code}",
                        item.fragment.storage.0, item.fragment.index
                    )),
                );
            }
        });
    }
}

/// Single-line body preview for a collapsed row: at most [`BODY_PREVIEW_CHARS`] characters, with
/// an explicit ellipsis when the body continues. The full text stays behind the row click.
const BODY_PREVIEW_CHARS: usize = 42;

fn body_preview(body: &str) -> String {
    let flattened = body.replace(['\r', '\n'], " ");
    let mut chars = flattened.chars();
    let preview: String = chars.by_ref().take(BODY_PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

trait LocalizedKeyText {
    fn to_string(self, language: Language) -> String;
}

impl LocalizedKeyText for TextKey {
    fn to_string(self, language: Language) -> String {
        crate::localization::LocalizedText::new(language, self).text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use dji4g_application::{CommandStateSnapshot, ControllerSnapshot, SettingsSnapshot};
    use dji4g_domain::{
        AppSnapshot, Availability, DeviceEpoch, FeatureStatus, Freshness, HotspotStatus,
        SmsInboxSummary, SmsMessage, SmsMultipartInfo, SmsStatus, SmsStorageId,
    };

    fn display_messages(messages: &[SmsMessage]) -> Vec<SmsDisplayMessage> {
        messages
            .iter()
            .map(|message| SmsDisplayMessage {
                message: message.clone(),
                fragments: vec![message.fragment_key()],
                delete_allowed: message.direction == SmsDirection::Incoming,
            })
            .collect()
    }

    fn snapshot(summary: SmsInboxSummary) -> ControllerSnapshot {
        ControllerSnapshot {
            rates_sampled_at: None,
            module_network_check: None,
            host_network: dji4g_application::HostNetworkSnapshot::default(),
            publication_revision: 7,
            app: Arc::new(AppSnapshot {
                revision: 7,
                observed_at: std::time::SystemTime::UNIX_EPOCH,
                freshness: Freshness::Fresh,
                availability: Availability::Available,
                hotspot: HotspotStatus::Off,
                device: None,
                cellular: None,
                network: None,
                active_operation: None,
                issues: Vec::new(),
            }),
            diagnostics: dji4g_application::DiagnosticSet::new(DeviceEpoch(1)),
            prepared_action: None,
            operation: None,
            settings: SettingsSnapshot::default(),
            command_state: CommandStateSnapshot::default(),
            action_readiness: Vec::new(),
            feedback: None,
            sim_epoch: 0,
            feature_status: None,
            adapter_metrics: None,
            timeline: Default::default(),
            sms_inbox: summary,
            sms_messages: Vec::new(),
            sms_send: None,
            sms_delete: None,
            serial_work_busy: false,
            sms_refresh_pending: false,
            sms_read_phase: None,
            sms_read_progress: 0,
            sms_read_report: None,
            sms_inbox_failure: None,
            device_tools: Default::default(),
        }
    }

    fn message(index: u32, read: Option<bool>, status: SmsStatus) -> SmsMessage {
        let mut message = SmsMessage::new(
            index,
            SmsStorageId("SM".to_owned()),
            1,
            0,
            "+8613800138000",
            "sensitive body",
            SmsEncoding::Ucs2,
            status,
        );
        message.service_centre_timestamp = Some("24/09/10,12:00:00+32".to_owned());
        message.multipart = Some(SmsMultipartInfo {
            reference: dji4g_domain::SmsConcatReference::EightBit(1),
            total: 3,
            sequence: 2,
        });
        message.read = read;
        message
    }

    fn outgoing_message(status: SmsStatus) -> SmsMessage {
        SmsMessage::new_outgoing(
            4,
            1,
            0,
            "+8613800138000",
            "sent body",
            SmsEncoding::Other,
            status,
        )
    }

    #[test]
    fn physical_storage_and_payload_changes_cannot_reuse_a_selection() {
        let original = message(7, Some(false), SmsStatus::Received);
        let mut other_storage = original.clone();
        other_storage.storage = SmsStorageId("ME".into());
        let replacement = SmsMessage::new(
            7,
            original.storage.clone(),
            1,
            0,
            "+8613800138000",
            "replacement",
            SmsEncoding::Ucs2,
            SmsStatus::Received,
        );
        assert_ne!(
            sms_row_vm(&original).stable_id,
            sms_row_vm(&other_storage).stable_id
        );
        assert_ne!(
            sms_row_vm(&original).stable_id,
            sms_row_vm(&replacement).stable_id
        );
        let mut read = original.clone();
        read.read = Some(true);
        assert_eq!(sms_row_vm(&original).stable_id, sms_row_vm(&read).stable_id);
    }

    #[test]
    fn delete_receipts_distinguish_all_partial_and_unknown() {
        use dji4g_application::{SmsDeleteItemSnapshot, SmsDeleteSnapshot};
        let fragment = message(7, None, SmsStatus::Received).fragment_key();
        let mut deletion = SmsDeleteSnapshot {
            request_id: 1,
            total: 2,
            finished: true,
            items: vec![
                SmsDeleteItemSnapshot {
                    fragment: fragment.clone(),
                    result: SmsDeleteItemResult::Deleted,
                    code: None,
                },
                SmsDeleteItemSnapshot {
                    fragment,
                    result: SmsDeleteItemResult::Failed,
                    code: Some("sms:delete_rejected".into()),
                },
            ],
        };
        assert!(
            delete_result_text(&deletion, Language::ZhCn)
                .1
                .contains("部分删除")
        );
        deletion.items[1].result = SmsDeleteItemResult::OutcomeUnknown;
        assert_eq!(
            delete_result_text(&deletion, Language::ZhCn).0,
            StatusTone::Caution
        );
        assert!(
            delete_result_text(&deletion, Language::ZhCn)
                .1
                .contains("不会自动重试")
        );
        deletion.items[1].result = SmsDeleteItemResult::Deleted;
        assert_eq!(
            delete_result_text(&deletion, Language::ZhCn).0,
            StatusTone::Positive
        );
        assert!(
            delete_result_text(&deletion, Language::ZhCn)
                .1
                .contains("全部 2")
        );
    }

    #[test]
    fn sms_vm_reports_status_counts_capacity_and_incomplete_flag() {
        let vm = sms_vm(
            &snapshot(SmsInboxSummary {
                message_count: 2,
                unread_count: 1,
                capacity: Some((2, 30)),
                status: FeatureStatus::Supported,
                has_incomplete: true,
                evicted: 0,
                ..Default::default()
            }),
            &[],
            Language::ZhCn,
        );
        assert_eq!(vm.status.text, "已读取");
        assert_eq!(vm.message_count, 2);
        assert_eq!(vm.unread_count, 1);
        assert_eq!(
            vm.capacity_text.as_ref().map(|text| text.text.as_str()),
            Some("已用 2 / 总数 30")
        );
        assert!(vm.has_incomplete);
        assert_eq!(vm.incomplete_warning.text, "存在未完整接收的长短信");
        assert!(vm.rows.is_empty());
        assert!(!vm.empty_text.text.trim().is_empty());
        assert!(!vm.list_pending_text.text.trim().is_empty());
    }

    #[test]
    fn sms_vm_wires_the_local_eviction_count_from_the_summary() {
        let vm = sms_vm(
            &snapshot(SmsInboxSummary {
                evicted: 2,
                ..SmsInboxSummary::default()
            }),
            &[],
            Language::ZhCn,
        );
        assert_eq!(vm.evicted, 2);
    }

    #[test]
    fn sms_vm_marks_an_unqueried_inbox_and_omits_absent_capacity() {
        let vm = sms_vm(&snapshot(SmsInboxSummary::default()), &[], Language::ZhCn);
        assert_eq!(vm.status.text, "尚未查询");
        assert_eq!(vm.capacity_text, None);
        assert!(!vm.has_incomplete);
    }

    #[test]
    fn sms_vm_maps_a_classified_failure_to_its_precise_note() {
        let vm = sms_vm(
            &snapshot(SmsInboxSummary {
                status: FeatureStatus::TransportFailure,
                ..SmsInboxSummary::default()
            }),
            &[],
            Language::ZhCn,
        );
        assert!(
            vm.status.text.contains("本次超时"),
            "got: {}",
            vm.status.text
        );
        assert!(!vm.status.text.contains("尚未查询"));
    }

    #[test]
    fn sms_row_vm_masks_the_sender_and_keeps_read_state_and_parts() {
        let row = sms_row_vm(&message(7, Some(false), SmsStatus::Incomplete));
        assert_eq!(row.index, 7);
        assert_eq!(row.direction, SmsDirection::Incoming);
        assert_eq!(row.unread, Some(true));
        assert_eq!(row.sender_masked, "****8000");
        assert_eq!(row.sender_full, "+8613800138000");
        assert_eq!(row.body, "sensitive body");
        assert_eq!(row.timestamp.as_deref(), Some("24/09/10,12:00:00+32"));
        assert_eq!(row.encoding, SmsEncoding::Ucs2);
        assert_eq!(row.multipart, Some((2, 3)));
        assert_eq!(row.status, SmsStatus::Incomplete);
    }

    #[test]
    fn sms_row_vm_projects_outgoing_records_as_local_bookkeeping() {
        let row = sms_row_vm(&outgoing_message(SmsStatus::Submitted));
        assert_eq!(row.direction, SmsDirection::Outgoing);
        assert_eq!(row.status, SmsStatus::Submitted);
        // Outgoing mail is never unread mail.
        assert_eq!(row.unread, Some(false));
        assert_eq!(row.timestamp, None);
        assert_eq!(row.body, "sent body");
        assert_eq!(row.sender_masked, "****8000");
    }

    #[test]
    fn sms_row_vm_keeps_an_unknown_read_state_unknown() {
        let row = sms_row_vm(&message(1, None, SmsStatus::Received));
        assert_eq!(row.unread, None);
    }

    #[test]
    fn sms_vm_projects_rows_in_store_order() {
        let messages = vec![
            message(3, Some(true), SmsStatus::Received),
            outgoing_message(SmsStatus::OutcomeUnknown),
            message(9, Some(false), SmsStatus::Received),
        ];
        let vm = sms_vm(
            &snapshot(SmsInboxSummary {
                message_count: 3,
                status: FeatureStatus::Supported,
                ..SmsInboxSummary::default()
            }),
            &messages,
            Language::ZhCn,
        );
        assert_eq!(
            vm.rows.iter().map(|row| row.index).collect::<Vec<_>>(),
            vec![3, 4, 9]
        );
        assert_eq!(vm.rows[0].unread, Some(false));
        assert_eq!(vm.rows[1].direction, SmsDirection::Outgoing);
        assert_eq!(vm.rows[2].unread, Some(true));
    }

    #[test]
    fn encoding_vocabulary_is_closed() {
        assert_eq!(sms_encoding_text(SmsEncoding::Gsm7, Language::ZhCn), "GSM7");
        assert_eq!(sms_encoding_text(SmsEncoding::Ucs2, Language::ZhCn), "UCS2");
        assert_eq!(
            sms_encoding_text(SmsEncoding::Other, Language::ZhCn),
            "其他"
        );
    }

    #[test]
    fn the_body_preview_is_single_line_bounded_and_marks_truncation() {
        assert_eq!(body_preview("短正文"), "短正文");
        let exact = "字".repeat(BODY_PREVIEW_CHARS);
        assert_eq!(
            body_preview(&exact),
            exact,
            "an exact-fit body is not elided"
        );
        let long = format!("{exact}尾");
        let preview = body_preview(&long);
        assert_eq!(preview.chars().count(), BODY_PREVIEW_CHARS + 1);
        assert!(preview.ends_with('…'));
        assert!(
            !body_preview("第一行\n第二行").contains('\n'),
            "the preview must stay on one line"
        );
    }

    #[test]
    fn the_read_marker_is_a_filled_progress_dot_only_when_unread() {
        assert_eq!(
            row_read_state(Some(true)),
            (StatusTone::Progress, "●", TextKey::SmsUnread)
        );
        assert_eq!(
            row_read_state(Some(false)),
            (StatusTone::Neutral, "○", TextKey::SmsRead)
        );
        assert_eq!(
            row_read_state(None),
            (StatusTone::Neutral, "○", TextKey::ValueUnknown)
        );
    }

    #[test]
    fn outgoing_submission_status_maps_to_its_tone_and_label() {
        assert_eq!(
            outgoing_state(SmsStatus::Submitted),
            (StatusTone::Positive, TextKey::SmsOutgoingSubmitted)
        );
        assert_eq!(
            outgoing_state(SmsStatus::Failed),
            (StatusTone::Negative, TextKey::SmsOutgoingFailed)
        );
        assert_eq!(
            outgoing_state(SmsStatus::OutcomeUnknown),
            (StatusTone::Caution, TextKey::SmsOutgoingUnknown)
        );
    }

    #[test]
    fn row_tags_keep_encoding_and_fragments_neutral_and_flag_the_incomplete_message() {
        let row = sms_row_vm(&message(7, Some(true), SmsStatus::Incomplete));
        let tags = row_tags(&row, Language::ZhCn);
        let texts = tags.iter().map(|tag| tag.text.as_str()).collect::<Vec<_>>();
        assert_eq!(texts, ["UCS2", "已读取 1 / 3 个分片", "未完整"]);
        assert_eq!(tags[0].tone, StatusTone::Neutral);
        assert_eq!(tags[1].tone, StatusTone::Neutral);
        assert_eq!(tags[2].tone, StatusTone::Caution);
    }

    #[test]
    fn row_tags_drop_the_incomplete_warning_for_a_complete_message() {
        let mut complete = message(8, Some(false), SmsStatus::Received);
        complete.multipart = None;
        let row = sms_row_vm(&complete);
        let tags = row_tags(&row, Language::ZhCn);
        let texts = tags.iter().map(|tag| tag.text.as_str()).collect::<Vec<_>>();
        assert_eq!(texts, ["UCS2"]);
        assert!(tags.iter().all(|tag| tag.tone == StatusTone::Neutral));
    }

    #[test]
    fn row_tags_are_empty_for_outgoing_rows() {
        let row = sms_row_vm(&outgoing_message(SmsStatus::Submitted));
        assert_eq!(row_tags(&row, Language::ZhCn), Vec::new());
    }

    #[test]
    fn the_page_renders_its_empty_state_without_panicking() {
        struct NoopSink;
        impl UiCommandSink for NoopSink {
            fn try_send(&self, _command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
                Ok(())
            }
        }

        let context = egui::Context::default();
        let snapshot = snapshot(SmsInboxSummary {
            message_count: 2,
            unread_count: 1,
            capacity: Some((2, 30)),
            status: FeatureStatus::Supported,
            has_incomplete: true,
            evicted: 0,
            ..Default::default()
        });
        let sink = NoopSink;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 600.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    render(
                        ui,
                        &snapshot,
                        &[],
                        Language::ZhCn,
                        &sink,
                        &mut SmsComposeState::default(),
                    );
                });
            },
        );
    }

    #[test]
    fn the_page_renders_message_rows_without_panicking() {
        struct NoopSink;
        impl UiCommandSink for NoopSink {
            fn try_send(&self, _command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
                Ok(())
            }
        }

        let context = egui::Context::default();
        let snapshot = snapshot(SmsInboxSummary {
            message_count: 3,
            unread_count: 1,
            capacity: Some((2, 30)),
            status: FeatureStatus::Supported,
            has_incomplete: true,
            evicted: 2,
            ..Default::default()
        });
        let messages = vec![
            message(1, Some(true), SmsStatus::Incomplete),
            outgoing_message(SmsStatus::Failed),
            message(2, Some(false), SmsStatus::Received),
        ];
        let sink = NoopSink;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 900.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    render(
                        ui,
                        &snapshot,
                        &display_messages(&messages),
                        Language::ZhCn,
                        &sink,
                        &mut SmsComposeState::default(),
                    );
                });
            },
        );
    }

    struct NoopSink;
    impl UiCommandSink for NoopSink {
        fn try_send(&self, _command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
            Ok(())
        }
    }

    #[test]
    fn both_refresh_controls_disable_during_serial_work_or_pending_reads() {
        for label in [
            t(Language::ZhCn, crate::localization::TextKey::SmsRefreshList),
            t(
                Language::ZhCn,
                crate::localization::TextKey::SmsRefreshMessages,
            ),
        ] {
            for (serial_busy, read_pending) in [(true, false), (false, true), (false, false)] {
                let context = egui::Context::default();
                let mut snapshot =
                    dji4g_application::Controller::for_test(std::time::SystemTime::UNIX_EPOCH)
                        .snapshot();
                snapshot.serial_work_busy = serial_busy;
                snapshot.sms_refresh_pending = read_pending;
                let mut state = SmsComposeState::default();
                let mut enabled = None;
                let _ = context.run(egui::RawInput::default(), |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        enabled = Some(
                            refresh_button(
                                ui,
                                Language::ZhCn,
                                &snapshot,
                                &NoopSink,
                                &mut state,
                                &label,
                            )
                            .enabled(),
                        );
                    });
                });
                assert_eq!(enabled, Some(!serial_busy && !read_pending), "{label}");
            }
        }
    }

    #[test]
    fn storage_confirmation_survives_closed_details_menu_and_obeys_confirmation_gate() {
        struct Sink {
            commands: std::sync::Mutex<Vec<UiCommand>>,
            fail: bool,
        }
        impl UiCommandSink for Sink {
            fn try_send(&self, command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
                self.commands.lock().unwrap().push(command);
                if self.fail {
                    Err(dji4g_application::UiSendError::QueueFull)
                } else {
                    Ok(())
                }
            }
        }
        for (action, busy, fail) in [
            (
                t(Language::ZhCn, crate::localization::TextKey::ButtonCancel),
                false,
                false,
            ),
            (
                t(Language::ZhCn, crate::localization::TextKey::SmsConfirmRead),
                false,
                false,
            ),
            (
                t(Language::ZhCn, crate::localization::TextKey::SmsConfirmRead),
                true,
                false,
            ),
            (
                t(Language::ZhCn, crate::localization::TextKey::SmsConfirmRead),
                false,
                true,
            ),
        ] {
            let context = egui::Context::default();
            let mut snapshot =
                dji4g_application::Controller::for_test(std::time::SystemTime::UNIX_EPOCH)
                    .snapshot();
            snapshot.serial_work_busy = busy;
            let mut state = SmsComposeState::default();
            state.auto_refresh_paused = true;
            state.storage_confirmation = Some((
                snapshot.app.device.as_ref().map(|device| device.epoch),
                snapshot.sim_epoch,
                dji4g_domain::SmsStorageId("SM".into()),
            ));
            let sink = Sink {
                commands: Default::default(),
                fail,
            };
            let mut point = None;
            let mut displayed_error = false;
            for tick in 0..5 {
                let events = if matches!(tick, 2 | 3) {
                    let pos =
                        point.expect("page should render storage confirmation with menu closed");
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: tick == 2,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                } else {
                    vec![]
                };
                let output = context.run(
                    egui::RawInput {
                        events,
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 600.0),
                        )),
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            render(ui, &snapshot, &[], Language::ZhCn, &sink, &mut state);
                        });
                    },
                );
                if tick < 2 {
                    point = output.shapes.iter().find_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape
                            && text.galley.text() == action
                        {
                            Some(text.pos + text.galley.size() * 0.5)
                        } else {
                            None
                        }
                    });
                }
                displayed_error = output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("操作队列已满"))
                });
            }
            let commands = sink.commands.lock().unwrap();
            if action == t(Language::ZhCn, crate::localization::TextKey::SmsConfirmRead) && !busy {
                assert_eq!(commands.len(), 1);
                assert!(
                    matches!(&commands[0], UiCommand::SmsReadStorage { storage } if storage.0 == "SM")
                );
                assert_eq!(state.storage_confirmation.is_some(), fail);
                if fail {
                    assert!(
                        displayed_error,
                        "queue failure must stay visible in confirmation"
                    );
                }
            } else {
                assert!(commands.is_empty());
                assert_eq!(state.storage_confirmation.is_some(), busy);
            }
        }
    }

    fn filter_fixture(language: Language) -> (ControllerSnapshot, SmsVm) {
        let snapshot = snapshot(SmsInboxSummary {
            status: FeatureStatus::Supported,
            message_count: 3,
            unread_count: 1,
            has_incomplete: true,
            ..Default::default()
        });
        let vm = sms_vm(
            &snapshot,
            &[
                message(1, Some(false), SmsStatus::Received),
                message(2, Some(true), SmsStatus::Incomplete),
                message(3, None, SmsStatus::Received),
                outgoing_message(SmsStatus::Submitted),
            ],
            language,
        );
        (snapshot, vm)
    }

    #[derive(Default)]
    struct LocalViewSink(std::sync::Mutex<Vec<UiCommand>>);
    impl UiCommandSink for LocalViewSink {
        fn try_send(&self, command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
            self.0.lock().unwrap().push(command);
            Ok(())
        }
    }

    fn inbox_frame(
        ctx: &egui::Context,
        width: f32,
        language: Language,
        state: &mut SmsComposeState,
        sink: &LocalViewSink,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let (snapshot, vm) = filter_fixture(language);
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    render_inbox(ui, &vm, &snapshot, language, sink, state);
                });
            },
        )
    }

    fn pointer_event(rect: egui::Rect, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(rect.center()),
            egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn inbox_filters_intersect_search_and_ignore_unknown_read_state_and_outgoing_records() {
        let (_, vm) = filter_fixture(Language::ZhCn);
        let mut state = SmsComposeState::default();
        assert_eq!(filtered_rows(&vm.rows, &state).len(), 3);
        state.inbox_filter = InboxFilter::Unread;
        let unread = filtered_rows(&vm.rows, &state);
        assert_eq!(unread.iter().map(|row| row.index).collect::<Vec<_>>(), [1]);
        assert_eq!(unread[0].unread, Some(true));
        state.search = "  SENSITIVE  ".into();
        assert_eq!(filtered_rows(&vm.rows, &state).len(), 1);
        state.inbox_filter = InboxFilter::Incomplete;
        assert_eq!(filtered_rows(&vm.rows, &state)[0].index, 2);
        state.search = "sent".into();
        assert!(filtered_rows(&vm.rows, &state).is_empty());
        state.outgoing = true;
        let outgoing = filtered_rows(&vm.rows, &state);
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].direction, SmsDirection::Outgoing);
        state.selected = Some(outgoing[0].stable_id);
        state.search = "not present".into();
        clear_hidden_selection(&filtered_rows(&vm.rows, &state), &mut state);
        assert!(state.selected.is_none());
    }

    #[test]
    fn filter_and_clear_search_controls_fit_narrow_and_wide_readers_in_every_language() {
        for width in [320.0, 552.0, 1100.0] {
            for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
                for outgoing in [false, true] {
                    let ctx = egui::Context::default();
                    crate::ui::initialize_visuals(&ctx);
                    let (_, vm) = filter_fixture(language);
                    let mut state = SmsComposeState::default();
                    state.outgoing = outgoing;
                    state.inbox_filter = InboxFilter::Incomplete;
                    state.selected = vm
                        .rows
                        .iter()
                        .find(|row| {
                            (row.direction == SmsDirection::Outgoing) == outgoing
                                && (outgoing || row.status == SmsStatus::Incomplete)
                        })
                        .map(|row| row.stable_id);
                    let sink = LocalViewSink::default();
                    for _ in 0..2 {
                        let _ = inbox_frame(&ctx, width, language, &mut state, &sink, vec![]);
                    }
                    for id in ["sms-search-rect", "sms-clear-search-rect"] {
                        let rect = ctx
                            .data(|data| data.get_temp::<egui::Rect>(egui::Id::new(id)))
                            .expect(id);
                        assert!(
                            rect.width() > 0.0 && rect.left() >= 0.0 && rect.right() <= width + 0.5,
                            "{id} exceeds {width}px in {language:?}: {rect:?}"
                        );
                        assert!(rect.bottom() <= 600.0);
                    }
                    let filter = ctx
                        .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("sms-filter-rect")));
                    if outgoing {
                        assert!(filter.is_none());
                    } else {
                        let rect =
                            filter.expect("inbox filter remains accessible above a narrow reader");
                        assert!(
                            rect.left() >= 0.0
                                && rect.right() <= width + 0.5
                                && rect.bottom() <= 600.0
                        );
                    }
                    assert!(sink.0.lock().unwrap().is_empty());
                }
            }
        }
    }

    #[test]
    fn the_complete_sms_page_keeps_new_controls_inside_the_minimum_content_width() {
        for width in [552.0, 1100.0] {
            for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
                let (snapshot, _) = filter_fixture(language);
                let messages = display_messages(&[
                    message(1, Some(false), SmsStatus::Received),
                    message(2, Some(true), SmsStatus::Incomplete),
                    outgoing_message(SmsStatus::Submitted),
                ]);
                let ctx = egui::Context::default();
                crate::ui::initialize_visuals(&ctx);
                let mut state = SmsComposeState::default();
                state.inbox_filter = InboxFilter::Incomplete;
                let sink = LocalViewSink::default();
                for _ in 0..2 {
                    let _ = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 900.0),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                render(ui, &snapshot, &messages, language, &sink, &mut state);
                                assert!(
                                    ui.min_rect().right() <= width + 0.5,
                                    "SMS page exceeds {width}px in {language:?}: {:?}",
                                    ui.min_rect()
                                );
                            });
                        },
                    );
                }
                assert!(sink.0.lock().unwrap().is_empty());
            }
        }
    }

    #[test]
    fn choosing_an_inbox_filter_clears_hidden_detail_and_never_dispatches_a_device_command() {
        for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
            let ctx = egui::Context::default();
            crate::ui::initialize_visuals(&ctx);
            let (_, vm) = filter_fixture(language);
            let mut state = SmsComposeState::default();
            state.selected = Some(vm.rows[1].stable_id);
            let sink = LocalViewSink::default();
            for _ in 0..2 {
                let _ = inbox_frame(&ctx, 552.0, language, &mut state, &sink, vec![]);
            }
            let combo = ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("sms-filter-rect")))
                .unwrap();
            for pressed in [true, false] {
                let _ = inbox_frame(
                    &ctx,
                    552.0,
                    language,
                    &mut state,
                    &sink,
                    pointer_event(combo, pressed),
                );
            }
            let _ = inbox_frame(&ctx, 552.0, language, &mut state, &sink, vec![]);
            let unread = ctx
                .data(|data| {
                    data.get_temp::<egui::Rect>(egui::Id::new((
                        "sms-filter-choice",
                        InboxFilter::Unread as u8,
                    )))
                })
                .expect("filter popup is open");
            for pressed in [true, false] {
                let _ = inbox_frame(
                    &ctx,
                    552.0,
                    language,
                    &mut state,
                    &sink,
                    pointer_event(unread, pressed),
                );
            }
            assert_eq!(state.inbox_filter, InboxFilter::Unread);
            assert!(state.selected.is_none());
            assert!(sink.0.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn search_clear_button_and_shortcuts_update_the_view_without_stealing_decision_keys() {
        let language = Language::EnUs;
        let ctx = egui::Context::default();
        crate::ui::initialize_visuals(&ctx);
        let mut state = SmsComposeState::default();
        state.search = "absent".into();
        let sink = LocalViewSink::default();
        let interaction = || {
            ctx.data(|data| {
                data.get_temp::<(egui::Id, egui::Rect, egui::Rect, bool, bool, bool)>(
                    egui::Id::new("sms-clear-search-interaction"),
                )
            })
            .unwrap()
        };
        let mut click_evidence = Vec::new();
        for _ in 0..2 {
            let _ = inbox_frame(&ctx, 552.0, language, &mut state, &sink, vec![]);
            click_evidence.push(interaction());
        }
        let clear = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("sms-clear-search-rect")))
            .unwrap();
        for pressed in [true, false] {
            let _ = inbox_frame(
                &ctx,
                552.0,
                language,
                &mut state,
                &sink,
                pointer_event(clear, pressed),
            );
            click_evidence.push(interaction());
        }
        assert!(
            state.search.is_empty(),
            "clear target {clear:?}; frames (id, interaction rect, clip, hover, down, click): {click_evidence:?}"
        );
        let key = |key, modifiers| {
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }]
        };
        ctx.memory_mut(|memory| memory.surrender_focus(search_id()));
        let _ = inbox_frame(
            &ctx,
            552.0,
            language,
            &mut state,
            &sink,
            key(egui::Key::F, egui::Modifiers::CTRL),
        );
        assert!(ctx.memory(|memory| memory.has_focus(search_id())));
        state.search = "text".into();
        let _ = inbox_frame(
            &ctx,
            552.0,
            language,
            &mut state,
            &sink,
            key(egui::Key::Escape, egui::Modifiers::NONE),
        );
        assert!(state.search.is_empty());
        assert!(
            ctx.memory(|memory| memory.has_focus(search_id())),
            "Esc clearing preserves focus for the next search"
        );
        let (_, vm) = filter_fixture(language);
        state.selected = Some(vm.rows[0].stable_id);
        let _ = inbox_frame(
            &ctx,
            552.0,
            language,
            &mut state,
            &sink,
            vec![egui::Event::Text("absent".into())],
        );
        assert_eq!(state.search, "absent");
        assert!(
            state.selected.is_none(),
            "typing must clear a hidden reader in the same frame"
        );
        state.search = "keep".into();
        state.selected = Some([9; 32]);
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new(("sms-row-delete-armed", [9u8; 32])),
                Instant::now(),
            )
        });
        let _ = inbox_frame(
            &ctx,
            552.0,
            language,
            &mut state,
            &sink,
            key(egui::Key::Escape, egui::Modifiers::NONE),
        );
        assert_eq!(state.search, "keep");
        state.open = true;
        ctx.memory_mut(|memory| memory.surrender_focus(search_id()));
        let _ = inbox_frame(
            &ctx,
            552.0,
            language,
            &mut state,
            &sink,
            key(egui::Key::F, egui::Modifiers::CTRL),
        );
        assert!(!ctx.memory(|memory| memory.has_focus(search_id())));
        assert_eq!(state.search, "keep");
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn an_empty_local_filter_explains_the_filter_without_claiming_the_module_inbox_is_empty() {
        for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
            let (snapshot, mut vm) = filter_fixture(language);
            vm.rows.retain(|row| row.unread != Some(true));
            let mut state = SmsComposeState::default();
            state.inbox_filter = InboxFilter::Unread;
            let sink = LocalViewSink::default();
            let ctx = egui::Context::default();
            crate::ui::initialize_visuals(&ctx);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(552.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        render_inbox(ui, &vm, &snapshot, language, &sink, &mut state);
                    });
                },
            );
            let texts: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text.galley.text())
                    } else {
                        None
                    }
                })
                .collect();
            assert!(texts.contains(&t(language, TextKey::SmsEmptyFilter).as_str()));
            assert!(!texts.contains(&t(language, TextKey::SmsInboxEmpty).as_str()));
            assert!(!texts.contains(&t(language, TextKey::SmsRefreshMessages).as_str()));
            assert!(sink.0.lock().unwrap().is_empty());
        }
    }

    /// Render the page into a window of `width` x `height` logical points and report the heights
    /// the workspace and the message list really received. Two passes are run so the values from
    /// the first frame (used by egui to size scroll areas) are settled by the second.
    fn measured_heights(width: f32, height: f32) -> (f32, f32) {
        let mut snapshot = snapshot(SmsInboxSummary {
            message_count: 3,
            unread_count: 1,
            capacity: Some((2, 30)),
            status: FeatureStatus::Supported,
            has_incomplete: true,
            evicted: 0,
            ..Default::default()
        });
        snapshot.sms_read_report = Some(dji4g_domain::SmsReadReport {
            raw_records: 23,
            decoded_records: 23,
            ..Default::default()
        });
        let messages = vec![
            message(1, Some(true), SmsStatus::Received),
            message(2, Some(false), SmsStatus::Received),
            message(3, Some(false), SmsStatus::Received),
        ];
        let sink = NoopSink;
        let mut state = SmsComposeState::default();
        let context = egui::Context::default();
        crate::ui::initialize_visuals(&context);
        for _ in 0..2 {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        render(
                            ui,
                            &snapshot,
                            &display_messages(&messages),
                            Language::ZhCn,
                            &sink,
                            &mut state,
                        );
                    });
                },
            );
        }
        context.data(|data| {
            (
                data.get_temp::<f32>(workspace_height_id()).unwrap_or(0.0),
                data.get_temp::<f32>(list_viewport_id()).unwrap_or(0.0),
            )
        })
    }

    #[test]
    fn material_sms_page_keeps_three_message_rows_with_read_report() {
        let (_, list) = measured_heights(800.0, 500.0);
        assert!(list >= 180.0, "usable message viewport is only {list}");
    }

    /// The user-visible regression: the list viewport used to be capped, so making the window
    /// taller did not show more messages. It must now track the window one-for-one.
    #[test]
    fn the_list_viewport_grows_with_the_window_instead_of_hitting_a_cap() {
        let (short_workspace, short_list) = measured_heights(1100.0, 760.0);
        let (tall_workspace, tall_list) = measured_heights(1100.0, 1000.0);
        let workspace_gain = tall_workspace - short_workspace;
        let list_gain = tall_list - short_list;
        eprintln!(
            "1100x760 -> workspace {short_workspace}, list {short_list}; \
             1100x1000 -> workspace {tall_workspace}, list {tall_list}"
        );
        assert!(
            (workspace_gain - 240.0).abs() <= 16.0,
            "workspace gain {workspace_gain} (short {short_workspace}, tall {tall_workspace})"
        );
        assert!(
            (list_gain - 240.0).abs() <= 16.0,
            "list gain {list_gain} (short {short_list}, tall {tall_list})"
        );
        // The old implementation froze the body at 340 - 88 - 48 = 204 points.
        assert!(
            short_list > 204.0 + 16.0,
            "760-point window still looks capped: {short_list}"
        );
    }

    #[test]
    fn the_workspace_stays_inside_the_page_and_never_panics_at_odd_sizes() {
        for (width, height) in [
            (320.0_f32, 600.0_f32),
            (719.0, 600.0),
            (720.0, 600.0),
            (800.0, 600.0),
            (1440.0, 1000.0),
            (1100.0, 300.0),
        ] {
            let (workspace, list) = measured_heights(width, height);
            assert!(
                workspace >= 0.0 && workspace.is_finite(),
                "{width}x{height} workspace {workspace}"
            );
            assert!(
                list >= 0.0 && list.is_finite(),
                "{width}x{height} list {list}"
            );
            assert!(
                list <= workspace + 0.5,
                "{width}x{height} list {list} exceeds workspace {workspace}"
            );
        }
    }
}

#[cfg(test)]
mod read_only_regressions {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Sink(Mutex<Vec<UiCommand>>);
    impl UiCommandSink for Sink {
        fn try_send(&self, command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
            self.0.lock().unwrap().push(command);
            Ok(())
        }
    }
    #[test]
    fn generic_sms_page_never_reads_on_entry_click_or_expired_timer() {
        for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
            let snapshot = dji4g_application::ReducerState::test_ready_for(
                dji4g_domain::QUECTEL_GENERIC,
                std::time::SystemTime::now(),
            )
            .snapshot();
            let ctx = egui::Context::default();
            crate::ui::apply_style(&ctx);
            let sink = Sink::default();
            let mut state = SmsComposeState::default();
            let mut button = egui::Rect::NOTHING;
            for tick in 0..5 {
                let events = if tick >= 3 {
                    vec![
                        egui::Event::PointerMoved(button.center()),
                        egui::Event::PointerButton {
                            pos: button.center(),
                            button: egui::PointerButton::Primary,
                            pressed: tick == 3,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                } else {
                    vec![]
                };
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(900.0, 700.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            render(ui, &snapshot, &[], language, &sink, &mut state);
                            let response = refresh_button(
                                ui, language, &snapshot, &sink, &mut state, "Refresh",
                            );
                            assert!(!response.enabled());
                            button = response.rect;
                        });
                    },
                );
            }
            state.auto_refresh(
                Instant::now() + Duration::from_secs(65),
                true,
                false,
                false,
                &sink,
            );
            assert!(state.module_read_only);
            assert!(sink.0.lock().unwrap().is_empty());
        }
    }
}
