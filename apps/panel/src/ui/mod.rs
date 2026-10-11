//! Pure view-model projection plus small egui rendering helpers.

use std::time::{Duration, SystemTime};

use dji4g_application::{
    DiagnosticCheckId, DiagnosticCheckState, DiagnosticSet, OperationPhase, PreparedActionSnapshot,
};
use dji4g_domain::{
    AppSnapshot, Availability, DeviceEpoch, Freshness, HotspotStatus, LimitedReason,
    OperationOutcome, UnavailableReason,
};
use eframe::egui::{self, Color32, RichText, Shape, Stroke, Ui};

use crate::localization::{
    Language, LocalizedText, TextArgs, TextKey, availability_reason, availability_title,
    diagnostic_state, error_text, failure_text, format_text_in, freshness_key, hotspot_title,
    hotspot_unsupported_reason, rollback_outcome, unexecuted_reason,
};

/// One catalog string in the language this page was rendered with.
fn t(language: crate::localization::Language, key: crate::localization::TextKey) -> String {
    crate::localization::LocalizedText::new(language, key).text
}

pub mod action_availability;
pub(crate) mod components;
pub mod device_tools;
pub mod diagnostics;
pub(crate) mod driver_setup;
pub mod module_network_check;
pub mod network_assistance;
pub mod onboarding;
pub mod overview;
mod rate_chart;
pub mod repairs;
pub mod settings;
pub mod sms;
pub(crate) mod sms_archive;
pub(crate) mod sms_balance;
pub(crate) mod sms_freshness;
pub(crate) mod sms_layout;
pub(crate) mod wireless;

pub use diagnostics::{DiagnosticRowVm, DiagnosticsVm, diagnostics_vm};
pub use overview::{OverviewVm, overview_vm, overview_vm_with_probes};
pub use repairs::{RepairActionVm, RepairsVm, repairs_vm};
pub use settings::{AutostartVm, SettingsVm, settings_vm, settings_vm_from};
pub use sms::{SmsRowVm, SmsVm, sms_row_vm, sms_status_text, sms_vm};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusTone {
    Positive,
    Caution,
    Negative,
    Progress,
    Neutral,
}

impl StatusTone {
    /// Tone is carried by exactly two hues: green for correct/available, red for wrong/failed.
    /// The in-between states stay on the grey ramp, so a page can never turn into a colour chart.
    #[must_use]
    pub fn color(self) -> Color32 {
        match self {
            Self::Positive => scale::success(),
            Self::Negative => scale::danger(),
            Self::Caution | Self::Progress => scale::secondary(),
            Self::Neutral => scale::faint(),
        }
    }

    /// Tinted background for a status surface, or `None` to use the plain sunken surface.
    #[must_use]
    pub fn fill(self) -> Option<Color32> {
        match self {
            Self::Positive => Some(scale::success_fill()),
            Self::Negative => Some(scale::danger_fill()),
            Self::Caution | Self::Progress | Self::Neutral => None,
        }
    }

    /// The glyph that carries a tone in front of its text.
    ///
    /// Deliberately never a triangle: a disclosure triangle used to sit in front of every
    /// collapsible section, so a triangular marker in front of a card reads as "click to open".
    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Positive => "●",
            Self::Caution => "◆",
            Self::Negative => "■",
            Self::Progress => "◌",
            Self::Neutral => "○",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayValue {
    pub text: String,
    pub copyable: bool,
}

impl DisplayValue {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            copyable: false,
        }
    }

    #[must_use]
    pub fn copyable(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            copyable: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvailabilityVm {
    pub tone: StatusTone,
    pub title: LocalizedText,
    pub reason: LocalizedText,
    pub freshness: LocalizedText,
    pub is_confirmed_usable: bool,
    /// True while the panel is actively collecting evidence (detecting, loading, or re-scanning
    /// stale data), so the header can show a loading indicator. Derived from the presented tone so
    /// it can never disagree with what the user sees.
    pub is_loading: bool,
}

#[must_use]
/// The most precise honest reason for a Limited/Unavailable verdict: when it traces to a
/// specific failed diagnostic row, that row's stable failure text beats the generic per-reason
/// sentence. The generic sentence stays the fallback whenever the row carries no code, is still
/// running/unexecuted, or has already aged past its TTL.
pub(crate) fn availability_reason_with_diagnostics(
    snapshot: &AppSnapshot,
    diagnostics: &DiagnosticSet,
    now: SystemTime,
    language: Language,
) -> LocalizedText {
    let generic = || availability_reason(snapshot.availability);
    let check_id = match snapshot.availability {
        Availability::Limited(LimitedReason::DnsFailure) => Some(DiagnosticCheckId::BoundDns),
        Availability::Limited(LimitedReason::AtControlUnavailable) => {
            Some(DiagnosticCheckId::AtControl)
        }
        Availability::Unavailable(UnavailableReason::CellularRejected) => {
            Some(DiagnosticCheckId::Cellular)
        }
        Availability::Unavailable(UnavailableReason::NoUsableAddressOrRoute) => {
            Some(DiagnosticCheckId::WindowsAdapter)
        }
        Availability::Unavailable(UnavailableReason::BoundPublicProbeFailed)
        | Availability::Unavailable(UnavailableReason::NoBoundReachability) => {
            Some(DiagnosticCheckId::BoundPublic)
        }
        _ => None,
    };
    let Some(check_id) = check_id else {
        return LocalizedText::new(language, generic());
    };
    if snapshot.freshness != Freshness::Fresh {
        return LocalizedText::new(language, generic());
    }
    let check = diagnostics.get(check_id);
    let code = match &check.state {
        DiagnosticCheckState::Failed { code } | DiagnosticCheckState::Unavailable { code } => code,
        _ => return LocalizedText::new(language, generic()),
    };
    if check.expires_at.is_some_and(|expiry| now >= expiry) {
        return LocalizedText::new(language, generic());
    }
    failure_text(code, language)
}

pub fn availability_vm(
    snapshot: &AppSnapshot,
    diagnostics: &DiagnosticSet,
    now: SystemTime,
    language: Language,
) -> AvailabilityVm {
    let status = snapshot.availability;
    let base_tone = match status {
        Availability::Available => StatusTone::Positive,
        Availability::Limited(_) => StatusTone::Caution,
        Availability::Unavailable(_) => StatusTone::Negative,
        Availability::Detecting => StatusTone::Progress,
        Availability::NotDetected | Availability::UnsupportedDevice => StatusTone::Neutral,
    };

    // The UI is a second safety boundary: stale/unknown evidence can never retain a green
    // presentation even if a producer publishes an old Available value while refreshing.
    let tone = if snapshot.freshness == Freshness::Fresh {
        base_tone
    } else {
        StatusTone::Progress
    };
    let title_key = match snapshot.freshness {
        Freshness::Fresh => availability_title(status),
        Freshness::Stale => TextKey::StatusExpired,
        Freshness::Unknown => {
            if matches!(status, Availability::Detecting) {
                TextKey::AvailabilityDetectingTitle
            } else {
                TextKey::StatusLoading
            }
        }
    };
    let reason = availability_reason_with_diagnostics(snapshot, diagnostics, now, language);
    let freshness = freshness_text(snapshot, now, language);
    AvailabilityVm {
        tone,
        title: LocalizedText::new(language, title_key),
        reason,
        freshness,
        is_confirmed_usable: matches!(status, Availability::Available)
            && snapshot.freshness == Freshness::Fresh,
        is_loading: matches!(tone, StatusTone::Progress),
    }
}

#[must_use]
pub fn freshness_text(
    snapshot: &AppSnapshot,
    now: SystemTime,
    language: Language,
) -> LocalizedText {
    let base = LocalizedText::new(language, freshness_key(snapshot.freshness));
    if snapshot.freshness == Freshness::Unknown {
        return base;
    }
    let age = now.duration_since(snapshot.observed_at).ok();
    let Some(age) = age else {
        return base;
    };
    let age_text = format_age(age, language);
    let mut result = format_text_in(
        language,
        TextKey::ObservedAgo,
        &TextArgs::age(age_text.text),
    );
    result.text = format!("{} · {}", base.text, result.text);
    result
}

#[must_use]
pub fn format_age(age: Duration, language: Language) -> LocalizedText {
    let seconds = age.as_secs();
    let (key, value) = if seconds < 60 {
        (TextKey::AgeSeconds, seconds)
    } else if seconds < 3600 {
        (TextKey::AgeMinutes, seconds / 60)
    } else {
        (TextKey::AgeHours, seconds / 3600)
    };
    format_text_in(language, key, &TextArgs::count(value as usize))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotspotAction {
    Enable,
    Disable,
    Retry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HotspotVm {
    pub tone: StatusTone,
    pub status: LocalizedText,
    pub reason: LocalizedText,
    pub action: Option<HotspotAction>,
    pub busy: bool,
}

#[must_use]
pub fn hotspot_vm(status: HotspotStatus, language: Language) -> HotspotVm {
    match status {
        HotspotStatus::Unsupported(reason) => HotspotVm {
            tone: StatusTone::Neutral,
            status: LocalizedText::new(language, hotspot_title(status)),
            reason: LocalizedText::new(language, hotspot_unsupported_reason(reason)),
            action: None,
            busy: false,
        },
        HotspotStatus::Off => HotspotVm {
            tone: StatusTone::Neutral,
            status: LocalizedText::new(language, TextKey::HotspotOff),
            reason: LocalizedText::new(language, TextKey::ValueNotApplicable),
            action: Some(HotspotAction::Enable),
            busy: false,
        },
        HotspotStatus::Starting => HotspotVm {
            tone: StatusTone::Progress,
            status: LocalizedText::new(language, TextKey::HotspotStarting),
            reason: LocalizedText::new(language, TextKey::OperationPreparing),
            action: None,
            busy: true,
        },
        HotspotStatus::On {
            clients: Some(count),
        } => HotspotVm {
            tone: StatusTone::Positive,
            status: format_text_in(
                language,
                TextKey::HotspotOnWithClients,
                &TextArgs::client_count(count),
            ),
            reason: LocalizedText::new(language, TextKey::ValueNotApplicable),
            action: Some(HotspotAction::Disable),
            busy: false,
        },
        HotspotStatus::On { clients: None } => HotspotVm {
            tone: StatusTone::Positive,
            status: LocalizedText::new(language, TextKey::HotspotOnClientsUnknown),
            reason: LocalizedText::new(language, TextKey::ValueNotApplicable),
            action: Some(HotspotAction::Disable),
            busy: false,
        },
        HotspotStatus::Stopping => HotspotVm {
            tone: StatusTone::Progress,
            status: LocalizedText::new(language, TextKey::HotspotStopping),
            reason: LocalizedText::new(language, TextKey::OperationVerifying),
            action: None,
            busy: true,
        },
        HotspotStatus::Failed { code } => HotspotVm {
            tone: StatusTone::Negative,
            status: LocalizedText::new(language, TextKey::HotspotFailed),
            reason: LocalizedText::new(language, error_text(code)),
            action: Some(HotspotAction::Retry),
            busy: false,
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticStateVm {
    pub tone: StatusTone,
    pub label: LocalizedText,
    pub detail: Option<LocalizedText>,
}

#[must_use]
pub fn diagnostic_state_vm(state: &DiagnosticCheckState, language: Language) -> DiagnosticStateVm {
    let (tone, detail) = match state {
        DiagnosticCheckState::Unexecuted { reason } => (
            StatusTone::Neutral,
            Some(LocalizedText::new(language, unexecuted_reason(*reason))),
        ),
        DiagnosticCheckState::Running { .. } => (StatusTone::Progress, None),
        DiagnosticCheckState::Passed => (StatusTone::Positive, None),
        DiagnosticCheckState::Failed { code } => {
            (StatusTone::Negative, Some(failure_text(code, language)))
        }
        DiagnosticCheckState::Unavailable { code } => {
            (StatusTone::Neutral, Some(failure_text(code, language)))
        }
        DiagnosticCheckState::Expired => (StatusTone::Caution, None),
    };
    DiagnosticStateVm {
        tone,
        label: LocalizedText::new(language, diagnostic_state(state)),
        detail,
    }
}

#[must_use]
pub fn operation_outcome_text(outcome: &OperationOutcome, language: Language) -> LocalizedText {
    match outcome {
        OperationOutcome::Applied { .. } => {
            LocalizedText::new(language, TextKey::OperationOutcomeApplied)
        }
        OperationOutcome::Failed { code, rollback } => {
            let mut result = LocalizedText::new(language, TextKey::OperationOutcomeFailed);
            result.text = format!(
                "{} {}；{}",
                result.text,
                LocalizedText::new(language, error_text(*code)),
                LocalizedText::new(language, rollback_outcome(*rollback))
            );
            result
        }
        OperationOutcome::OutcomeUnknown { code } => {
            let mut result = LocalizedText::new(language, TextKey::OperationOutcomeUnknown);
            result.text = format!(
                "{} {}",
                result.text,
                LocalizedText::new(language, error_text(*code))
            );
            result
        }
    }
}

/// Saving the USB configuration does not prove that the new network mode is active.
#[must_use]
pub fn operation_result_text(
    operation: &dji4g_application::OperationUiSnapshot,
    language: Language,
) -> LocalizedText {
    let dji4g_application::OperationState::Finished { outcome, .. } = &operation.state else {
        return LocalizedText::new(language, TextKey::OperationVerifying);
    };
    if operation.action == dji4g_application::ActionKindTag::SetVerifiedUsbNetworkProfile
        && matches!(outcome, OperationOutcome::Applied { .. })
    {
        LocalizedText::new(language, TextKey::OperationUsbConfigurationSaved)
    } else {
        operation_outcome_text(outcome, language)
    }
}

#[must_use]
pub fn operation_phase_text(
    phase: OperationPhase,
    action: Option<LocalizedText>,
    language: Language,
) -> LocalizedText {
    match phase {
        OperationPhase::Revalidating => {
            LocalizedText::new(language, TextKey::OperationRevalidating)
        }
        OperationPhase::AwaitingElevation => {
            LocalizedText::new(language, TextKey::OperationAwaitingElevation)
        }
        OperationPhase::Executing => action.map_or_else(
            || LocalizedText::new(language, TextKey::OperationExecuting),
            |operation| {
                format_text_in(
                    language,
                    TextKey::OperationExecuting,
                    &TextArgs::operation(operation),
                )
            },
        ),
        OperationPhase::Verifying => LocalizedText::new(language, TextKey::OperationVerifying),
    }
}

#[must_use]
pub fn prepared_action_text(
    prepared: &PreparedActionSnapshot,
    language: Language,
) -> LocalizedText {
    crate::localization::action_tag_text(prepared.action, language)
}

pub(crate) mod icons;
/// The shared dialog component. It is complete and covered by its own tests, but no page calls it
/// yet — wiring it in is a separate change — so its API is explicitly allowed to be unused rather
/// than deleted and rebuilt later.
// The dialog is one small component with a deliberately complete surface: every option it exposes
// (a destructive title, a non-dismissible card, an action that stays open, the measured footer
// rects) is covered by the module's own tests, but the panel does not need all of them yet. The
// allowance is scoped to this module so it cannot hide dead code anywhere else.
#[allow(dead_code)]
pub(crate) mod modal;
pub(crate) mod shell;
pub(crate) mod theme;
/// Install the production font and visual styles without opening any native surfaces.
pub fn initialize_visuals(ctx: &egui::Context) {
    let _ = crate::font::install_chinese_font(ctx);
    theme::apply_style(ctx);
}

#[cfg(test)]
pub(crate) use theme::apply_style;
/// Re-exported for the crate's own tests, which pin one theme explicitly instead of following the
/// system preference that `apply_style` resolves at runtime.
#[cfg(test)]
pub(crate) use theme::style_root;
pub(crate) use theme::{apply_settings_theme, scale};

/// Pin the active theme, for review captures that must render one specific ramp instead of
/// following the operating system.
///
/// Production never calls this: the settings page owns the user's real choice, and `apply_style`
/// reads it every pass. A capture pass needs the light ramp pinned because the machine that takes
/// the screenshots reports a dark system preference.
#[cfg(debug_assertions)]
pub fn set_review_theme(ctx: &egui::Context, preference: egui::ThemePreference) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("review-theme-override"), preference));
    theme::store_theme(ctx, preference);
}

/// A flat, shadowless card. Every page composes its content from these, which is what makes the
/// blocks line up: identical fill, radius, padding and separation.
///
/// The card hugs its content. It is never padded out to match a neighbour: a card stretched to
/// someone else's height just holds a slab of empty surface, which reads as a layout bug no matter
/// how level the edges are.
pub(crate) fn section_frame(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    egui::Frame::none()
        .fill(scale::surface())
        .rounding(egui::Rounding::same(scale::RADIUS_CONTAINER))
        .stroke(Stroke::new(1.0_f32, scale::line()))
        .inner_margin(egui::Margin::same(scale::CARD_PAD))
        .outer_margin(egui::Margin {
            bottom: scale::BLOCK_GAP,
            ..Default::default()
        })
        .show(ui, |ui| {
            // Stretch every card to the panel width so the blocks align as even columns instead
            // of hugging their content and leaving ragged right edges.
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = scale::SECTION_GAP;
            add_contents(ui);
        });
}

/// Lay two cards side by side at half the content width.
///
/// This is the page's default arrangement on a desktop-width window: content that used to be
/// stacked into one narrow column now fills the width, so a card can be read without scrolling
/// past its neighbour. Below `TWO_COLUMN_MIN_WIDTH` the page falls back to a single column, which
/// is what keeps the narrow layout working instead of squeezing both cards.
///
/// Each card keeps its own height. Rows are therefore aligned at the top and may end at different
/// depths — the honest shape of uneven content, and the only shape that wastes no space.
pub(crate) const TWO_COLUMN_MIN_WIDTH: f32 = 760.0;

pub(crate) fn two_columns(ui: &mut Ui, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    let available = ui.available_width();
    if available < TWO_COLUMN_MIN_WIDTH {
        left(ui);
        right(ui);
        return;
    }
    let gap = scale::BLOCK_GAP;
    let column_width = ((available - gap) / 2.0).floor();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(
            egui::vec2(column_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_width(column_width);
                ui.set_max_width(column_width);
                left(ui);
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(column_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_width(column_width);
                ui.set_max_width(column_width);
                right(ui);
            },
        );
    });
}

/// A label/value grid with the reference's 88px label column, shared by every page.
pub(crate) fn info_grid(ui: &mut Ui, id: &str, rows: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .min_col_width(scale::LABEL_COLUMN)
        .spacing([scale::COLUMN_GAP, scale::ROW_GAP])
        .show(ui, rows);
}

/// Clock time (`HH:MM:SS`) derived from the seconds since the UNIX epoch.
///
/// This deliberately performs no timezone conversion — the panel has no date-time dependency and
/// the operating-system zone is not modelled here — so the stamp is the UTC wall clock, presented
/// as a plain time-of-day. `None` for instants before the epoch rather than a fabricated 00:00.
#[must_use]
pub fn clock_hms(at: SystemTime) -> Option<String> {
    let seconds = at.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    let seconds_of_day = seconds % 86_400;
    Some(format!(
        "{:02}:{:02}:{:02}",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    ))
}

/// Interface link speed in the human-readable form the overview asks for: bit/s as Mbps with one
/// decimal. This is the negotiated link rate, never a measured throughput.
#[must_use]
pub fn format_mbps(bits_per_second: u64) -> String {
    format!("{:.1} Mbps", bits_per_second as f64 / 1_000_000.0)
}

/// Add user-facing text with wrapping forced at the current available width.
///
/// egui's default label mode follows the parent layout and can elide long Chinese strings in a
/// horizontal row. Every variable-length status, explanation, error, and confirmation string
/// should use this helper so the compact panel never loses its key reason text.
pub(crate) fn wrapped_label(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(egui::Label::new(text).wrap())
}

/// Group heading inside a page.
pub(crate) fn section_heading(text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .size(scale::HEADING)
        .strong()
        .color(scale::ink())
}

/// Label of a label/value row. Muted rather than bold so the value carries the emphasis and a
/// column of values reads as one continuous vertical run.
pub(crate) fn field_label(text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .size(scale::BODY)
        .color(scale::secondary())
}

/// Hints, timestamps, and descriptions: the quietest tier of the scale.
pub(crate) fn meta_text(text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .size(scale::META)
        .color(scale::faint())
}

/// Supporting evidence: the same quiet size as meta copy, in the mid tier so it reads apart from
/// timestamps without introducing a colour.
pub(crate) fn detail_text(text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .size(scale::META)
        .color(scale::secondary())
}

/// Presentation-side ring of the measured throughput samples backing the overview chart.
/// Snapshots are immutable, so the rolling window lives here and only the newest sample ever
/// enters; `None` samples render as an honest gap.
#[derive(Debug)]
pub struct RateHistory {
    samples: std::collections::VecDeque<(SystemTime, Option<u64>, Option<u64>)>,
    capacity: usize,
}

/// Ring capacity: 60 samples at the 1 s cadence cover the last minute of throughput.
pub(crate) const RATE_HISTORY_CAPACITY: usize = 60;

/// Nominal backend sampling cadence. The UI records actual counter timestamps once each,
/// rather than repeating the latest publication on every frame. The dashboard derives its
/// window label from capacity × nominal period.
pub(crate) const RATE_SAMPLE_PERIOD: Duration = Duration::from_secs(1);

impl Default for RateHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl RateHistory {
    #[must_use]
    pub fn new() -> Self {
        Self {
            samples: std::collections::VecDeque::with_capacity(RATE_HISTORY_CAPACITY),
            capacity: RATE_HISTORY_CAPACITY,
        }
    }

    pub fn push(&mut self, sample: (SystemTime, Option<u64>, Option<u64>)) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn iter(&self) -> impl Iterator<Item = &(SystemTime, Option<u64>, Option<u64>)> {
        self.samples.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    #[must_use]
    pub fn last(&self) -> Option<&(SystemTime, Option<u64>, Option<u64>)> {
        self.samples.back()
    }
}

/// Presentation-side ring of the module-temperature samples (§7.5) behind the overview's
/// 「模块温度」 trend.
///
/// One sample is recorded per published evidence cycle, stamped with that cycle's `observed_at`
/// rather than the frame time, so the horizontal axis is real elapsed time: a cycle that was
/// skipped (the port was busy with SMS or a tool task) leaves a wider gap instead of a fabricated
/// point.  `None` is an honest 「本次未读取到」 and stays a gap — never a zero.
#[derive(Debug, Default)]
pub struct TemperatureHistory {
    samples: std::collections::VecDeque<(SystemTime, Option<i16>)>,
    capacity: usize,
}

/// Ring capacity: one sample per observation cycle covers roughly the last ten minutes.
pub(crate) const TEMPERATURE_HISTORY_CAPACITY: usize = 60;

/// The cadence behind the ring: the panel records at most one sample per evidence cycle, which the
/// monitoring DAG publishes every [`crate::ui::TEMPERATURE_SAMPLE_PERIOD`]-worth of time.  The
/// displayed window is derived from `capacity × period` instead of being hardcoded.
pub(crate) const TEMPERATURE_SAMPLE_PERIOD: Duration = Duration::from_secs(10);

/// Chart height of the temperature trend — compact, because it lives inside the overview section
/// rather than in the rate dashboard.
pub(crate) const TEMPERATURE_CHART_HEIGHT: f32 = 64.0;

impl TemperatureHistory {
    #[must_use]
    pub fn new() -> Self {
        Self {
            samples: std::collections::VecDeque::with_capacity(TEMPERATURE_HISTORY_CAPACITY),
            capacity: TEMPERATURE_HISTORY_CAPACITY,
        }
    }

    pub fn push(&mut self, sample: (SystemTime, Option<i16>)) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn iter(&self) -> impl Iterator<Item = &(SystemTime, Option<i16>)> {
        self.samples.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    #[must_use]
    pub fn last(&self) -> Option<&(SystemTime, Option<i16>)> {
        self.samples.back()
    }

    /// The most recent reading *before* the newest sample, i.e. what 「较上次」 compares against.
    /// A sample that could not be read is skipped rather than read as a change, and a ring with
    /// only one sample has nothing to compare with.
    #[must_use]
    pub fn previous_reading(&self) -> Option<i16> {
        let mut samples = self.samples.iter().rev();
        samples.next()?;
        samples.find_map(|(_, value)| *value)
    }
}

/// How the newest module temperature compares with the previous reading.  `None` means there is no
/// earlier reading to compare with, so the view shows no delta at all instead of a fabricated one.
#[must_use]
pub fn temperature_delta(current: i16, previous: Option<i16>) -> Option<TemperatureDelta> {
    let previous = previous?;
    Some(match current - previous {
        0 => TemperatureDelta::Flat,
        delta if delta > 0 => TemperatureDelta::Up(delta),
        delta => TemperatureDelta::Down(-delta),
    })
}

/// A temperature change since the previous reading, in whole degrees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TemperatureDelta {
    Up(i16),
    Down(i16),
    Flat,
}

/// Closed-set throughput formatting: 1024-based units, one decimal below 10, none at or above.
#[must_use]
pub fn format_rate(bytes_per_sec: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes_per_sec as f64;
    let (scaled, unit) = if value >= GB {
        (value / GB, "GB/s")
    } else if value >= MB {
        (value / MB, "MB/s")
    } else if value >= KB {
        (value / KB, "KB/s")
    } else {
        (value, "B/s")
    };
    if unit == "B/s" || scaled >= 100.0 || (scaled * 10.0).round() % 10.0 == 0.0 {
        format!("{} {unit}", scaled.round() as u64)
    } else {
        format!("{scaled:.1} {unit}")
    }
}

/// Speed grading (evaluated against the current download sample): None=待测速, 0=空闲,
/// <512 KB/s 基础, <2 MB/s 良好, <10 MB/s 优秀, ≥10 MB/s 极速. Grading is display vocabulary
/// only — it never feeds availability classification. One closed set shared by the overview
/// signal row and the header dashboard chip; the tone drives the chip colour.
#[must_use]
pub fn speed_grade(down_bytes_per_sec: Option<u64>) -> (TextKey, StatusTone) {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    match down_bytes_per_sec {
        None => (TextKey::RateGradePending, StatusTone::Neutral),
        Some(0) => (TextKey::RateGradeIdle, StatusTone::Neutral),
        Some(value) if value < 512 * KB => (TextKey::RateGradeBasic, StatusTone::Caution),
        Some(value) if value < 2 * MB => (TextKey::RateGradeGood, StatusTone::Progress),
        Some(value) if value < 10 * MB => (TextKey::RateGradeExcellent, StatusTone::Positive),
        Some(_) => (TextKey::RateGradeVeryFast, StatusTone::Positive),
    }
}

/// Map a raw AT operator name onto its Chinese carrier display form
/// (`CHN-UNICOM` → `CHN-UNICOM（中国联通）`).  The match is a closed substring set over the
/// big-four carriers; anything unknown or empty passes through untouched — nothing is guessed.
#[must_use]
pub fn carrier_display_name(raw: &str, language: crate::localization::Language) -> String {
    let upper = raw.to_ascii_uppercase();
    let chinese = if upper.contains("UNICOM") || upper.contains("CUCC") {
        t(language, crate::localization::TextKey::CarrierChinaUnicom)
    } else if upper.contains("MOBILE") || upper.contains("CMCC") {
        t(language, crate::localization::TextKey::CarrierChinaMobile)
    } else if upper.contains("TELECOM") || upper.contains("CTCC") {
        t(language, crate::localization::TextKey::CarrierChinaTelecom)
    } else if upper.contains("CBN") || upper.contains("BROADCAST") {
        t(language, crate::localization::TextKey::CarrierChinaBroadnet)
    } else {
        return raw.to_owned();
    };
    format!("{raw}（{chinese}）")
}

#[must_use]
pub fn status_tone_for_availability(snapshot: &AppSnapshot) -> StatusTone {
    // The tone never depends on the diagnostic rows, so an empty set is honest here.
    availability_vm(
        snapshot,
        &DiagnosticSet::new(DeviceEpoch(1)),
        snapshot.observed_at,
        Language::ZhCn,
    )
    .tone
}

/// Series colours for the rate chart and the hero numbers, from the HTML reference
/// (`--download #0f6cbd`, `--upload #b26700`).
pub(crate) const DOWN_COLOR: Color32 = scale::DOWNLOAD;
pub(crate) const UP_COLOR: Color32 = scale::UPLOAD;

/// Chart height of the rate section. Tall enough to read a one-minute trend properly now that the
/// legend sits under the plot instead of above it.
pub(crate) const RATE_CHART_HEIGHT: f32 = 150.0;
/// X-axis span in seconds: the ring covers `capacity × cadence` ending at 现在.
const RATE_CHART_X_SPAN_SECS: f32 = RATE_SAMPLE_PERIOD.as_secs_f32() * RATE_HISTORY_CAPACITY as f32;

/// Axis ceiling follows the visible window peak on every frame, with 8% headroom.
/// Grid labels never determine the ceiling: rounded bands must not leave excess empty space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RateAxis {
    pub(crate) max_bytes: f32,
    pub(crate) step_count: u32,
    pub(crate) unit: &'static str,
}

impl RateAxis {
    #[must_use]
    pub(crate) fn step_bytes(self) -> f32 {
        self.max_bytes / self.step_count as f32
    }

    #[must_use]
    pub(crate) fn tick_label(self, index: u32) -> String {
        let divisor = match self.unit {
            "MB/s" => 1024.0 * 1024.0,
            "KB/s" => 1024.0,
            _ => 1.0,
        };
        let value = f64::from(self.step_bytes() * index as f32) / divisor;
        let decimals = if f64::from(self.step_bytes()) / divisor < 1.0 {
            2
        } else {
            1
        };
        let label = format!("{value:.decimals$}");
        label.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

#[must_use]
pub(crate) fn rate_axis(peak_bytes: Option<u64>) -> RateAxis {
    let peak = peak_bytes.unwrap_or(0);
    let unit = if peak >= 1024 * 1024 {
        "MB/s"
    } else if peak >= 1024 {
        "KB/s"
    } else {
        "B/s"
    };
    RateAxis {
        max_bytes: if peak == 0 {
            1.0
        } else {
            (peak as f64 * 1.08) as f32
        },
        step_count: 4,
        unit,
    }
}

/// Highest down or up sample in the ring; `None` while the ring is empty or holds only gaps.
#[cfg(test)]
fn rate_history_peak(history: &RateHistory) -> Option<u64> {
    history
        .iter()
        .flat_map(|(_, down, up)| down.iter().chain(up.iter()).copied())
        .max()
}

/// Split a byte rate into (scaled value, unit) on the 1024 ladder.
fn rate_parts(bytes_per_sec: u64) -> (f64, &'static str) {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes_per_sec as f64;
    if value >= GB {
        (value / GB, "GB/s")
    } else if value >= MB {
        (value / MB, "MB/s")
    } else if value >= KB {
        (value / KB, "KB/s")
    } else {
        (value, "B/s")
    }
}

/// HTML-style one-decimal rate text (`46.5 KB/s`), 1024-based — the hero number format of the
/// reference (`toFixed(1)`).
#[must_use]
pub fn format_rate_1dp(bytes_per_sec: u64) -> String {
    let (value, unit) = rate_parts(bytes_per_sec);
    format!("{value:.1} {unit}")
}

/// HTML-style peak text (`146 KB/s`): one decimal with a trailing `.0` trimmed, like the
/// reference's `shortNumber`.
#[must_use]
pub fn format_rate_peak(bytes_per_sec: u64) -> String {
    let (value, unit) = rate_parts(bytes_per_sec);
    let text = format!("{value:.1}");
    format!("{} {unit}", text.strip_suffix(".0").unwrap_or(&text))
}

/// The overview page's live rate section, matching the HTML reference: hero numbers (34px, one
/// decimal) under the captioned series colours, a 218px white chart whose y-axis follows the
/// visible-window peak with 8% headroom over a −60s…现在 x-scale, and a peak caption below.
/// The values come from the same snapshot and ring the rest of the panel uses, so every surface
/// agrees.
pub fn render_rate_section(
    ui: &mut Ui,
    history: &RateHistory,
    down: Option<u64>,
    up: Option<u64>,
    language: Language,
) -> egui::Response {
    let width = ui.available_width();
    ui.set_min_width(width);
    ui.set_max_width(width);
    let (down_peak, up_peak) = rate_chart::peaks(history);
    let window = format_age(RATE_SAMPLE_PERIOD * RATE_HISTORY_CAPACITY as u32, language).text;
    let window_text = format_text_in(language, TextKey::RateWindow, &TextArgs::age(window)).text;
    egui::Frame::none()
        .show(ui, |ui| {
            // The frame's child ui does not inherit the caller's width caps, so re-apply them
            // here; otherwise a wrapped row would measure against the unbounded parent width.
            ui.set_min_width(width);
            ui.set_max_width(width);
            // Heading row: the live-rate caption with the window length at the right, like the reference.
            ui.horizontal(|ui| {
                ui.label(section_heading(t(
                    language,
                    crate::localization::TextKey::RateHeading,
                )));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    wrapped_label(
                        ui,
                        RichText::new(window_text.clone())
                            .size(scale::RATE_AUX)
                            .color(scale::secondary()),
                    );
                });
            });
            ui.add_space(6.0);
            // The plot comes first and takes the height the legend row used to occupy. The live
            // readings then sit under it, next to the window peaks, so the numbers read as the
            // chart's footer rather than as a separate block on top of it.
            rate_chart::paint(ui, history, language);
            ui.add_space(8.0);
            // Hero numbers: caption row (↓下载 / ↑上传) over a reading and a unit. Wrapped so the
            // two readings fold onto a second line in the narrowest column.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                rate_hero_block(
                    ui,
                    TextKey::RateCaptionDown,
                    "↓",
                    down,
                    DOWN_COLOR,
                    language,
                );
                rate_hero_block(ui, TextKey::RateCaptionUp, "↑", up, UP_COLOR, language);
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 24.0;
                for (caption, value) in [
                    (
                        t(language, crate::localization::TextKey::RatePeakDownload),
                        down_peak,
                    ),
                    (
                        t(language, crate::localization::TextKey::RatePeakUpload),
                        up_peak,
                    ),
                ] {
                    let value = value.map(format_rate_peak).unwrap_or_else(|| {
                        t(language, crate::localization::TextKey::ValueNotAvailable)
                    });
                    ui.label(detail_text(format!("{caption}  {value}")));
                }
            });
        })
        .response
}

/// One hero reading: a 13px coloured caption row (↓ 下载) over a 34px value and a 13px unit.
/// `None` renders the honest 未获取 at the same size so the row never reflows when sampling
/// resumes.
fn rate_hero_block(
    ui: &mut Ui,
    caption: TextKey,
    marker: &str,
    rate: Option<u64>,
    color: Color32,
    language: Language,
) {
    // One exact-size text run (caption line over the hero number line), measured from the
    // galley so a wrapped parent row sees the block's true width and wraps it correctly —
    // the same pattern the previous dashboard used for its compact stat row.
    let mut job = egui::text::LayoutJob::default();
    let caption = LocalizedText::new(language, caption).text;
    job.append(
        &format!("{marker} {caption}"),
        0.0,
        egui::text::TextFormat::simple(egui::FontId::proportional(scale::RATE_AUX), color),
    );
    match rate {
        Some(value) => {
            let (number, unit) = rate_parts(value);
            job.append(
                "\n",
                0.0,
                egui::text::TextFormat::simple(
                    egui::FontId::proportional(scale::RATE_AUX),
                    scale::secondary(),
                ),
            );
            job.append(
                &format!("{number:.1}"),
                0.0,
                egui::text::TextFormat::simple(
                    egui::FontId::proportional(scale::RATE_NUMBER),
                    scale::ink(),
                ),
            );
            job.append(
                &format!(" {unit}"),
                0.0,
                egui::text::TextFormat::simple(
                    egui::FontId::proportional(scale::RATE_AUX),
                    scale::secondary(),
                ),
            );
        }
        None => {
            job.append(
                "\n",
                0.0,
                egui::text::TextFormat::simple(
                    egui::FontId::proportional(scale::RATE_AUX),
                    scale::secondary(),
                ),
            );
            job.append(
                LocalizedText::new(language, TextKey::ValueNotAvailable).as_str(),
                0.0,
                egui::text::TextFormat::simple(
                    egui::FontId::proportional(scale::BODY),
                    scale::faint(),
                ),
            );
        }
    }
    job.wrap.max_width = ui.available_width();
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter().galley(rect.min, galley, scale::ink());
}

/// Hand-rolled compact trend for the measured module temperature: a raised plot, the window's own
/// min/max as the y-range (padded by one degree so a flat run draws as a line instead of hugging an
/// edge), three labelled gridlines in degrees, one violet series, no fill, no animation.  Samples
/// are placed by their real timestamps, so a cycle that reported nothing leaves an honest gap
/// rather than a fabricated point, and the newest reading keeps a dot so 现在 is unambiguous.
pub(crate) fn paint_temperature_chart(
    ui: &mut Ui,
    history: &TemperatureHistory,
    language: Language,
) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(
        egui::Vec2::new(width, TEMPERATURE_CHART_HEIGHT),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, scale::surface_raised());
    let plot = egui::Rect::from_min_max(
        egui::Pos2::new(rect.left() + 42.0, rect.top() + 8.0),
        egui::Pos2::new(rect.right() - 8.0, rect.bottom() - 8.0),
    );

    let samples: Vec<(SystemTime, Option<i16>)> = history.iter().copied().collect();
    let readings: Vec<i16> = samples.iter().filter_map(|(_, value)| *value).collect();
    if readings.len() < 2 {
        painter.text(
            plot.center(),
            egui::Align2::CENTER_CENTER,
            LocalizedText::new(language, TextKey::TemperatureTrendSampling).text,
            egui::FontId::proportional(scale::RATE_AUX),
            scale::secondary(),
        );
        return;
    }
    let low = f32::from(readings.iter().copied().min().expect("non-empty")) - 1.0;
    let high = f32::from(readings.iter().copied().max().expect("non-empty")) + 1.0;
    let span = (high - low).max(1.0);
    let y_of = |value: i16| plot.bottom() - ((f32::from(value) - low) / span) * plot.height();
    for (fraction, label) in [(0.0_f32, low), (0.5, (low + high) / 2.0), (1.0, high)] {
        let y = plot.bottom() - fraction * plot.height();
        painter.line_segment(
            [
                egui::Pos2::new(plot.left(), y),
                egui::Pos2::new(plot.right(), y),
            ],
            Stroke::new(1.0_f32, scale::line()),
        );
        painter.text(
            egui::Pos2::new(plot.left() - 6.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{label:.0}"),
            egui::FontId::proportional(scale::META),
            scale::faint(),
        );
    }
    painter.text(
        egui::Pos2::new(plot.right(), plot.top()),
        egui::Align2::RIGHT_TOP,
        "°C",
        egui::FontId::proportional(scale::META),
        scale::faint(),
    );

    let newest = samples
        .last()
        .map(|(at, _)| *at)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let window_secs = TEMPERATURE_SAMPLE_PERIOD.as_secs_f32() * TEMPERATURE_HISTORY_CAPACITY as f32;
    let x_of = |at: SystemTime| {
        let age = newest
            .duration_since(at)
            .map(|age| age.as_secs_f32())
            .unwrap_or(0.0);
        plot.right() - (age / window_secs).min(1.0) * plot.width()
    };

    let series_painter = painter.with_clip_rect(plot);
    let mut runs: Vec<Vec<egui::Pos2>> = Vec::new();
    let mut points: Vec<egui::Pos2> = Vec::new();
    for (at, value) in &samples {
        match value {
            Some(value) => points.push(egui::Pos2::new(x_of(*at), y_of(*value))),
            None if !points.is_empty() => runs.push(std::mem::take(&mut points)),
            None => {}
        }
    }
    if !points.is_empty() {
        runs.push(points);
    }
    for run in &runs {
        match run.as_slice() {
            [point] => {
                series_painter.circle_filled(*point, 2.0, scale::secondary());
            }
            [_, _, ..] => {
                series_painter.add(Shape::line(
                    run.clone(),
                    Stroke::new(2.0_f32, scale::secondary()),
                ));
            }
            _ => {}
        }
    }
    if let Some((at, Some(value))) = samples.last() {
        series_painter.circle_filled(egui::Pos2::new(x_of(*at), y_of(*value)), 3.0, scale::ink());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_chinese_text_uses_multiple_rows_at_panel_width() {
        let context = egui::Context::default();
        let mut height = None;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(240.0, 160.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    height = Some(
                        wrapped_label(
                            ui,
                            "模块数据通路可达，但通过该接口的 DNS 解析失败，请刷新后重试。",
                        )
                        .rect
                        .height(),
                    );
                });
            },
        );
        assert!(height.is_some_and(|value| value > 20.0));
    }

    #[test]
    fn speed_grades_follow_one_closed_threshold_ladder() {
        assert_eq!(speed_grade(None).0, TextKey::RateGradePending);
        assert_eq!(speed_grade(Some(0)).0, TextKey::RateGradeIdle);
        assert_eq!(speed_grade(Some(512 * 1024 - 1)).0, TextKey::RateGradeBasic);
        assert_eq!(speed_grade(Some(512 * 1024)).0, TextKey::RateGradeGood);
        assert_eq!(
            speed_grade(Some(2 * 1024 * 1024 - 1)).0,
            TextKey::RateGradeGood
        );
        assert_eq!(
            speed_grade(Some(2 * 1024 * 1024)).0,
            TextKey::RateGradeExcellent
        );
        assert_eq!(
            speed_grade(Some(10 * 1024 * 1024 - 1)).0,
            TextKey::RateGradeExcellent
        );
        assert_eq!(
            speed_grade(Some(10 * 1024 * 1024)).0,
            TextKey::RateGradeVeryFast
        );
    }

    #[test]
    fn the_window_label_is_derived_from_the_ring() {
        let window = RATE_SAMPLE_PERIOD * RATE_HISTORY_CAPACITY as u32;
        let text = format_text_in(
            Language::ZhCn,
            TextKey::RateWindow,
            &TextArgs::age(format_age(window, Language::ZhCn).text),
        )
        .text;
        assert_eq!(text, "最近 1 分钟");
    }

    #[test]
    fn the_rate_section_fits_the_right_column_at_minimum_window_width() {
        // The narrowest overview column (280px at the 800px window floor) must still render the
        // rate section: hero numbers, the 218px chart, and the peak caption without overflow.
        let context = egui::Context::default();
        let mut history = RateHistory::new();
        history.push((SystemTime::now(), Some(69_734), Some(13_517)));
        history.push((SystemTime::now(), Some(1_047_552), Some(1_047_552)));
        let mut rect = None;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(280.0, 700.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    rect = Some(
                        render_rate_section(
                            ui,
                            &history,
                            Some(1_047_552),
                            Some(1_047_552),
                            Language::ZhCn,
                        )
                        .rect,
                    );
                });
            },
        );
        let rect = rect.expect("rate section rendered");
        assert!(
            rect.width() <= 280.0 + 0.5,
            "rate section overflowed its column: {rect:?}"
        );
        assert!(
            (180.0..460.0).contains(&rect.height()),
            "unexpected rate section height: {rect:?}"
        );
    }

    #[test]
    fn html_rate_number_formats_match_the_reference() {
        // Hero numbers use one decimal (`toFixed(1)`); the peak trims a trailing `.0`.
        assert_eq!(format_rate_1dp(47_616), "46.5 KB/s");
        assert_eq!(format_rate_1dp(4_608), "4.5 KB/s");
        assert_eq!(format_rate_1dp(149_504), "146.0 KB/s");
        assert_eq!(format_rate_peak(149_504), "146 KB/s");
        assert_eq!(format_rate_peak(47_616), "46.5 KB/s");
        assert_eq!(format_rate_1dp(0), "0.0 B/s");
    }

    #[test]
    fn rate_axis_preserves_small_headroom_and_follows_the_visible_window() {
        for peak in [
            1_u64,
            50,
            307,
            1023,
            1024,
            5600,
            19000,
            88_269,
            121_600,
            208_077,
            400 * 1024,
            1024 * 1024,
            1024 * 1024 + 1,
            5 * 1024 * 1024,
            u64::MAX,
        ] {
            let axis = rate_axis(Some(peak));
            let ratio = axis.max_bytes as f64 / peak as f64;
            assert!(
                (1.07999..=1.08001).contains(&ratio),
                "peak {peak}: {axis:?}"
            );
            assert!(axis.max_bytes.is_finite());
            assert!((3..=5).contains(&axis.step_count));
            let divisor = match axis.unit {
                "MB/s" => 1024.0 * 1024.0,
                "KB/s" => 1024.0,
                _ => 1.0,
            };
            let mut previous = -1.0_f64;
            for index in 0..=axis.step_count {
                let label = axis.tick_label(index).parse::<f64>().unwrap();
                let actual = (axis.step_bytes() * index as f32) as f64 / divisor;
                assert!(label > previous);
                assert!((label - actual).abs() <= 0.051_f64.max(actual.abs() * 1e-6));
                previous = label;
            }
        }
        for peak in [None, Some(0)] {
            assert_eq!(rate_axis(peak).max_bytes, 1.0);
        }
        let mut history = RateHistory::new();
        let now = SystemTime::UNIX_EPOCH;
        history.push((now, Some(100 * 1024), Some(203 * 1024)));
        let high = rate_axis(rate_history_peak(&history));
        for tick in 1..=RATE_HISTORY_CAPACITY {
            history.push((
                now + Duration::from_secs(tick as u64),
                Some(6 * 1024),
                Some(121 * 1024),
            ));
        }
        let low = rate_axis(rate_history_peak(&history));
        assert_eq!(rate_history_peak(&history), Some(121 * 1024));
        assert!(low.max_bytes < high.max_bytes);
        assert!((low.max_bytes / 1024.0 - 130.68).abs() < 0.001);
        history.push((now + Duration::from_secs(61), Some(300 * 1024), None));
        assert_eq!(rate_history_peak(&history), Some(300 * 1024));
        assert!(rate_axis(rate_history_peak(&history)).max_bytes > high.max_bytes);
    }

    #[test]
    fn every_control_keeps_one_height_and_one_quiet_type_scale() {
        let context = egui::Context::default();
        apply_style(&context);
        let style = context.style();
        // One control height everywhere is what stops a page growing oversized buttons.
        assert_eq!(style.spacing.interact_size.y, scale::CONTROL_H);
        assert_eq!(style.text_styles[&egui::TextStyle::Body].size, scale::BODY);
        assert_eq!(style.text_styles[&egui::TextStyle::Small].size, scale::META);
        assert_eq!(
            style.text_styles[&egui::TextStyle::Button].size,
            scale::BODY
        );
        let fill = style.visuals.selection.bg_fill;
        assert_eq!(fill, scale::selected());
        assert_eq!(style.visuals.hyperlink_color, scale::accent());
        assert_ne!(fill, scale::accent());
    }

    #[test]
    fn clock_hms_is_the_time_of_day_and_refuses_pre_epoch_instants() {
        assert_eq!(
            clock_hms(SystemTime::UNIX_EPOCH).as_deref(),
            Some("00:00:00")
        );
        assert_eq!(
            clock_hms(SystemTime::UNIX_EPOCH + Duration::from_secs(3_661)).as_deref(),
            Some("01:01:01")
        );
        assert_eq!(
            clock_hms(SystemTime::UNIX_EPOCH + Duration::from_secs(86_400 + 45_296)).as_deref(),
            Some("12:34:56")
        );
        assert_eq!(
            clock_hms(SystemTime::UNIX_EPOCH - Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn link_speeds_render_as_mbps_with_one_decimal() {
        assert_eq!(format_mbps(0), "0.0 Mbps");
        assert_eq!(format_mbps(100_000_000), "100.0 Mbps");
        assert_eq!(format_mbps(54_000_000), "54.0 Mbps");
        assert_eq!(format_mbps(1_000_000_000), "1000.0 Mbps");
        assert_eq!(format_mbps(1_500_000), "1.5 Mbps");
    }

    #[test]
    fn temperature_history_keeps_one_point_per_cycle_and_an_honest_gap() {
        let mut history = TemperatureHistory::new();
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        assert!(history.is_empty());
        assert!(history.previous_reading().is_none());
        history.push((start, Some(57)));
        assert_eq!(history.len(), 1);
        assert!(
            history.previous_reading().is_none(),
            "a single reading has nothing to compare with"
        );
        history.push((start + Duration::from_secs(10), Some(57)));
        assert_eq!(history.previous_reading(), Some(57));
        // A cycle that reported nothing stays a gap and is skipped when comparing readings.
        history.push((start + Duration::from_secs(20), None));
        assert_eq!(history.previous_reading(), Some(57));
        assert_eq!(
            history.last(),
            Some(&(start + Duration::from_secs(20), None))
        );
        // The ring is bounded and drops the oldest samples first.
        for step in 0..TEMPERATURE_HISTORY_CAPACITY + 5 {
            history.push((start + Duration::from_secs(30 + step as u64 * 10), Some(60)));
        }
        assert_eq!(history.len(), TEMPERATURE_HISTORY_CAPACITY);
    }

    #[test]
    fn temperature_delta_reports_only_real_changes() {
        assert_eq!(temperature_delta(57, None), None);
        assert_eq!(
            temperature_delta(57, Some(57)),
            Some(TemperatureDelta::Flat)
        );
        assert_eq!(
            temperature_delta(58, Some(57)),
            Some(TemperatureDelta::Up(1))
        );
        assert_eq!(
            temperature_delta(51, Some(57)),
            Some(TemperatureDelta::Down(6))
        );
    }

    #[test]
    fn the_temperature_chart_paints_every_window_shape() {
        // An empty ring, a single reading, a window with a gap and a full ring must all paint
        // without panicking; the first two states show the sampling caption instead of a curve.
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let shapes: [Vec<(SystemTime, Option<i16>)>; 4] = [
            Vec::new(),
            vec![(start, Some(57))],
            vec![
                (start, Some(57)),
                (start + Duration::from_secs(10), None),
                (start + Duration::from_secs(20), Some(58)),
            ],
            (0..TEMPERATURE_HISTORY_CAPACITY)
                .map(|step| {
                    (
                        start + Duration::from_secs(step as u64 * 10),
                        Some(50 + (step % 7) as i16),
                    )
                })
                .collect(),
        ];
        for samples in shapes {
            let mut history = TemperatureHistory::new();
            for sample in samples {
                history.push(sample);
            }
            let context = egui::Context::default();
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(420.0, 200.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        paint_temperature_chart(ui, &history, Language::ZhCn);
                    });
                },
            );
        }
    }
}

pub mod update;
