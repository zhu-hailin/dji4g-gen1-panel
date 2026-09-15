//! Official native dialog surface for repair confirmation and results.
//!
//! Every side-effectful operation is confirmed and reported through the system's own
//! `MessageBoxW` (MB_YESNO for confirmation, MB_OK for the result) — the same native dialogs
//! every Windows application uses — instead of an in-app window. The backend is a trait so
//! headless tests can record requests without opening real message boxes.
//!
//! A confirmation box is composed and presented by the button click itself (the message follows
//! from the click's own action metadata), so it never waits for the controller round-trip that
//! publishes the prepared plan.  Result boxes stay snapshot-driven: a finished operation opens
//! one informational box, dismissed through the ordinary command sink.

use std::sync::Arc;

use dji4g_application::{ActionKindTag, ControllerSnapshot, OperationState};
use dji4g_domain::{ActionKind, DisruptionLevel, RiskLevel};

use crate::localization::{
    Language, LocalizedText, TextKey, action_tag_key, disruption_level, risk_level,
};

/// A blockable native dialog; implementations run on a dedicated worker thread, never on the
/// egui frame thread.  `owner` is the panel's raw `HWND` when available; an owned box is modal
/// to the panel, so Windows itself disables the window for the duration.
pub trait NativeDialogBackend: Send + Sync {
    /// Present a Yes/No confirm box. Blocks until the user answers; `true` = Yes.
    fn confirm(&self, owner: Option<isize>, title: &str, message: &str) -> bool;

    /// Present an informational box. Blocks until dismissed.
    fn inform(&self, owner: Option<isize>, title: &str, message: &str);
}

/// Production backend: the system's own modal message boxes.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeMessageBoxBackend;

impl NativeDialogBackend for NativeMessageBoxBackend {
    fn confirm(&self, owner: Option<isize>, title: &str, message: &str) -> bool {
        dji4g_windows_platform::confirm_message_box(owner, title, message)
    }

    fn inform(&self, _owner: Option<isize>, title: &str, message: &str) {
        // The platform's informational box has no owner by design: it is also the surface for
        // the tray-side 「热点状态」 answer, which must never be tied to a window lifetime.
        dji4g_windows_platform::show_message_box(title, message);
    }
}

/// A dialog the panel wants to present, with the exact strings already composed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DialogRequest {
    Result {
        operation_id: u64,
        title: String,
        message: String,
    },
}

/// Compose the native confirm box for an action the user is about to trigger.  The message is
/// built synchronously from the click itself — never from a later snapshot — so the box can
/// appear the moment the button is pressed while the controller prepares the plan in parallel.
/// The content matches the box the snapshot-driven flow used to compose.  `None` when the action
/// carries no confirmation metadata (only `Refresh`): such actions are never confirmed.
#[must_use]
pub fn confirm_message_for_action(
    action: &ActionKind,
    requires_elevation: bool,
    disruption: Option<DisruptionLevel>,
    risk: Option<RiskLevel>,
    dev_mode: bool,
    language: Language,
) -> Option<(String, String)> {
    let tag = ActionKindTag::from_action(action)?;
    let disruption = disruption?;
    let risk = risk?;
    let mut message = format!(
        "操作：{}\n",
        LocalizedText::new(language, action_tag_key(tag)).text
    );
    message.push_str(&format!(
        "中断：{}\n",
        LocalizedText::new(language, disruption_level(disruption)).text
    ));
    message.push_str(&format!(
        "风险：{}\n",
        LocalizedText::new(language, risk_level(risk)).text
    ));
    message.push_str(&format!(
        "提权：{}\n",
        if requires_elevation {
            LocalizedText::new(language, TextKey::ConfirmationElevationRequired).text
        } else {
            LocalizedText::new(language, TextKey::ConfirmationElevationNotRequired).text
        }
    ));
    message.push_str(&format!(
        "{}\n{}",
        LocalizedText::new(language, TextKey::ConfirmationStateRecheck).text,
        LocalizedText::new(language, TextKey::ConfirmationNoAutomaticRetry).text
    ));
    if dev_mode && requires_elevation {
        message.push('\n');
        message.push_str(&LocalizedText::new(language, TextKey::ConfirmationDevModeWarning).text);
    }
    Some((
        LocalizedText::new(language, TextKey::ConfirmationTitle).text,
        message,
    ))
}

/// Compose the native result box for a finished operation.
#[must_use]
pub fn result_request(
    snapshot: &ControllerSnapshot,
    operation_id: u64,
    language: Language,
) -> Option<DialogRequest> {
    let operation = snapshot.operation.as_ref()?;
    if operation.operation_id != operation_id {
        return None;
    }
    let OperationState::Finished { outcome, .. } = &operation.state else {
        return None;
    };
    let message = crate::ui::operation_outcome_text(outcome, language).text;
    Some(DialogRequest::Result {
        operation_id,
        title: LocalizedText::new(language, TextKey::OperationResultTitle).text,
        message,
    })
}

/// Shared arc alias so callers can hold `Option<Arc<dyn NativeDialogBackend>>`.
pub type DialogBackend = Arc<dyn NativeDialogBackend>;
