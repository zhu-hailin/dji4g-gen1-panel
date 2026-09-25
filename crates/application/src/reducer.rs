use std::{
    ops::Index,
    sync::Arc,
    time::{Duration, SystemTime},
};

use dji4g_domain::{
    AdapterBinding, AdapterState, AppSnapshot, AtControlAvailability, Availability, BoundDnsStatus,
    BoundEvidence, BoundPublicStatus, CellularBlock, CellularSnapshot, ClassificationInput,
    ClassificationPhase, DefaultRouteOwner, DeviceEpoch, DevicePresence, DeviceProfile,
    DeviceSnapshot, Evidence, EvidenceSource, FeatureStatus, Freshness, GlobalConnectivity,
    HotspotStatus, Issue, IssueLayer, IssueSeverity, NetworkSnapshot, ProtocolCoverage,
    RegistrationState, ServingCell, SmsInboxSummary, SmsMessage, SmsStorageId,
    StableDeviceIdentity, Timeline, TimelineEvent, TimelineEventKind, classify,
};

use crate::sms::SmsStore;
use crate::{AdapterContext, TargetContext};
use crate::{
    AdapterMetrics, AdapterObservationDto, AdapterStateDto, AtObservation, DefaultRouteDto,
    DevicePresenceDto, FeatureCapability, FeatureKey, HotspotObservation, InventoryObservation,
    ProbeObservationDto, ProbeStageDto,
};
use crate::{
    AutostartApplyOutcome, AutostartStatus, CommandStateSnapshot, FailureCode, LanguageCode,
    LogLevel, SettingsPersistenceState, SettingsSnapshot, StableCode,
};

const EVIDENCE_TTL: Duration = Duration::from_secs(30);

/// Sampling baseline for the interface byte counters behind the rate display.  Kept outside the
/// snapshot: only the computed rates are evidence, the raw counters are an implementation detail.
#[derive(Clone, Debug)]
struct RateState {
    adapter_id: String,
    epoch: DeviceEpoch,
    prev_rx: u64,
    prev_tx: u64,
    prev_at: SystemTime,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RefreshCycleId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EpochInvalidationReason {
    PhysicalRemoval,
    IdentityChanged,
    Reenumeration,
    ExplicitRefresh,
    /// A backend event carried a strictly newer epoch (the inventory port re-enumerates
    /// from scratch after a transient empty scan); the reducer must follow it monotonically.
    Advanced,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DiagnosticCheckId {
    UsbDevice,
    AtControl,
    Cellular,
    WindowsAdapter,
    BoundGateway,
    BoundPublic,
    BoundDns,
    SystemRoute,
    Hotspot,
}

impl DiagnosticCheckId {
    pub const ORDERED: [Self; 9] = [
        Self::UsbDevice,
        Self::AtControl,
        Self::Cellular,
        Self::WindowsAdapter,
        Self::BoundGateway,
        Self::BoundPublic,
        Self::BoundDns,
        Self::SystemRoute,
        Self::Hotspot,
    ];

    const fn index(self) -> usize {
        match self {
            Self::UsbDevice => 0,
            Self::AtControl => 1,
            Self::Cellular => 2,
            Self::WindowsAdapter => 3,
            Self::BoundGateway => 4,
            Self::BoundPublic => 5,
            Self::BoundDns => 6,
            Self::SystemRoute => 7,
            Self::Hotspot => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnexecutedReason {
    DisabledBySetting,
    NotScheduled,
    Superseded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticCheckState {
    Unexecuted { reason: UnexecutedReason },
    Running { cycle: RefreshCycleId },
    Passed,
    Failed { code: FailureCode },
    Unavailable { code: FailureCode },
    Expired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticCheckSnapshot {
    pub id: DiagnosticCheckId,
    pub state: DiagnosticCheckState,
    pub epoch: DeviceEpoch,
    pub started_at: Option<SystemTime>,
    pub finished_at: Option<SystemTime>,
    pub observed_at: Option<SystemTime>,
    pub expires_at: Option<SystemTime>,
}

impl DiagnosticCheckSnapshot {
    #[must_use]
    pub fn freshness(&self, now: SystemTime) -> Freshness {
        match self.state {
            DiagnosticCheckState::Passed
            | DiagnosticCheckState::Failed { .. }
            | DiagnosticCheckState::Unavailable { .. } => {
                if self.expires_at.is_some_and(|expiry| now <= expiry) {
                    Freshness::Fresh
                } else {
                    Freshness::Stale
                }
            }
            DiagnosticCheckState::Expired => Freshness::Stale,
            DiagnosticCheckState::Running { .. } => Freshness::Unknown,
            DiagnosticCheckState::Unexecuted { .. } => Freshness::Unknown,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticSet([DiagnosticCheckSnapshot; 9]);

impl DiagnosticSet {
    #[must_use]
    pub fn new(epoch: DeviceEpoch) -> Self {
        Self(std::array::from_fn(|index| DiagnosticCheckSnapshot {
            id: DiagnosticCheckId::ORDERED[index],
            state: DiagnosticCheckState::Unexecuted {
                reason: UnexecutedReason::NotScheduled,
            },
            epoch,
            started_at: None,
            finished_at: None,
            observed_at: None,
            expires_at: None,
        }))
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &DiagnosticCheckSnapshot> {
        self.0.iter()
    }

    #[must_use]
    pub fn get(&self, id: DiagnosticCheckId) -> &DiagnosticCheckSnapshot {
        &self.0[id.index()]
    }

    fn get_mut(&mut self, id: DiagnosticCheckId) -> &mut DiagnosticCheckSnapshot {
        &mut self.0[id.index()]
    }

    fn reset(&mut self, epoch: DeviceEpoch) {
        *self = Self::new(epoch);
    }
}

impl Index<DiagnosticCheckId> for DiagnosticSet {
    type Output = DiagnosticCheckSnapshot;

    fn index(&self, index: DiagnosticCheckId) -> &Self::Output {
        self.get(index)
    }
}

#[derive(Clone, Debug)]
pub enum CheckResult<T> {
    Passed {
        value: T,
        observed_at: SystemTime,
    },
    Failed {
        code: FailureCode,
        observed_at: SystemTime,
    },
    Unavailable {
        code: FailureCode,
        observed_at: SystemTime,
    },
    Unexecuted {
        reason: UnexecutedReason,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ControllerSnapshot {
    pub host_network: crate::HostNetworkSnapshot,
    pub publication_revision: u64,
    pub app: Arc<AppSnapshot>,
    pub diagnostics: DiagnosticSet,
    pub prepared_action: Option<crate::PreparedActionSnapshot>,
    pub operation: Option<crate::OperationUiSnapshot>,
    pub settings: SettingsSnapshot,
    pub command_state: CommandStateSnapshot,
    /// Per-action readiness at snapshot time, so the UI can enable only what the reducer's
    /// safety prerequisites actually allow and explain why a button is disabled.
    pub action_readiness: Vec<ActionReadiness>,
    /// Last user-visible command rejection (a prepare or confirm that failed with a definite
    /// reason), surfaced as a toast. `None` when the last command was accepted.
    pub feedback: Option<UiFeedback>,
    /// SIM-session counter (research document §4.3). Advances only when a card change is proven by
    /// a fingerprint difference while the device epoch stays put; it is never inferred while the
    /// ICCID is unreadable (`sim_identity == None`). Display-only in this phase: plan revalidation
    /// keeps using the device epoch and the evidence revision.
    pub sim_epoch: u64,
    /// Module capability verdicts scoped to the current device context (§8.1); `None` until a
    /// device profile (and, when reported, firmware) can be anchored. Query one feature through
    /// [`Self::feature`] or [`FeatureCapability::get`].
    pub feature_status: Option<FeatureCapability>,
    /// Last full interface-metrics sample for the bound adapter (§7.1); `None` when no sample has
    /// been taken yet or the most recent sample could not be read (honest gap, never stale data).
    pub adapter_metrics: Option<AdapterMetrics>,
    /// Bounded, oldest-first log of observed device/network transitions (§5.4). Only witnessed
    /// changes are recorded; the timeline never claims to have captured every handover.
    pub timeline: Timeline,
    /// Aggregate SMS inbox state for the current device/SIM epoch (§6.2). Bodies never travel in
    /// the snapshot; the UI reads them from the application store on demand.
    pub sms_inbox: SmsInboxSummary,
    /// Read-only presentation copy of the stored messages for the current epoch, oldest-first,
    /// with long-message fragments already merged into single entries (§6.2). `SmsMessage`
    /// redacts sender/body in `Debug` and `Serialize`, so this list can travel in the snapshot
    /// without entering logs or diagnostics exports.
    pub sms_messages: Vec<dji4g_domain::SmsDisplayMessage>,
    pub sms_delete: Option<crate::SmsDeleteSnapshot>,
    pub serial_work_busy: bool,
    pub sms_send: Option<dji4g_domain::SmsSendSnapshot>,
    pub sms_refresh_pending: bool,
    pub sms_read_phase: Option<dji4g_domain::SmsReadPhase>,
    pub sms_read_progress: usize,
    pub sms_read_report: Option<dji4g_domain::SmsReadReport>,
    pub sms_inbox_failure: Option<crate::PortError>,
    /// Device-tool task, capability evidence, module profile and bounded transcript history
    /// (§7). The type redacts request and response text from `Debug` and never implements
    /// `Serialize`, so this section can ride in the snapshot without reaching a log or an export.
    pub device_tools: crate::DeviceToolsSnapshot,
}

impl ControllerSnapshot {
    /// Current capability verdict for one probed module feature, for the anchored device context.
    /// `NotProbed` before any verdict has been recorded (or when no device context is anchored).
    #[must_use]
    pub fn feature(&self, key: FeatureKey) -> FeatureStatus {
        self.feature_status
            .as_ref()
            .map_or(FeatureStatus::NotProbed, |capability| capability.get(key))
    }
}

/// Parameterless readiness key: the reducer's prerequisites never depend on an action's
/// parameter values (DNS servers, APN, CID, hotspot direction), only on its kind.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ActionReadinessKey {
    RenewDhcp,
    ApplyDnsProfile,
    RestartAdapter,
    ReenumerateDevice,
    RestartModule,
    EditApn,
    SetUsbNetworkProfile,
    ToggleHotspot,
}

/// One action's readiness verdict, derived from the same `action_prerequisite` gate the
/// controller enforces at prepare time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionReadiness {
    pub key: ActionReadinessKey,
    pub ready: Result<(), FailureCode>,
}

/// A command rejection the user must see (prepare/confirm failed after dispatch).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiFeedback {
    /// Monotonic per-controller sequence; the UI shows a toast exactly once per rejection.
    pub seq: u64,
    pub code: FailureCode,
}

/// The `AtFinished` variant carries the full AT observation (which now includes the cellular
/// snapshot with serving-cell, number-lookup, and SIM-identity data) by value, so the enum is
/// legitimately large. Boxing the payload would churn every construction and match site across
/// concurrently evolving crates for a pure memory-layout nit; the variant is built only on the
/// monitor thread and cloned once per controller event, so the size has no hot path.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum BackendEvent {
    EpochInvalidated {
        next_epoch: DeviceEpoch,
        reason: EpochInvalidationReason,
    },
    RefreshStarted {
        cycle: RefreshCycleId,
        epoch: DeviceEpoch,
        scheduled: CheckMask,
    },
    InventoryFinished {
        cycle: RefreshCycleId,
        epoch: DeviceEpoch,
        result: CheckResult<InventoryObservation>,
    },
    AtFinished {
        cycle: RefreshCycleId,
        epoch: DeviceEpoch,
        result: CheckResult<AtObservation>,
    },
    AdapterFinished {
        cycle: RefreshCycleId,
        epoch: DeviceEpoch,
        result: CheckResult<AdapterObservationDto>,
    },
    ProbeFinished {
        cycle: RefreshCycleId,
        epoch: DeviceEpoch,
        result: CheckResult<ProbeObservationDto>,
    },
    HotspotFinished {
        cycle: RefreshCycleId,
        epoch: DeviceEpoch,
        result: CheckResult<HotspotObservation>,
    },
    RefreshFinished {
        cycle: RefreshCycleId,
        epoch: DeviceEpoch,
    },
    SettingsChanged {
        settings: SettingsSnapshot,
    },
    ExpirationTick,
    /// A rates-only tick: the bound module adapter's monotonic byte counters were re-read on the
    /// 1 s cadence to refresh the throughput chart. This recomputes `down/up_bytes_per_sec` from
    /// the shared baseline logic and republishes, but it is *not* an evidence refresh: it never
    /// advances `observed_at`, never bumps `evidence_revision`, and never touches freshness, the
    /// diagnostic checks, or availability. `rx`/`tx` are `None` when the counters could not be
    /// read, which yields honest `None` rates rather than a stale or fabricated number.
    RatesSampled {
        adapter_id: String,
        epoch: DeviceEpoch,
        rx: Option<u64>,
        tx: Option<u64>,
        sampled_at: SystemTime,
    },
    /// A full interface-metrics sample for the bound adapter (§7.1): byte/error/discard counters
    /// and negotiated link rates. `metrics: None` means the platform could not read the interface
    /// this time; the snapshot honestly reports the gap instead of keeping a stale sample. The
    /// sample is keyed to the bound adapter/epoch, so a mismatched or future-dated sample is
    /// ignored. It is publication-only: it never advances `observed_at` or the evidence revision.
    AdapterMetricsSampled {
        adapter_id: String,
        epoch: DeviceEpoch,
        metrics: Option<AdapterMetrics>,
        sampled_at: SystemTime,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckMask(u16);

impl CheckMask {
    pub const ALL: Self = Self((1 << 9) - 1);

    #[must_use]
    pub const fn all() -> Self {
        Self::ALL
    }

    #[must_use]
    pub const fn contains(self, id: DiagnosticCheckId) -> bool {
        self.0 & (1 << id.index()) != 0
    }

    #[must_use]
    pub const fn only(id: DiagnosticCheckId) -> Self {
        Self(1 << id.index())
    }
}

#[derive(Clone, Debug)]
pub struct ReducerState {
    epoch: DeviceEpoch,
    current_cycle: RefreshCycleId,
    phase: ClassificationPhase,
    evidence_revision: u64,
    publication_revision: u64,
    active_probe: bool,
    settings: SettingsSnapshot,
    diagnostics: DiagnosticSet,
    device_presence: Option<Evidence<DevicePresenceDto>>,
    target_identity: Option<Evidence<StableDeviceIdentity>>,
    cellular: Option<Evidence<CellularSnapshot>>,
    cellular_block: Option<Evidence<CellularBlock>>,
    at_control: Option<Evidence<AtControlAvailability>>,
    adapter_binding: Option<Evidence<AdapterBinding>>,
    inventory_at_port: Option<String>,
    inventory_adapter_id: Option<String>,
    inventory_problem_code: Option<u32>,
    adapter: Option<Evidence<AdapterState>>,
    network: Option<NetworkSnapshot>,
    bound_public: Option<Evidence<BoundEvidence<BoundPublicStatus>>>,
    bound_dns: Option<Evidence<BoundEvidence<BoundDnsStatus>>>,
    protocol_coverage: Option<Evidence<BoundEvidence<ProtocolCoverage>>>,
    system_default_route: Option<Evidence<DefaultRouteOwner>>,
    global_connectivity: Option<Evidence<GlobalConnectivity>>,
    hotspot: HotspotStatus,
    consecutive_public_failures: u8,
    rate_state: Option<RateState>,
    /// SIM-session counter (research document §4.3): advances only when a card change is proven by
    /// a fingerprint difference while the device epoch stays put. ICCID 不可读（`sim_identity ==
    /// None`）时不做 epoch 推断——保守语义：无法确认连续性时，阶段 C 在 UI 侧按 None 处理旧号码
    /// 数据，而不是靠推断继续沿用。Display-only in this phase; plan revalidation keeps using the
    /// device epoch and the evidence revision.
    sim_epoch: u64,
    /// Last seen SIM fingerprint; `None` until the first readable AT+QCCID identity. Kept across
    /// device-epoch invalidations so a card swapped during an unplug/replug is still detected.
    sim_fingerprint: Option<[u8; 8]>,
    /// Module capability container for the current device context (§8.1); `None` until a device
    /// profile can be anchored.
    features: Option<FeatureCapability>,
    /// See [`ControllerSnapshot::adapter_metrics`].
    adapter_metrics: Option<AdapterMetrics>,
    /// See [`ControllerSnapshot::timeline`].
    timeline: Timeline,
    /// Deduplicated SMS inbox for the current device/SIM epoch (research §6.2).
    sms_store: SmsStore,
    /// Last SMS probe verdict and `CPMS` capacity; [`SmsStore::summary`] combines them with the
    /// stored messages on every snapshot.
    sms_status: FeatureStatus,
    sms_capacity: Option<(u32, u32)>,
    /// True between an observed device removal and the next supported inventory observation, so a
    /// reappearance is recorded exactly once and the first contact of a session is not an arrival.
    device_removed: bool,
    last_observed_at: SystemTime,
}

impl ReducerState {
    #[must_use]
    pub fn new(now: SystemTime) -> Self {
        Self {
            epoch: DeviceEpoch(0),
            current_cycle: RefreshCycleId(0),
            phase: ClassificationPhase::Startup,
            evidence_revision: 0,
            publication_revision: 0,
            active_probe: true,
            settings: SettingsSnapshot::default(),
            diagnostics: DiagnosticSet::new(DeviceEpoch(0)),
            device_presence: None,
            target_identity: None,
            cellular: None,
            cellular_block: None,
            at_control: None,
            adapter_binding: None,
            inventory_at_port: None,
            inventory_adapter_id: None,
            inventory_problem_code: None,
            adapter: None,
            network: None,
            bound_public: None,
            bound_dns: None,
            protocol_coverage: None,
            system_default_route: None,
            global_connectivity: None,
            hotspot: HotspotStatus::Unsupported(
                dji4g_domain::HotspotUnsupportedReason::SourceProfileUnavailable,
            ),
            consecutive_public_failures: 0,
            rate_state: None,
            sim_epoch: 0,
            sim_fingerprint: None,
            features: None,
            adapter_metrics: None,
            timeline: Timeline::new(),
            sms_store: SmsStore::new(),
            sms_status: FeatureStatus::NotProbed,
            sms_capacity: None,
            device_removed: false,
            last_observed_at: now,
        }
    }

    #[must_use]
    pub fn default_time() -> SystemTime {
        SystemTime::UNIX_EPOCH
    }

    #[must_use]
    pub const fn epoch(&self) -> DeviceEpoch {
        self.epoch
    }

    #[must_use]
    pub const fn evidence_revision(&self) -> u64 {
        self.evidence_revision
    }

    #[must_use]
    pub const fn publication_revision(&self) -> u64 {
        self.publication_revision
    }

    /// Monotonic SIM-session counter: advances when a SIM change is detected (a fingerprint
    /// difference) within the same USB device epoch; never inferred while the ICCID is unreadable
    /// (research document §4.3). Display-only.
    #[must_use]
    pub const fn sim_epoch(&self) -> u64 {
        self.sim_epoch
    }

    /// Current verdict for one probed module feature, for the anchored device context.
    #[must_use]
    pub fn feature_status(&self, key: FeatureKey) -> FeatureStatus {
        self.features
            .as_ref()
            .map_or(FeatureStatus::NotProbed, |capability| capability.get(key))
    }

    #[must_use]
    pub const fn current_cycle(&self) -> RefreshCycleId {
        self.current_cycle
    }

    #[must_use]
    pub const fn active_probe(&self) -> bool {
        self.active_probe
    }

    pub fn set_active_probe(&mut self, enabled: bool) {
        // Equality guard like the other setters: a no-op command must not bump the settings
        // revision (which would trigger an unnecessary config write).
        if self.settings.active_probe != enabled {
            self.active_probe = enabled;
            self.settings.active_probe = enabled;
            self.reset_active_probe_evidence(self.last_observed_at);
            self.settings.revision = self.settings.revision.saturating_add(1);
            self.publication_revision = self.publication_revision.saturating_add(1);
        }
    }

    /// A disabled probe is not a continuing positive observation. Re-enabling starts without
    /// reviving the previous proof, and in-flight results are ignored by `apply_probe` while off.
    fn reset_active_probe_evidence(&mut self, now: SystemTime) {
        self.bound_public = None;
        self.bound_dns = None;
        self.protocol_coverage = None;
        self.consecutive_public_failures = 0;
        if let Some(network) = self.network.as_mut() {
            network.bound_public = BoundPublicStatus::Incomplete;
            network.bound_dns = BoundDnsStatus::Incomplete;
            network.protocol_coverage = ProtocolCoverage::SingleFamilyOnly;
        }
        let reason = if self.active_probe {
            UnexecutedReason::NotScheduled
        } else {
            UnexecutedReason::DisabledBySetting
        };
        for id in [
            DiagnosticCheckId::BoundGateway,
            DiagnosticCheckId::BoundPublic,
            DiagnosticCheckId::BoundDns,
        ] {
            self.set_check_terminal(id, DiagnosticCheckState::Unexecuted { reason }, now);
        }
        self.evidence_revision = self.evidence_revision.saturating_add(1);
    }

    pub fn set_language(&mut self, language: LanguageCode) {
        if self.settings.language != language {
            self.settings.language = language;
            self.settings.revision = self.settings.revision.saturating_add(1);
            self.publication_revision = self.publication_revision.saturating_add(1);
        }
    }

    pub(crate) fn publish_only_change(&mut self) {
        self.publication_revision = self.publication_revision.saturating_add(1);
    }

    pub fn set_start_minimized(&mut self, value: bool) {
        if self.settings.start_minimized != value {
            self.settings.start_minimized = value;
            self.settings.revision = self.settings.revision.saturating_add(1);
            self.publication_revision = self.publication_revision.saturating_add(1);
        }
    }

    pub fn set_log_level(&mut self, value: LogLevel) {
        if self.settings.log_level != value {
            self.settings.log_level = value;
            self.settings.revision = self.settings.revision.saturating_add(1);
            self.publication_revision = self.publication_revision.saturating_add(1);
        }
    }

    pub fn set_autostart(&mut self, value: AutostartStatus) {
        self.settings.autostart = value;
        self.settings.revision = self.settings.revision.saturating_add(1);
        self.publication_revision = self.publication_revision.saturating_add(1);
    }

    /// Current settings revision. Bumped by every user-visible setting change; the panel uses it
    /// to decide when a `config.toml` write is due and to tag the resulting outcome.
    #[must_use]
    pub const fn settings_revision(&self) -> u64 {
        self.settings.revision
    }

    /// Begin a user-initiated autostart change: capture the last known registry state as
    /// `previous` so a failed apply can still be rendered against something, mark the settings
    /// write pending, and record the desired direction.
    ///
    /// This runs unconditionally (even when the desired value repeats): the registry write must
    /// still be attempted, e.g. re-enabling after drift.
    pub fn begin_autostart_change(&mut self, desired_enabled: bool) {
        let previous = match &self.settings.autostart {
            AutostartStatus::Ready(state) => Some(*state),
            AutostartStatus::Saving { previous, .. } | AutostartStatus::Failed { previous, .. } => {
                *previous
            }
            AutostartStatus::Loading => None,
        };
        self.settings.autostart = AutostartStatus::Saving {
            desired_enabled,
            previous,
        };
        self.settings.revision = self.settings.revision.saturating_add(1);
        self.settings.persistence = SettingsPersistenceState::Saving;
        self.publication_revision = self.publication_revision.saturating_add(1);
    }

    /// Apply the terminal result of a panel-driven autostart registration write.
    ///
    /// Only a state still saving the same desired value is transitioned; anything else is a stale
    /// outcome from a superseded request and is ignored. The settings revision deliberately does
    /// not change here: the config write for this toggle already happened while the state was
    /// `Saving`, and a registry readback is not a new setting value.
    pub fn finish_autostart(&mut self, outcome: AutostartApplyOutcome) {
        if let AutostartStatus::Saving {
            desired_enabled,
            previous,
        } = &self.settings.autostart
        {
            if *desired_enabled == outcome.desired_enabled {
                self.settings.autostart = match outcome.observed {
                    Ok(state) => AutostartStatus::Ready(state),
                    Err(code) => AutostartStatus::Failed {
                        code,
                        previous: *previous,
                    },
                };
                self.publication_revision = self.publication_revision.saturating_add(1);
            }
        }
    }

    /// Mark a settings write as pending. Startup seeding calls the low-level setters directly and
    /// stays `Clean`; only user-initiated commands imply a `config.toml` write.
    pub fn begin_settings_persistence(&mut self) {
        self.settings.persistence = SettingsPersistenceState::Saving;
        self.publication_revision = self.publication_revision.saturating_add(1);
    }

    /// Apply the terminal result of a panel-driven `config.toml` write for one settings revision.
    ///
    /// Outcomes for any other revision than the current one are stale (a newer change is already
    /// pending or the snapshot the panel saved from no longer exists) and are ignored.
    pub fn finish_settings_persistence(&mut self, revision: u64, result: Result<(), FailureCode>) {
        if revision != self.settings.revision {
            return;
        }
        self.settings.persistence = match result {
            Ok(()) => SettingsPersistenceState::Clean,
            Err(code) => SettingsPersistenceState::Failed { code },
        };
        self.publication_revision = self.publication_revision.saturating_add(1);
    }

    pub fn bump_evidence_for_test(&mut self) {
        self.evidence_revision = self.evidence_revision.saturating_add(1);
        self.publication_revision = self.publication_revision.saturating_add(1);
    }

    /// The AT port the inventory proved for the current device, if one was observed.
    pub(crate) fn inventory_at_port(&self) -> Option<String> {
        self.inventory_at_port.clone()
    }

    pub(crate) fn target_identity(&self) -> Option<StableDeviceIdentity> {
        self.target_identity
            .as_ref()
            .map(|value| value.value.clone())
    }

    pub(crate) fn adapter_id(&self) -> Option<String> {
        self.adapter_binding
            .as_ref()
            .map(|value| value.value.adapter_id.clone())
    }

    pub(crate) fn adapter_context(&self) -> Option<AdapterContext> {
        let binding = self.adapter_binding.as_ref()?.value.clone();
        let network = self.network.as_ref()?;
        Some(AdapterContext {
            epoch: self.epoch,
            binding,
            addresses: network.addresses.clone(),
        })
    }

    pub(crate) fn target_context(&self) -> Option<TargetContext> {
        let identity = self.target_identity()?;
        let mut target = TargetContext::new(
            self.epoch,
            identity,
            self.inventory_at_port.clone(),
            self.inventory_adapter_id
                .clone()
                .or_else(|| self.adapter_id()),
        )
        .ok()?;
        target.sim_fingerprint = self
            .cellular
            .as_ref()
            .and_then(|evidence| evidence.value.sim_identity.as_ref())
            .map(|identity| identity.fingerprint);
        Some(target)
    }

    pub(crate) fn action_prerequisite(
        &self,
        action: &dji4g_domain::ActionKind,
        now: SystemTime,
    ) -> Result<(), FailureCode> {
        self.target_identity
            .as_ref()
            .filter(|value| value.is_fresh_for(self.epoch, now))
            .map(|value| &value.value)
            .filter(|value| value.is_supported())
            .ok_or_else(|| failure(ErrorCodeForStage::Missing, "app:target_not_ready"))?;

        let check_ready = |id: DiagnosticCheckId| {
            matches!(self.diagnostics.get(id).state, DiagnosticCheckState::Passed)
        };
        match action {
            dji4g_domain::ActionKind::Refresh => {
                return Err(failure(
                    ErrorCodeForStage::Missing,
                    "app:refresh_not_action",
                ));
            }
            dji4g_domain::ActionKind::RenewDhcp
            | dji4g_domain::ActionKind::ApplyDnsProfile { .. }
            | dji4g_domain::ActionKind::RestartAdapter
            | dji4g_domain::ActionKind::SetVerifiedUsbNetworkProfile { .. } => {
                if !check_ready(DiagnosticCheckId::WindowsAdapter)
                    || self
                        .adapter_binding
                        .as_ref()
                        .is_none_or(|value| !value.is_fresh_for(self.epoch, now))
                    || self
                        .adapter
                        .as_ref()
                        .is_none_or(|value| !value.is_fresh_for(self.epoch, now))
                {
                    return Err(failure(ErrorCodeForStage::Missing, "app:adapter_not_ready"));
                }
            }
            dji4g_domain::ActionKind::RestartModule | dji4g_domain::ActionKind::EditApn { .. } => {
                if !check_ready(DiagnosticCheckId::AtControl)
                    || !matches!(
                        self.at_control.as_ref().map(|value| value.value),
                        Some(AtControlAvailability::Available)
                    )
                    || self
                        .at_control
                        .as_ref()
                        .is_none_or(|value| !value.is_fresh_for(self.epoch, now))
                {
                    return Err(failure(ErrorCodeForStage::Missing, "app:at_not_ready"));
                }
            }
            dji4g_domain::ActionKind::ReenumerateDevice => {}
            dji4g_domain::ActionKind::ToggleHotspot { .. } => {
                if !check_ready(DiagnosticCheckId::WindowsAdapter)
                    || self
                        .adapter_binding
                        .as_ref()
                        .is_none_or(|value| !value.is_fresh_for(self.epoch, now))
                    || self
                        .adapter
                        .as_ref()
                        .is_none_or(|value| !value.is_fresh_for(self.epoch, now))
                    || matches!(self.hotspot, HotspotStatus::Unsupported(_))
                {
                    return Err(failure(ErrorCodeForStage::Missing, "app:hotspot_not_ready"));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn network_snapshot(&self) -> Option<NetworkSnapshot> {
        self.network.clone()
    }

    pub(crate) fn hotspot_status(&self) -> HotspotStatus {
        self.hotspot
    }

    #[must_use]
    pub fn snapshot(&self) -> ControllerSnapshot {
        self.snapshot_with(None, None)
    }

    pub(crate) fn snapshot_with(
        &self,
        prepared_action: Option<crate::PreparedActionSnapshot>,
        operation: Option<crate::OperationUiSnapshot>,
    ) -> ControllerSnapshot {
        self.snapshot_with_at(prepared_action, operation, self.last_observed_at)
    }

    pub(crate) fn snapshot_with_at(
        &self,
        prepared_action: Option<crate::PreparedActionSnapshot>,
        operation: Option<crate::OperationUiSnapshot>,
        now: SystemTime,
    ) -> ControllerSnapshot {
        ControllerSnapshot {
            host_network: crate::HostNetworkSnapshot::default(),
            publication_revision: self.publication_revision,
            app: Arc::new(self.app_snapshot(now)),
            diagnostics: self.diagnostics.clone(),
            prepared_action,
            operation,
            settings: self.settings.clone(),
            command_state: CommandStateSnapshot::default(),
            action_readiness: self.action_readiness(now),
            feedback: None,
            sim_epoch: self.sim_epoch,
            feature_status: self.features.clone(),
            adapter_metrics: self.adapter_metrics,
            timeline: self.timeline.clone(),
            sms_inbox: self.sms_store.summary(self.sms_status, self.sms_capacity),
            sms_messages: self.sms_store.display_messages(),
            sms_delete: None,
            serial_work_busy: false,
            sms_send: None,
            sms_refresh_pending: false,
            sms_read_phase: None,
            sms_read_progress: 0,
            sms_read_report: None,
            sms_inbox_failure: None,
            device_tools: crate::DeviceToolsSnapshot::default(),
        }
    }

    /// Readiness verdict for every modeled action, keyed by its parameterless kind. This is the
    /// same gate `prepare_action` runs, so a button the UI enables can never be rejected for a
    /// missing prerequisite at prepare time.
    pub(crate) fn action_readiness(&self, now: SystemTime) -> Vec<ActionReadiness> {
        use dji4g_domain::{DnsProfile, UsbNetworkProfile};
        let actions = [
            dji4g_domain::ActionKind::RenewDhcp,
            dji4g_domain::ActionKind::ApplyDnsProfile {
                profile: DnsProfile::Automatic,
            },
            dji4g_domain::ActionKind::RestartAdapter,
            dji4g_domain::ActionKind::ReenumerateDevice,
            dji4g_domain::ActionKind::RestartModule,
            dji4g_domain::ActionKind::EditApn {
                cid: 1,
                apn: String::new(),
            },
            dji4g_domain::ActionKind::SetVerifiedUsbNetworkProfile {
                profile: UsbNetworkProfile::DjiNdis,
            },
            dji4g_domain::ActionKind::ToggleHotspot { enabled: false },
            dji4g_domain::ActionKind::ToggleHotspot { enabled: true },
        ];
        let mut seen = std::collections::HashSet::new();
        let mut readiness = actions
            .into_iter()
            .filter_map(|action| {
                let key = readiness_key(&action)?;
                if !seen.insert(key) {
                    return None;
                }
                Some(ActionReadiness {
                    key,
                    ready: self.action_prerequisite(&action, now),
                })
            })
            .collect::<Vec<_>>();
        readiness.sort_by_key(|entry| entry.key as u8);
        readiness
    }

    #[must_use]
    pub fn test_ready(now: SystemTime) -> Self {
        let mut state = Self::new(now);
        let epoch = DeviceEpoch(1);
        let identity = test_identity();
        let binding = AdapterBinding {
            target: identity.clone(),
            adapter_id: "{adapter}".into(),
        };
        state.epoch = epoch;
        state.phase = ClassificationPhase::Stable;
        state.device_presence = Some(evidence(
            epoch,
            EvidenceSource::Pnp,
            DevicePresence::Supported(dji4g_domain::DJI_GEN1),
            now,
        ));
        state.target_identity = Some(evidence(epoch, EvidenceSource::Pnp, identity, now));
        state.adapter_binding = Some(evidence(
            epoch,
            EvidenceSource::WindowsAdapter,
            binding.clone(),
            now,
        ));
        state.inventory_at_port = Some("COM9".into());
        state.inventory_adapter_id = Some("{adapter}".into());
        state.adapter = Some(evidence(
            epoch,
            EvidenceSource::WindowsAdapter,
            AdapterState::UsableAddressAndRoute,
            now,
        ));
        state.at_control = Some(evidence(
            epoch,
            EvidenceSource::AtControl,
            AtControlAvailability::Available,
            now,
        ));
        let bound = BoundEvidence {
            binding: binding.clone(),
            value: BoundPublicStatus::Succeeded,
        };
        state.bound_public = Some(evidence(
            epoch,
            EvidenceSource::BoundPublicProbe,
            bound,
            now,
        ));
        let dns = BoundEvidence {
            binding: binding.clone(),
            value: BoundDnsStatus::Succeeded,
        };
        state.bound_dns = Some(evidence(epoch, EvidenceSource::BoundDnsProbe, dns, now));
        let coverage = BoundEvidence {
            binding,
            value: ProtocolCoverage::AllRequiredFamilies,
        };
        state.protocol_coverage = Some(evidence(
            epoch,
            EvidenceSource::BoundPublicProbe,
            coverage,
            now,
        ));
        state.system_default_route = Some(evidence(
            epoch,
            EvidenceSource::GlobalRoute,
            DefaultRouteOwner::TargetAdapter,
            now,
        ));
        state.network = Some(NetworkSnapshot {
            adapter_id: "{adapter}".into(),
            addresses: vec!["192.168.225.30".into()],
            gateways: vec!["192.168.225.1".into()],
            dns_servers: vec!["192.168.225.1".into()],
            adapter_state: AdapterState::UsableAddressAndRoute,
            bound_public: BoundPublicStatus::Succeeded,
            bound_dns: BoundDnsStatus::Succeeded,
            protocol_coverage: ProtocolCoverage::AllRequiredFamilies,
            system_default_route: DefaultRouteOwner::TargetAdapter,
            down_bytes_per_sec: None,
            up_bytes_per_sec: None,
        });
        state.diagnostics = DiagnosticSet::new(epoch);
        for id in [
            DiagnosticCheckId::UsbDevice,
            DiagnosticCheckId::AtControl,
            DiagnosticCheckId::Cellular,
            DiagnosticCheckId::WindowsAdapter,
            DiagnosticCheckId::BoundGateway,
            DiagnosticCheckId::BoundPublic,
            DiagnosticCheckId::BoundDns,
            DiagnosticCheckId::SystemRoute,
        ] {
            state.set_check_terminal(id, DiagnosticCheckState::Passed, now);
        }
        state.hotspot = HotspotStatus::Off;
        state.set_check_terminal(
            DiagnosticCheckId::Hotspot,
            DiagnosticCheckState::Passed,
            now,
        );
        state.last_observed_at = now;
        state
    }

    fn app_snapshot(&self, now: SystemTime) -> AppSnapshot {
        let input = ClassificationInput {
            current_epoch: self.epoch,
            phase: self.phase,
            device_presence: self.device_presence.clone(),
            target_identity: self.target_identity.clone(),
            cellular_block: self.cellular_block.clone(),
            adapter_binding: self.adapter_binding.clone(),
            adapter: self.adapter.clone(),
            bound_public: self.bound_public.clone(),
            bound_dns: self.bound_dns.clone(),
            protocol_coverage: self.protocol_coverage.clone(),
            at_control: self.at_control.clone(),
            system_default_route: self.system_default_route.clone(),
            global_connectivity: self.global_connectivity.clone(),
        };
        let availability = classify(&input, now).status;
        let freshness =
            if self.device_presence.is_none() || self.phase != ClassificationPhase::Stable {
                Freshness::Unknown
            } else if has_expired_evidence(&input, now) {
                Freshness::Stale
            } else {
                Freshness::Fresh
            };

        let device = self.target_identity.as_ref().and_then(|identity| {
            self.device_presence
                .as_ref()
                .and_then(|presence| match presence.value {
                    DevicePresence::Supported(_) => Some(DeviceSnapshot {
                        epoch: self.epoch,
                        identity: identity.value.clone(),
                        problem_code: self.inventory_problem_code,
                        at_port: self.inventory_at_port.clone(),
                        adapter_id: self.inventory_adapter_id.clone().or_else(|| {
                            self.adapter_binding
                                .as_ref()
                                .map(|value| value.value.adapter_id.clone())
                        }),
                    }),
                    _ => None,
                })
        });
        AppSnapshot {
            revision: self.evidence_revision,
            observed_at: self.last_observed_at,
            freshness,
            availability,
            hotspot: self.hotspot,
            device,
            cellular: self.cellular.as_ref().map(|value| value.value.clone()),
            network: self.network.clone(),
            active_operation: None,
            issues: self.issues(),
        }
    }

    fn issues(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        if let Some(check) = self
            .diagnostics
            .iter()
            .find(|check| check.id == DiagnosticCheckId::BoundDns)
        {
            if let DiagnosticCheckState::Failed { code } = &check.state {
                issues.push(Issue {
                    code: code.category,
                    severity: IssueSeverity::Warning,
                    layer: IssueLayer::BoundProbe,
                });
            }
        }
        if let Some(check) = self
            .diagnostics
            .iter()
            .find(|check| check.id == DiagnosticCheckId::AtControl)
        {
            if let DiagnosticCheckState::Unavailable { code }
            | DiagnosticCheckState::Failed { code } = &check.state
            {
                issues.push(Issue {
                    code: code.category,
                    severity: IssueSeverity::Warning,
                    layer: IssueLayer::Cellular,
                });
            }
        }
        issues
    }

    fn set_check_running(
        &mut self,
        id: DiagnosticCheckId,
        cycle: RefreshCycleId,
        started_at: SystemTime,
    ) {
        let check = self.diagnostics.get_mut(id);
        check.state = DiagnosticCheckState::Running { cycle };
        check.epoch = self.epoch;
        check.started_at = Some(started_at);
        check.finished_at = None;
        check.observed_at = None;
        check.expires_at = None;
    }

    fn set_check_terminal(
        &mut self,
        id: DiagnosticCheckId,
        state: DiagnosticCheckState,
        observed_at: SystemTime,
    ) {
        let check = self.diagnostics.get_mut(id);
        check.state = state;
        check.epoch = self.epoch;
        check.finished_at = Some(observed_at);
        check.observed_at = Some(observed_at);
        check.expires_at = Some(observed_at + EVIDENCE_TTL);
    }

    fn accept_epoch_event(&mut self, epoch: DeviceEpoch, now: SystemTime) -> bool {
        if self.epoch == DeviceEpoch(0) && epoch.0 > 0 {
            self.epoch = epoch;
            self.diagnostics.reset(epoch);
            return true;
        }
        if epoch.0 > self.epoch.0 {
            // A strictly newer epoch must always be adopted: the inventory port re-enumerates
            // after a transient empty scan, and dropping that event would strand the panel on
            // evidence from the previous epoch until it ages past its TTL forever. A newer epoch
            // is also the reducer's only evidence that the module re-enumerated, so the timeline
            // records the reappearance here exactly once.
            self.device_removed = false;
            self.record_timeline(now, TimelineEventKind::DeviceArrived, "设备已重新枚举");
            self.invalidate(epoch, EpochInvalidationReason::Advanced, now);
            return true;
        }
        epoch == self.epoch
    }

    fn accept_cycle(&mut self, cycle: RefreshCycleId) -> bool {
        if cycle < self.current_cycle {
            return false;
        }
        if cycle > self.current_cycle {
            self.current_cycle = cycle;
        }
        true
    }

    fn accept_observed(&self, observed_at: SystemTime, now: SystemTime) -> bool {
        observed_at <= now
    }

    fn mark_evidence_changed(&mut self, observed_at: SystemTime) {
        self.last_observed_at = observed_at;
        self.evidence_revision = self.evidence_revision.saturating_add(1);
        self.publication_revision = self.publication_revision.saturating_add(1);
    }

    fn invalidate(
        &mut self,
        next_epoch: DeviceEpoch,
        _reason: EpochInvalidationReason,
        now: SystemTime,
    ) {
        self.epoch = next_epoch;
        self.current_cycle = RefreshCycleId(0);
        self.phase = ClassificationPhase::RecentInsertion;
        self.device_presence = None;
        self.target_identity = None;
        self.cellular = None;
        self.cellular_block = None;
        self.at_control = None;
        self.adapter_binding = None;
        self.inventory_at_port = None;
        self.inventory_adapter_id = None;
        self.inventory_problem_code = None;
        self.adapter = None;
        self.network = None;
        self.bound_public = None;
        self.bound_dns = None;
        self.protocol_coverage = None;
        self.system_default_route = None;
        self.global_connectivity = None;
        self.hotspot = HotspotStatus::Unsupported(
            dji4g_domain::HotspotUnsupportedReason::SourceProfileUnavailable,
        );
        self.consecutive_public_failures = 0;
        self.rate_state = None;
        self.adapter_metrics = None;
        // The device context is gone: cached capability verdicts may no longer apply to whatever
        // is enumerated next and are dropped. SIM continuity fields are deliberately retained so a
        // card swapped during the re-enumeration is still detected by a fingerprint change.
        self.features = None;
        // Messages belong to the lost device/SIM session and are dropped with it.
        self.sms_store.clear();
        self.sms_status = FeatureStatus::NotProbed;
        self.sms_capacity = None;
        self.last_observed_at = now;
        self.diagnostics.reset(next_epoch);
        self.evidence_revision = self.evidence_revision.saturating_add(1);
        self.publication_revision = self.publication_revision.saturating_add(1);
    }
}

#[must_use]
pub fn reduce_state(previous: &ReducerState, event: BackendEvent, now: SystemTime) -> ReducerState {
    let mut next = previous.clone();
    match event {
        BackendEvent::EpochInvalidated { next_epoch, reason } => {
            if next_epoch > next.epoch {
                if reason == EpochInvalidationReason::PhysicalRemoval {
                    next.device_removed = true;
                    next.record_timeline(now, TimelineEventKind::DeviceRemoved, "设备已断开");
                }
                next.invalidate(next_epoch, reason, now);
            }
        }
        BackendEvent::RefreshStarted {
            cycle,
            epoch,
            scheduled,
        } => {
            if next.accept_epoch_event(epoch, now) && next.accept_cycle(cycle) {
                for id in DiagnosticCheckId::ORDERED {
                    if scheduled.contains(id) {
                        next.set_check_running(id, cycle, now);
                    }
                }
                if next.device_presence.is_none() || next.phase != ClassificationPhase::Stable {
                    next.phase = ClassificationPhase::RecentInsertion;
                }
                next.publication_revision = next.publication_revision.saturating_add(1);
            }
        }
        BackendEvent::InventoryFinished {
            cycle,
            epoch,
            result,
        } => {
            if !next.accept_epoch_event(epoch, now) || !next.accept_cycle(cycle) {
                return next;
            }
            match result {
                CheckResult::Passed { value, observed_at }
                    if value.epoch == epoch && next.accept_observed(observed_at, now) =>
                {
                    next.apply_inventory(value, observed_at);
                    next.mark_evidence_changed(observed_at);
                }
                CheckResult::Failed { code, observed_at }
                    if next.accept_observed(observed_at, now) =>
                {
                    next.device_presence = Some(evidence(
                        epoch,
                        EvidenceSource::Pnp,
                        DevicePresence::PermissionDenied,
                        observed_at,
                    ));
                    next.set_check_terminal(
                        DiagnosticCheckId::UsbDevice,
                        DiagnosticCheckState::Failed { code },
                        observed_at,
                    );
                    next.mark_evidence_changed(observed_at);
                }
                CheckResult::Unavailable { code, observed_at }
                    if next.accept_observed(observed_at, now) =>
                {
                    next.set_check_terminal(
                        DiagnosticCheckId::UsbDevice,
                        DiagnosticCheckState::Unavailable { code },
                        observed_at,
                    );
                    next.mark_evidence_changed(observed_at);
                }
                CheckResult::Unexecuted { reason } => {
                    next.set_check_terminal(
                        DiagnosticCheckId::UsbDevice,
                        DiagnosticCheckState::Unexecuted { reason },
                        now,
                    );
                    next.publication_revision = next.publication_revision.saturating_add(1);
                }
                _ => {}
            }
        }
        BackendEvent::AtFinished {
            cycle,
            epoch,
            result,
        } => {
            if !next.accept_epoch_event(epoch, now) || !next.accept_cycle(cycle) {
                return next;
            }
            let previous_registration = next
                .cellular
                .as_ref()
                .map(|evidence| evidence.value.registration);
            let previous_cell = next
                .cellular
                .as_ref()
                .and_then(|evidence| evidence.value.serving_cell.as_ref())
                .map(serving_cell_identity);
            next.apply_at(result, epoch, now);
            let mut registration_detail = None;
            let mut cell_changed = false;
            if let Some(cellular) = next.cellular.as_ref().map(|evidence| &evidence.value) {
                if let Some(previous) = previous_registration {
                    if previous != cellular.registration {
                        registration_detail = Some(format!(
                            "注册状态：{} → {}",
                            registration_label(previous),
                            registration_label(cellular.registration)
                        ));
                    }
                }
                if let Some(current_cell) = cellular.serving_cell.as_ref() {
                    if let Some(previous) = previous_cell.as_ref() {
                        if *previous != serving_cell_identity(current_cell) {
                            cell_changed = true;
                        }
                    }
                }
            }
            if let Some(detail) = registration_detail {
                next.record_timeline(now, TimelineEventKind::RegistrationChanged, detail);
            }
            if cell_changed {
                next.record_timeline(now, TimelineEventKind::CellChanged, "服务小区已变化");
            }
            next.observe_sim_identity(now);
            next.sync_feature_context();
        }
        BackendEvent::AdapterFinished {
            cycle,
            epoch,
            result,
        } => {
            if !next.accept_epoch_event(epoch, now) || !next.accept_cycle(cycle) {
                return next;
            }
            let previous_adapter = next.adapter.as_ref().map(|evidence| evidence.value);
            next.apply_adapter(result, epoch, now);
            let current_adapter = next.adapter.as_ref().map(|evidence| evidence.value);
            if previous_adapter.is_some()
                && current_adapter.is_some()
                && previous_adapter != current_adapter
            {
                next.record_timeline(
                    now,
                    TimelineEventKind::AdapterLinkChanged,
                    "网卡链路状态变化",
                );
            }
        }
        BackendEvent::ProbeFinished {
            cycle,
            epoch,
            result,
        } => {
            if !next.accept_epoch_event(epoch, now) || !next.accept_cycle(cycle) {
                return next;
            }
            let previous_dns =
                dns_status_from_check(&next.diagnostics.get(DiagnosticCheckId::BoundDns).state);
            next.apply_probe(result, epoch, now);
            let current_dns =
                dns_status_from_check(&next.diagnostics.get(DiagnosticCheckId::BoundDns).state);
            if let (Some(previous), Some(current)) = (previous_dns, current_dns) {
                if previous != current {
                    next.record_timeline(
                        now,
                        TimelineEventKind::DnsChanged,
                        format!("DNS 探测：{} → {}", dns_label(previous), dns_label(current)),
                    );
                }
            }
        }
        BackendEvent::HotspotFinished {
            cycle,
            epoch,
            result,
        } => {
            if !next.accept_epoch_event(epoch, now) || !next.accept_cycle(cycle) {
                return next;
            }
            next.apply_hotspot(result, epoch, now);
        }
        BackendEvent::RefreshFinished { cycle, epoch } => {
            if !next.accept_epoch_event(epoch, now) || !next.accept_cycle(cycle) {
                return next;
            }
            let mut changed = false;
            for check in &mut next.diagnostics.0 {
                if matches!(check.state, DiagnosticCheckState::Running { cycle: active } if active == cycle)
                {
                    let code = failure(ErrorCodeForStage::Missing, "app:stage_missing");
                    check.state = DiagnosticCheckState::Failed { code };
                    check.finished_at = Some(now);
                    check.observed_at = Some(now);
                    check.expires_at = Some(now + EVIDENCE_TTL);
                    changed = true;
                }
            }
            next.phase = ClassificationPhase::Stable;
            if changed {
                next.evidence_revision = next.evidence_revision.saturating_add(1);
                next.publication_revision = next.publication_revision.saturating_add(1);
            }
        }
        BackendEvent::SettingsChanged { settings } => {
            let probe_changed = next.active_probe != settings.active_probe;
            next.settings = settings;
            next.active_probe = next.settings.active_probe;
            if probe_changed {
                next.reset_active_probe_evidence(now);
            }
            next.publication_revision = next.publication_revision.saturating_add(1);
        }
        BackendEvent::ExpirationTick => {
            let mut changed = false;
            for check in &mut next.diagnostics.0 {
                if matches!(
                    check.state,
                    DiagnosticCheckState::Passed
                        | DiagnosticCheckState::Failed { .. }
                        | DiagnosticCheckState::Unavailable { .. }
                ) && check.expires_at.is_some_and(|expiry| now >= expiry)
                {
                    check.state = DiagnosticCheckState::Expired;
                    changed = true;
                }
            }
            if has_expired_evidence(
                &ClassificationInput {
                    current_epoch: next.epoch,
                    phase: next.phase,
                    device_presence: next.device_presence.clone(),
                    target_identity: next.target_identity.clone(),
                    cellular_block: next.cellular_block.clone(),
                    adapter_binding: next.adapter_binding.clone(),
                    adapter: next.adapter.clone(),
                    bound_public: next.bound_public.clone(),
                    bound_dns: next.bound_dns.clone(),
                    protocol_coverage: next.protocol_coverage.clone(),
                    at_control: next.at_control.clone(),
                    system_default_route: next.system_default_route.clone(),
                    global_connectivity: next.global_connectivity.clone(),
                },
                now,
            ) && next.phase != ClassificationPhase::Stable
            {
                next.phase = ClassificationPhase::Stable;
                changed = true;
            }
            if changed {
                next.evidence_revision = next.evidence_revision.saturating_add(1);
                next.publication_revision = next.publication_revision.saturating_add(1);
            }
        }
        BackendEvent::RatesSampled {
            adapter_id,
            epoch,
            rx,
            tx,
            sampled_at,
        } => {
            // Rates-only tick. Recompute throughput from the bound adapter's freshly read counters
            // using the exact same baseline/regression logic as a full refresh, then republish.
            // Only `down/up_bytes_per_sec` may change: `publish_only_change` bumps the publication
            // revision alone, so `observed_at`, `evidence_revision`, freshness, the diagnostic
            // checks, and availability all stay exactly as the last evidence refresh left them.
            // The guards reject a sample for any adapter/epoch other than the currently bound one,
            // so a stale or mismatched read can never move the numbers.
            if epoch == next.epoch
                && next.accept_observed(sampled_at, now)
                && next
                    .network
                    .as_ref()
                    .is_some_and(|network| network.adapter_id == adapter_id)
            {
                let (down_bytes_per_sec, up_bytes_per_sec) = ReducerState::measured_rates(
                    &mut next.rate_state,
                    &adapter_id,
                    epoch,
                    rx,
                    tx,
                    sampled_at,
                );
                if let Some(network) = next.network.as_mut() {
                    network.down_bytes_per_sec = down_bytes_per_sec;
                    network.up_bytes_per_sec = up_bytes_per_sec;
                }
                next.publish_only_change();
            }
        }
        BackendEvent::AdapterMetricsSampled {
            adapter_id,
            epoch,
            metrics,
            sampled_at,
        } => {
            // Publication-only sample keyed to the currently bound adapter/epoch, mirroring the
            // rates tick: a mismatched adapter, a stale epoch, or a future timestamp cannot move
            // the snapshot, and `None` records an honest gap instead of keeping a stale sample.
            if epoch == next.epoch
                && next.accept_observed(sampled_at, now)
                && next
                    .network
                    .as_ref()
                    .is_some_and(|network| network.adapter_id == adapter_id)
            {
                next.adapter_metrics = metrics;
                next.publish_only_change();
            }
        }
    }
    next
}

impl ReducerState {
    fn apply_inventory(&mut self, value: InventoryObservation, observed_at: SystemTime) {
        let was_present = matches!(
            self.device_presence
                .as_ref()
                .map(|evidence| &evidence.value),
            Some(DevicePresence::Supported(_))
        );
        self.last_observed_at = observed_at;
        self.inventory_at_port = value.at_port.clone();
        self.inventory_adapter_id = value.adapter_id.clone();
        self.inventory_problem_code = value.problem_code;
        self.device_presence = Some(evidence(
            self.epoch,
            EvidenceSource::Pnp,
            value.presence.clone(),
            observed_at,
        ));
        let is_absent = matches!(value.presence, DevicePresence::NotDetected);
        match value.presence {
            DevicePresence::Supported(_) => {
                if self.device_removed {
                    self.device_removed = false;
                    self.record_timeline(
                        observed_at,
                        TimelineEventKind::DeviceArrived,
                        "设备已重新枚举",
                    );
                }
                if self.phase != ClassificationPhase::RecentInsertion {
                    self.phase = ClassificationPhase::Stable;
                }
                if let Some(identity) = value.identity {
                    self.target_identity = Some(evidence(
                        self.epoch,
                        EvidenceSource::Pnp,
                        identity.clone(),
                        observed_at,
                    ));
                    if let Some(adapter_id) = self.inventory_adapter_id.clone() {
                        self.adapter_binding = Some(evidence(
                            self.epoch,
                            EvidenceSource::WindowsAdapter,
                            AdapterBinding {
                                target: identity,
                                adapter_id,
                            },
                            observed_at,
                        ));
                    } else {
                        self.adapter_binding = None;
                        self.adapter = None;
                        self.network = None;
                        self.bound_public = None;
                        self.bound_dns = None;
                        self.protocol_coverage = None;
                    }
                } else {
                    self.target_identity = None;
                    self.adapter_binding = None;
                    self.adapter = None;
                    self.network = None;
                    self.bound_public = None;
                    self.bound_dns = None;
                    self.protocol_coverage = None;
                }
                self.diagnostics.set_terminal_from_result(
                    DiagnosticCheckId::UsbDevice,
                    true,
                    None,
                    observed_at,
                    self.epoch,
                );
            }
            DevicePresence::NotDetected | DevicePresence::Unsupported { .. } => {
                if is_absent && was_present {
                    self.device_removed = true;
                    self.record_timeline(
                        observed_at,
                        TimelineEventKind::DeviceRemoved,
                        "设备已断开",
                    );
                }
                self.phase = ClassificationPhase::Stable;
                self.target_identity = None;
                self.adapter_binding = None;
                self.inventory_at_port = None;
                self.inventory_adapter_id = None;
                self.adapter = None;
                self.network = None;
                self.bound_public = None;
                self.bound_dns = None;
                self.protocol_coverage = None;
                let code = if is_absent {
                    failure(ErrorCodeForStage::Absent, "app:target_absent")
                } else {
                    failure(ErrorCodeForStage::Unsupported, "pnp:unsupported_device")
                };
                self.diagnostics.set_terminal_from_result(
                    DiagnosticCheckId::UsbDevice,
                    true,
                    None,
                    observed_at,
                    self.epoch,
                );
                for id in [
                    DiagnosticCheckId::AtControl,
                    DiagnosticCheckId::Cellular,
                    DiagnosticCheckId::WindowsAdapter,
                    DiagnosticCheckId::BoundGateway,
                    DiagnosticCheckId::BoundPublic,
                    DiagnosticCheckId::BoundDns,
                    DiagnosticCheckId::SystemRoute,
                    DiagnosticCheckId::Hotspot,
                ] {
                    self.set_check_terminal(
                        id,
                        DiagnosticCheckState::Unavailable { code: code.clone() },
                        observed_at,
                    );
                }
            }
            DevicePresence::PermissionDenied => {
                self.phase = ClassificationPhase::Startup;
                self.target_identity = None;
                self.adapter_binding = None;
                self.inventory_at_port = None;
                self.inventory_adapter_id = None;
                self.adapter = None;
                self.network = None;
                self.bound_public = None;
                self.bound_dns = None;
                self.protocol_coverage = None;
                self.diagnostics.set_terminal_from_result(
                    DiagnosticCheckId::UsbDevice,
                    false,
                    Some(failure(
                        ErrorCodeForStage::Permission,
                        "pnp:permission_denied",
                    )),
                    observed_at,
                    self.epoch,
                );
                let code = failure(ErrorCodeForStage::Permission, "pnp:permission_denied");
                for id in [
                    DiagnosticCheckId::AtControl,
                    DiagnosticCheckId::Cellular,
                    DiagnosticCheckId::WindowsAdapter,
                    DiagnosticCheckId::BoundGateway,
                    DiagnosticCheckId::BoundPublic,
                    DiagnosticCheckId::BoundDns,
                    DiagnosticCheckId::SystemRoute,
                    DiagnosticCheckId::Hotspot,
                ] {
                    self.set_check_terminal(
                        id,
                        DiagnosticCheckState::Unavailable { code: code.clone() },
                        observed_at,
                    );
                }
            }
        }
        self.sync_feature_context();
    }

    fn apply_at(
        &mut self,
        result: CheckResult<AtObservation>,
        epoch: DeviceEpoch,
        now: SystemTime,
    ) {
        match result {
            CheckResult::Passed { value, observed_at }
                if self.accept_observed(observed_at, now) =>
            {
                self.at_control = Some(evidence(
                    epoch,
                    EvidenceSource::AtControl,
                    value.availability,
                    observed_at,
                ));
                self.cellular = value
                    .cellular
                    .map(|value| evidence(epoch, EvidenceSource::AtControl, value, observed_at));
                let (cellular_check, block) =
                    cellular_verdict(self.cellular.as_ref().map(|entry| &entry.value));
                self.cellular_block = block
                    .map(|value| evidence(epoch, EvidenceSource::AtControl, value, observed_at));
                self.set_check_terminal(
                    DiagnosticCheckId::AtControl,
                    DiagnosticCheckState::Passed,
                    observed_at,
                );
                self.set_check_terminal(DiagnosticCheckId::Cellular, cellular_check, observed_at);
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Failed { code, observed_at } if self.accept_observed(observed_at, now) => {
                self.at_control = Some(evidence(
                    epoch,
                    EvidenceSource::AtControl,
                    AtControlAvailability::Unavailable,
                    observed_at,
                ));
                self.set_check_terminal(
                    DiagnosticCheckId::AtControl,
                    DiagnosticCheckState::Failed { code: code.clone() },
                    observed_at,
                );
                self.set_check_terminal(
                    DiagnosticCheckId::Cellular,
                    DiagnosticCheckState::Unavailable { code },
                    observed_at,
                );
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Unavailable { code, observed_at }
                if self.accept_observed(observed_at, now) =>
            {
                self.at_control = Some(evidence(
                    epoch,
                    EvidenceSource::AtControl,
                    AtControlAvailability::Unavailable,
                    observed_at,
                ));
                self.set_check_terminal(
                    DiagnosticCheckId::AtControl,
                    DiagnosticCheckState::Unavailable { code: code.clone() },
                    observed_at,
                );
                self.set_check_terminal(
                    DiagnosticCheckId::Cellular,
                    DiagnosticCheckState::Unavailable { code },
                    observed_at,
                );
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Unexecuted { reason } => {
                self.set_check_terminal(
                    DiagnosticCheckId::AtControl,
                    DiagnosticCheckState::Unexecuted { reason },
                    now,
                );
                self.set_check_terminal(
                    DiagnosticCheckId::Cellular,
                    DiagnosticCheckState::Unexecuted { reason },
                    now,
                );
                self.publication_revision = self.publication_revision.saturating_add(1);
            }
            _ => {}
        }
    }

    /// Compare the freshly stored cellular SIM identity against the last known fingerprint and
    /// advance `sim_epoch` when the card changed (research document §4.3).
    ///
    /// Conservative semantics: an unreadable identity (`sim_identity == None`) never advances the
    /// epoch — when continuity cannot be confirmed, old-card data must not keep being treated as
    /// current (阶段 C renders such stale data as `None`/unknown on the UI side instead of deriving
    /// an epoch). The first readable identity merely anchors `sim_fingerprint`; only a *change*
    /// from a known fingerprint advances the epoch.
    fn observe_sim_identity(&mut self, now: SystemTime) -> bool {
        let Some(fingerprint) = self
            .cellular
            .as_ref()
            .and_then(|evidence| evidence.value.sim_identity.as_ref())
            .map(|identity| identity.fingerprint)
        else {
            return false;
        };
        match self.sim_fingerprint {
            Some(stored) if stored == fingerprint => false,
            None => {
                self.sim_fingerprint = Some(fingerprint);
                false
            }
            Some(_) => {
                self.sim_fingerprint = Some(fingerprint);
                self.sim_epoch = self.sim_epoch.saturating_add(1);
                self.sms_store.clear();
                self.record_timeline(now, TimelineEventKind::SimChanged, "SIM 已更换");
                true
            }
        }
    }

    /// The device profile (型号 + USB 组合) currently bound, preferring the PnP-verified profile
    /// and falling back to the validated identity's VID/PID.
    fn device_profile(&self) -> Option<DeviceProfile> {
        self.device_presence
            .as_ref()
            .and_then(|presence| match presence.value {
                DevicePresence::Supported(profile) => Some(profile),
                _ => None,
            })
            .or_else(|| {
                self.target_identity.as_ref().map(|identity| DeviceProfile {
                    vid: identity.value.vid,
                    pid: identity.value.pid,
                })
            })
    }

    /// The capability context (profile + firmware) implied by the current evidence, if any.
    fn current_feature_context(&self) -> Option<(DeviceProfile, Option<String>)> {
        let profile = self.device_profile()?;
        let firmware = self
            .cellular
            .as_ref()
            .and_then(|evidence| evidence.value.firmware.clone());
        Some((profile, firmware))
    }

    /// Re-anchor the module capability container to the current device context.  A different USB
    /// device replaces the container outright; a firmware change (including the first firmware
    /// observation) clears the cached verdicts back to `NotProbed` (§8.1: 更换固件时失效模块缓存).
    fn sync_feature_context(&mut self) {
        let Some((profile, firmware)) = self.current_feature_context() else {
            self.features = None;
            return;
        };
        match self.features.as_mut() {
            None => self.features = Some(FeatureCapability::new(profile, firmware)),
            Some(capability) => {
                if capability.profile() != profile {
                    self.features = Some(FeatureCapability::new(profile, firmware));
                } else {
                    capability.adopt_firmware(firmware);
                }
            }
        }
    }

    /// Record one module-feature verdict from the collection layer, scoped to the current device
    /// context.  A firmware change detected in the latest AT evidence clears the cached verdicts
    /// before the new one is recorded.
    ///
    /// Returns `true` when the verdict was newly recorded (the caller should republish); `false`
    /// when it repeated the current value or no device context is anchored yet.
    pub fn set_feature_status(&mut self, key: FeatureKey, status: FeatureStatus) -> bool {
        self.sync_feature_context();
        let Some(capability) = self.features.as_mut() else {
            return false;
        };
        if capability.get(key) == status {
            return false;
        }
        capability.set(key, status);
        true
    }

    /// Read-only access to the deduplicated SMS store.
    #[must_use]
    pub fn sms_store(&self) -> &SmsStore {
        &self.sms_store
    }

    /// Store one listed SMS message; returns whether it was new. A new message republishes.
    pub fn reconcile_sms_listed_slots(&mut self, listed: &[SmsMessage]) {
        if self.sms_store.reconcile_listed_slots(listed) {
            self.publish_only_change();
        }
    }

    pub fn ingest_sms(&mut self, message: SmsMessage) -> bool {
        if self.sms_store.ingest(message) {
            self.publish_only_change();
            true
        } else {
            false
        }
    }

    /// Mark a stored SMS message read once the module-side read succeeded; republishes on change.
    pub fn mark_sms_read(&mut self, index: u32, storage: &SmsStorageId) -> bool {
        if self.sms_store.mark_read(index, storage) {
            self.publish_only_change();
            true
        } else {
            false
        }
    }

    /// Remove every stored message carrying this index (panel-side delete bookkeeping).
    pub fn remove_sms_by_index(&mut self, index: u32) -> bool {
        if self.sms_store.remove_by_index(index) {
            self.publish_only_change();
            true
        } else {
            false
        }
    }

    pub fn remove_sms_fragment(&mut self, fragment: &dji4g_domain::SmsFragmentKey) -> bool {
        if self.sms_store.remove_fragment(fragment) {
            self.publish_only_change();
            true
        } else {
            false
        }
    }

    /// Drop the whole inbox. Device/SIM epoch transitions call this: messages read under the old
    /// session are no longer trustworthy evidence for the new one.
    pub fn clear_sms(&mut self) {
        if !self.sms_store.is_empty() {
            self.sms_store.clear();
            self.publish_only_change();
        }
    }

    /// Record the latest SMS probe verdict and `CPMS` capacity; the summary derives from these
    /// plus the store on every snapshot.
    pub fn record_sms_probe(&mut self, status: FeatureStatus, capacity: Option<(u32, u32)>) {
        if self.sms_status != status || self.sms_capacity != capacity {
            self.sms_status = status;
            self.sms_capacity = capacity;
            self.publish_only_change();
        }
    }

    /// Push one observed transition onto the bounded timeline, deduplicating an exact repeat of
    /// the newest entry. Recording is publication-only: the surrounding evidence handling decides
    /// what the transition means and already owns the evidence revision.
    fn record_timeline(
        &mut self,
        at: SystemTime,
        kind: TimelineEventKind,
        detail: impl Into<String>,
    ) {
        let detail = detail.into();
        if self
            .timeline
            .events()
            .last()
            .is_some_and(|event| event.kind == kind && event.detail == detail)
        {
            return;
        }
        self.timeline.push(TimelineEvent { at, kind, detail });
        self.publish_only_change();
    }

    /// Advance the counter baseline and derive this cycle's throughput.  A missing sample,
    /// adapter/epoch change, counter regression (restart/replug), or a gap beyond the evidence
    /// TTL resets the baseline honestly to `None` instead of presenting a fabricated rate.
    fn measured_rates(
        rate_state: &mut Option<RateState>,
        adapter_id: &str,
        epoch: DeviceEpoch,
        rx: Option<u64>,
        tx: Option<u64>,
        now: SystemTime,
    ) -> (Option<u64>, Option<u64>) {
        let (Some(rx), Some(tx)) = (rx, tx) else {
            *rate_state = None;
            return (None, None);
        };
        let rates = match rate_state {
            Some(previous)
                if previous.adapter_id == adapter_id
                    && previous.epoch == epoch
                    && now > previous.prev_at
                    && now.duration_since(previous.prev_at).unwrap_or_default() <= EVIDENCE_TTL
                    && rx >= previous.prev_rx
                    && tx >= previous.prev_tx =>
            {
                let secs = now
                    .duration_since(previous.prev_at)
                    .unwrap_or_default()
                    .as_secs()
                    .max(1);
                (
                    Some(rx.saturating_sub(previous.prev_rx) / secs),
                    Some(tx.saturating_sub(previous.prev_tx) / secs),
                )
            }
            _ => (None, None),
        };
        *rate_state = Some(RateState {
            adapter_id: adapter_id.to_owned(),
            epoch,
            prev_rx: rx,
            prev_tx: tx,
            prev_at: now,
        });
        rates
    }

    fn apply_adapter(
        &mut self,
        result: CheckResult<AdapterObservationDto>,
        epoch: DeviceEpoch,
        now: SystemTime,
    ) {
        match result {
            CheckResult::Passed { value, observed_at }
                if self.accept_observed(observed_at, now) =>
            {
                if self
                    .target_identity
                    .as_ref()
                    .is_none_or(|target| target.value != value.binding.target)
                    || value.epoch != epoch
                {
                    return;
                }
                self.adapter_binding = Some(evidence(
                    epoch,
                    EvidenceSource::WindowsAdapter,
                    value.binding.clone(),
                    observed_at,
                ));
                self.adapter = Some(evidence(
                    epoch,
                    EvidenceSource::WindowsAdapter,
                    match value.state {
                        AdapterStateDto::UsableAddressAndRoute => {
                            AdapterState::UsableAddressAndRoute
                        }
                        AdapterStateDto::NoUsableAddressOrRoute => {
                            AdapterState::NoUsableAddressOrRoute
                        }
                    },
                    observed_at,
                ));
                let (down_bytes_per_sec, up_bytes_per_sec) = Self::measured_rates(
                    &mut self.rate_state,
                    &value.binding.adapter_id,
                    epoch,
                    value.rx_bytes,
                    value.tx_bytes,
                    observed_at,
                );
                self.network = Some(NetworkSnapshot {
                    adapter_id: value.binding.adapter_id,
                    addresses: value.addresses,
                    gateways: value.gateways,
                    dns_servers: value.dns_servers,
                    adapter_state: match value.state {
                        AdapterStateDto::UsableAddressAndRoute => {
                            AdapterState::UsableAddressAndRoute
                        }
                        AdapterStateDto::NoUsableAddressOrRoute => {
                            AdapterState::NoUsableAddressOrRoute
                        }
                    },
                    bound_public: self
                        .network
                        .as_ref()
                        .map_or(BoundPublicStatus::Incomplete, |network| {
                            network.bound_public
                        }),
                    bound_dns: self
                        .network
                        .as_ref()
                        .map_or(BoundDnsStatus::Incomplete, |network| network.bound_dns),
                    protocol_coverage: self
                        .network
                        .as_ref()
                        .map_or(ProtocolCoverage::SingleFamilyOnly, |network| {
                            network.protocol_coverage
                        }),
                    system_default_route: self
                        .network
                        .as_ref()
                        .map_or(DefaultRouteOwner::Other, |network| {
                            network.system_default_route
                        }),
                    down_bytes_per_sec,
                    up_bytes_per_sec,
                });
                self.set_check_terminal(
                    DiagnosticCheckId::WindowsAdapter,
                    DiagnosticCheckState::Passed,
                    observed_at,
                );
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Failed { code, observed_at } if self.accept_observed(observed_at, now) => {
                self.set_check_terminal(
                    DiagnosticCheckId::WindowsAdapter,
                    DiagnosticCheckState::Failed { code },
                    observed_at,
                );
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Unavailable { code, observed_at }
                if self.accept_observed(observed_at, now) =>
            {
                self.set_check_terminal(
                    DiagnosticCheckId::WindowsAdapter,
                    DiagnosticCheckState::Unavailable { code },
                    observed_at,
                );
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Unexecuted { reason } => {
                self.set_check_terminal(
                    DiagnosticCheckId::WindowsAdapter,
                    DiagnosticCheckState::Unexecuted { reason },
                    now,
                );
                self.publication_revision = self.publication_revision.saturating_add(1);
            }
            _ => {}
        }
    }

    fn apply_probe(
        &mut self,
        result: CheckResult<ProbeObservationDto>,
        epoch: DeviceEpoch,
        now: SystemTime,
    ) {
        if !self.active_probe {
            if let CheckResult::Unexecuted { reason } = result {
                for id in [
                    DiagnosticCheckId::BoundGateway,
                    DiagnosticCheckId::BoundPublic,
                    DiagnosticCheckId::BoundDns,
                ] {
                    self.set_check_terminal(id, DiagnosticCheckState::Unexecuted { reason }, now);
                }
                self.publication_revision = self.publication_revision.saturating_add(1);
            }
            return;
        }
        match result {
            CheckResult::Passed { value, observed_at }
                if self.accept_observed(observed_at, now) =>
            {
                let Some(binding) = self
                    .adapter_binding
                    .as_ref()
                    .map(|value| value.value.clone())
                else {
                    return;
                };
                if value.epoch != epoch || value.adapter_id != binding.adapter_id {
                    return;
                }
                let gateway = apply_probe_stage(
                    &value.gateway,
                    DiagnosticCheckId::BoundGateway,
                    self,
                    observed_at,
                );
                let (public_status, dns_status) = if gateway {
                    let _ = apply_probe_public(&value.public, self, observed_at);
                    let _ = apply_probe_dns(&value.dns, self, observed_at);
                    let public_status =
                        probe_public_status(&value.public, self.consecutive_public_failures);
                    let dns_status = probe_dns_status(&value.dns);
                    self.bound_public = Some(evidence(
                        epoch,
                        EvidenceSource::BoundPublicProbe,
                        BoundEvidence {
                            binding: binding.clone(),
                            value: public_status,
                        },
                        observed_at,
                    ));
                    self.bound_dns = Some(evidence(
                        epoch,
                        EvidenceSource::BoundDnsProbe,
                        BoundEvidence {
                            binding: binding.clone(),
                            value: dns_status,
                        },
                        observed_at,
                    ));
                    if let Some(coverage) = value.protocol_coverage {
                        let coverage_evidence = BoundEvidence {
                            binding: binding.clone(),
                            value: coverage,
                        };
                        self.protocol_coverage = Some(evidence(
                            epoch,
                            EvidenceSource::BoundPublicProbe,
                            coverage_evidence,
                            observed_at,
                        ));
                    } else {
                        self.protocol_coverage = None;
                    }
                    (public_status, dns_status)
                } else {
                    let code = failure(ErrorCodeForStage::Missing, "probe:gateway_not_proven");
                    for id in [DiagnosticCheckId::BoundPublic, DiagnosticCheckId::BoundDns] {
                        self.set_check_terminal(
                            id,
                            DiagnosticCheckState::Unavailable { code: code.clone() },
                            observed_at,
                        );
                    }
                    self.bound_public = None;
                    self.bound_dns = None;
                    self.protocol_coverage = None;
                    (BoundPublicStatus::Incomplete, BoundDnsStatus::Incomplete)
                };
                if let Some(route) = value.system_route {
                    if route.explanation_only {
                        let owner = match route.owner {
                            DefaultRouteDto::TargetAdapter => DefaultRouteOwner::TargetAdapter,
                            DefaultRouteDto::VpnOrTun => DefaultRouteOwner::VpnOrTun,
                            DefaultRouteDto::Other => DefaultRouteOwner::Other,
                        };
                        self.system_default_route = Some(evidence(
                            epoch,
                            EvidenceSource::GlobalRoute,
                            owner,
                            observed_at,
                        ));
                        self.set_check_terminal(
                            DiagnosticCheckId::SystemRoute,
                            DiagnosticCheckState::Passed,
                            observed_at,
                        );
                        if let Some(network) = self.network.as_mut() {
                            network.system_default_route = owner;
                        }
                    }
                } else {
                    self.system_default_route = None;
                    self.set_check_terminal(
                        DiagnosticCheckId::SystemRoute,
                        DiagnosticCheckState::Unavailable {
                            code: failure(ErrorCodeForStage::Missing, "route:not_observed"),
                        },
                        observed_at,
                    );
                    if let Some(network) = self.network.as_mut() {
                        network.system_default_route = DefaultRouteOwner::Other;
                    }
                }
                if let Some(network) = self.network.as_mut() {
                    network.bound_public = public_status;
                    network.bound_dns = dns_status;
                    if let Some(coverage) = self.protocol_coverage.as_ref() {
                        network.protocol_coverage = coverage.value.value;
                    } else {
                        network.protocol_coverage = ProtocolCoverage::SingleFamilyOnly;
                    }
                }
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Failed { code, observed_at } if self.accept_observed(observed_at, now) => {
                self.set_check_terminal(
                    DiagnosticCheckId::BoundGateway,
                    DiagnosticCheckState::Unavailable { code: code.clone() },
                    observed_at,
                );
                self.set_check_terminal(
                    DiagnosticCheckId::BoundPublic,
                    DiagnosticCheckState::Unavailable { code: code.clone() },
                    observed_at,
                );
                self.set_check_terminal(
                    DiagnosticCheckId::BoundDns,
                    DiagnosticCheckState::Unavailable { code: code.clone() },
                    observed_at,
                );
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Unavailable { code, observed_at }
                if self.accept_observed(observed_at, now) =>
            {
                self.set_check_terminal(
                    DiagnosticCheckId::BoundGateway,
                    DiagnosticCheckState::Unavailable { code: code.clone() },
                    observed_at,
                );
                self.set_check_terminal(
                    DiagnosticCheckId::BoundPublic,
                    DiagnosticCheckState::Unavailable { code: code.clone() },
                    observed_at,
                );
                self.set_check_terminal(
                    DiagnosticCheckId::BoundDns,
                    DiagnosticCheckState::Unavailable { code: code.clone() },
                    observed_at,
                );
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Unexecuted { reason } => {
                for id in [
                    DiagnosticCheckId::BoundGateway,
                    DiagnosticCheckId::BoundPublic,
                    DiagnosticCheckId::BoundDns,
                ] {
                    self.set_check_terminal(id, DiagnosticCheckState::Unexecuted { reason }, now);
                }
                self.publication_revision = self.publication_revision.saturating_add(1);
            }
            _ => {}
        }
    }

    fn apply_hotspot(
        &mut self,
        result: CheckResult<HotspotObservation>,
        epoch: DeviceEpoch,
        now: SystemTime,
    ) {
        match result {
            CheckResult::Passed { value, observed_at }
                if self.accept_observed(observed_at, now) =>
            {
                self.hotspot = value.status;
                self.set_check_terminal(
                    DiagnosticCheckId::Hotspot,
                    DiagnosticCheckState::Passed,
                    observed_at,
                );
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Failed { code, observed_at } if self.accept_observed(observed_at, now) => {
                self.hotspot = HotspotStatus::Failed {
                    code: code.category,
                };
                self.set_check_terminal(
                    DiagnosticCheckId::Hotspot,
                    DiagnosticCheckState::Failed { code },
                    observed_at,
                );
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Unavailable { code, observed_at }
                if self.accept_observed(observed_at, now) =>
            {
                self.hotspot = HotspotStatus::Unsupported(
                    dji4g_domain::HotspotUnsupportedReason::SourceProfileUnavailable,
                );
                self.set_check_terminal(
                    DiagnosticCheckId::Hotspot,
                    DiagnosticCheckState::Unavailable { code },
                    observed_at,
                );
                self.mark_evidence_changed(observed_at);
            }
            CheckResult::Unexecuted { reason } => {
                self.set_check_terminal(
                    DiagnosticCheckId::Hotspot,
                    DiagnosticCheckState::Unexecuted { reason },
                    now,
                );
                self.publication_revision = self.publication_revision.saturating_add(1);
            }
            _ => {
                let _ = epoch;
            }
        }
    }
}

fn apply_probe_stage(
    stage: &ProbeStageDto,
    id: DiagnosticCheckId,
    state: &mut ReducerState,
    observed_at: SystemTime,
) -> bool {
    match stage {
        ProbeStageDto::Passed => {
            state.set_check_terminal(id, DiagnosticCheckState::Passed, observed_at);
            true
        }
        ProbeStageDto::Failed { code } => {
            state.set_check_terminal(
                id,
                DiagnosticCheckState::Failed { code: code.clone() },
                observed_at,
            );
            false
        }
        ProbeStageDto::Unavailable { code } => {
            state.set_check_terminal(
                id,
                DiagnosticCheckState::Unavailable { code: code.clone() },
                observed_at,
            );
            false
        }
        ProbeStageDto::Unexecuted { code } => {
            state.set_check_terminal(
                id,
                DiagnosticCheckState::Unexecuted {
                    reason: UnexecutedReason::NotScheduled,
                },
                observed_at,
            );
            let _ = code;
            false
        }
    }
}

fn apply_probe_public(
    stage: &ProbeStageDto,
    state: &mut ReducerState,
    observed_at: SystemTime,
) -> bool {
    match stage {
        ProbeStageDto::Passed => {
            state.consecutive_public_failures = 0;
            state.set_check_terminal(
                DiagnosticCheckId::BoundPublic,
                DiagnosticCheckState::Passed,
                observed_at,
            );
            true
        }
        ProbeStageDto::Failed { code } => {
            state.consecutive_public_failures = state.consecutive_public_failures.saturating_add(1);
            state.set_check_terminal(
                DiagnosticCheckId::BoundPublic,
                DiagnosticCheckState::Failed { code: code.clone() },
                observed_at,
            );
            false
        }
        ProbeStageDto::Unavailable { code } => {
            state.set_check_terminal(
                DiagnosticCheckId::BoundPublic,
                DiagnosticCheckState::Unavailable { code: code.clone() },
                observed_at,
            );
            false
        }
        ProbeStageDto::Unexecuted { .. } => {
            state.set_check_terminal(
                DiagnosticCheckId::BoundPublic,
                DiagnosticCheckState::Unexecuted {
                    reason: UnexecutedReason::NotScheduled,
                },
                observed_at,
            );
            false
        }
    }
}

fn apply_probe_dns(
    stage: &ProbeStageDto,
    state: &mut ReducerState,
    observed_at: SystemTime,
) -> bool {
    match stage {
        ProbeStageDto::Passed => {
            state.set_check_terminal(
                DiagnosticCheckId::BoundDns,
                DiagnosticCheckState::Passed,
                observed_at,
            );
            true
        }
        ProbeStageDto::Failed { code } => {
            state.set_check_terminal(
                DiagnosticCheckId::BoundDns,
                DiagnosticCheckState::Failed { code: code.clone() },
                observed_at,
            );
            false
        }
        ProbeStageDto::Unavailable { code } => {
            state.set_check_terminal(
                DiagnosticCheckId::BoundDns,
                DiagnosticCheckState::Unavailable { code: code.clone() },
                observed_at,
            );
            false
        }
        ProbeStageDto::Unexecuted { .. } => {
            state.set_check_terminal(
                DiagnosticCheckId::BoundDns,
                DiagnosticCheckState::Unexecuted {
                    reason: UnexecutedReason::NotScheduled,
                },
                observed_at,
            );
            false
        }
    }
}

fn probe_public_status(stage: &ProbeStageDto, consecutive_failures: u8) -> BoundPublicStatus {
    match stage {
        ProbeStageDto::Passed => BoundPublicStatus::Succeeded,
        ProbeStageDto::Failed { .. } => BoundPublicStatus::Failed {
            consecutive_cycles: consecutive_failures,
        },
        ProbeStageDto::Unavailable { .. } | ProbeStageDto::Unexecuted { .. } => {
            BoundPublicStatus::Incomplete
        }
    }
}

fn probe_dns_status(stage: &ProbeStageDto) -> BoundDnsStatus {
    match stage {
        ProbeStageDto::Passed => BoundDnsStatus::Succeeded,
        ProbeStageDto::Failed { .. } => BoundDnsStatus::Failed,
        ProbeStageDto::Unavailable { .. } | ProbeStageDto::Unexecuted { .. } => {
            BoundDnsStatus::Incomplete
        }
    }
}

/// Closed Chinese label for one observed registration state (timeline detail only).
fn registration_label(state: RegistrationState) -> &'static str {
    match state {
        RegistrationState::RegisteredHome => "已注册到本地网络",
        RegistrationState::RegisteredRoaming => "已注册到漫游网络",
        RegistrationState::Searching => "正在搜索",
        RegistrationState::Denied => "注册被拒绝",
        RegistrationState::NotRegistered => "未注册",
        RegistrationState::Unknown => "未知",
    }
}

/// Closed Chinese label for one bound-DNS verdict (timeline detail only).
fn dns_label(status: BoundDnsStatus) -> &'static str {
    match status {
        BoundDnsStatus::Succeeded => "通过",
        BoundDnsStatus::Failed => "失败",
        BoundDnsStatus::Incomplete => "未完成",
    }
}

/// The probe verdict behind one bound-DNS diagnostic check, if the check carries a terminal
/// probe result. `Unexecuted`/`Running`/`Expired` are not verdicts, so a timeline change is never
/// claimed against them.
fn dns_status_from_check(state: &DiagnosticCheckState) -> Option<BoundDnsStatus> {
    match state {
        DiagnosticCheckState::Passed => Some(BoundDnsStatus::Succeeded),
        DiagnosticCheckState::Failed { .. } => Some(BoundDnsStatus::Failed),
        DiagnosticCheckState::Unavailable { .. } => Some(BoundDnsStatus::Incomplete),
        DiagnosticCheckState::Unexecuted { .. }
        | DiagnosticCheckState::Running { .. }
        | DiagnosticCheckState::Expired => None,
    }
}

/// Identity fields of a serving cell. Deliberately excludes the signal measurements: RSRP/RSRQ/
/// SINR move every sample and must never read as "the serving cell changed".
type ServingCellIdentity = (
    Option<String>,
    Option<String>,
    Option<u32>,
    Option<u16>,
    Option<u32>,
);

fn serving_cell_identity(cell: &ServingCell) -> ServingCellIdentity {
    (
        cell.mcc.clone(),
        cell.mnc.clone(),
        cell.cell_id,
        cell.pci,
        cell.earfcn,
    )
}

fn has_expired_evidence(input: &ClassificationInput, now: SystemTime) -> bool {
    [
        input
            .device_presence
            .as_ref()
            .map(|value| value.is_fresh_for(input.current_epoch, now)),
        input
            .target_identity
            .as_ref()
            .map(|value| value.is_fresh_for(input.current_epoch, now)),
        input
            .cellular_block
            .as_ref()
            .map(|value| value.is_fresh_for(input.current_epoch, now)),
        input
            .adapter_binding
            .as_ref()
            .map(|value| value.is_fresh_for(input.current_epoch, now)),
        input
            .adapter
            .as_ref()
            .map(|value| value.is_fresh_for(input.current_epoch, now)),
        input
            .bound_public
            .as_ref()
            .map(|value| value.is_fresh_for(input.current_epoch, now)),
        input
            .bound_dns
            .as_ref()
            .map(|value| value.is_fresh_for(input.current_epoch, now)),
        input
            .protocol_coverage
            .as_ref()
            .map(|value| value.is_fresh_for(input.current_epoch, now)),
        input
            .at_control
            .as_ref()
            .map(|value| value.is_fresh_for(input.current_epoch, now)),
        input
            .system_default_route
            .as_ref()
            .map(|value| value.is_fresh_for(input.current_epoch, now)),
    ]
    .into_iter()
    .flatten()
    .any(|fresh| !fresh)
}

fn evidence<T>(
    epoch: DeviceEpoch,
    source: EvidenceSource,
    value: T,
    observed_at: SystemTime,
) -> Evidence<T> {
    Evidence {
        epoch,
        observed_at,
        ttl: EVIDENCE_TTL,
        source,
        value,
    }
}

fn test_identity() -> StableDeviceIdentity {
    StableDeviceIdentity {
        container_id: "{container}".into(),
        device_instance_id: "USB\\VID_2CA3&PID_4006\\INSTANCE".into(),
        vid: 0x2CA3,
        pid: 0x4006,
    }
}

#[derive(Clone, Copy)]
enum ErrorCodeForStage {
    Missing,
    Permission,
    Absent,
    Unsupported,
}

/// Map an action onto its parameterless readiness key. The reducer's prerequisites never
/// depend on parameter values, so every variant of one action shares one verdict.
fn readiness_key(action: &dji4g_domain::ActionKind) -> Option<ActionReadinessKey> {
    Some(match action {
        dji4g_domain::ActionKind::RenewDhcp => ActionReadinessKey::RenewDhcp,
        dji4g_domain::ActionKind::ApplyDnsProfile { .. } => ActionReadinessKey::ApplyDnsProfile,
        dji4g_domain::ActionKind::RestartAdapter => ActionReadinessKey::RestartAdapter,
        dji4g_domain::ActionKind::ReenumerateDevice => ActionReadinessKey::ReenumerateDevice,
        dji4g_domain::ActionKind::RestartModule => ActionReadinessKey::RestartModule,
        dji4g_domain::ActionKind::EditApn { .. } => ActionReadinessKey::EditApn,
        dji4g_domain::ActionKind::SetVerifiedUsbNetworkProfile { .. } => {
            ActionReadinessKey::SetUsbNetworkProfile
        }
        dji4g_domain::ActionKind::ToggleHotspot { .. } => ActionReadinessKey::ToggleHotspot,
        dji4g_domain::ActionKind::Refresh => return None,
    })
}

fn cellular_verdict(
    cellular: Option<&CellularSnapshot>,
) -> (DiagnosticCheckState, Option<CellularBlock>) {
    use dji4g_domain::{AttachState, SimState};
    let unavailable = |code| {
        (
            DiagnosticCheckState::Unavailable {
                code: failure(ErrorCodeForStage::Missing, code),
            },
            None,
        )
    };
    let Some(cellular) = cellular else {
        return unavailable("app:cellular_unobserved");
    };
    let sim_block = match cellular.sim {
        SimState::Missing => Some("app:sim_missing"),
        SimState::PinRequired => Some("app:sim_pin_required"),
        SimState::PukRequired => Some("app:sim_puk_required"),
        SimState::Rejected => Some("app:sim_rejected"),
        SimState::Unknown => return unavailable("app:sim_unobserved"),
        SimState::Ready => None,
    };
    if let Some(code) = sim_block {
        return (
            DiagnosticCheckState::Failed {
                code: failure(ErrorCodeForStage::Missing, code),
            },
            Some(CellularBlock::SimRejected),
        );
    }
    match cellular.registration {
        RegistrationState::Denied => {
            return (
                DiagnosticCheckState::Failed {
                    code: failure(ErrorCodeForStage::Missing, "app:registration_rejected"),
                },
                Some(CellularBlock::RegistrationRejected),
            );
        }
        RegistrationState::RegisteredHome | RegistrationState::RegisteredRoaming => {}
        _ => return unavailable("app:registration_not_ready"),
    }
    if cellular.attached != AttachState::Attached {
        return unavailable("app:packet_not_attached");
    }
    (DiagnosticCheckState::Passed, None)
}

fn failure(kind: ErrorCodeForStage, stable: &'static str) -> FailureCode {
    let category = match kind {
        ErrorCodeForStage::Missing => dji4g_domain::ErrorCode::ProbeFailed,
        ErrorCodeForStage::Permission => dji4g_domain::ErrorCode::PermissionDenied,
        ErrorCodeForStage::Absent => dji4g_domain::ErrorCode::DeviceRemoved,
        ErrorCodeForStage::Unsupported => dji4g_domain::ErrorCode::Unsupported,
    };
    FailureCode::new(
        category,
        StableCode::try_from_static(stable).expect("stable stage code"),
    )
}

/// Compatibility helper required by the implementation plan. New code should use
/// [`ReducerState`] so it can retain epoch/cycle and diagnostic lifecycle state.
#[must_use]
pub fn reduce(previous: &AppSnapshot, event: BackendEvent, now: SystemTime) -> AppSnapshot {
    let mut state = ReducerState::new(previous.observed_at);
    state.evidence_revision = previous.revision;
    state.publication_revision = previous.revision;
    state.epoch = previous
        .device
        .as_ref()
        .map_or(DeviceEpoch(0), |device| device.epoch);
    state.phase = match previous.availability {
        Availability::Detecting => ClassificationPhase::Startup,
        _ => ClassificationPhase::Stable,
    };
    state.hotspot = previous.hotspot;
    let state = reduce_state(&state, event, now);
    state.app_snapshot(now)
}

trait DiagnosticSetExt {
    fn set_terminal_from_result(
        &mut self,
        id: DiagnosticCheckId,
        passed: bool,
        error: Option<FailureCode>,
        observed_at: SystemTime,
        epoch: DeviceEpoch,
    );
}

impl DiagnosticSetExt for DiagnosticSet {
    fn set_terminal_from_result(
        &mut self,
        id: DiagnosticCheckId,
        passed: bool,
        error: Option<FailureCode>,
        observed_at: SystemTime,
        epoch: DeviceEpoch,
    ) {
        let check = self.get_mut(id);
        check.epoch = epoch;
        check.state = if passed {
            DiagnosticCheckState::Passed
        } else {
            DiagnosticCheckState::Failed {
                code: error.expect("failed check code"),
            }
        };
        check.started_at = Some(observed_at);
        check.finished_at = Some(observed_at);
        check.observed_at = Some(observed_at);
        check.expires_at = Some(observed_at + EVIDENCE_TTL);
    }
}
