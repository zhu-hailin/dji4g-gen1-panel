//! Device-tool tasks: request/response contracts, outcome classification and evidence.
//!
//! One task is one bounded AT interaction with the module over the same serial actor the
//! monitoring and SMS paths use. This module owns the vocabulary (mode, phase, outcome, context)
//! and the evidence bookkeeping; the actual port call lives behind [`crate::DeviceToolsPort`] and
//! the arbitration lives in [`crate::ControllerRunner`].
//!
//! Privacy: a request line and a response line are user/device data, never log material. The types
//! here keep that text behind explicit accessors, redact it from `Debug`, and never implement
//! `Serialize`, so a general-purpose exporter cannot pick it up by accident.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use dji4g_at_protocol::{
    AtFinalCode, PdpContext, SensorTemperature, ToolReadId, ToolResponse, ValidatedToolLine,
    VerifiedUsbNetProfile, parse_qtemp_lines,
};
use dji4g_domain::{DeviceEpoch, FeatureStatus, StableDeviceIdentity};

/// Cancellation and write-attempt handle shared with the platform layer. Re-exported under the
/// name the tool pipeline uses.
pub use dji4g_domain::ToolTransactionControl as ToolControl;

/// Absolute deadline for a single tool command.
pub const TOOL_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(10);
/// Total budget for one capability sweep; items that cannot start inside it stay unqueried.
pub const PROBE_BATCH_BUDGET: Duration = Duration::from_secs(45);
/// How long a frozen expert request stays confirmable.
pub const EXPERT_PLAN_LIFETIME: Duration = Duration::from_secs(30);
/// Bounded tool history, mirroring the message-list bounds.
pub const MAX_TOOL_HISTORY_ITEMS: usize = 100;
pub const MAX_TOOL_HISTORY_BYTES: usize = 256 * 1024;
/// Largest transcript a single task may hand to the UI.
pub const MAX_TRANSCRIPT_BYTES: usize = 64 * 1024;

/// The single deadline a sub-request may use inside a batch.
#[must_use]
pub fn item_deadline(batch_remaining: Duration) -> Duration {
    batch_remaining.min(TOOL_TRANSACTION_TIMEOUT)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolMode {
    /// Read-only module profile and capability evidence, refreshed on request.
    Preset,
    /// One whitelisted query typed by the user.
    Query,
    /// One arbitrary single-line command, confirmed for this exact line.
    Expert,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolPhase {
    Idle,
    Queued,
    Running,
    Cancelling,
    Finished,
}

impl ToolPhase {
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Cancelling)
    }

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Finished => "finished",
        }
    }
}

/// How a tool task ended.
///
/// The distinction that matters: a module that answered `ERROR` talked to us successfully and
/// merely refused the command, while a transport failure means we never got an answer. Collapsing
/// the two would let the panel claim a firmware is unsupported when the cable was loose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolOutcome {
    /// The module returned a final `OK`.
    Ok,
    /// The module answered, and refused this command.
    Rejected,
    /// The module reported that it does not implement the command.
    Unsupported,
    /// No usable answer: port error, timeout, disconnect.
    TransportFailure,
    /// A final `OK` arrived but the payload could not be parsed.
    FormatMismatch,
    /// Cancelled before anything was written, so nothing changed.
    CancelledBeforeWrite,
    /// Written, but no final answer was obtained: the effect is unknown and must not be repeated
    /// automatically.
    OutcomeUnknown,
    /// The device or SIM changed while the task was running; the result belongs to another
    /// context and is discarded.
    ContextChanged,
}

impl ToolOutcome {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Ok => "tool:ok",
            Self::Rejected => "tool:rejected",
            Self::Unsupported => "tool:unsupported",
            Self::TransportFailure => "tool:transport_failure",
            Self::FormatMismatch => "tool:format_mismatch",
            Self::CancelledBeforeWrite => "tool:cancelled_before_write",
            Self::OutcomeUnknown => "tool:outcome_unknown",
            Self::ContextChanged => "tool:context_changed",
        }
    }

    #[must_use]
    pub const fn is_failure(self) -> bool {
        !matches!(self, Self::Ok)
    }

    /// Classify a final code.
    ///
    /// A bare `ERROR` says this command failed; it does not say the firmware lacks the command, so
    /// it maps to [`Self::Rejected`]. `+CME ERROR: 4` is the one refusal whose meaning is fixed by
    /// the standard ("operation not supported"), so that single case is reported as unsupported.
    #[must_use]
    pub fn from_final_code(final_code: &AtFinalCode) -> Self {
        match final_code {
            AtFinalCode::Ok => Self::Ok,
            AtFinalCode::CmeError(detail) if detail.trim() == "4" => Self::Unsupported,
            _ => Self::Rejected,
        }
    }

    /// The status a capability entry takes after this outcome.
    ///
    /// `Ok` maps to `Supported`; a caller that received an empty payload records
    /// [`FeatureStatus::Empty`] instead, because "the module answered nothing" is not "the module
    /// gave us a value".
    #[must_use]
    pub const fn feature_status(self) -> FeatureStatus {
        match self {
            Self::Ok => FeatureStatus::Supported,
            Self::Rejected | Self::OutcomeUnknown => FeatureStatus::TemporarilyUnavailable,
            Self::Unsupported => FeatureStatus::UnsupportedConfirmed,
            Self::TransportFailure => FeatureStatus::TransportFailure,
            Self::FormatMismatch => FeatureStatus::FormatMismatch,
            Self::CancelledBeforeWrite | Self::ContextChanged => FeatureStatus::NotProbed,
        }
    }
}

/// Which request a task performs.
#[derive(Clone, Eq, PartialEq)]
pub enum ToolOperation {
    Read(ToolReadId),
    /// Every whitelisted read, run as an ordered batch. The port layer never receives this
    /// variant: the runner expands it into individual [`ToolOperation::Read`] sub-requests.
    ProbeAll,
    Expert(ValidatedToolLine),
}

impl fmt::Debug for ToolOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(id) => formatter.debug_tuple("Read").field(id).finish(),
            Self::ProbeAll => formatter.write_str("ProbeAll"),
            // The expert request text is not printed, not even here.
            Self::Expert(_) => formatter.write_str("Expert([REDACTED_TOOL_COMMAND])"),
        }
    }
}

/// A tool operation without its text, for history entries that outlive the terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolOperationKind {
    Read(ToolReadId),
    ProbeAll,
    Expert,
}

impl ToolOperation {
    #[must_use]
    pub fn kind(&self) -> ToolOperationKind {
        match self {
            Self::Read(id) => ToolOperationKind::Read(*id),
            Self::ProbeAll => ToolOperationKind::ProbeAll,
            Self::Expert(_) => ToolOperationKind::Expert,
        }
    }

    /// True when the request text must never be retried automatically after an unknown result.
    #[must_use]
    pub fn is_expert(&self) -> bool {
        matches!(self, Self::Expert(_))
    }
}

/// The device, SIM and port a task is bound to.
///
/// Captured when the task is created and re-checked before it is allowed to write: a task that
/// started against one module must never be completed against another.
#[derive(Clone, Eq, PartialEq)]
pub struct ToolContext {
    pub device_epoch: DeviceEpoch,
    pub sim_epoch: u64,
    pub identity: StableDeviceIdentity,
    /// The AT port this task is allowed to use.
    pub at_port: String,
}

impl fmt::Debug for ToolContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolContext")
            .field("device_epoch", &self.device_epoch)
            .field("sim_epoch", &self.sim_epoch)
            .field("identity", &MaskedIdentity(&self.identity))
            // The port string can be a device interface path that embeds a serial number, so it is
            // masked together with the identifiers.
            .field("at_port", &"[REDACTED_TOOL_PORT]")
            .finish()
    }
}

struct MaskedIdentity<'a>(&'a StableDeviceIdentity);

impl fmt::Debug for MaskedIdentity<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableDeviceIdentity")
            .field("container_id", &"[REDACTED]")
            .field("device_instance_id", &"[REDACTED]")
            .field("vid", &format_args!("{:#06x}", self.0.vid))
            .field("pid", &format_args!("{:#06x}", self.0.pid))
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolRequest {
    pub id: u64,
    pub context: ToolContext,
    pub operation: ToolOperation,
}

/// Response text captured for one task.
///
/// Bounded, redacted in `Debug`, never serialized. The UI renders [`ToolTranscript::lines`]
/// directly and copies them only through an explicit user action.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct ToolTranscript {
    lines: Vec<String>,
    bytes: usize,
    truncated: bool,
}

impl fmt::Debug for ToolTranscript {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED_TOOL_TRANSCRIPT]")
    }
}

impl ToolTranscript {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a transcript from response lines, dropping anything past the bound.
    #[must_use]
    pub fn from_lines<I: IntoIterator<Item = String>>(lines: I) -> Self {
        let mut transcript = Self::new();
        for line in lines {
            transcript.push(line);
        }
        transcript
    }

    /// Append one line. Lines past the bound are dropped and the transcript is marked truncated —
    /// never silently presented as the whole response.
    pub fn push(&mut self, line: String) {
        if self.bytes + line.len() > MAX_TRANSCRIPT_BYTES {
            self.truncated = true;
            return;
        }
        self.bytes += line.len();
        self.lines.push(line);
    }

    /// Explicit reveal, for on-screen display and the copy action only.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    #[must_use]
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

/// One finished task.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolReceipt {
    pub id: u64,
    pub context: ToolContext,
    pub operation: ToolOperationKind,
    pub outcome: ToolOutcome,
    pub elapsed: Duration,
    pub transcript: Arc<ToolTranscript>,
    /// Whether the module returned a final code at all. `false` with `Ok` is impossible; `false`
    /// with any other outcome explains that the answer never arrived.
    pub saw_final_code: bool,
    /// How many response lines the module sent, excluding URCs. Zero with [`ToolOutcome::Ok`] is
    /// "the module answered nothing", which is [`FeatureStatus::Empty`], not a value.
    pub payload_lines: usize,
}

/// The task the runner is currently working on (or the last one it finished).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolTaskSnapshot {
    pub id: u64,
    pub context: ToolContext,
    pub operation: ToolOperationKind,
    pub phase: ToolPhase,
    pub outcome: Option<ToolOutcome>,
    pub completed_items: usize,
    pub total_items: usize,
}

impl ToolTaskSnapshot {
    #[must_use]
    pub fn new(request: &ToolRequest, total_items: usize) -> Self {
        Self {
            id: request.id,
            context: request.context.clone(),
            operation: request.operation.kind(),
            phase: ToolPhase::Queued,
            outcome: None,
            completed_items: 0,
            total_items,
        }
    }
}

/// One whitelisted read's evidence.
///
/// The row deliberately holds no response text: the reason and the context are enough for the UI
/// to explain what happened, and the raw answer stays in the task transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCapabilityRow {
    pub id: ToolReadId,
    pub status: FeatureStatus,
    pub reason: ToolOutcome,
    pub observed_at: SystemTime,
    pub context: ToolContext,
}

impl ToolCapabilityRow {
    #[must_use]
    pub fn new(id: ToolReadId, reason: ToolOutcome, context: ToolContext, now: SystemTime) -> Self {
        Self {
            id,
            status: reason.feature_status(),
            reason,
            observed_at: now,
            context,
        }
    }

    /// A read that answered with a final `OK` but carried nothing: the query works, there is just
    /// nothing to report. Distinct from a failure, and distinct from a value.
    #[must_use]
    pub fn empty(id: ToolReadId, context: ToolContext, now: SystemTime) -> Self {
        Self {
            id,
            status: FeatureStatus::Empty,
            reason: ToolOutcome::Ok,
            observed_at: now,
            context,
        }
    }
}

/// How the module reports its USB network mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsbNetReading {
    /// A value this build has verified and may offer to switch between.
    Verified(VerifiedUsbNetProfile),
    /// A value the module reported that this build does not recognise. Shown as unrecognised and
    /// never switched automatically.
    Unrecognised,
}

/// Module identity and configuration as far as it has been observed.
#[derive(Clone, Default, PartialEq)]
pub struct ModuleProfile {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub revision: Option<String>,
    pub usb_net: Option<UsbNetReading>,
    pub pdp_contexts: Vec<PdpContext>,
    /// Sensor readings in report order, exactly as the module reported them; a missing sensor is
    /// absent rather than reported as zero, and a channel the firmware did not name keeps
    /// `name: None`.
    pub temperature: Vec<SensorTemperature>,
    pub observed_at: Option<SystemTime>,
    pub context: Option<ToolContext>,
}

impl fmt::Debug for ModuleProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `PdpContext`'s own Debug already redacts the APN; the rest here carries no user data.
        formatter
            .debug_struct("ModuleProfile")
            .field("manufacturer", &self.manufacturer)
            .field("model", &self.model)
            .field("revision", &self.revision)
            .field("usb_net", &self.usb_net)
            .field("pdp_contexts", &self.pdp_contexts.len())
            .field("temperature", &self.temperature)
            .field("observed_at", &self.observed_at)
            .field("context", &self.context)
            .finish()
    }
}

impl ModuleProfile {
    /// Drop everything learned about one device. Used when the device or SIM changes.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.manufacturer.is_none()
            && self.model.is_none()
            && self.revision.is_none()
            && self.usb_net.is_none()
            && self.pdp_contexts.is_empty()
            && self.temperature.is_empty()
    }
}

/// One finished task in the bounded terminal history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolHistoryEntry {
    pub id: u64,
    pub operation: ToolOperationKind,
    pub outcome: ToolOutcome,
    pub elapsed: Duration,
    pub finished_at: SystemTime,
    pub transcript: Arc<ToolTranscript>,
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct ToolHistory {
    entries: VecDeque<ToolHistoryEntry>,
    bytes: usize,
}

impl fmt::Debug for ToolHistory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolHistory")
            .field("entries", &self.entries.len())
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl ToolHistory {
    #[must_use]
    pub fn entries(&self) -> &VecDeque<ToolHistoryEntry> {
        &self.entries
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Append one entry, evicting the oldest until both caps hold. A bounded history keeps the
    /// panel's memory flat without ever growing a log file.
    pub fn push(&mut self, entry: ToolHistoryEntry) {
        self.bytes += entry.transcript.bytes();
        self.entries.push_back(entry);
        while self.entries.len() > MAX_TOOL_HISTORY_ITEMS || self.bytes > MAX_TOOL_HISTORY_BYTES {
            let Some(removed) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(removed.transcript.bytes());
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
}

/// The frozen expert request awaiting its one confirmation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingExpertTool {
    pub id: u64,
    /// The exact text that will be written. Its `Debug` is redacted; the UI displays it through
    /// [`ValidatedToolLine::expose_for_confirmation`].
    pub line: ValidatedToolLine,
    pub expires_at: SystemTime,
}

/// Everything the device-tools page renders.
#[derive(Clone, Default, PartialEq)]
pub struct DeviceToolsSnapshot {
    pub task: Option<ToolTaskSnapshot>,
    pub capabilities: Vec<ToolCapabilityRow>,
    pub profile: ModuleProfile,
    pub history: ToolHistory,
    pub pending_expert: Option<PendingExpertTool>,
    /// Why the last tool request was refused, if it was.
    pub last_refusal: Option<ToolOutcome>,
}

impl fmt::Debug for DeviceToolsSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceToolsSnapshot")
            .field("task", &self.task)
            .field("capabilities", &self.capabilities)
            .field("profile", &self.profile)
            .field("history", &self.history)
            .field("pending_expert", &self.pending_expert)
            .field("last_refusal", &self.last_refusal)
            .finish()
    }
}

impl DeviceToolsSnapshot {
    #[must_use]
    pub fn capability(&self, id: ToolReadId) -> Option<&ToolCapabilityRow> {
        self.capabilities.iter().find(|row| row.id == id)
    }

    /// Record one capability result, replacing any earlier row for the same read.
    ///
    /// A failure stays attached to the item it happened on: the other rows keep their evidence.
    pub fn record_capability(&mut self, row: ToolCapabilityRow) {
        match self
            .capabilities
            .iter_mut()
            .find(|existing| existing.id == row.id)
        {
            Some(existing) => *existing = row,
            None => self.capabilities.push(row),
        }
    }

    /// True while a task is queued, running or cancelling.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.task
            .as_ref()
            .is_some_and(|task| task.phase.is_active())
    }

    /// Forget everything tied to the previous device or SIM.
    pub fn invalidate_context(&mut self) {
        self.task = None;
        self.capabilities.clear();
        self.profile.clear();
        self.pending_expert = None;
        self.history.clear();
    }
}

// ---------------------------------------------------------------------------------------------
// Response extraction
// ---------------------------------------------------------------------------------------------

/// The payload of the first line carrying `prefix`, without the prefix.
#[must_use]
pub fn extract_payload<'a>(lines: &'a [String], prefix: &str) -> Option<&'a str> {
    lines.iter().find_map(|line| {
        line.strip_prefix(prefix)
            .map(str::trim)
            .filter(|payload| !payload.is_empty())
    })
}

/// Manufacturer/model/revision from their own typed query.
///
/// Some modules answer with a bare line instead of the `+CGMI:` form; both are accepted, but the
/// value is never taken from an arbitrary line of some other command's response.
#[must_use]
pub fn extract_identity(lines: &[String], prefix: &str) -> Option<String> {
    if let Some(payload) = extract_payload(lines, prefix) {
        return Some(payload.to_owned());
    }
    // A single unattributed line is this command's answer when nothing else was reported.
    let candidate = lines.iter().find(|line| !line.trim().is_empty())?;
    if candidate.trim_start().starts_with('+') {
        return None;
    }
    Some(candidate.trim().to_owned())
}

/// Parse the `AT+QCFG="usbnet"` answer. An unparsable value is reported as unrecognised rather
/// than rounded to a nearby profile.
#[must_use]
pub fn parse_usb_net(lines: &[String]) -> Option<UsbNetReading> {
    let payload = extract_payload(lines, "+QCFG:")?;
    let value = payload.rsplit(',').next()?.trim().trim_matches('"');
    match value.parse::<u8>() {
        Ok(raw) => Some(
            VerifiedUsbNetProfile::from_raw(raw).map_or(UsbNetReading::Unrecognised, |profile| {
                UsbNetReading::Verified(profile)
            }),
        ),
        Err(_) => Some(UsbNetReading::Unrecognised),
    }
}

/// Rebuild the typed response value the existing parsers take, so the tool page parses the same
/// bytes with the same code as the rest of the panel instead of growing a second interpretation.
#[must_use]
pub fn as_at_response(
    epoch: DeviceEpoch,
    command: dji4g_at_protocol::AtCommand,
    response: &ToolResponse,
) -> dji4g_at_protocol::AtResponse {
    dji4g_at_protocol::AtResponse {
        epoch,
        command,
        lines: response.lines.clone(),
        final_code: response.final_code.clone(),
    }
}

/// Temperatures using the existing parser. Missing sensors stay missing, and channels the firmware
/// left unnamed stay unnamed instead of being given invented labels here.
#[must_use]
pub fn parse_profile_temperature(response: &ToolResponse) -> Vec<SensorTemperature> {
    let refs = response
        .lines
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    parse_qtemp_lines(&refs)
}

/// The response lines a UI transcript should show: the response payload plus the URCs, which are
/// marked so they are not read as this command's answer.
#[must_use]
pub fn transcript_from_response(response: &ToolResponse) -> ToolTranscript {
    let mut transcript = ToolTranscript::new();
    for line in &response.lines {
        transcript.push(line.clone());
    }
    for line in &response.urc_lines {
        transcript.push(format!("[模块主动上报] {line}"));
    }
    transcript
}
