use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

use crate::ValidatedActionToken;
use dji4g_domain::{
    AdapterBinding, AtControlAvailability, CellularSnapshot, DeviceEpoch, DevicePresence,
    ErrorCode, HotspotStatus, NetworkSnapshot, ProtocolCoverage, SmsMessage, StableDeviceIdentity,
};

/// An owned, object-safe future used at the application/platform boundary.
pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct MonoTime(u64);

impl MonoTime {
    #[must_use]
    pub const fn from_ticks(ticks: u64) -> Self {
        Self(ticks)
    }

    #[must_use]
    pub const fn ticks(self) -> u64 {
        self.0
    }
}

pub trait Clock: Send + Sync {
    fn system_now(&self) -> SystemTime;
    fn monotonic_now(&self) -> MonoTime;
}

#[derive(Debug)]
pub struct FakeClock {
    wall_millis: AtomicU64,
    monotonic_millis: AtomicU64,
}

impl FakeClock {
    #[must_use]
    pub fn new(now: SystemTime) -> Self {
        let wall_millis = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |value| {
                value.as_millis().min(u128::from(u64::MAX)) as u64
            });
        Self {
            wall_millis: AtomicU64::new(wall_millis),
            monotonic_millis: AtomicU64::new(0),
        }
    }

    pub fn advance_wall(&self, by: Duration) {
        let millis = by.as_millis().min(u128::from(u64::MAX)) as u64;
        self.wall_millis.fetch_add(millis, Ordering::SeqCst);
    }

    pub fn set_wall_backwards(&self, by: Duration) {
        let millis = by.as_millis().min(u128::from(u64::MAX)) as u64;
        let _ = self
            .wall_millis
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                Some(value.saturating_sub(millis))
            });
    }

    pub fn advance_mono(&self, by: Duration) {
        let millis = by.as_millis().min(u128::from(u64::MAX)) as u64;
        self.monotonic_millis.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn system_now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_millis(self.wall_millis.load(Ordering::SeqCst))
    }

    fn monotonic_now(&self) -> MonoTime {
        MonoTime(self.monotonic_millis.load(Ordering::SeqCst))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StableCode(Arc<str>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StableCodeError {
    Empty,
    TooLong,
    UnsafeCharacter,
}

impl StableCode {
    pub fn try_from_static(value: &'static str) -> Result<Self, StableCodeError> {
        Self::try_from_owned(value.to_owned())
    }

    pub fn try_from_owned(value: String) -> Result<Self, StableCodeError> {
        if value.is_empty() {
            return Err(StableCodeError::Empty);
        }
        if value.len() > 64 {
            return Err(StableCodeError::TooLong);
        }
        if !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b':' | b'-')
        }) {
            return Err(StableCodeError::UnsafeCharacter);
        }
        Ok(Self(Arc::from(value)))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailureCode {
    pub category: ErrorCode,
    pub stable: StableCode,
}

impl FailureCode {
    #[must_use]
    pub fn new(category: ErrorCode, stable: StableCode) -> Self {
        Self { category, stable }
    }

    #[must_use]
    pub const fn category(&self) -> ErrorCode {
        self.category
    }

    #[must_use]
    pub fn stable(&self) -> &StableCode {
        &self.stable
    }
}

impl std::fmt::Display for FailureCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.stable.as_str())
    }
}

impl std::error::Error for FailureCode {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortError {
    pub code: FailureCode,
    pub os_code: Option<u32>,
}

impl PortError {
    #[must_use]
    pub fn new(category: ErrorCode, stable: &'static str) -> Self {
        Self {
            code: FailureCode::new(
                category,
                StableCode::try_from_static(stable).expect("static stable code"),
            ),
            os_code: None,
        }
    }
}

impl std::fmt::Display for PortError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.code.fmt(formatter)
    }
}

impl std::error::Error for PortError {}

pub type DevicePresenceDto = DevicePresence;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InventoryObservation {
    pub epoch: DeviceEpoch,
    pub presence: DevicePresenceDto,
    pub identity: Option<StableDeviceIdentity>,
    pub problem_code: Option<u32>,
    pub at_port: Option<String>,
    pub adapter_id: Option<String>,
}

pub trait InventoryPort: Send + Sync {
    fn scan(&self) -> PortFuture<'_, Result<InventoryObservation, PortError>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetContext {
    pub(crate) epoch: DeviceEpoch,
    pub(crate) identity: StableDeviceIdentity,
    pub(crate) at_port: Option<String>,
    pub(crate) adapter_id: Option<String>,
}

impl TargetContext {
    pub(crate) fn new(
        epoch: DeviceEpoch,
        identity: StableDeviceIdentity,
        at_port: Option<String>,
        adapter_id: Option<String>,
    ) -> Result<Self, PortError> {
        if !identity.is_supported() {
            return Err(PortError::new(
                ErrorCode::Unsupported,
                "pnp:unsupported_device",
            ));
        }
        Ok(Self {
            epoch,
            identity,
            at_port,
            adapter_id,
        })
    }

    #[must_use]
    pub const fn epoch(&self) -> DeviceEpoch {
        self.epoch
    }

    #[must_use]
    pub fn identity(&self) -> &StableDeviceIdentity {
        &self.identity
    }

    #[must_use]
    pub fn at_port(&self) -> Option<&str> {
        self.at_port.as_deref()
    }

    #[must_use]
    pub fn adapter_id(&self) -> Option<&str> {
        self.adapter_id.as_deref()
    }
}

pub trait AtPort: Send + Sync {
    fn observe(&self, target: &TargetContext) -> PortFuture<'_, Result<AtObservation, PortError>>;
    fn invalidate(&self, epoch: DeviceEpoch);
}

#[derive(Clone, Debug, PartialEq)]
pub struct AtObservation {
    pub availability: AtControlAvailability,
    pub cellular: Option<CellularSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterStateDto {
    UsableAddressAndRoute,
    NoUsableAddressOrRoute,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterObservationDto {
    pub epoch: DeviceEpoch,
    pub binding: AdapterBinding,
    pub state: AdapterStateDto,
    pub addresses: Vec<String>,
    pub gateways: Vec<String>,
    pub dns_servers: Vec<String>,
    pub ipv4: bool,
    pub ipv6: bool,
    /// Monotonic interface byte counters sampled this cycle; `None` = unavailable.
    pub rx_bytes: Option<u64>,
    pub tx_bytes: Option<u64>,
}

impl AdapterObservationDto {
    pub fn context(&self) -> AdapterContext {
        AdapterContext {
            epoch: self.epoch,
            binding: self.binding.clone(),
            addresses: self.addresses.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterContext {
    pub(crate) epoch: DeviceEpoch,
    pub(crate) binding: AdapterBinding,
    pub(crate) addresses: Vec<String>,
}

impl AdapterContext {
    #[must_use]
    pub const fn epoch(&self) -> DeviceEpoch {
        self.epoch
    }

    #[must_use]
    pub fn binding(&self) -> &AdapterBinding {
        &self.binding
    }

    #[must_use]
    pub fn addresses(&self) -> &[String] {
        &self.addresses
    }

    /// Derive the target context for an adapter the application already resolved and bound.
    ///
    /// This is the only public way to obtain a [`TargetContext`] outside the crate: it reuses the
    /// validated binding (which still rejects unsupported identities) instead of exposing free
    /// construction for an arbitrary device.
    pub fn target_context(&self) -> Result<TargetContext, PortError> {
        TargetContext::new(
            self.epoch,
            self.binding.target.clone(),
            None,
            Some(self.binding.adapter_id.clone()),
        )
    }
}

/// Read-only interface metrics for one bound adapter (research document §7.1, appendix B).
///
/// These are per-interface counters and link speeds, not measured internet throughput: the byte
/// counters include every source traversing the interface and the link rates are negotiated
/// interface speeds. An unreadable metric is reported as `Err`/`None` — never a fabricated zero.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdapterMetrics {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub in_errors: u64,
    pub out_errors: u64,
    pub in_discards: u64,
    pub out_discards: u64,
    /// Negotiated receive link rate in bit/s (`MIB_IF_ROW2::ReceiveLinkSpeed`).
    pub link_rx_bits_per_second: u64,
    /// Negotiated transmit link rate in bit/s (`MIB_IF_ROW2::TransmitLinkSpeed`).
    pub link_tx_bits_per_second: u64,
}

pub trait AdapterPort: Send + Sync {
    fn resolve(
        &self,
        target: &TargetContext,
    ) -> PortFuture<'_, Result<AdapterObservationDto, PortError>>;

    /// Read-only sample of the monotonic byte counters for an already-bound adapter GUID.
    ///
    /// This backs the 1 s rates-only tick: it reads *only* the interface octet counters
    /// (`rx`, `tx`) for the module adapter the reducer has already resolved and bound. It never
    /// re-enumerates adapters, never resolves a target, and never touches the AT or repair/write
    /// paths, so it cannot prepare, confirm, or execute any action. An unreadable counter is an
    /// honest gap reported as `Err` (the caller derives `None` rates) — never a fabricated zero
    /// and never a stale value.
    fn read_byte_counters(&self, adapter_id: &str)
    -> PortFuture<'_, Result<(u64, u64), PortError>>;

    /// Read the full interface metrics for an already-bound adapter GUID.
    ///
    /// Like [`Self::read_byte_counters`] this is strictly read-only and keyed by the adapter the
    /// reducer already bound; it never re-enumerates. Platform implementations that do not model
    /// the extended metrics keep the default: an honest `Unsupported` error that the caller
    /// records as an unavailable sample.
    fn read_metrics(&self, adapter_id: &str) -> PortFuture<'_, Result<AdapterMetrics, PortError>> {
        let _ = adapter_id;
        Box::pin(async {
            Err(PortError::new(
                ErrorCode::Unsupported,
                "app:metrics_unavailable",
            ))
        })
    }
}

/// One inbox listing: the decoded stored messages plus the module-reported `CPMS` capacity.
///
/// Capacity is `(used, total)` slots; either component may be absent when the firmware does not
/// report it, and a malformed or missing line records `None` rather than a guessed value.
#[derive(Clone, Debug, Default)]
pub struct SmsListing {
    pub messages: Vec<SmsMessage>,
    pub capacity: Option<(u32, u32)>,
}

/// Terminal result of one module-side send attempt (research document §6.3).
///
/// `Submitted` is submission only — the module accepted the PDU — and never means the peer
/// received the message. `OutcomeUnknown` may or may not have been submitted: a send is attempted
/// exactly once and never retried automatically.
pub use dji4g_domain::SmsSendResult;

/// One completed send attempt: the terminal outcome plus the recipient masked for display.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmsSendReceipt {
    pub result: SmsSendResult,
    pub recipient_masked: String,
    pub failure: Option<dji4g_domain::SmsFailureDetail>,
}

impl SmsSendReceipt {
    #[must_use]
    pub fn new(result: SmsSendResult, recipient: &str) -> Self {
        Self {
            result,
            recipient_masked: mask_recipient(recipient),
            failure: None,
        }
    }
}

/// Mask one recipient address for display: keep at most the last four characters and mask the
/// whole address when it is short enough that four characters would reveal it. This mirrors the
/// domain's sender mask rule and is a pure function so every display path can share one rule.
#[must_use]
pub fn mask_recipient(recipient: &str) -> String {
    let count = recipient.chars().count();
    if count <= 4 {
        return "****".to_owned();
    }
    format!(
        "****{}",
        recipient.chars().skip(count - 4).collect::<String>()
    )
}

/// Read, send, and delete SMS messages for one validated target (research document §6.2/§6.3).
///
/// The port is synchronous in shape but object-safe and future-based like the other ports. Every
/// call is one serial transaction owned by the module-side actor; the application layer must not
/// issue these calls concurrently on one target (`CMGL`/`CMGR`/`CMGS` transactions share the AT
/// port).
///
/// `list` requires the module to already be in PDU mode and fails closed with
/// `sms:pdu_mode_required` otherwise: [`Self::query_pdu_mode`] observes the current mode and
/// [`Self::enable_pdu_mode`] performs the session-setting change after the user has consented.
pub trait SmsPort: Send + Sync {
    /// One confirmed transaction owns preflight, mode switch and submission. Production ports
    /// must honour this shared deadline/cancellation through the actual serial I/O.
    fn send_controlled(
        &self,
        target: &TargetContext,
        recipient: &str,
        body: &str,
        control: dji4g_domain::SmsTransactionControl,
    ) -> PortFuture<'_, Result<SmsSendReceipt, PortError>> {
        // Compatibility for in-memory ports; real ports override this method.
        let _ = control;
        self.send(target, recipient, body)
    }
    /// Observe the current message format (`AT+CMGF?`).
    ///
    /// `Some(true)` is PDU mode, `Some(false)` text mode, and `None` when the response cannot be
    /// confirmed. A transport/protocol failure is an error, never a guess.
    fn query_pdu_mode(
        &self,
        target: &TargetContext,
    ) -> PortFuture<'_, Result<Option<bool>, PortError>>;

    /// Switch the module to PDU mode (`AT+CMGF=0`). This is a session setting: the user consent
    /// flow is owned by the UI, which must confirm before the panel dispatches an SMS refresh or
    /// send. The application performs at most one switch per confirmed attempt and never retries.
    fn enable_pdu_mode(&self, target: &TargetContext) -> PortFuture<'_, Result<(), PortError>>;

    /// List stored messages (`AT+CMGL=4`); requires PDU mode.
    fn list(&self, target: &TargetContext) -> PortFuture<'_, Result<SmsListing, PortError>>;

    /// Read one stored message (`AT+CMGR=<index>`). Reading may itself mark the message read.
    fn read(
        &self,
        target: &TargetContext,
        index: u32,
    ) -> PortFuture<'_, Result<SmsMessage, PortError>>;

    /// Delete one stored message (`AT+CMGD=<index>`); the final `OK` is the success proof.
    fn delete(&self, target: &TargetContext, index: u32) -> PortFuture<'_, Result<(), PortError>>;

    /// Submit one user-confirmed message; the port owns PDU preflight and one CMGS transaction.
    ///
    /// The UI owns the single per-send consent: the application only reaches this call after the
    /// user explicitly confirmed this exact recipient and body. The port switches an unconfirmed
    /// module to PDU mode once (with a confirming re-read) as part of that same authorised
    /// attempt. `Submitted` proves only that the module accepted the PDU — never that the peer
    /// received it. The attempt is made once and the application never retries it automatically.
    ///
    /// The default refuses the call so a build whose platform layer has not wired sending yet
    /// fails honestly instead of pretending to send.
    fn send(
        &self,
        target: &TargetContext,
        recipient: &str,
        body: &str,
    ) -> PortFuture<'_, Result<SmsSendReceipt, PortError>> {
        let _ = (target, recipient, body);
        Box::pin(async {
            Err(PortError::new(
                ErrorCode::Unsupported,
                "sms:send_unavailable",
            ))
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProbeStageDto {
    Passed,
    Failed { code: FailureCode },
    Unavailable { code: FailureCode },
    Unexecuted { code: FailureCode },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefaultRouteDto {
    TargetAdapter,
    VpnOrTun,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemRouteDto {
    pub owner: DefaultRouteDto,
    pub explanation_only: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeObservationDto {
    pub epoch: DeviceEpoch,
    pub adapter_id: String,
    pub gateway: ProbeStageDto,
    pub public: ProbeStageDto,
    pub dns: ProbeStageDto,
    pub protocol_coverage: Option<ProtocolCoverage>,
    pub system_route: Option<SystemRouteDto>,
}

pub trait NetworkProbePort: Send + Sync {
    fn observe(
        &self,
        adapter: &AdapterContext,
        active: bool,
    ) -> PortFuture<'_, Result<ProbeObservationDto, PortError>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HotspotObservation {
    pub status: HotspotStatus,
}

pub trait HotspotControl: Send + Sync {
    fn observe(
        &self,
        adapter: Option<&AdapterContext>,
    ) -> PortFuture<'_, Result<HotspotObservation, PortError>>;
    fn revalidate_toggle(
        &self,
        target: &TargetContext,
        enabled: bool,
    ) -> PortFuture<'_, Result<ActionPreconditions, PortError>>;
    fn set_enabled_once(
        &self,
        token: &ValidatedActionToken,
        enabled: bool,
    ) -> PortFuture<'_, Result<ExecutionReceipt, PortError>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionPreconditions {
    pub epoch: DeviceEpoch,
    pub before_state_hash: dji4g_domain::BeforeStateHash,
}

pub trait AutostartControl: Send + Sync {
    fn status(&self) -> PortFuture<'_, Result<AutostartKnownState, PortError>>;
    fn set_enabled(&self, enabled: bool) -> PortFuture<'_, Result<AutostartKnownState, PortError>>;
}

pub trait PrivilegedExecutor: Send + Sync {
    fn execute_once(
        &self,
        token: ValidatedActionToken,
    ) -> PortFuture<'_, Result<ExecutionReceipt, PortError>>;
}

pub trait ActionExecutor: Send + Sync {
    fn execute_once(&self, token: ValidatedActionToken) -> Result<ExecutionReceipt, PortError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionReceiptOutcome {
    Applied,
    Failed { code: ErrorCode },
    OutcomeUnknown { code: ErrorCode },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionReceipt {
    pub outcome: ExecutionReceiptOutcome,
    pub after_state_hash: Option<dji4g_domain::AfterStateHash>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedNetworkEvidence {
    pub network: NetworkSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LanguageCode {
    ZhCn,
    EnUs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutostartKnownState {
    Disabled,
    Enabled,
    Drift,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutostartStatus {
    Loading,
    Ready(AutostartKnownState),
    Saving {
        desired_enabled: bool,
        previous: Option<AutostartKnownState>,
    },
    Failed {
        code: FailureCode,
        previous: Option<AutostartKnownState>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsPersistenceState {
    Clean,
    Saving,
    Failed { code: FailureCode },
}

/// Terminal result of one panel-side `config.toml` write, tagged with the settings revision the
/// panel read when it decided to save. The reducer uses the revision to reject outcomes that no
/// longer describe the current settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsSaveOutcome {
    pub revision: u64,
    pub result: Result<(), FailureCode>,
}

/// Terminal result of one autostart registration write for the desired value the UI requested.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutostartApplyOutcome {
    pub desired_enabled: bool,
    pub observed: Result<AutostartKnownState, FailureCode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsSnapshot {
    pub revision: u64,
    pub language: LanguageCode,
    pub autostart: AutostartStatus,
    pub start_minimized: bool,
    pub active_probe: bool,
    pub log_level: LogLevel,
    pub persistence: SettingsPersistenceState,
}

impl Default for SettingsSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            language: LanguageCode::ZhCn,
            autostart: AutostartStatus::Loading,
            start_minimized: false,
            active_probe: true,
            log_level: LogLevel::Info,
            persistence: SettingsPersistenceState::Clean,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandState {
    Ready,
    Busy,
    QueueFull,
    BackendUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandStateSnapshot {
    pub state: CommandState,
    pub last_error: Option<FailureCode>,
}

impl Default for CommandStateSnapshot {
    fn default() -> Self {
        Self {
            state: CommandState::Ready,
            last_error: None,
        }
    }
}

/// A small fake executor used by application scenario tests and by downstream UI tests.
#[derive(Debug)]
pub struct FakeActionExecutor {
    calls: AtomicUsize,
    result: Mutex<Result<ExecutionReceipt, PortError>>,
}

impl Default for FakeActionExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeActionExecutor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            result: Mutex::new(Ok(ExecutionReceipt {
                outcome: ExecutionReceiptOutcome::Applied,
                after_state_hash: None,
            })),
        }
    }

    pub fn set_result(&self, result: Result<ExecutionReceipt, PortError>) {
        *self.result.lock().expect("fake executor result lock") = result;
    }

    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ActionExecutor for FakeActionExecutor {
    fn execute_once(&self, token: ValidatedActionToken) -> Result<ExecutionReceipt, PortError> {
        // Touch all capability fields to keep the fake representative of the real executor
        // boundary; the values are never exposed or logged.
        let _ = (&token.target, token.before_state_hash, token.plan_id);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result
            .lock()
            .expect("fake executor result lock")
            .clone()
    }
}
