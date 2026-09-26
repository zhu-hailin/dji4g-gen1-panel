#![forbid(unsafe_code)]

//! Application orchestration for the DJI 4G panel.

mod confirmation;
mod controller;
mod device_tools;
mod features;
mod host_network;
mod module_network_check;
mod monitor;
mod ports;
mod reducer;
mod sms;
mod sms_delete;
pub mod sync;

pub use confirmation::{
    ActionKindTag, ActionPlanId, ActionRequest, ConfirmError, ConfirmResult,
    ConfirmationInvalidationReason, ControlledRepairError, ControlledRepairRequest, OperationPhase,
    OperationState, OperationUiSnapshot, PreparedActionSnapshot, PreparedActionState,
    ValidatedActionToken, action_disruption, action_requires_elevation, action_risk,
};
pub use controller::{
    COMMAND_QUEUE_CAPACITY, CommandReceipt, Controller, ControllerHandle, PLAN_LIFETIME,
    PrepareError, SmsRequest, UiCommand, UiSendError,
};
pub use device_tools::{
    DeviceToolsSnapshot, EXPERT_PLAN_LIFETIME, MAX_TOOL_HISTORY_BYTES, MAX_TOOL_HISTORY_ITEMS,
    MAX_TRANSCRIPT_BYTES, ModuleProfile, PROBE_BATCH_BUDGET, PendingExpertTool,
    TOOL_TRANSACTION_TIMEOUT, ToolCapabilityRow, ToolContext, ToolControl, ToolHistory,
    ToolHistoryEntry, ToolMode, ToolOperation, ToolOperationKind, ToolOutcome, ToolPhase,
    ToolReceipt, ToolRequest, ToolTaskSnapshot, ToolTranscript, UsbNetReading, as_at_response,
    extract_identity, extract_payload, item_deadline, parse_profile_temperature, parse_usb_net,
    transcript_from_response,
};
pub use features::{FeatureCapability, FeatureKey};
pub use host_network::{
    HostNetworkPhase, HostNetworkPort, HostNetworkSnapshot, ProxyRepairPreview, ProxyRepairResult,
};
pub use module_network_check::{
    ModuleNetworkCheckPhase, ModuleNetworkCheckSnapshot, NetworkRepairKind, step_state,
};
pub use monitor::{
    ControllerRunner, MonitorPorts, RATE_READ_TIMEOUT, RATE_TICK_INTERVAL, REFRESH_INTERVAL,
    STAGE_TIMEOUT, periodic_refresh_due, rate_tick_due,
};
pub use ports::{
    ActionExecutor, ActionPreconditions, AdapterContext, AdapterMetrics, AdapterNetworkDetails,
    AdapterObservationDto, AdapterPort, AdapterStateDto, AtObservation, AtPort,
    AutostartApplyOutcome, AutostartControl, AutostartKnownState, AutostartStatus, Clock,
    CommandState, CommandStateSnapshot, DefaultRouteDto, DevicePresenceDto, DeviceToolsPort,
    ExecutionReceipt, ExecutionReceiptOutcome, FailureCode, FakeActionExecutor, FakeClock,
    HotspotControl, HotspotObservation, InventoryObservation, InventoryPort, LanguageCode,
    LogLevel, MonoTime, NetworkProbePort, NetworkRouteChoice, NormalizedNetworkEvidence, PortError,
    PortFuture, PrivilegedExecutor, ProbeObservationDto, ProbeStageDto, SettingsPersistenceState,
    SettingsSaveOutcome, SettingsSnapshot, SmsListing, SmsPort, SmsReadResult, SmsSendReceipt,
    SmsSendResult, StableCode, StableCodeError, SystemRouteDto, TargetContext, mask_recipient,
};
pub use reducer::{
    ActionReadiness, ActionReadinessKey, BackendEvent, CheckMask, CheckResult, ControllerSnapshot,
    DiagnosticCheckId, DiagnosticCheckSnapshot, DiagnosticCheckState, DiagnosticSet,
    EpochInvalidationReason, ReducerState, RefreshCycleId, UiFeedback, UnexecutedReason, reduce,
    reduce_state,
};

pub use dji4g_domain::{
    ActionKind, ActionSafetyError, AdapterBinding, AdapterState, AppSnapshot,
    AtControlAvailability, Availability, BoundDnsStatus, BoundEvidence, BoundPublicStatus,
    CellularBlock, CellularSnapshot, ClassificationInput, ClassificationPhase, DefaultRouteOwner,
    DeviceEpoch, DevicePresence, DeviceProfile, DisruptionLevel, DnsProfile, ErrorCode,
    FeatureStatus, Freshness, GlobalConnectivity, HotspotStatus, Issue, IssueLayer, IssueSeverity,
    NetworkSnapshot, OperationOutcome, ProtocolCoverage, RiskLevel, RollbackOutcome,
    SmsInboxSummary, SmsMessage, SmsStorageId, StableDeviceIdentity, Timeline, TimelineEvent,
    TimelineEventKind, UsbNetworkProfile,
};
pub use dji4g_domain::{
    SMS_SEND_TIMEOUT, SmsFailureDetail, SmsSendPhase, SmsSendSnapshot, SmsTransactionControl,
};
pub use dji4g_domain::{
    SmsDeleteControl, SmsDeleteItemResult, SmsDeleteReceipt, SmsDisplayMessage, SmsFragmentKey,
};
pub use sms::{MAX_STORED, SmsStore};
pub use sms_delete::{SmsDeleteItemSnapshot, SmsDeleteSnapshot};
