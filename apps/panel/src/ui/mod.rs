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
    Language, LocalizedText, TextArgs, TextKey, action_tag_key, availability_reason,
    availability_title, diagnostic_state, error_text, failure_text, format_text_in, freshness_key,
    hotspot_title, hotspot_unsupported_reason, rollback_outcome, unexecuted_reason,
};

pub mod device_tools;
pub mod diagnostics;
pub(crate) mod driver_setup;
pub mod overview;
pub mod repairs;
pub mod settings;
pub mod sms;
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
    #[must_use]
    pub const fn color(self) -> Color32 {
        match self {
            Self::Positive => Color32::from_rgb(24, 116, 74),
            Self::Caution => scale::WARNING,
            Self::Negative => Color32::from_rgb(170, 48, 48),
            Self::Progress => scale::DOWNLOAD,
            Self::Neutral => scale::SECONDARY,
        }
    }

    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Positive => "●",
            Self::Caution => "▲",
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
pub fn format_age(age: Duration, _language: Language) -> LocalizedText {
    let seconds = age.as_secs();
    let text = if seconds < 60 {
        format!("{seconds} 秒")
    } else if seconds < 3600 {
        format!("{} 分钟", seconds / 60)
    } else {
        format!("{} 小时", seconds / 3600)
    };
    LocalizedText {
        key: TextKey::ObservedAgo,
        text,
    }
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
    LocalizedText::new(language, action_tag_key(prepared.action))
}

pub(crate) mod icons;
pub(crate) mod shell;
pub(crate) mod theme;
pub(crate) use theme::{scale, style_root};

pub(crate) fn section_frame(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    // Uniform gap above every box so the sections of a page share one vertical rhythm.
    ui.add_space(scale::SECTION_GAP);
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::symmetric(
            scale::SECTION_MARGIN[0],
            scale::SECTION_MARGIN[1],
        ))
        // Borderless cards: set the stroke on the frame itself so no global visuals state can
        // reintroduce an outline (and with it the line that collided with the scrollbar).
        .fill(Color32::WHITE)
        .rounding(12.0)
        .stroke(Stroke::NONE)
        .show(ui, |ui| {
            // Stretch every section to the panel width so the grouped boxes align as even
            // columns instead of hugging their content and leaving ragged right edges.
            ui.set_min_width(ui.available_width());
            add_contents(ui);
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

/// Group heading inside a page (设备 / 蜂窝网络 / 低风险与网络恢复 / …).
pub(crate) fn section_heading(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).size(scale::SECTION).strong()
}

/// Label of a label/value row. Muted rather than bold so the value carries the emphasis and a
/// column of values reads as one continuous vertical run.
pub(crate) fn field_label(text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .size(scale::LABEL)
        .color(scale::MUTED)
}

/// Hints, timestamps, and descriptions: the quietest tier of the scale.
pub(crate) fn meta_text(text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .size(scale::META)
        .color(scale::FAINT)
}

/// Supporting evidence (the diagnostics 「详情」 rows and the explanation under a status badge):
/// same quiet size as meta copy, but in the distinct `DETAIL` slate so it reads separately from
/// hints and timestamps.
pub(crate) fn detail_text(text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .size(scale::META)
        .color(scale::DETAIL)
}

/// Presentation-side ring of the measured throughput samples backing the overview chart.
/// Snapshots are immutable, so the rolling window lives here and only the newest sample ever
/// enters; `None` samples render as an honest gap.
#[derive(Debug, Default)]
pub struct RateHistory {
    samples: std::collections::VecDeque<(SystemTime, Option<u64>, Option<u64>)>,
    capacity: usize,
}

/// Ring capacity: 60 samples at the 1 s cadence cover the last minute of throughput.
pub(crate) const RATE_HISTORY_CAPACITY: usize = 60;

/// Sampling cadence behind the ring: the panel records one sample per second from the latest
/// published snapshot for the chart, independent of the slower evidence refresh. The dashboard
/// derives its 「最近 N」 window label from `capacity × period` instead of hardcoding a duration.
pub(crate) const RATE_SAMPLE_PERIOD: Duration = Duration::from_secs(1);

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
pub fn carrier_display_name(raw: &str) -> String {
    let upper = raw.to_ascii_uppercase();
    let chinese = if upper.contains("UNICOM") || upper.contains("CUCC") {
        "中国联通"
    } else if upper.contains("MOBILE") || upper.contains("CMCC") {
        "中国移动"
    } else if upper.contains("TELECOM") || upper.contains("CTCC") {
        "中国电信"
    } else if upper.contains("CBN") || upper.contains("BROADCAST") {
        "中国广电"
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

/// Chart height of the rate section (the reference's 218px `chart-container`).
pub(crate) const RATE_CHART_HEIGHT: f32 = 218.0;
/// X-axis span in seconds: the ring covers `capacity × cadence` ending at 现在.
const RATE_CHART_X_SPAN_SECS: f32 = RATE_SAMPLE_PERIOD.as_secs_f32() * RATE_HISTORY_CAPACITY as f32;

/// Adaptive y-axis of the rate chart: a ceiling, an interval count, and the unit shared by the
/// tick labels and the axis name.
///
/// The bands keep the reference's compact 0–160 KB/s scale for quiet windows and grow through
/// 400 KB/s and 1 MB/s to a nice MB/s ceiling, so a fast 4G window (MB/s) is drawn to scale
/// instead of being clipped by the old fixed 160 KB/s limit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RateAxis {
    pub(crate) max_bytes: f32,
    pub(crate) step_count: u32,
    pub(crate) unit: &'static str,
}

impl RateAxis {
    /// Bytes per grid interval (`max_bytes / step_count`), the distance between two gridlines.
    #[must_use]
    pub(crate) fn step_bytes(self) -> f32 {
        self.max_bytes / self.step_count as f32
    }

    /// Label of gridline `index` (0 is the baseline) in the axis unit: whole KB/s numbers for the
    /// compact bands, one decimal for MB/s, matching the reference's axis-label style.
    #[must_use]
    pub(crate) fn tick_label(self, index: u32) -> String {
        let value = self.step_bytes() * index as f32;
        if self.unit == "MB/s" {
            format!("{:.1}", f64::from(value) / (1024.0 * 1024.0))
        } else {
            format!("{}", (value / 1024.0).round() as u32)
        }
    }
}

/// Nice MB/s grid steps (1/2/2.5/5 × 10ⁿ): decimal steps whose one-decimal labels are exact.
const MB_RATE_STEPS: [f64; 26] = [
    0.5,
    1.0,
    2.0,
    2.5,
    5.0,
    10.0,
    20.0,
    25.0,
    50.0,
    100.0,
    200.0,
    250.0,
    500.0,
    1_000.0,
    2_000.0,
    2_500.0,
    5_000.0,
    10_000.0,
    20_000.0,
    25_000.0,
    50_000.0,
    100_000.0,
    200_000.0,
    250_000.0,
    500_000.0,
    1_000_000.0,
];

/// Smallest nice step that divides `peak_mb` into at most five intervals.
fn nice_rate_step_mb(peak_mb: f64) -> f64 {
    let target = peak_mb / 5.0;
    MB_RATE_STEPS
        .iter()
        .copied()
        .find(|step| *step >= target)
        .unwrap_or(MB_RATE_STEPS[MB_RATE_STEPS.len() - 1])
}

/// Axis for a window peak. `None` (no samples yet) and anything up to 160 KB/s keep the
/// reference's default 0–160 KB/s axis; the ceiling then steps through 400 KB/s and 1 MB/s and
/// switches to a nice MB/s ceiling, so 4G rates of many MB/s are drawn to scale, never clipped.
#[must_use]
pub(crate) fn rate_axis(peak_bytes: Option<u64>) -> RateAxis {
    const KB: f32 = 1024.0;
    const MB: f32 = 1024.0 * KB;
    let peak = peak_bytes.unwrap_or(0) as f32;
    if peak <= 160.0 * KB {
        RateAxis {
            max_bytes: 160.0 * KB,
            step_count: 4,
            unit: "KB/s",
        }
    } else if peak <= 400.0 * KB {
        RateAxis {
            max_bytes: 400.0 * KB,
            step_count: 4,
            unit: "KB/s",
        }
    } else if peak <= MB {
        // 1 MB / 4 = 256 KB, the binary form of the reference's 250 KB/s step.
        RateAxis {
            max_bytes: MB,
            step_count: 4,
            unit: "KB/s",
        }
    } else {
        let peak_mb = f64::from(peak) / f64::from(MB);
        let step_mb = nice_rate_step_mb(peak_mb);
        let step_count = (peak_mb / step_mb).ceil() as u32;
        RateAxis {
            max_bytes: (step_mb * f64::from(step_count)) as f32 * MB,
            step_count,
            unit: "MB/s",
        }
    }
}

/// Highest down or up sample in the ring; `None` while the ring is empty or holds only gaps.
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
/// window peak (the reference's 0–160 KB/s default, growing through 400 KB/s and 1 MB/s to
/// MB/s ceilings for fast links) over a −60s…现在 x-scale, and a peak caption under the plot.
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
    let peak = rate_history_peak(history);
    let window = format_age(RATE_SAMPLE_PERIOD * RATE_HISTORY_CAPACITY as u32, language).text;
    let window_text = format_text_in(language, TextKey::RateWindow, &TextArgs::age(window)).text;
    egui::Frame::none()
        .show(ui, |ui| {
            // The frame's child ui does not inherit the caller's width caps, so re-apply them
            // here; otherwise a wrapped row would measure against the unbounded parent width.
            ui.set_min_width(width);
            ui.set_max_width(width);
            // Heading row: 实时速率 with the window length at the right, like the reference.
            ui.horizontal(|ui| {
                ui.label(section_heading("实时速率"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    wrapped_label(
                        ui,
                        RichText::new(window_text.clone())
                            .size(scale::RATE_AUX)
                            .color(scale::SECONDARY),
                    );
                });
            });
            ui.add_space(6.0);
            // Hero numbers: caption row (↓下载 / ↑上传) over a 34px reading and a 13px unit.
            // Wrapped so the two readings fold onto a second line in the narrowest column.
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
            ui.add_space(11.0);
            paint_rate_chart(ui, history, language);
            ui.add_space(8.0);
            // Peak caption: 最近 1 分钟峰值 <strong>N KB/s</strong>.
            if let Some(peak) = peak {
                let peak_text = format_text_in(
                    language,
                    TextKey::RatePeak,
                    &TextArgs::detail(format_rate_peak(peak)),
                )
                .text;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    wrapped_label(
                        ui,
                        RichText::new(window_text)
                            .size(scale::RATE_AUX)
                            .color(scale::SECONDARY),
                    );
                    wrapped_label(ui, RichText::new(peak_text).size(scale::RATE_AUX).strong());
                });
            }
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
                    scale::SECONDARY,
                ),
            );
            job.append(
                &format!("{number:.1}"),
                0.0,
                egui::text::TextFormat::simple(
                    egui::FontId::proportional(scale::RATE_NUMBER),
                    scale::INK,
                ),
            );
            job.append(
                &format!(" {unit}"),
                0.0,
                egui::text::TextFormat::simple(
                    egui::FontId::proportional(scale::RATE_AUX),
                    scale::SECONDARY,
                ),
            );
        }
        None => {
            job.append(
                "\n",
                0.0,
                egui::text::TextFormat::simple(
                    egui::FontId::proportional(scale::RATE_AUX),
                    scale::SECONDARY,
                ),
            );
            job.append(
                LocalizedText::new(language, TextKey::ValueNotAvailable).as_str(),
                0.0,
                egui::text::TextFormat::simple(
                    egui::FontId::proportional(scale::RATE_NUMBER),
                    scale::FAINT,
                ),
            );
        }
    }
    job.wrap.max_width = ui.available_width();
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter().galley(rect.min, galley, scale::INK);
}

/// Hand-rolled dual-series line chart matching the reference's ECharts options: white plot,
/// adaptive y-scale (0–160 KB/s by default, stepping through 400 KB/s and 1 MB/s to nice MB/s
/// ceilings, always covered by 4–5 grid intervals with a label each), a −60s…现在 x-scale with
/// a label every 15 seconds, download as a solid 2px blue line, upload as a dashed 1.8px orange
/// line, no fill, no animation, no symbols. `None` samples stay honest gaps; a partially filled
/// ring right-anchors its samples against 现在.
fn paint_rate_chart(ui: &mut Ui, history: &RateHistory, language: Language) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(
        egui::Vec2::new(width, RATE_CHART_HEIGHT),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::WHITE);

    // The axis follows the window's own peak, so a fast 4G window is never clipped by the old
    // fixed 160 KB/s ceiling while quiet windows keep the reference's compact scale.
    let axis = rate_axis(rate_history_peak(history));

    // Plot insets leave room for the y labels (left), the x labels (bottom) and the unit name.
    let plot = egui::Rect::from_min_max(
        egui::Pos2::new(rect.left() + 42.0, rect.top() + 8.0),
        egui::Pos2::new(rect.right() - 8.0, rect.bottom() - 22.0),
    );
    let y_of = |value: f32| plot.bottom() - (value / axis.max_bytes) * plot.height();

    // Horizontal gridlines and y labels at the axis's own intervals, zero baseline in the darker
    // axis colour and the tick text in whichever unit the axis selected.
    for index in 0..=axis.step_count {
        let y = y_of(axis.step_bytes() * index as f32);
        let baseline = index == 0;
        painter.line_segment(
            [
                egui::Pos2::new(plot.left(), y),
                egui::Pos2::new(plot.right(), y),
            ],
            Stroke::new(1.0_f32, if baseline { scale::AXIS } else { scale::GRID }),
        );
        painter.text(
            egui::Pos2::new(plot.left() - 6.0, y),
            egui::Align2::RIGHT_CENTER,
            axis.tick_label(index),
            egui::FontId::proportional(scale::META),
            scale::AXIS_LABEL,
        );
    }
    // Unit name at the top right of the plot, like the reference's axis name.
    painter.text(
        egui::Pos2::new(plot.right(), plot.top()),
        egui::Align2::RIGHT_TOP,
        axis.unit,
        egui::FontId::proportional(scale::META),
        scale::AXIS_LABEL,
    );
    // X labels every 15 seconds: 「N 秒前」 up to 现在. The oldest edge label is skipped so it
    // never clips at the plot border.
    for sec in [-45, -30, -15, 0] {
        let x = plot.right() - ((-sec) as f32 / RATE_CHART_X_SPAN_SECS) * plot.width();
        let text = if sec == 0 {
            "现在".to_owned()
        } else {
            format!("{} 秒前", -sec)
        };
        painter.text(
            egui::Pos2::new(x, plot.bottom() + 15.0),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(scale::META),
            scale::AXIS_LABEL,
        );
    }

    let samples: Vec<_> = history.iter().collect();
    if samples.len() < 2 {
        painter.text(
            plot.center(),
            egui::Align2::CENTER_CENTER,
            LocalizedText::new(language, TextKey::RateSampling).text,
            egui::FontId::proportional(scale::RATE_AUX),
            scale::SECONDARY,
        );
        return;
    }

    // Right-anchored time axis: the newest sample sits on 现在 at the right edge, history grows
    // leftward at one second per sample, so a partially filled ring reads as live data of the
    // last N seconds instead of a stub hugging the left.
    let last = samples.len() - 1;
    let x_of = |index: usize| {
        let age = (last - index) as f32;
        plot.right() - (age / RATE_CHART_X_SPAN_SECS) * plot.width()
    };
    let series_painter = painter.with_clip_rect(plot);
    for (series_index, color, dashed) in [(1_usize, DOWN_COLOR, false), (2_usize, UP_COLOR, true)] {
        let mut runs: Vec<Vec<egui::Pos2>> = Vec::new();
        let mut points: Vec<egui::Pos2> = Vec::new();
        for (index, (_, down, up)) in samples.iter().enumerate() {
            let value = match series_index {
                1 => *down,
                _ => *up,
            };
            match value {
                Some(value) => points.push(egui::Pos2::new(x_of(index), y_of(value as f32))),
                None if !points.is_empty() => runs.push(std::mem::take(&mut points)),
                None => {}
            }
        }
        if !points.is_empty() {
            runs.push(points);
        }
        for run in runs {
            if run.len() >= 2 {
                if dashed {
                    // egui 0.29 has no dashed stroke; epaint provides the dashed line shape.
                    series_painter.add(Shape::dashed_line(
                        &run,
                        Stroke::new(1.8_f32, color),
                        4.0_f32,
                        3.0_f32,
                    ));
                } else {
                    series_painter.add(Shape::line(run, Stroke::new(2.0_f32, color)));
                }
            }
        }
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
            (260.0..460.0).contains(&rect.height()),
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
    fn rate_axis_chooses_nice_bands_and_units() {
        fn assert_axis(peak: Option<u64>, max_kb: f32, step_count: u32, unit: &str) {
            let axis = rate_axis(peak);
            assert_eq!(axis.max_bytes, max_kb * 1024.0, "peak {peak:?}");
            assert_eq!(axis.step_count, step_count, "peak {peak:?}");
            assert_eq!(axis.unit, unit, "peak {peak:?}");
        }
        // The default and the compact band keep the reference's 0-160 KB/s axis.
        assert_axis(None, 160.0, 4, "KB/s");
        assert_axis(Some(0), 160.0, 4, "KB/s");
        assert_axis(Some(46 * 1024), 160.0, 4, "KB/s");
        assert_axis(Some(160 * 1024), 160.0, 4, "KB/s");
        // Above 160 KB/s the axis grows in the documented KB/s bands...
        assert_axis(Some(200 * 1024), 400.0, 4, "KB/s");
        assert_axis(Some(400 * 1024), 400.0, 4, "KB/s");
        assert_axis(Some(1024 * 1024), 1024.0, 4, "KB/s");
        // ...and switches to the MB/s unit with nice ceilings once a megabyte is passed.
        assert_axis(Some(3 * 1024 * 1024 / 2), 1536.0, 3, "MB/s");
        assert_axis(Some(5 * 1024 * 1024), 5.0 * 1024.0, 5, "MB/s");
        assert_axis(Some(30 * 1024 * 1024), 30.0 * 1024.0, 3, "MB/s");
    }

    #[test]
    fn rate_axis_tick_labels_follow_the_axis_unit() {
        // KB/s axes label whole numbers, so the 0.25 MB binary step reads 256/512/768/1024.
        let kb_axis = rate_axis(Some(1024 * 1024));
        assert_eq!(kb_axis.tick_label(0), "0");
        assert_eq!(kb_axis.tick_label(1), "256");
        assert_eq!(kb_axis.tick_label(2), "512");
        assert_eq!(kb_axis.tick_label(3), "768");
        assert_eq!(kb_axis.tick_label(4), "1024");
        // MB/s axes label one decimal, so 0.5 MB ceilings stay honest instead of rounding away.
        let mb_axis = rate_axis(Some(5 * 1024 * 1024));
        assert_eq!(mb_axis.tick_label(0), "0.0");
        assert_eq!(mb_axis.tick_label(1), "1.0");
        assert_eq!(mb_axis.tick_label(5), "5.0");
        let half_mb_axis = rate_axis(Some(3 * 1024 * 1024 / 2));
        assert_eq!(half_mb_axis.tick_label(1), "0.5");
        assert_eq!(half_mb_axis.tick_label(3), "1.5");
    }

    #[test]
    fn the_rate_axis_never_clips_a_sample_at_or_below_the_peak_it_was_built_from() {
        for peak in [
            1_u64,
            46 * 1024,
            160 * 1024,
            161 * 1024,
            400 * 1024,
            1024 * 1024 + 1,
            5 * 1024 * 1024,
            29 * 1024 * 1024,
            1024 * 1024 * 1024,
        ] {
            let axis = rate_axis(Some(peak));
            assert!(
                axis.max_bytes >= peak as f32,
                "peak {peak} escaped the axis: {axis:?}"
            );
            assert!(
                (3..=5).contains(&axis.step_count),
                "implausible grid density for peak {peak}: {axis:?}"
            );
        }
    }

    #[test]
    fn style_root_keeps_controls_readable_and_accessible() {
        let context = egui::Context::default();
        style_root(&context);
        let style = context.style();
        assert!(style.spacing.interact_size.y >= 36.0);
        assert!(style.text_styles[&egui::TextStyle::Body].size >= 14.0);
        assert!(style.text_styles[&egui::TextStyle::Small].size >= 12.0);
        assert!(!style.visuals.dark_mode);
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
}
