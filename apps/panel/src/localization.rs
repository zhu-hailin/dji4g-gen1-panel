//! Closed, user-facing text catalog for the panel.
//!
//! Platform and application crates intentionally carry stable codes and typed values only. This
//! module is the sole place where those values become prose. The default language is simplified
//! Chinese. English is kept behind `english_available()` until a complete, reviewed catalog is
//! shipped; callers must not mix languages key-by-key.

use std::fmt;

use dji4g_application::{
    ActionKindTag, ConfirmationInvalidationReason, DiagnosticCheckId, DiagnosticCheckState,
    FailureCode, LogLevel, OperationPhase, UnexecutedReason,
};
use dji4g_domain::{
    ActionKind, ActionSafetyError, AtControlAvailability, AttachState, Availability,
    BoundDnsStatus, BoundPublicStatus, CellularBlock, ClassificationPhase, DefaultRouteOwner,
    DevicePresence, DisruptionLevel, DnsProfile, ErrorCode, EvidenceSource, FeatureStatus,
    Freshness, GlobalConnectivity, HotspotStatus, HotspotUnsupportedReason, IssueLayer,
    IssueSeverity, LimitedReason, ProtocolCoverage, RegistrationState, RiskLevel, RollbackOutcome,
    SimState, TimelineEventKind, UnavailableReason, UsbNetworkProfile,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Language {
    ZhCn,
    EnUs,
}

/// Stable identifiers for every string that can reach the normal UI.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[allow(clippy::enum_variant_names)]
pub enum TextKey {
    AvailabilityDetectingTitle,
    AvailabilityDetectingReason,
    AvailabilityAvailableTitle,
    AvailabilityAvailableReason,
    AvailabilityLimitedTitle,
    AvailabilityUnavailableTitle,
    AvailabilityNotDetectedTitle,
    AvailabilityNotDetectedReason,
    AvailabilityUnsupportedTitle,
    AvailabilityUnsupportedReason,
    LimitedReasonDnsFailure,
    LimitedReasonSingleProtocolFamily,
    LimitedReasonCompetingDefaultRoute,
    LimitedReasonAtControlUnavailable,
    LimitedReasonIncompleteEvidence,
    UnavailableReasonCellularRejected,
    UnavailableReasonNoUsableAddressOrRoute,
    UnavailableReasonBoundPublicProbeFailed,
    UnavailableReasonNoBoundReachability,
    HotspotUnsupportedTitle,
    HotspotOff,
    HotspotStarting,
    HotspotOnWithClients,
    HotspotOnClientsUnknown,
    HotspotStopping,
    HotspotFailed,
    HotspotUnsupportedMissingPackageIdentity,
    HotspotUnsupportedMissingWifiControlCapability,
    HotspotUnsupportedNoWifiAdapter,
    HotspotUnsupportedPolicyDisabled,
    HotspotUnsupportedOperatingSystem,
    HotspotUnsupportedSourceProfileUnavailable,
    IssueSeverityInfo,
    IssueSeverityWarning,
    IssueSeverityError,
    IssueLayerDevice,
    IssueLayerCellular,
    IssueLayerNetwork,
    IssueLayerBoundProbe,
    IssueLayerHotspot,
    IssueLayerOperation,
    EvidenceSourcePnp,
    EvidenceSourceAtControl,
    EvidenceSourceWindowsAdapter,
    EvidenceSourceBoundGatewayProbe,
    EvidenceSourceBoundDnsProbe,
    EvidenceSourceBoundPublicProbe,
    EvidenceSourceGlobalRoute,
    EvidenceSourceGlobalConnectivity,
    EvidenceSourceHotspot,
    ClassificationPhaseStartup,
    ClassificationPhaseRecentInsertion,
    ClassificationPhaseReenumerating,
    ClassificationPhasePostWriteVerification,
    ClassificationPhaseStable,
    ActionRefresh,
    ActionRenewDhcp,
    ActionApplyDnsAutomatic,
    ActionApplyDnsStatic,
    ActionRestartAdapter,
    ActionReenumerateDevice,
    ActionRestartModule,
    ActionEditApn,
    ActionSetUsbProfileDjiNdis,
    ActionSetUsbProfileEcm,
    ActionEnableHotspot,
    ActionDisableHotspot,
    RiskLevelLow,
    RiskLevelMedium,
    RiskLevelHigh,
    OperationOutcomeApplied,
    OperationOutcomeFailed,
    OperationOutcomeUnknown,
    DnsProfileAutomatic,
    DnsProfileStatic,
    UsbNetworkProfileDjiNdis,
    UsbNetworkProfileEcm,
    DisruptionNone,
    DisruptionBrief,
    DisruptionConnectionInterrupting,
    DisruptionDeviceReenumeration,
    ActionSafetyUnsupportedDevice,
    ActionSafetyStaleEpoch,
    ActionSafetyStaleSnapshot,
    ActionSafetyTargetIdentityChanged,
    ActionSafetyBeforeStateChanged,
    ActionSafetyExpired,
    RollbackNotRequired,
    RollbackApplied,
    RollbackFailed,
    RollbackNotAttempted,
    FreshnessFresh,
    FreshnessStale,
    FreshnessUnknown,
    LastObservedAt,
    ObservedAgo,
    ErrorPermissionDenied,
    ErrorDeviceRemoved,
    ErrorDeviceIdentityChanged,
    ErrorEvidenceExpired,
    ErrorProbeFailed,
    ErrorDnsFailed,
    ErrorTimeout,
    ErrorUnsupported,
    ErrorCapabilityUnavailable,
    ErrorOperationCancelled,
    ErrorVerificationFailed,
    ErrorRollbackFailed,
    ErrorInternal,
    ErrorHelperUnsigned,
    ErrorHelperUnverified,
    SimReady,
    SimMissing,
    SimPinRequired,
    SimPukRequired,
    SimRejected,
    SimUnknown,
    RegistrationHome,
    RegistrationRoaming,
    RegistrationSearching,
    RegistrationDenied,
    RegistrationNotRegistered,
    RegistrationUnknown,
    AttachAttached,
    AttachDetached,
    AttachUnknown,
    CellularBlockSimRejected,
    CellularBlockRegistrationRejected,
    DevicePresenceSupported,
    DevicePresenceNotDetected,
    DevicePresenceUnsupported,
    DevicePresencePermissionDenied,
    AdapterUsableAddressAndRoute,
    AdapterNoUsableAddressOrRoute,
    BoundPublicSucceeded,
    BoundPublicFailed,
    BoundPublicIncomplete,
    BoundDnsSucceeded,
    BoundDnsFailed,
    BoundDnsIncomplete,
    ProtocolCoverageAllRequired,
    ProtocolCoverageSingleFamily,
    AtControlAvailable,
    AtControlUnavailable,
    DefaultRouteTargetAdapter,
    DefaultRouteVpnOrTun,
    DefaultRouteOther,
    GlobalConnectivityOnline,
    GlobalConnectivityOffline,
    ProtocolApnEmpty,
    ProtocolApnTooLong,
    ProtocolApnUnsafeCharacter,
    ProtocolPdpContextIdOutOfRange,
    ProtocolWrongPortData,
    ProtocolLineTooLong,
    ProtocolResponseTooLarge,
    ProtocolTimeout,
    ProtocolDeviceRemoved,
    ProtocolUnexpectedData,
    AtFinalOk,
    AtFinalError,
    AtFinalCmeError,
    AtFinalCmsError,
    AtFinalNoCarrier,
    AtFinalNoAnswer,
    AtFinalBusy,
    AtFinalNoDialTone,
    PlatformNoSafeAtPort,
    PlatformAmbiguousAtPort,
    PlatformAtPortUnverified,
    PlatformUnsupportedPlatform,
    PlatformPnpEnumerateFailed,
    PlatformInterfaceEnumerateFailed,
    PlatformPnpPermissionDenied,
    PlatformPnpOpenFailed,
    SerialQueueFull,
    SerialSessionClosed,
    SerialIoFailed,
    SerialAtFinalError,
    NavOverview,
    NavDiagnostics,
    NavRepairs,
    NavSettings,
    DiagnosticsTitle,
    DiagnosticsIntro,
    FieldDeviceIdentity,
    FieldDeviceModel,
    FieldUsbIdentity,
    FieldProblemCode,
    FieldAtPort,
    FieldAdapter,
    FieldCarrier,
    FieldRadioAccessTechnology,
    FieldSignal,
    FieldSimState,
    FieldRegistration,
    FieldAttachState,
    FieldApn,
    FieldPdpAddress,
    FieldWindowsAddresses,
    FieldGateway,
    FieldDnsServers,
    FieldDefaultRoute,
    FieldBoundGatewayProbe,
    FieldBoundPublicProbe,
    FieldBoundDnsProbe,
    FieldProtocolCoverage,
    FieldGlobalConnectivity,
    FieldHotspot,
    FieldEvidenceSource,
    FieldObservedAt,
    FieldPhoneNumber,
    FieldNumberSource,
    FieldVerificationState,
    FieldCaptureTime,
    FieldIccid,
    ValueUnknown,
    ValueNotAvailable,
    ValueRedacted,
    ValueNotApplicable,
    ValueNumberNotProvided,
    ValuePhoneNumberNotRead,
    ValueNumberSourceSimReport,
    ValueVerificationNotCarrierChecked,
    ValueCaptureTimeSimSession,
    ValueIccidNotRead,
    IdentityHeading,
    ButtonShow,
    ButtonCopy,
    ButtonCopied,
    ServingCellLayoutProvisional,
    FeatureStatusUnsupportedConfirmed,
    FeatureStatusFormatMismatch,
    FeatureStatusTransportFailure,
    FeatureStatusTemporarilyUnavailable,
    CheckPassed,
    CheckFailed,
    CheckUnavailable,
    CheckUnexecuted,
    CheckRunning,
    CheckExpired,
    UnexecutedDisabledBySetting,
    UnexecutedNotScheduled,
    UnexecutedSuperseded,
    AppTitle,
    UnofficialNotice,
    OverviewQuestion,
    OverviewLastObservation,
    RateCaptionDown,
    RateCaptionUp,
    RateWindow,
    RatePeak,
    RateSampling,
    RateGradeChip,
    RateGradePending,
    RateGradeIdle,
    RateGradeBasic,
    RateGradeGood,
    RateGradeExcellent,
    RateGradeVeryFast,
    ButtonRefresh,
    ButtonDiagnostics,
    ButtonRepair,
    ButtonConfirm,
    ButtonCancel,
    ButtonClose,
    ButtonBack,
    ButtonRetry,
    ButtonDone,
    ButtonViewDiagnostics,
    ButtonCopyAddress,
    ButtonExportDiagnostics,
    ButtonOpenReleases,
    StatusLoading,
    StatusNoActiveOperation,
    StatusExpired,
    StatusQueueFull,
    CommandFeedbackBusy,
    CommandFeedbackConfirmRejected,
    CommandFeedbackRejected,
    StatusBackendUnavailable,
    UnknownBackendError,
    SystemErrorNumber,
    TrayOpen,
    TrayRefreshNow,
    TrayHotspotStatus,
    TrayExit,
    TrayUnavailableFallback,
    CloseToTrayHint,
    SettingsTitle,
    SettingsLanguage,
    LanguageZhCn,
    LanguageEnUs,
    SettingsAutostart,
    SettingsAutostartDescription,
    SettingsStartMinimized,
    SettingsActiveProbe,
    SettingsActiveProbeDescription,
    SettingsLogLevel,
    SettingsLogLevelRestart,
    LogLevelError,
    LogLevelWarn,
    LogLevelInfo,
    LogLevelDebug,
    SettingsPrivacy,
    SettingsPrivacyDescription,
    SettingsConfigDrift,
    SettingsSaved,
    SettingsSaveFailed,
    SettingsCorruptConfig,
    SettingsAutostartLoading,
    SettingsAutostartSaving,
    SettingsAutostartNotOwned,
    SettingsPathUnavailable,
    SettingsReadFailed,
    SingleInstanceActivationFailed,
    LoggingInitFailed,
    LoggingRotationFailed,
    RepairsTitle,
    RepairsReadOnlyNotice,
    RepairsDriverNotIncluded,
    ConfirmationTitle,
    ConfirmationOperation,
    ConfirmationTarget,
    ConfirmationExpectedEffect,
    ConfirmationInterruption,
    ConfirmationElevation,
    ConfirmationRisk,
    ConfirmationElevationRequired,
    ConfirmationElevationNotRequired,
    ConfirmationStateRecheck,
    ConfirmationNoAutomaticRetry,
    ConfirmationApnContext,
    ConfirmationApnNewValue,
    OperationPreparing,
    OperationRevalidating,
    OperationAwaitingElevation,
    OperationExecuting,
    OperationVerifying,
    OperationUacCancelled,
    OperationDeviceRemoved,
    OperationAuditRecorded,
    NoPreparedAction,
    PreparedActionAwaitingConfirmation,
    PlanExpired,
    ConfirmationDevModeWarning,
    OperationResultTitle,
    DiagnosticsExportTitle,
    DiagnosticsExportDescription,
    DiagnosticsExportRedactionNotice,
    DiagnosticsExportSuccess,
    DiagnosticsExportFailed,
    BuildDevelopmentUnsigned,
    BuildStableSigned,
    FeatureUnavailablePortable,
    UiCjkFontUnavailable,
    NoAutomaticUpdate,
    DemoUsage,
    DemoRejectedRelease,
    DemoInvalidScenario,
    NavSms,
    NavDeviceTools,
    SmsTitle,
    SmsIntro,
    ButtonSmsRefresh,
    FieldSmsStatus,
    FieldSmsMessageCount,
    FieldSmsUnreadCount,
    FieldSmsCapacity,
    SmsCapacityUsed,
    SmsStatusNotQueried,
    SmsStatusRead,
    SmsIncompleteWarning,
    SmsEmpty,
    SmsListPending,
    SmsUnread,
    SmsRead,
    FieldSmsSender,
    FieldSmsTime,
    FieldSmsEncoding,
    FieldSmsParts,
    FieldSmsBody,
    SmsEncodingOther,
    SmsReadNote,
    ButtonSmsDelete,
    ButtonSmsDeleteConfirm,
    ButtonSmsSend,
    ButtonSmsSendConfirm,
    SmsEvictedWarning,
    FieldSmsRecipient,
    SmsSendNotice,
    SmsIncompleteTag,
    ButtonSmsExpand,
    ButtonSmsCollapse,
    SmsInboxHeading,
    SmsOutgoingSubmitted,
    SmsOutgoingFailed,
    SmsOutgoingUnknown,
    SmsBodyCharCount,
    SmsErrorPduModeRequired,
    SmsErrorPduConfirmFailed,
    SmsErrorInvalidMessage,
    SmsErrorSendFailed,
    SmsErrorTimeout,
    SmsErrorDeviceRemoved,
    SmsErrorUnsupported,
    SmsErrorVerificationFailed,
    SmsErrorInternal,
    SmsErrorGeneric,
    FieldTemperature,
    TemperatureNotRead,
    TemperatureSensorNote,
    TemperatureSectionHeading,
    TemperatureTrendWindow,
    TemperatureTrendNote,
    TemperatureTrendSampling,
    TemperatureDeltaUp,
    TemperatureDeltaDown,
    TemperatureDeltaFlat,
    TemperatureSensorsReported,
    FieldAdapterErrors,
    FieldAdapterDiscards,
    FieldAdapterLinkRate,
    AdapterRxTx,
    AdapterLinkRateNote,
    TimelineHeading,
    TimelineEmpty,
    TimelineSimChanged,
    TimelineRegistrationChanged,
    TimelineCellChanged,
    TimelineDeviceRemoved,
    TimelineDeviceArrived,
    TimelineAdapterLinkChanged,
    TimelineDnsChanged,
}

impl TextKey {
    /// The closed catalog. Keeping this list beside the enum makes completeness tests cheap and
    /// lets downstream code iterate without depending on enum discriminants.
    pub const ALL: &'static [Self] = &[
        Self::AvailabilityDetectingTitle,
        Self::AvailabilityDetectingReason,
        Self::AvailabilityAvailableTitle,
        Self::AvailabilityAvailableReason,
        Self::AvailabilityLimitedTitle,
        Self::AvailabilityUnavailableTitle,
        Self::AvailabilityNotDetectedTitle,
        Self::AvailabilityNotDetectedReason,
        Self::AvailabilityUnsupportedTitle,
        Self::AvailabilityUnsupportedReason,
        Self::LimitedReasonDnsFailure,
        Self::LimitedReasonSingleProtocolFamily,
        Self::LimitedReasonCompetingDefaultRoute,
        Self::LimitedReasonAtControlUnavailable,
        Self::LimitedReasonIncompleteEvidence,
        Self::UnavailableReasonCellularRejected,
        Self::UnavailableReasonNoUsableAddressOrRoute,
        Self::UnavailableReasonBoundPublicProbeFailed,
        Self::UnavailableReasonNoBoundReachability,
        Self::HotspotUnsupportedTitle,
        Self::HotspotOff,
        Self::HotspotStarting,
        Self::HotspotOnWithClients,
        Self::HotspotOnClientsUnknown,
        Self::HotspotStopping,
        Self::HotspotFailed,
        Self::HotspotUnsupportedMissingPackageIdentity,
        Self::HotspotUnsupportedMissingWifiControlCapability,
        Self::HotspotUnsupportedNoWifiAdapter,
        Self::HotspotUnsupportedPolicyDisabled,
        Self::HotspotUnsupportedOperatingSystem,
        Self::HotspotUnsupportedSourceProfileUnavailable,
        Self::IssueSeverityInfo,
        Self::IssueSeverityWarning,
        Self::IssueSeverityError,
        Self::IssueLayerDevice,
        Self::IssueLayerCellular,
        Self::IssueLayerNetwork,
        Self::IssueLayerBoundProbe,
        Self::IssueLayerHotspot,
        Self::IssueLayerOperation,
        Self::EvidenceSourcePnp,
        Self::EvidenceSourceAtControl,
        Self::EvidenceSourceWindowsAdapter,
        Self::EvidenceSourceBoundGatewayProbe,
        Self::EvidenceSourceBoundDnsProbe,
        Self::EvidenceSourceBoundPublicProbe,
        Self::EvidenceSourceGlobalRoute,
        Self::EvidenceSourceGlobalConnectivity,
        Self::EvidenceSourceHotspot,
        Self::ClassificationPhaseStartup,
        Self::ClassificationPhaseRecentInsertion,
        Self::ClassificationPhaseReenumerating,
        Self::ClassificationPhasePostWriteVerification,
        Self::ClassificationPhaseStable,
        Self::ActionRefresh,
        Self::ActionRenewDhcp,
        Self::ActionApplyDnsAutomatic,
        Self::ActionApplyDnsStatic,
        Self::ActionRestartAdapter,
        Self::ActionReenumerateDevice,
        Self::ActionRestartModule,
        Self::ActionEditApn,
        Self::ActionSetUsbProfileDjiNdis,
        Self::ActionSetUsbProfileEcm,
        Self::ActionEnableHotspot,
        Self::ActionDisableHotspot,
        Self::RiskLevelLow,
        Self::RiskLevelMedium,
        Self::RiskLevelHigh,
        Self::OperationOutcomeApplied,
        Self::OperationOutcomeFailed,
        Self::OperationOutcomeUnknown,
        Self::DnsProfileAutomatic,
        Self::DnsProfileStatic,
        Self::UsbNetworkProfileDjiNdis,
        Self::UsbNetworkProfileEcm,
        Self::DisruptionNone,
        Self::DisruptionBrief,
        Self::DisruptionConnectionInterrupting,
        Self::DisruptionDeviceReenumeration,
        Self::ActionSafetyUnsupportedDevice,
        Self::ActionSafetyStaleEpoch,
        Self::ActionSafetyStaleSnapshot,
        Self::ActionSafetyTargetIdentityChanged,
        Self::ActionSafetyBeforeStateChanged,
        Self::ActionSafetyExpired,
        Self::RollbackNotRequired,
        Self::RollbackApplied,
        Self::RollbackFailed,
        Self::RollbackNotAttempted,
        Self::FreshnessFresh,
        Self::FreshnessStale,
        Self::FreshnessUnknown,
        Self::LastObservedAt,
        Self::ObservedAgo,
        Self::ErrorPermissionDenied,
        Self::ErrorDeviceRemoved,
        Self::ErrorDeviceIdentityChanged,
        Self::ErrorEvidenceExpired,
        Self::ErrorProbeFailed,
        Self::ErrorDnsFailed,
        Self::ErrorTimeout,
        Self::ErrorUnsupported,
        Self::ErrorCapabilityUnavailable,
        Self::ErrorOperationCancelled,
        Self::ErrorVerificationFailed,
        Self::ErrorRollbackFailed,
        Self::ErrorInternal,
        Self::ErrorHelperUnsigned,
        Self::ErrorHelperUnverified,
        Self::SimReady,
        Self::SimMissing,
        Self::SimPinRequired,
        Self::SimPukRequired,
        Self::SimRejected,
        Self::SimUnknown,
        Self::RegistrationHome,
        Self::RegistrationRoaming,
        Self::RegistrationSearching,
        Self::RegistrationDenied,
        Self::RegistrationNotRegistered,
        Self::RegistrationUnknown,
        Self::AttachAttached,
        Self::AttachDetached,
        Self::AttachUnknown,
        Self::CellularBlockSimRejected,
        Self::CellularBlockRegistrationRejected,
        Self::DevicePresenceSupported,
        Self::DevicePresenceNotDetected,
        Self::DevicePresenceUnsupported,
        Self::DevicePresencePermissionDenied,
        Self::AdapterUsableAddressAndRoute,
        Self::AdapterNoUsableAddressOrRoute,
        Self::BoundPublicSucceeded,
        Self::BoundPublicFailed,
        Self::BoundPublicIncomplete,
        Self::BoundDnsSucceeded,
        Self::BoundDnsFailed,
        Self::BoundDnsIncomplete,
        Self::ProtocolCoverageAllRequired,
        Self::ProtocolCoverageSingleFamily,
        Self::AtControlAvailable,
        Self::AtControlUnavailable,
        Self::DefaultRouteTargetAdapter,
        Self::DefaultRouteVpnOrTun,
        Self::DefaultRouteOther,
        Self::GlobalConnectivityOnline,
        Self::GlobalConnectivityOffline,
        Self::ProtocolApnEmpty,
        Self::ProtocolApnTooLong,
        Self::ProtocolApnUnsafeCharacter,
        Self::ProtocolPdpContextIdOutOfRange,
        Self::ProtocolWrongPortData,
        Self::ProtocolLineTooLong,
        Self::ProtocolResponseTooLarge,
        Self::ProtocolTimeout,
        Self::ProtocolDeviceRemoved,
        Self::ProtocolUnexpectedData,
        Self::AtFinalOk,
        Self::AtFinalError,
        Self::AtFinalCmeError,
        Self::AtFinalCmsError,
        Self::AtFinalNoCarrier,
        Self::AtFinalNoAnswer,
        Self::AtFinalBusy,
        Self::AtFinalNoDialTone,
        Self::PlatformNoSafeAtPort,
        Self::PlatformAmbiguousAtPort,
        Self::PlatformAtPortUnverified,
        Self::PlatformUnsupportedPlatform,
        Self::PlatformPnpEnumerateFailed,
        Self::PlatformInterfaceEnumerateFailed,
        Self::PlatformPnpPermissionDenied,
        Self::PlatformPnpOpenFailed,
        Self::SerialQueueFull,
        Self::SerialSessionClosed,
        Self::SerialIoFailed,
        Self::SerialAtFinalError,
        Self::NavOverview,
        Self::NavDiagnostics,
        Self::NavRepairs,
        Self::NavSettings,
        Self::DiagnosticsTitle,
        Self::DiagnosticsIntro,
        Self::FieldDeviceIdentity,
        Self::FieldDeviceModel,
        Self::FieldUsbIdentity,
        Self::FieldProblemCode,
        Self::FieldAtPort,
        Self::FieldAdapter,
        Self::FieldCarrier,
        Self::FieldRadioAccessTechnology,
        Self::FieldSignal,
        Self::FieldSimState,
        Self::FieldRegistration,
        Self::FieldAttachState,
        Self::FieldApn,
        Self::FieldPdpAddress,
        Self::FieldWindowsAddresses,
        Self::FieldGateway,
        Self::FieldDnsServers,
        Self::FieldDefaultRoute,
        Self::FieldBoundGatewayProbe,
        Self::FieldBoundPublicProbe,
        Self::FieldBoundDnsProbe,
        Self::FieldProtocolCoverage,
        Self::FieldGlobalConnectivity,
        Self::FieldHotspot,
        Self::FieldEvidenceSource,
        Self::FieldObservedAt,
        Self::FieldPhoneNumber,
        Self::FieldNumberSource,
        Self::FieldVerificationState,
        Self::FieldCaptureTime,
        Self::FieldIccid,
        Self::ValueUnknown,
        Self::ValueNotAvailable,
        Self::ValueRedacted,
        Self::ValueNotApplicable,
        Self::ValueNumberNotProvided,
        Self::ValuePhoneNumberNotRead,
        Self::ValueNumberSourceSimReport,
        Self::ValueVerificationNotCarrierChecked,
        Self::ValueCaptureTimeSimSession,
        Self::ValueIccidNotRead,
        Self::IdentityHeading,
        Self::ButtonShow,
        Self::ButtonCopy,
        Self::ButtonCopied,
        Self::ServingCellLayoutProvisional,
        Self::FeatureStatusUnsupportedConfirmed,
        Self::FeatureStatusFormatMismatch,
        Self::FeatureStatusTransportFailure,
        Self::FeatureStatusTemporarilyUnavailable,
        Self::CheckPassed,
        Self::CheckFailed,
        Self::CheckUnavailable,
        Self::CheckUnexecuted,
        Self::CheckRunning,
        Self::CheckExpired,
        Self::UnexecutedDisabledBySetting,
        Self::UnexecutedNotScheduled,
        Self::UnexecutedSuperseded,
        Self::AppTitle,
        Self::UnofficialNotice,
        Self::OverviewQuestion,
        Self::OverviewLastObservation,
        Self::RateCaptionDown,
        Self::RateCaptionUp,
        Self::RateWindow,
        Self::RatePeak,
        Self::RateSampling,
        Self::RateGradeChip,
        Self::RateGradePending,
        Self::RateGradeIdle,
        Self::RateGradeBasic,
        Self::RateGradeGood,
        Self::RateGradeExcellent,
        Self::RateGradeVeryFast,
        Self::ButtonRefresh,
        Self::ButtonDiagnostics,
        Self::ButtonRepair,
        Self::ButtonConfirm,
        Self::ButtonCancel,
        Self::ButtonClose,
        Self::ButtonBack,
        Self::ButtonRetry,
        Self::ButtonDone,
        Self::ButtonViewDiagnostics,
        Self::ButtonCopyAddress,
        Self::ButtonExportDiagnostics,
        Self::ButtonOpenReleases,
        Self::StatusLoading,
        Self::StatusNoActiveOperation,
        Self::StatusExpired,
        Self::StatusQueueFull,
        Self::CommandFeedbackBusy,
        Self::CommandFeedbackConfirmRejected,
        Self::CommandFeedbackRejected,
        Self::StatusBackendUnavailable,
        Self::UnknownBackendError,
        Self::SystemErrorNumber,
        Self::TrayOpen,
        Self::TrayRefreshNow,
        Self::TrayHotspotStatus,
        Self::TrayExit,
        Self::TrayUnavailableFallback,
        Self::CloseToTrayHint,
        Self::SettingsTitle,
        Self::SettingsLanguage,
        Self::LanguageZhCn,
        Self::LanguageEnUs,
        Self::SettingsAutostart,
        Self::SettingsAutostartDescription,
        Self::SettingsStartMinimized,
        Self::SettingsActiveProbe,
        Self::SettingsActiveProbeDescription,
        Self::SettingsLogLevel,
        Self::SettingsLogLevelRestart,
        Self::LogLevelError,
        Self::LogLevelWarn,
        Self::LogLevelInfo,
        Self::LogLevelDebug,
        Self::SettingsPrivacy,
        Self::SettingsPrivacyDescription,
        Self::SettingsConfigDrift,
        Self::SettingsSaved,
        Self::SettingsSaveFailed,
        Self::SettingsCorruptConfig,
        Self::SettingsAutostartLoading,
        Self::SettingsAutostartSaving,
        Self::SettingsAutostartNotOwned,
        Self::SettingsPathUnavailable,
        Self::SettingsReadFailed,
        Self::SingleInstanceActivationFailed,
        Self::LoggingInitFailed,
        Self::LoggingRotationFailed,
        Self::RepairsTitle,
        Self::RepairsReadOnlyNotice,
        Self::RepairsDriverNotIncluded,
        Self::ConfirmationTitle,
        Self::ConfirmationOperation,
        Self::ConfirmationTarget,
        Self::ConfirmationExpectedEffect,
        Self::ConfirmationInterruption,
        Self::ConfirmationRisk,
        Self::ConfirmationElevationRequired,
        Self::ConfirmationElevationNotRequired,
        Self::ConfirmationStateRecheck,
        Self::ConfirmationNoAutomaticRetry,
        Self::ConfirmationApnContext,
        Self::ConfirmationApnNewValue,
        Self::OperationPreparing,
        Self::OperationRevalidating,
        Self::OperationAwaitingElevation,
        Self::OperationExecuting,
        Self::OperationVerifying,
        Self::OperationUacCancelled,
        Self::OperationDeviceRemoved,
        Self::OperationAuditRecorded,
        Self::NoPreparedAction,
        Self::PreparedActionAwaitingConfirmation,
        Self::PlanExpired,
        Self::ConfirmationDevModeWarning,
        Self::OperationResultTitle,
        Self::DiagnosticsExportTitle,
        Self::DiagnosticsExportDescription,
        Self::DiagnosticsExportRedactionNotice,
        Self::DiagnosticsExportSuccess,
        Self::DiagnosticsExportFailed,
        Self::BuildDevelopmentUnsigned,
        Self::BuildStableSigned,
        Self::FeatureUnavailablePortable,
        Self::UiCjkFontUnavailable,
        Self::NoAutomaticUpdate,
        Self::DemoUsage,
        Self::DemoRejectedRelease,
        Self::DemoInvalidScenario,
        Self::NavSms,
        Self::NavDeviceTools,
        Self::SmsTitle,
        Self::SmsIntro,
        Self::ButtonSmsRefresh,
        Self::FieldSmsStatus,
        Self::FieldSmsMessageCount,
        Self::FieldSmsUnreadCount,
        Self::FieldSmsCapacity,
        Self::SmsCapacityUsed,
        Self::SmsStatusNotQueried,
        Self::SmsStatusRead,
        Self::SmsIncompleteWarning,
        Self::SmsEmpty,
        Self::SmsListPending,
        Self::SmsUnread,
        Self::SmsRead,
        Self::FieldSmsSender,
        Self::FieldSmsTime,
        Self::FieldSmsEncoding,
        Self::FieldSmsParts,
        Self::FieldSmsBody,
        Self::SmsEncodingOther,
        Self::SmsReadNote,
        Self::ButtonSmsDelete,
        Self::ButtonSmsDeleteConfirm,
        Self::ButtonSmsSend,
        Self::ButtonSmsSendConfirm,
        Self::SmsEvictedWarning,
        Self::FieldSmsRecipient,
        Self::SmsSendNotice,
        Self::SmsIncompleteTag,
        Self::ButtonSmsExpand,
        Self::ButtonSmsCollapse,
        Self::SmsInboxHeading,
        Self::SmsOutgoingSubmitted,
        Self::SmsOutgoingFailed,
        Self::SmsOutgoingUnknown,
        Self::SmsBodyCharCount,
        Self::SmsErrorPduModeRequired,
        Self::SmsErrorPduConfirmFailed,
        Self::SmsErrorInvalidMessage,
        Self::SmsErrorSendFailed,
        Self::SmsErrorTimeout,
        Self::SmsErrorDeviceRemoved,
        Self::SmsErrorUnsupported,
        Self::SmsErrorVerificationFailed,
        Self::SmsErrorInternal,
        Self::SmsErrorGeneric,
        Self::FieldTemperature,
        Self::TemperatureNotRead,
        Self::TemperatureSensorNote,
        Self::TemperatureSectionHeading,
        Self::TemperatureTrendWindow,
        Self::TemperatureTrendNote,
        Self::TemperatureTrendSampling,
        Self::TemperatureDeltaUp,
        Self::TemperatureDeltaDown,
        Self::TemperatureDeltaFlat,
        Self::TemperatureSensorsReported,
        Self::FieldAdapterErrors,
        Self::FieldAdapterDiscards,
        Self::FieldAdapterLinkRate,
        Self::AdapterRxTx,
        Self::AdapterLinkRateNote,
        Self::TimelineHeading,
        Self::TimelineEmpty,
        Self::TimelineSimChanged,
        Self::TimelineRegistrationChanged,
        Self::TimelineCellChanged,
        Self::TimelineDeviceRemoved,
        Self::TimelineDeviceArrived,
        Self::TimelineAdapterLinkChanged,
        Self::TimelineDnsChanged,
    ];
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalizedText {
    pub key: TextKey,
    pub text: String,
}

impl LocalizedText {
    #[must_use]
    pub fn new(language: Language, key: TextKey) -> Self {
        Self {
            key,
            text: template(language, key).to_owned(),
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl AsRef<str> for LocalizedText {
    fn as_ref(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for LocalizedText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextArgs {
    pub client_count: Option<u32>,
    pub count: Option<usize>,
    pub cid: Option<u8>,
    pub apn_masked: Option<String>,
    pub server_count: Option<usize>,
    pub profile: Option<LocalizedText>,
    pub operation: Option<LocalizedText>,
    pub time: Option<String>,
    pub age: Option<String>,
    pub detail: Option<String>,
    pub used: Option<u32>,
    pub total: Option<u32>,
    pub rx: Option<String>,
    pub tx: Option<String>,
}

impl TextArgs {
    #[must_use]
    pub fn client_count(value: u32) -> Self {
        Self {
            client_count: Some(value),
            ..Self::default()
        }
    }
    #[must_use]
    pub fn count(value: usize) -> Self {
        Self {
            count: Some(value),
            ..Self::default()
        }
    }
    #[must_use]
    pub fn cid(value: u8) -> Self {
        Self {
            cid: Some(value.clamp(1, 16)),
            ..Self::default()
        }
    }
    #[must_use]
    pub fn server_count(value: usize) -> Self {
        Self {
            server_count: Some(value),
            ..Self::default()
        }
    }
    #[must_use]
    pub fn operation(value: LocalizedText) -> Self {
        Self {
            operation: Some(value),
            ..Self::default()
        }
    }
    #[must_use]
    pub fn detail(value: impl Into<String>) -> Self {
        Self {
            detail: Some(value.into()),
            ..Self::default()
        }
    }
    #[must_use]
    pub fn time(value: impl Into<String>) -> Self {
        Self {
            time: Some(value.into()),
            ..Self::default()
        }
    }
    #[must_use]
    pub fn age(value: impl Into<String>) -> Self {
        Self {
            age: Some(value.into()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn used_total(used: u32, total: u32) -> Self {
        Self {
            used: Some(used),
            total: Some(total),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn rx_tx(rx: impl Into<String>, tx: impl Into<String>) -> Self {
        Self {
            rx: Some(rx.into()),
            tx: Some(tx.into()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn apn_masked() -> Self {
        Self {
            apn_masked: Some("已隐藏".into()),
            ..Self::default()
        }
    }
}

/// The current release exposes only zh-CN. This is intentionally explicit so no half-translated
/// language can be selected by users or persisted by settings.
#[must_use]
pub const fn english_available() -> bool {
    false
}

#[must_use]
pub fn available_languages() -> &'static [Language] {
    &[Language::ZhCn]
}

/// Return the complete default catalog. `EnUs` deliberately uses the same safe catalog until a
/// complete English review is available; the language selector does not expose it meanwhile.
#[must_use]
pub fn template(_language: Language, key: TextKey) -> &'static str {
    match key {
        TextKey::AvailabilityDetectingTitle => "正在检测",
        TextKey::AvailabilityDetectingReason => "正在收集并校验当前设备的连接证据。",
        TextKey::AvailabilityAvailableTitle => "可用",
        TextKey::AvailabilityAvailableReason => {
            "已确认此模块的网络地址、路由、公共网络连接和 DNS 均可用。"
        }
        TextKey::AvailabilityLimitedTitle => "受限",
        TextKey::AvailabilityUnavailableTitle => "不可用",
        TextKey::AvailabilityNotDetectedTitle => "未检测到",
        TextKey::AvailabilityNotDetectedReason => {
            "当前枚举未发现受支持的 DJI 一代 4G 模块（VID 2CA3、PID 4006）。"
        }
        TextKey::AvailabilityUnsupportedTitle => "不受支持",
        TextKey::AvailabilityUnsupportedReason => {
            "检测到相关设备，但它不是受支持的一代模块（PID 4006）；不会执行写入或修复。"
        }
        TextKey::LimitedReasonDnsFailure => "模块数据通路可达，但通过该接口的 DNS 解析失败。",
        TextKey::LimitedReasonSingleProtocolFamily => {
            "模块仅通过一种所需的 IP 协议族完成连接验证。"
        }
        TextKey::LimitedReasonCompetingDefaultRoute => {
            "模块通路已通过验证，但系统默认路由由 VPN 或 TUN 接口占用。"
        }
        TextKey::LimitedReasonAtControlUnavailable => {
            "设备已识别，但 AT 端口不可用（串口异常），无法读取蜂窝状态。"
        }
        TextKey::LimitedReasonIncompleteEvidence => "现有证据不足，暂不能确认全部连接能力。",
        TextKey::UnavailableReasonCellularRejected => "SIM 或蜂窝网络注册被明确拒绝。",
        TextKey::UnavailableReasonNoUsableAddressOrRoute => "模块网卡没有可用的地址或路由。",
        TextKey::UnavailableReasonBoundPublicProbeFailed => {
            "经模块网卡绑定的公共网络探测已连续失败。"
        }
        TextKey::UnavailableReasonNoBoundReachability => {
            "Windows 可通过其他网络联网，但尚无证据证明此模块通路可达。"
        }
        TextKey::HotspotUnsupportedTitle => "热点不可用",
        TextKey::HotspotOff => "已关闭",
        TextKey::HotspotStarting => "正在开启",
        TextKey::HotspotOnWithClients => "已开启（{client_count} 台设备已连接）",
        TextKey::HotspotOnClientsUnknown => "已开启（连接设备数未知）",
        TextKey::HotspotStopping => "正在关闭",
        TextKey::HotspotFailed => "热点操作失败。",
        TextKey::HotspotUnsupportedMissingPackageIdentity => {
            "当前运行方式没有热点功能所需的应用包身份。"
        }
        TextKey::HotspotUnsupportedMissingWifiControlCapability => {
            "当前安装包未获得控制移动热点所需的系统能力。"
        }
        TextKey::HotspotUnsupportedNoWifiAdapter => "未检测到可用于共享网络的 Wi‑Fi 适配器。",
        TextKey::HotspotUnsupportedPolicyDisabled => "移动热点已被系统或组织策略禁用。",
        TextKey::HotspotUnsupportedOperatingSystem => "当前 Windows 版本不支持此热点控制方式。",
        TextKey::HotspotUnsupportedSourceProfileUnavailable => {
            "无法把移动热点的上行连接安全地绑定到此模块。"
        }
        TextKey::IssueSeverityInfo => "提示",
        TextKey::IssueSeverityWarning => "警告",
        TextKey::IssueSeverityError => "错误",
        TextKey::IssueLayerDevice => "USB 设备",
        TextKey::IssueLayerCellular => "蜂窝网络",
        TextKey::IssueLayerNetwork => "Windows 网络",
        TextKey::IssueLayerBoundProbe => "模块绑定探测",
        TextKey::IssueLayerHotspot => "移动热点",
        TextKey::IssueLayerOperation => "操作与修复",
        TextKey::EvidenceSourcePnp => "Windows 设备枚举",
        TextKey::EvidenceSourceAtControl => "AT 控制通道",
        TextKey::EvidenceSourceWindowsAdapter => "Windows 网卡状态",
        TextKey::EvidenceSourceBoundGatewayProbe => "模块网关绑定探测",
        TextKey::EvidenceSourceBoundDnsProbe => "模块 DNS 绑定探测",
        TextKey::EvidenceSourceBoundPublicProbe => "模块公共网络绑定探测",
        TextKey::EvidenceSourceGlobalRoute => "系统默认路由",
        TextKey::EvidenceSourceGlobalConnectivity => "Windows 全局联网状态",
        TextKey::EvidenceSourceHotspot => "Windows 移动热点",
        TextKey::ClassificationPhaseStartup => "正在启动检测",
        TextKey::ClassificationPhaseRecentInsertion => "已发现新插入的设备，正在检测",
        TextKey::ClassificationPhaseReenumerating => "设备正在重新枚举",
        TextKey::ClassificationPhasePostWriteVerification => "正在验证操作后的状态",
        TextKey::ClassificationPhaseStable => "检测完成",
        TextKey::ActionRefresh => "刷新并重新检测",
        TextKey::ActionRenewDhcp => "更新模块网卡的 DHCP 租约",
        TextKey::ActionApplyDnsAutomatic => "恢复自动获取 DNS",
        TextKey::ActionApplyDnsStatic => "应用静态 DNS（{server_count} 个服务器）",
        TextKey::ActionRestartAdapter => "重启模块网卡",
        TextKey::ActionReenumerateDevice => "重新枚举模块设备",
        TextKey::ActionRestartModule => "重启蜂窝模块",
        TextKey::ActionEditApn => "修改 PDP 上下文 {cid} 的 APN",
        TextKey::ActionSetUsbProfileDjiNdis => "切换为电脑网卡（DJI NDIS）",
        TextKey::ActionSetUsbProfileEcm => "切换为 ECM 网卡",
        TextKey::ActionEnableHotspot => "开启移动热点",
        TextKey::ActionDisableHotspot => "关闭移动热点",
        TextKey::RiskLevelLow => "低风险",
        TextKey::RiskLevelMedium => "中等风险",
        TextKey::RiskLevelHigh => "高风险",
        TextKey::OperationOutcomeApplied => "操作已应用，并已完成状态回读。",
        TextKey::OperationOutcomeFailed => "操作失败。",
        TextKey::OperationOutcomeUnknown => {
            "无法确认操作结果；系统不会自动重试。请刷新后核对设备状态。"
        }
        TextKey::DnsProfileAutomatic => "自动获取 DNS",
        TextKey::DnsProfileStatic => "静态 DNS",
        TextKey::UsbNetworkProfileDjiNdis => "项目已验证的 DJI NDIS 配置",
        TextKey::UsbNetworkProfileEcm => "项目已验证的 ECM 配置",
        TextKey::DisruptionNone => "不会中断连接",
        TextKey::DisruptionBrief => "连接可能短暂波动",
        TextKey::DisruptionConnectionInterrupting => "将暂时中断网络连接",
        TextKey::DisruptionDeviceReenumeration => "设备将断开并重新出现",
        TextKey::ActionSafetyUnsupportedDevice => "目标不是受支持的 DJI 一代 4G 模块，操作已阻止。",
        TextKey::ActionSafetyStaleEpoch => "设备已重新连接或重新枚举，请重新准备操作。",
        TextKey::ActionSafetyStaleSnapshot => "设备状态已经变化，请刷新后重试。",
        TextKey::ActionSafetyTargetIdentityChanged => "目标设备身份已经变化，操作已阻止。",
        TextKey::ActionSafetyBeforeStateChanged => "操作前状态已经变化，请重新确认。",
        TextKey::ActionSafetyExpired => "此确认已过期，请重新准备操作。",
        TextKey::RollbackNotRequired => "无需回滚",
        TextKey::RollbackApplied => "已恢复原状态",
        TextKey::RollbackFailed => "回滚失败，请检查当前状态",
        TextKey::RollbackNotAttempted => "未执行回滚",
        TextKey::FreshnessFresh => "状态为最新",
        TextKey::FreshnessStale => "状态已过期，正在重新检测",
        TextKey::FreshnessUnknown => "尚无有效的更新时间",
        TextKey::LastObservedAt => "上次检测：{time}",
        TextKey::ObservedAgo => "更新于 {age}前",
        TextKey::ErrorPermissionDenied => "权限不足，无法完成此操作。",
        TextKey::ErrorDeviceRemoved => "操作期间设备已断开。",
        TextKey::ErrorDeviceIdentityChanged => "设备身份已经变化，操作已停止。",
        TextKey::ErrorEvidenceExpired => "检测依据已过期，请刷新状态。",
        TextKey::ErrorProbeFailed => "网络探测未成功完成。",
        TextKey::ErrorDnsFailed => "通过模块接口的 DNS 解析失败。",
        TextKey::ErrorTimeout => "操作等待超时。",
        TextKey::ErrorUnsupported => "当前设备、系统或操作不受支持。",
        TextKey::ErrorCapabilityUnavailable => "所需的系统能力当前不可用。",
        TextKey::ErrorOperationCancelled => "操作已取消。",
        TextKey::ErrorVerificationFailed => "操作后的状态验证未通过。",
        TextKey::ErrorRollbackFailed => "未能恢复操作前的状态，请检查当前配置。",
        TextKey::ErrorInternal => "应用发生内部错误。请刷新状态；如仍出现，请导出诊断信息。",
        TextKey::ErrorHelperUnsigned => "helper 未签名；特权修复保持关闭（开发候选不包含签名）。",
        TextKey::ErrorHelperUnverified => "无法运行签名校验；特权修复保持关闭。",
        TextKey::SimReady => "SIM 已就绪",
        TextKey::SimMissing => "未检测到 SIM",
        TextKey::SimPinRequired => "SIM 需要 PIN（本应用不会提交 PIN）",
        TextKey::SimPukRequired => "SIM 需要 PUK（本应用不会提交 PUK）",
        TextKey::SimRejected => "SIM 被拒绝",
        TextKey::SimUnknown => "SIM 状态未知",
        TextKey::RegistrationHome => "已注册到本地网络",
        TextKey::RegistrationRoaming => "已注册到漫游网络",
        TextKey::RegistrationSearching => "正在搜索网络",
        TextKey::RegistrationDenied => "网络注册被拒绝",
        TextKey::RegistrationNotRegistered => "尚未注册到网络",
        TextKey::RegistrationUnknown => "注册状态未知",
        TextKey::AttachAttached => "分组数据已附着",
        TextKey::AttachDetached => "分组数据未附着",
        TextKey::AttachUnknown => "分组数据附着状态未知",
        TextKey::CellularBlockSimRejected => "SIM 被明确拒绝",
        TextKey::CellularBlockRegistrationRejected => "蜂窝网络注册被明确拒绝",
        TextKey::DevicePresenceSupported => "已检测到受支持的 DJI 一代 4G 模块",
        TextKey::DevicePresenceNotDetected => "未检测到受支持的模块",
        TextKey::DevicePresenceUnsupported => "检测到相关但不受支持的 USB 设备",
        TextKey::DevicePresencePermissionDenied => "无法读取设备信息：权限不足",
        TextKey::AdapterUsableAddressAndRoute => "网卡具有可用地址和路由",
        TextKey::AdapterNoUsableAddressOrRoute => "网卡没有可用地址或路由",
        TextKey::BoundPublicSucceeded => "模块绑定的公共网络探测通过",
        TextKey::BoundPublicFailed => "模块绑定的公共网络探测失败（连续 {count} 次）",
        TextKey::BoundPublicIncomplete => "公共网络探测尚未完成",
        TextKey::BoundDnsSucceeded => "模块绑定的 DNS 解析通过",
        TextKey::BoundDnsFailed => "模块绑定的 DNS 解析失败",
        TextKey::BoundDnsIncomplete => "DNS 探测尚未完成",
        TextKey::ProtocolCoverageAllRequired => "所需 IP 协议族均通过验证",
        TextKey::ProtocolCoverageSingleFamily => "仅一种所需 IP 协议族通过验证",
        TextKey::AtControlAvailable => "AT 控制通道可用",
        TextKey::AtControlUnavailable => "AT 控制通道不可用",
        TextKey::DefaultRouteTargetAdapter => "系统默认路由由模块网卡提供",
        TextKey::DefaultRouteVpnOrTun => "系统默认路由由 VPN 或 TUN 接口提供",
        TextKey::DefaultRouteOther => "系统默认路由由其他网络接口提供",
        TextKey::GlobalConnectivityOnline => "Windows 当前可通过某个网络联网",
        TextKey::GlobalConnectivityOffline => "Windows 当前未检测到全局联网",
        TextKey::ProtocolApnEmpty => "APN 不能为空。",
        TextKey::ProtocolApnTooLong => "APN 不能超过 100 个 ASCII 字节。",
        TextKey::ProtocolApnUnsafeCharacter => {
            "APN 含有不允许的字符；请使用不含引号、逗号、分号或控制字符的 ASCII 文本。"
        }
        TextKey::ProtocolPdpContextIdOutOfRange => "PDP 上下文编号必须在 1 到 16 之间。",
        TextKey::ProtocolWrongPortData => "当前串口返回了非 AT 数据，已停止使用该端口。",
        TextKey::ProtocolLineTooLong => "模块返回的数据行超过安全长度限制。",
        TextKey::ProtocolResponseTooLarge => "模块响应超过安全大小限制。",
        TextKey::ProtocolTimeout => "等待模块响应超时。",
        TextKey::ProtocolDeviceRemoved => "等待响应时设备已断开。",
        TextKey::ProtocolUnexpectedData => "模块返回了无法安全解析的数据。",
        TextKey::AtFinalOk => "模块已确认命令",
        TextKey::AtFinalError => "模块拒绝了命令",
        TextKey::AtFinalCmeError => "模块返回 CME 错误（{detail}）",
        TextKey::AtFinalCmsError => "模块返回 CMS 错误（{detail}）",
        TextKey::AtFinalNoCarrier => "未建立载波连接",
        TextKey::AtFinalNoAnswer => "对端无响应",
        TextKey::AtFinalBusy => "模块当前忙",
        TextKey::AtFinalNoDialTone => "未检测到拨号音",
        TextKey::PlatformNoSafeAtPort => "未找到可安全使用的 AT 端口；不会尝试未知端口。",
        TextKey::PlatformAmbiguousAtPort => "找到多个同等候选的 AT 端口，为避免误操作已禁用写入。",
        TextKey::PlatformAtPortUnverified => {
            "找到多个候选端口，安全握手均未确认 AT 协议，为避免误操作已禁用写入。"
        }
        TextKey::PlatformUnsupportedPlatform => "当前平台不支持 Windows 设备枚举。",
        TextKey::PlatformPnpEnumerateFailed => "无法完成 Windows 设备枚举。",
        TextKey::PlatformInterfaceEnumerateFailed => "无法枚举设备接口。",
        TextKey::PlatformPnpPermissionDenied => "Windows 拒绝读取设备信息。",
        TextKey::PlatformPnpOpenFailed => "无法打开 Windows 设备信息集。",
        TextKey::SerialQueueFull => "AT 请求队列已满，请稍后重试。",
        TextKey::SerialSessionClosed => "AT 会话已关闭。",
        TextKey::SerialIoFailed => "与模块串口通信失败。",
        TextKey::SerialAtFinalError => "模块未接受该 AT 命令。",
        TextKey::NavOverview => "概览",
        TextKey::NavDiagnostics => "诊断",
        TextKey::NavRepairs => "修复",
        TextKey::NavSettings => "设置",
        TextKey::DiagnosticsTitle => "连接证据",
        TextKey::DiagnosticsIntro => {
            "以下检查按设备、蜂窝网络、Windows 网卡和模块绑定探测分层显示。"
        }
        TextKey::FieldDeviceIdentity => "设备身份",
        TextKey::FieldDeviceModel => "设备型号",
        TextKey::FieldUsbIdentity => "USB 标识",
        TextKey::FieldProblemCode => "Windows 问题代码",
        TextKey::FieldAtPort => "AT 端口",
        TextKey::FieldAdapter => "模块网卡",
        TextKey::FieldCarrier => "运营商",
        TextKey::FieldRadioAccessTechnology => "接入制式",
        TextKey::FieldSignal => "信号",
        TextKey::FieldSimState => "SIM 状态",
        TextKey::FieldRegistration => "网络注册",
        TextKey::FieldAttachState => "分组数据附着",
        TextKey::FieldApn => "接入点（APN）",
        TextKey::FieldPdpAddress => "PDP 地址",
        TextKey::FieldWindowsAddresses => "Windows 地址",
        TextKey::FieldGateway => "网关",
        TextKey::FieldDnsServers => "DNS 服务器",
        TextKey::FieldDefaultRoute => "系统默认路由",
        TextKey::FieldBoundGatewayProbe => "模块网关探测",
        TextKey::FieldBoundPublicProbe => "模块公共网络探测",
        TextKey::FieldBoundDnsProbe => "模块 DNS 探测",
        TextKey::FieldProtocolCoverage => "IP 协议覆盖",
        TextKey::FieldGlobalConnectivity => "Windows 全局联网状态",
        TextKey::FieldHotspot => "移动热点",
        TextKey::FieldEvidenceSource => "证据来源",
        TextKey::FieldObservedAt => "检测时间",
        TextKey::FieldPhoneNumber => "本机号码",
        TextKey::FieldNumberSource => "来源",
        TextKey::FieldVerificationState => "验证状态",
        TextKey::FieldCaptureTime => "采集时间",
        TextKey::FieldIccid => "SIM 卡 ICCID",
        TextKey::ValueUnknown => "未知",
        TextKey::ValueNotAvailable => "未获取",
        TextKey::ValueRedacted => "已隐藏",
        TextKey::ValueNotApplicable => "不适用",
        TextKey::ValueNumberNotProvided => "SIM/设备未提供本机号码",
        TextKey::ValuePhoneNumberNotRead => "未读取到本机号码",
        TextKey::ValueNumberSourceSimReport => "SIM/设备报告",
        TextKey::ValueVerificationNotCarrierChecked => "未通过运营商账户核验",
        TextKey::ValueCaptureTimeSimSession => "本次 SIM 会话内",
        TextKey::ValueIccidNotRead => "未读取到 ICCID",
        TextKey::IdentityHeading => "身份信息",
        TextKey::ButtonShow => "显示",
        TextKey::ButtonCopy => "复制",
        TextKey::ButtonCopied => "已复制",
        TextKey::ServingCellLayoutProvisional => "服务小区字段布局为候选方案，待实机确认",
        TextKey::FeatureStatusUnsupportedConfirmed => {
            "固件不支持此查询（设备已返回明确的不支持错误）"
        }
        TextKey::FeatureStatusFormatMismatch => "格式不匹配：设备有应答，但响应格式未被识别",
        TextKey::FeatureStatusTransportFailure => "本次超时：查询未完成（设备无应答或已断开）",
        TextKey::FeatureStatusTemporarilyUnavailable => "暂时不可用：本次查询未成功，原因尚未确认",
        TextKey::CheckPassed => "已通过",
        TextKey::CheckFailed => "未通过",
        TextKey::CheckUnavailable => "不可用",
        TextKey::CheckUnexecuted => "未执行",
        TextKey::CheckRunning => "检测中",
        TextKey::CheckExpired => "已过期",
        TextKey::UnexecutedDisabledBySetting => "未执行：已被设置关闭",
        TextKey::UnexecutedNotScheduled => "未执行：尚未排程",
        TextKey::UnexecutedSuperseded => "未执行：已被更新的检测取代",
        TextKey::AppTitle => "DJI 一代 4G 面板",
        TextKey::UnofficialNotice => {
            "非官方开源工具，与 DJI、白旺、Quectel、Microsoft 或运营商无隶属或认可关系。"
        }
        TextKey::OverviewQuestion => "此模块当前能否作为 Windows 的可用网络上行？",
        TextKey::OverviewLastObservation => "上次检测：{time}",
        TextKey::RateCaptionDown => "下载",
        TextKey::RateCaptionUp => "上传",
        // {age} is derived from the ring (capacity × cadence), never hardcoded at the call site.
        TextKey::RateWindow => "最近 {age}",
        TextKey::RatePeak => "峰值 {detail}",
        TextKey::RateSampling => "正在采样网速……（每秒一个点）",
        TextKey::RateGradeChip => "速度：{detail}",
        TextKey::RateGradePending => "待测速",
        TextKey::RateGradeIdle => "空闲",
        TextKey::RateGradeBasic => "基础",
        TextKey::RateGradeGood => "良好",
        TextKey::RateGradeExcellent => "优秀",
        TextKey::RateGradeVeryFast => "极速",
        TextKey::ButtonRefresh => "刷新",
        TextKey::ButtonDiagnostics => "诊断",
        TextKey::ButtonRepair => "修复",
        TextKey::ButtonConfirm => "确认",
        TextKey::ButtonCancel => "取消",
        TextKey::ButtonClose => "关闭",
        TextKey::ButtonBack => "返回",
        TextKey::ButtonRetry => "重试",
        TextKey::ButtonDone => "完成",
        TextKey::ButtonViewDiagnostics => "查看诊断",
        TextKey::ButtonCopyAddress => "复制地址",
        TextKey::ButtonExportDiagnostics => "导出诊断信息",
        TextKey::ButtonOpenReleases => "打开发布页面",
        TextKey::StatusLoading => "正在加载",
        TextKey::StatusNoActiveOperation => "当前没有正在执行的操作",
        TextKey::StatusExpired => "状态已过期",
        TextKey::StatusQueueFull => "请求队列已满，请稍后重试",
        TextKey::CommandFeedbackBusy => "已有操作正在执行，请稍候再试。",
        TextKey::CommandFeedbackConfirmRejected => "确认未生效：操作计划已失效，请重新准备。",
        TextKey::CommandFeedbackRejected => "操作未执行，请刷新后重试。",
        TextKey::StatusBackendUnavailable => "后台暂不可用，请稍后重试",
        TextKey::UnknownBackendError => "发生未识别的错误。请刷新状态；如仍出现，请导出诊断信息。",
        TextKey::SystemErrorNumber => "系统错误编号：{detail}",
        TextKey::TrayOpen => "打开面板",
        TextKey::TrayRefreshNow => "立即刷新",
        TextKey::TrayHotspotStatus => "热点状态",
        TextKey::TrayExit => "退出",
        TextKey::TrayUnavailableFallback => "无法创建系统托盘图标，窗口将保持显示。",
        TextKey::CloseToTrayHint => "窗口已隐藏到系统托盘。",
        TextKey::SettingsTitle => "设置",
        TextKey::SettingsLanguage => "界面语言",
        TextKey::LanguageZhCn => "简体中文",
        TextKey::LanguageEnUs => "English（稍后提供）",
        TextKey::SettingsAutostart => "登录 Windows 时启动",
        TextKey::SettingsAutostartDescription => "默认关闭；启用后将直接启动到系统托盘。",
        TextKey::SettingsStartMinimized => "启动时隐藏到托盘",
        TextKey::SettingsActiveProbe => "允许主动连接探测",
        TextKey::SettingsActiveProbeDescription => {
            "使用模块网卡进行小流量、严格绑定的联网与 DNS 检查。"
        }
        TextKey::SettingsLogLevel => "日志详细程度",
        TextKey::SettingsLogLevelRestart => "更改将在下次启动时生效。",
        TextKey::LogLevelError => "仅错误",
        TextKey::LogLevelWarn => "警告及以上",
        TextKey::LogLevelInfo => "常规",
        TextKey::LogLevelDebug => "调试",
        TextKey::SettingsPrivacy => "隐私",
        TextKey::SettingsPrivacyDescription => "诊断信息默认脱敏，不会自动上传。",
        TextKey::SettingsConfigDrift => "启动项与当前程序位置不一致，请重新启用自动启动。",
        TextKey::SettingsSaved => "设置已保存",
        TextKey::SettingsSaveFailed => "无法保存设置。",
        TextKey::SettingsCorruptConfig => "配置文件已损坏，已保留原文件并恢复安全默认值。",
        TextKey::SettingsAutostartLoading => "正在读取启动设置。",
        TextKey::SettingsAutostartSaving => "正在保存启动设置。",
        TextKey::SettingsAutostartNotOwned => "启动项内容与当前程序不一致，未自动删除。",
        TextKey::SettingsPathUnavailable => "无法确定设置目录，设置不会持久化。",
        TextKey::SettingsReadFailed => "无法读取设置，已使用安全默认值；原文件未覆盖。",
        TextKey::SingleInstanceActivationFailed => "已有面板正在运行，但无法唤醒它。",
        TextKey::LoggingInitFailed => "无法启用本地日志；应用仍可运行。",
        TextKey::LoggingRotationFailed => "日志轮换失败；应用仍可运行。",
        TextKey::RepairsTitle => "修复操作",
        TextKey::RepairsReadOnlyNotice => "只有在目标身份和当前证据均有效时，才会启用相应操作。",
        TextKey::RepairsDriverNotIncluded => "驱动安装需单独确认；正常工作的接口无需重装。",
        TextKey::ConfirmationTitle => "确认执行",
        TextKey::ConfirmationOperation => "操作：{operation}",
        TextKey::ConfirmationTarget => "目标：DJI 一代 4G 模块（VID 2CA3、PID 4006）",
        TextKey::ConfirmationExpectedEffect => "预期效果",
        TextKey::ConfirmationInterruption => "连接影响",
        TextKey::ConfirmationElevation => "提权要求",
        TextKey::ConfirmationRisk => "风险级别",
        TextKey::ConfirmationElevationRequired => "此操作需要 Windows 管理员授权。",
        TextKey::ConfirmationElevationNotRequired => "此操作不需要管理员授权。",
        TextKey::ConfirmationStateRecheck => "执行前将再次核对设备身份和当前状态。",
        TextKey::ConfirmationNoAutomaticRetry => "写入操作只执行一次；超时后不会自动重试。",
        TextKey::ConfirmationApnContext => "PDP 上下文：{cid}",
        TextKey::ConfirmationApnNewValue => "新 APN：{apn_masked}",
        TextKey::OperationPreparing => "正在准备操作",
        TextKey::OperationRevalidating => "正在重新核对目标状态",
        TextKey::OperationAwaitingElevation => "等待管理员授权",
        TextKey::OperationExecuting => "正在执行：{operation}",
        TextKey::OperationVerifying => "正在重新检测并验证结果",
        TextKey::OperationUacCancelled => "管理员授权已取消，未执行操作。",
        TextKey::OperationDeviceRemoved => "设备已断开，操作已停止。",
        TextKey::OperationAuditRecorded => "操作结果已记录到本地审计日志。",
        TextKey::NoPreparedAction => "当前没有可确认的操作计划。",
        TextKey::PreparedActionAwaitingConfirmation => "操作已准备，等待你的确认。",
        TextKey::PlanExpired => "此操作计划已过期，请重新准备。",
        TextKey::OperationResultTitle => "操作结果",
        TextKey::ConfirmationDevModeWarning => {
            "开发构建：dji4g-helper.exe 未签名，仅供开发测试，请谨慎操作。"
        }
        TextKey::DiagnosticsExportTitle => "导出诊断信息",
        TextKey::DiagnosticsExportDescription => {
            "将生成一份便于阅读的报告和一份结构化数据文件；默认隐藏敏感标识。"
        }
        TextKey::DiagnosticsExportRedactionNotice => {
            "完整 IMEI、IMSI、ICCID、电话号码、PIN/PUK、原始串口数据和完整配置不会写入导出。"
        }
        // No template argument exists for the destination, so the path is stated literally; the
        // production export directory is always `%LOCALAPPDATA%\Dji4GPanel\exports`.
        TextKey::DiagnosticsExportSuccess => {
            "诊断信息已导出到 %LOCALAPPDATA%\\Dji4GPanel\\exports。"
        }
        TextKey::DiagnosticsExportFailed => "无法导出诊断信息。",
        TextKey::BuildDevelopmentUnsigned => "开发版（未签名）",
        TextKey::BuildStableSigned => "稳定版（已签名）",
        TextKey::FeatureUnavailablePortable => "当前运行方式不提供此功能。",
        TextKey::UiCjkFontUnavailable => {
            "未找到可用的 Windows 中文字体；界面文字可能无法完整显示。"
        }
        TextKey::NoAutomaticUpdate => "本应用不会自动更新。",
        TextKey::DemoUsage => "调试演示：available、limited、unavailable、absent 或 detecting",
        TextKey::DemoRejectedRelease => "发布版本不允许使用演示模式。",
        TextKey::DemoInvalidScenario => {
            "未知演示场景，请使用 available、limited、unavailable、absent 或 detecting。"
        }
        TextKey::NavSms => "短信",
        TextKey::NavDeviceTools => "设备工具",
        TextKey::SmsTitle => "短信",
        TextKey::SmsIntro => {
            "短信功能首次启用会把模块短信格式设为 PDU；读取消息可能将未读标记为已读。"
        }
        TextKey::ButtonSmsRefresh => "刷新短信",
        TextKey::FieldSmsStatus => "状态",
        TextKey::FieldSmsMessageCount => "消息数",
        TextKey::FieldSmsUnreadCount => "未读数",
        TextKey::FieldSmsCapacity => "容量",
        TextKey::SmsCapacityUsed => "已用 {used} / 总数 {total}",
        TextKey::SmsStatusNotQueried => "尚未查询",
        TextKey::SmsStatusRead => "已读取",
        TextKey::SmsIncompleteWarning => "存在未完整接收的长短信",
        TextKey::SmsEmpty => "暂无短信（或尚未刷新）",
        TextKey::SmsListPending => "点击「刷新短信」读取收件箱。",
        TextKey::SmsUnread => "未读",
        TextKey::SmsRead => "已读",
        TextKey::FieldSmsSender => "发送方",
        TextKey::FieldSmsTime => "时间",
        TextKey::FieldSmsEncoding => "编码",
        TextKey::FieldSmsParts => "分片",
        TextKey::FieldSmsBody => "正文",
        TextKey::SmsEncodingOther => "其他",
        TextKey::SmsReadNote => "读取可能已将其标记为已读",
        TextKey::ButtonSmsDelete => "删除",
        TextKey::ButtonSmsDeleteConfirm => "确认删除",
        TextKey::ButtonSmsSend => "发送短信",
        TextKey::ButtonSmsSendConfirm => "确认发送（可能产生费用）",
        TextKey::FieldSmsRecipient => "收件人",
        TextKey::SmsEvictedWarning => {
            "本地缓存已满，较早的 {count} 条消息已从本地视图移除（模块中可能仍存在）。"
        }
        TextKey::SmsSendNotice => {
            "发送可能产生费用；提交成功不代表对方收到。失败或超时不会自动重试。"
        }
        TextKey::SmsIncompleteTag => "未完整",
        TextKey::ButtonSmsExpand => "展开",
        TextKey::ButtonSmsCollapse => "收起",
        TextKey::SmsInboxHeading => "收件箱",
        TextKey::SmsOutgoingSubmitted => "已提交",
        TextKey::SmsOutgoingFailed => "发送失败",
        TextKey::SmsOutgoingUnknown => "结果未知",
        TextKey::SmsBodyCharCount => "字数 {count} / 70",
        TextKey::SmsErrorPduModeRequired => {
            "短信需要 PDU 模式：请先在短信页点击「刷新短信」启用（首次会切换模块短信格式）。"
        }
        TextKey::SmsErrorPduConfirmFailed => "切换 PDU 模式后未能确认，请重试刷新短信。",
        TextKey::SmsErrorInvalidMessage => {
            "短信内容或收件人不符合要求（收件人需为 + 开头的国际格式，正文 ≤140 字节且仅限 BMP 字符）。"
        }
        TextKey::SmsErrorSendFailed => "模块拒绝了本次短信提交。",
        TextKey::SmsErrorTimeout => "短信操作超时：结果可能未知，不会自动重试。",
        TextKey::SmsErrorDeviceRemoved => "短信操作期间设备已断开。",
        TextKey::SmsErrorUnsupported => "该固件不支持短信 AT 命令。",
        TextKey::SmsErrorVerificationFailed => "短信响应格式未被识别。",
        TextKey::SmsErrorInternal => "短信内部错误，请刷新后重试。",
        TextKey::SmsErrorGeneric => "短信操作失败。",
        TextKey::FieldTemperature => "温度",
        TextKey::TemperatureNotRead => "未读取到",
        TextKey::TemperatureSensorNote => "传感器定义以固件为准",
        TextKey::TemperatureSectionHeading => "模块温度",
        // {age} is derived from the ring (capacity × cadence), never hardcoded at the call site.
        TextKey::TemperatureTrendWindow => "最近 {age}",
        TextKey::TemperatureTrendNote => "折线为第 1 个报告值 · 每 {age}一个采样点",
        TextKey::TemperatureTrendSampling => "正在采样模块温度……（每个刷新周期一个点）",
        TextKey::TemperatureDeltaUp => "较上次 +{detail} °C",
        TextKey::TemperatureDeltaDown => "较上次 -{detail} °C",
        TextKey::TemperatureDeltaFlat => "与上次相同",
        // The channel order and meaning belong to the firmware: {count} values came back, and the
        // panel shows them in report order without claiming which sensor is which.
        TextKey::TemperatureSensorsReported => {
            "设备报告 {count} 个传感器值：{detail} °C · 顺序与含义以固件为准"
        }
        TextKey::FieldAdapterErrors => "接口错误",
        TextKey::FieldAdapterDiscards => "接口丢弃",
        TextKey::FieldAdapterLinkRate => "链路速率",
        TextKey::AdapterRxTx => "收 {rx} / 发 {tx}",
        TextKey::AdapterLinkRateNote => "接口链路速率，不是实测吞吐",
        TextKey::TimelineHeading => "网络变化记录",
        TextKey::TimelineEmpty => "暂无记录（仅记录观察到的变化）",
        TextKey::TimelineSimChanged => "SIM 已更换",
        TextKey::TimelineRegistrationChanged => "网络注册变化",
        TextKey::TimelineCellChanged => "服务小区变化",
        TextKey::TimelineDeviceRemoved => "设备已断开",
        TextKey::TimelineDeviceArrived => "设备已重新枚举",
        TextKey::TimelineAdapterLinkChanged => "网卡链路变化",
        TextKey::TimelineDnsChanged => "DNS 探测变化",
    }
}

#[must_use]
pub fn format_text(key: TextKey, args: &TextArgs) -> LocalizedText {
    format_text_in(Language::ZhCn, key, args)
}

#[must_use]
pub fn format_text_in(language: Language, key: TextKey, args: &TextArgs) -> LocalizedText {
    let mut output = template(language, key).to_owned();
    let replace = |output: &mut String, needle: &str, value: Option<String>| {
        if let Some(value) = value {
            let clean = value.replace(['\r', '\n'], " ");
            output.replace_range_if_present(needle, &clean);
        }
    };
    replace(
        &mut output,
        "{client_count}",
        args.client_count.map(|value| value.to_string()),
    );
    replace(
        &mut output,
        "{count}",
        args.count.map(|value| value.to_string()),
    );
    replace(
        &mut output,
        "{cid}",
        args.cid.map(|value| value.to_string()),
    );
    replace(
        &mut output,
        "{server_count}",
        args.server_count.map(|value| value.to_string()),
    );
    replace(
        &mut output,
        "{profile}",
        args.profile.as_ref().map(|value| value.text.clone()),
    );
    replace(
        &mut output,
        "{operation}",
        args.operation.as_ref().map(|value| value.text.clone()),
    );
    replace(&mut output, "{time}", args.time.clone());
    replace(&mut output, "{age}", args.age.clone());
    replace(
        &mut output,
        "{used}",
        args.used.map(|value| value.to_string()),
    );
    replace(
        &mut output,
        "{total}",
        args.total.map(|value| value.to_string()),
    );
    replace(&mut output, "{rx}", args.rx.clone());
    replace(&mut output, "{tx}", args.tx.clone());
    // APNs and arbitrary backend details are never displayed raw. Callers may pass a deliberately
    // masked value such as "已隐藏"; the replacement still strips line breaks and caps length.
    replace(
        &mut output,
        "{apn_masked}",
        args.apn_masked.as_ref().map(|_| "已隐藏".into()),
    );
    replace(
        &mut output,
        "{detail}",
        args.detail
            .clone()
            .map(|value| value.chars().take(32).collect()),
    );
    LocalizedText { key, text: output }
}

trait ReplaceRangeIfPresent {
    fn replace_range_if_present(&mut self, needle: &str, value: &str);
}

impl ReplaceRangeIfPresent for String {
    fn replace_range_if_present(&mut self, needle: &str, value: &str) {
        if self.contains(needle) {
            *self = self.replace(needle, value);
        }
    }
}

#[must_use]
pub fn availability_title(value: Availability) -> TextKey {
    match value {
        Availability::Detecting => TextKey::AvailabilityDetectingTitle,
        Availability::Available => TextKey::AvailabilityAvailableTitle,
        Availability::Limited(_) => TextKey::AvailabilityLimitedTitle,
        Availability::Unavailable(_) => TextKey::AvailabilityUnavailableTitle,
        Availability::NotDetected => TextKey::AvailabilityNotDetectedTitle,
        Availability::UnsupportedDevice => TextKey::AvailabilityUnsupportedTitle,
    }
}

#[must_use]
pub fn availability_reason(value: Availability) -> TextKey {
    match value {
        Availability::Detecting => TextKey::AvailabilityDetectingReason,
        Availability::Available => TextKey::AvailabilityAvailableReason,
        Availability::Limited(reason) => limited_reason(reason),
        Availability::Unavailable(reason) => unavailable_reason(reason),
        Availability::NotDetected => TextKey::AvailabilityNotDetectedReason,
        Availability::UnsupportedDevice => TextKey::AvailabilityUnsupportedReason,
    }
}

#[must_use]
pub fn limited_reason(value: LimitedReason) -> TextKey {
    match value {
        LimitedReason::DnsFailure => TextKey::LimitedReasonDnsFailure,
        LimitedReason::SingleProtocolFamily => TextKey::LimitedReasonSingleProtocolFamily,
        LimitedReason::CompetingDefaultRoute => TextKey::LimitedReasonCompetingDefaultRoute,
        LimitedReason::AtControlUnavailable => TextKey::LimitedReasonAtControlUnavailable,
        LimitedReason::IncompleteEvidence => TextKey::LimitedReasonIncompleteEvidence,
    }
}

#[must_use]
pub fn unavailable_reason(value: UnavailableReason) -> TextKey {
    match value {
        UnavailableReason::CellularRejected => TextKey::UnavailableReasonCellularRejected,
        UnavailableReason::NoUsableAddressOrRoute => {
            TextKey::UnavailableReasonNoUsableAddressOrRoute
        }
        UnavailableReason::BoundPublicProbeFailed => {
            TextKey::UnavailableReasonBoundPublicProbeFailed
        }
        UnavailableReason::NoBoundReachability => TextKey::UnavailableReasonNoBoundReachability,
    }
}

#[must_use]
pub fn hotspot_title(value: HotspotStatus) -> TextKey {
    match value {
        HotspotStatus::Unsupported(_) => TextKey::HotspotUnsupportedTitle,
        HotspotStatus::Off => TextKey::HotspotOff,
        HotspotStatus::Starting => TextKey::HotspotStarting,
        HotspotStatus::On { clients: Some(_) } => TextKey::HotspotOnWithClients,
        HotspotStatus::On { clients: None } => TextKey::HotspotOnClientsUnknown,
        HotspotStatus::Stopping => TextKey::HotspotStopping,
        HotspotStatus::Failed { .. } => TextKey::HotspotFailed,
    }
}

#[must_use]
pub fn hotspot_unsupported_reason(value: HotspotUnsupportedReason) -> TextKey {
    match value {
        HotspotUnsupportedReason::MissingPackageIdentity => {
            TextKey::HotspotUnsupportedMissingPackageIdentity
        }
        HotspotUnsupportedReason::MissingWifiControlCapability => {
            TextKey::HotspotUnsupportedMissingWifiControlCapability
        }
        HotspotUnsupportedReason::NoWifiAdapter => TextKey::HotspotUnsupportedNoWifiAdapter,
        HotspotUnsupportedReason::PolicyDisabled => TextKey::HotspotUnsupportedPolicyDisabled,
        HotspotUnsupportedReason::UnsupportedOperatingSystem => {
            TextKey::HotspotUnsupportedOperatingSystem
        }
        HotspotUnsupportedReason::SourceProfileUnavailable => {
            TextKey::HotspotUnsupportedSourceProfileUnavailable
        }
    }
}

#[must_use]
pub fn error_text(code: ErrorCode) -> TextKey {
    match code {
        ErrorCode::PermissionDenied => TextKey::ErrorPermissionDenied,
        ErrorCode::DeviceRemoved => TextKey::ErrorDeviceRemoved,
        ErrorCode::DeviceIdentityChanged => TextKey::ErrorDeviceIdentityChanged,
        ErrorCode::EvidenceExpired => TextKey::ErrorEvidenceExpired,
        ErrorCode::ProbeFailed => TextKey::ErrorProbeFailed,
        ErrorCode::DnsFailed => TextKey::ErrorDnsFailed,
        ErrorCode::Timeout => TextKey::ErrorTimeout,
        ErrorCode::Unsupported => TextKey::ErrorUnsupported,
        ErrorCode::CapabilityUnavailable => TextKey::ErrorCapabilityUnavailable,
        ErrorCode::OperationCancelled => TextKey::ErrorOperationCancelled,
        ErrorCode::VerificationFailed => TextKey::ErrorVerificationFailed,
        ErrorCode::RollbackFailed => TextKey::ErrorRollbackFailed,
        ErrorCode::Internal => TextKey::ErrorInternal,
    }
}

#[must_use]
pub fn stable_code_text(code: &str) -> Option<TextKey> {
    Some(match code {
        "apn:empty" => TextKey::ProtocolApnEmpty,
        "apn:too_long" => TextKey::ProtocolApnTooLong,
        "apn:unsafe_character" => TextKey::ProtocolApnUnsafeCharacter,
        "pdp_context_id:out_of_range" => TextKey::ProtocolPdpContextIdOutOfRange,
        "at_protocol:wrong_port_data" => TextKey::ProtocolWrongPortData,
        "at_protocol:line_too_long" => TextKey::ProtocolLineTooLong,
        "at_protocol:response_too_large" => TextKey::ProtocolResponseTooLarge,
        "at_protocol:timeout" => TextKey::ProtocolTimeout,
        "at_protocol:device_removed" => TextKey::ProtocolDeviceRemoved,
        "at_protocol:unexpected_data" => TextKey::ProtocolUnexpectedData,
        "pnp:no_safe_at_port" => TextKey::PlatformNoSafeAtPort,
        "pnp:ambiguous_at_port" => TextKey::PlatformAmbiguousAtPort,
        "pnp:at_port_unverified" => TextKey::PlatformAtPortUnverified,
        "pnp:unsupported_platform" => TextKey::PlatformUnsupportedPlatform,
        "pnp:enumerate_failed" => TextKey::PlatformPnpEnumerateFailed,
        "pnp:interface_enumerate_failed" => TextKey::PlatformInterfaceEnumerateFailed,
        "pnp:permission_denied" => TextKey::PlatformPnpPermissionDenied,
        "pnp:open_failed" => TextKey::PlatformPnpOpenFailed,
        "serial_actor:queue_full" => TextKey::SerialQueueFull,
        "serial_actor:closed" => TextKey::SerialSessionClosed,
        "serial_actor:io" => TextKey::SerialIoFailed,
        "serial_actor:at_final_error" => TextKey::SerialAtFinalError,
        "net:permission_denied" => TextKey::ErrorPermissionDenied,
        "net:unsupported_platform" => TextKey::PlatformUnsupportedPlatform,
        "net:no_usable_address" => TextKey::AdapterNoUsableAddressOrRoute,
        "net:adapter_identity_mismatch" => TextKey::ErrorDeviceIdentityChanged,
        "net:adapter_not_found" | "net:adapter_ambiguous" => TextKey::ErrorCapabilityUnavailable,
        "net:netcfg_id_invalid" | "net:netcfg_id_missing" => TextKey::ErrorCapabilityUnavailable,
        "net:adapter_enumeration_failed" | "net:route_enumeration_failed" => {
            TextKey::ErrorProbeFailed
        }
        "probe:dns_failed"
        | "probe:dns_timeout"
        | "probe:dns_api_failed"
        | "probe:dns_cancelled" => TextKey::ErrorDnsFailed,
        "probe:connect_timeout"
        | "probe:http_timeout"
        | "probe:tls_timeout"
        | "probe:total_timeout" => TextKey::ErrorTimeout,
        "probe:dependency_unavailable" | "probe:policy_invalid" => {
            TextKey::ErrorCapabilityUnavailable
        }
        "probe:connect_failed"
        | "probe:bind_failed"
        | "probe:route_identity_mismatch"
        | "probe:route_unavailable"
        | "probe:source_mismatch"
        | "probe:tls_failed"
        | "probe:http_invalid"
        | "probe:http_too_large"
        | "probe:socket_option_failed" => TextKey::ErrorProbeFailed,
        "app:sim_missing" => TextKey::SimMissing,
        "app:sim_pin_required" => TextKey::SimPinRequired,
        "app:sim_puk_required" => TextKey::SimPukRequired,
        "app:sim_rejected" => TextKey::SimRejected,
        "app:sim_unobserved" => TextKey::SimUnknown,
        "app:registration_rejected" => TextKey::RegistrationDenied,
        "app:registration_not_ready" => TextKey::RegistrationNotRegistered,
        "app:packet_not_attached" => TextKey::AttachDetached,
        "app:cellular_unobserved" => TextKey::ErrorCapabilityUnavailable,
        "app:missing_before_state" => TextKey::ErrorEvidenceExpired,
        "app:target_absent" => TextKey::ErrorDeviceRemoved,
        "app:refresh_not_action" => TextKey::ErrorUnsupported,
        "app:adapter_not_ready"
        | "app:at_not_ready"
        | "app:hotspot_not_ready"
        | "app:hotspot_unavailable"
        | "app:stage_missing"
        | "app:target_not_ready" => TextKey::ErrorCapabilityUnavailable,
        "app:busy" => TextKey::CommandFeedbackBusy,
        "app:confirm_rejected" => TextKey::CommandFeedbackConfirmRejected,
        "app:unsupported_action" | "app:safety_rejected" => TextKey::ErrorUnsupported,
        "app:plan_expired" => TextKey::PlanExpired,
        "app:before_state_changed" => TextKey::ErrorEvidenceExpired,
        "operation:uac_cancelled" => TextKey::OperationUacCancelled,
        "privilege:helper_unsigned" => TextKey::ErrorHelperUnsigned,
        "privilege:helper_unverified" => TextKey::ErrorHelperUnverified,
        "ui:cjk_font_unavailable" => TextKey::UiCjkFontUnavailable,
        "route:not_observed" | "at:future" => TextKey::ErrorProbeFailed,
        "config:path_unavailable" => TextKey::SettingsPathUnavailable,
        "config:read_failed"
        | "config:directory_create_failed"
        | "config:temp_create_failed"
        | "config:write_failed"
        | "config:flush_failed"
        | "config:sync_failed"
        | "config:replace_failed"
        | "config:replace_stat_failed"
        | "config:readback_failed"
        | "config:default_restore_failed" => TextKey::SettingsReadFailed,
        "config:preserve_failed" => TextKey::SettingsSaveFailed,
        "config:parse_failed" | "config:unsupported_version" => TextKey::SettingsCorruptConfig,
        "export:write_failed" | "export:path_unavailable" => TextKey::DiagnosticsExportFailed,
        code if code.starts_with("export:") => TextKey::DiagnosticsExportFailed,
        "autostart:registration_not_owned" => TextKey::SettingsAutostartNotOwned,
        "autostart:drift" => TextKey::SettingsConfigDrift,
        code if code.starts_with("autostart:") => TextKey::SettingsSaveFailed,
        code if code.starts_with("single_instance:") => TextKey::SingleInstanceActivationFailed,
        "tray:native_unavailable"
        | "tray:unsupported_platform"
        | "tray:create_failed"
        | "tray:recreate_failed"
        | "tray:tooltip_failed" => TextKey::TrayUnavailableFallback,
        code if code.starts_with("tray:") => TextKey::TrayUnavailableFallback,
        "logging:init_failed" | "logging:directory_create_failed" => TextKey::LoggingInitFailed,
        code if code.starts_with("logging:") => TextKey::LoggingRotationFailed,
        // SMS codes keep their precise, actionable meaning; the namespace guard must stay after
        // every concrete code so only genuinely unknown `sms:` failures fall back to the generic
        // line (and never to a neighbouring feature's prose).
        "sms:pdu_mode_required" => TextKey::SmsErrorPduModeRequired,
        "sms:pdu_confirm_failed" => TextKey::SmsErrorPduConfirmFailed,
        "sms:invalid_message" => TextKey::SmsErrorInvalidMessage,
        "sms:send_failed" => TextKey::SmsErrorSendFailed,
        "sms:timeout" => TextKey::SmsErrorTimeout,
        "sms:device_removed" => TextKey::SmsErrorDeviceRemoved,
        "sms:unsupported" => TextKey::SmsErrorUnsupported,
        "sms:verification_failed" => TextKey::SmsErrorVerificationFailed,
        "sms:internal" => TextKey::SmsErrorInternal,
        code if code.starts_with("sms:") => TextKey::SmsErrorGeneric,
        code if code.starts_with("pnp:") => TextKey::PlatformPnpEnumerateFailed,
        code if code.starts_with("net:") => TextKey::ErrorProbeFailed,
        code if code.starts_with("probe:") => TextKey::ErrorProbeFailed,
        code if code.starts_with("app:") => TextKey::ErrorInternal,
        code if code.starts_with("operation:") => TextKey::ErrorInternal,
        code if code.starts_with("route:") || code.starts_with("at:") => TextKey::ErrorProbeFailed,
        _ => return None,
    })
}

#[must_use]
pub fn failure_text(code: &FailureCode, language: Language) -> LocalizedText {
    let key =
        stable_code_text(code.stable().as_str()).unwrap_or_else(|| error_text(code.category()));
    LocalizedText::new(language, key)
}

#[must_use]
pub fn unknown_backend_error(language: Language) -> LocalizedText {
    LocalizedText::new(language, TextKey::UnknownBackendError)
}

#[must_use]
pub fn freshness_key(value: Freshness) -> TextKey {
    match value {
        Freshness::Fresh => TextKey::FreshnessFresh,
        Freshness::Stale => TextKey::FreshnessStale,
        Freshness::Unknown => TextKey::FreshnessUnknown,
    }
}

#[must_use]
pub fn issue_severity(value: IssueSeverity) -> TextKey {
    match value {
        IssueSeverity::Info => TextKey::IssueSeverityInfo,
        IssueSeverity::Warning => TextKey::IssueSeverityWarning,
        IssueSeverity::Error => TextKey::IssueSeverityError,
    }
}

#[must_use]
pub fn issue_layer(value: IssueLayer) -> TextKey {
    match value {
        IssueLayer::Device => TextKey::IssueLayerDevice,
        IssueLayer::Cellular => TextKey::IssueLayerCellular,
        IssueLayer::Network => TextKey::IssueLayerNetwork,
        IssueLayer::BoundProbe => TextKey::IssueLayerBoundProbe,
        IssueLayer::Hotspot => TextKey::IssueLayerHotspot,
        IssueLayer::Operation => TextKey::IssueLayerOperation,
    }
}

#[must_use]
pub fn evidence_source(value: EvidenceSource) -> TextKey {
    match value {
        EvidenceSource::Pnp => TextKey::EvidenceSourcePnp,
        EvidenceSource::AtControl => TextKey::EvidenceSourceAtControl,
        EvidenceSource::WindowsAdapter => TextKey::EvidenceSourceWindowsAdapter,
        EvidenceSource::BoundGatewayProbe => TextKey::EvidenceSourceBoundGatewayProbe,
        EvidenceSource::BoundDnsProbe => TextKey::EvidenceSourceBoundDnsProbe,
        EvidenceSource::BoundPublicProbe => TextKey::EvidenceSourceBoundPublicProbe,
        EvidenceSource::GlobalRoute => TextKey::EvidenceSourceGlobalRoute,
        EvidenceSource::GlobalConnectivity => TextKey::EvidenceSourceGlobalConnectivity,
        EvidenceSource::Hotspot => TextKey::EvidenceSourceHotspot,
    }
}

#[must_use]
pub fn classification_phase(value: ClassificationPhase) -> TextKey {
    match value {
        ClassificationPhase::Startup => TextKey::ClassificationPhaseStartup,
        ClassificationPhase::RecentInsertion => TextKey::ClassificationPhaseRecentInsertion,
        ClassificationPhase::Reenumerating => TextKey::ClassificationPhaseReenumerating,
        ClassificationPhase::PostWriteVerification => {
            TextKey::ClassificationPhasePostWriteVerification
        }
        ClassificationPhase::Stable => TextKey::ClassificationPhaseStable,
    }
}

#[must_use]
pub fn action_key(value: &ActionKind) -> TextKey {
    match value {
        ActionKind::Refresh => TextKey::ActionRefresh,
        ActionKind::RenewDhcp => TextKey::ActionRenewDhcp,
        ActionKind::ApplyDnsProfile {
            profile: dji4g_domain::DnsProfile::Automatic,
        } => TextKey::ActionApplyDnsAutomatic,
        ActionKind::ApplyDnsProfile {
            profile: dji4g_domain::DnsProfile::Static { .. },
        } => TextKey::ActionApplyDnsStatic,
        ActionKind::RestartAdapter => TextKey::ActionRestartAdapter,
        ActionKind::ReenumerateDevice => TextKey::ActionReenumerateDevice,
        ActionKind::RestartModule => TextKey::ActionRestartModule,
        ActionKind::EditApn { .. } => TextKey::ActionEditApn,
        ActionKind::SetVerifiedUsbNetworkProfile {
            profile: UsbNetworkProfile::DjiNdis,
        } => TextKey::ActionSetUsbProfileDjiNdis,
        ActionKind::SetVerifiedUsbNetworkProfile {
            profile: UsbNetworkProfile::Ecm,
        } => TextKey::ActionSetUsbProfileEcm,
        ActionKind::ToggleHotspot { enabled: true } => TextKey::ActionEnableHotspot,
        ActionKind::ToggleHotspot { enabled: false } => TextKey::ActionDisableHotspot,
    }
}

#[must_use]
pub fn action_text(value: &ActionKind, language: Language) -> LocalizedText {
    match value {
        ActionKind::ApplyDnsProfile {
            profile: dji4g_domain::DnsProfile::Static { servers },
        } => format_text_in(
            language,
            TextKey::ActionApplyDnsStatic,
            &TextArgs::server_count(servers.len()),
        ),
        ActionKind::EditApn { cid, .. } => {
            format_text_in(language, TextKey::ActionEditApn, &TextArgs::cid(*cid))
        }
        _ => LocalizedText::new(language, action_key(value)),
    }
}

#[must_use]
pub fn action_tag_key(value: ActionKindTag) -> TextKey {
    match value {
        ActionKindTag::RenewDhcp => TextKey::ActionRenewDhcp,
        ActionKindTag::ApplyDnsProfile => TextKey::ActionApplyDnsAutomatic,
        ActionKindTag::RestartAdapter => TextKey::ActionRestartAdapter,
        ActionKindTag::ReenumerateDevice => TextKey::ActionReenumerateDevice,
        ActionKindTag::RestartModule => TextKey::ActionRestartModule,
        ActionKindTag::EditApn { .. } => TextKey::ActionEditApn,
        ActionKindTag::SetVerifiedUsbNetworkProfile => TextKey::ActionSetUsbProfileDjiNdis,
        ActionKindTag::ToggleHotspot { enabled: true } => TextKey::ActionEnableHotspot,
        ActionKindTag::ToggleHotspot { enabled: false } => TextKey::ActionDisableHotspot,
    }
}

#[must_use]
pub fn risk_level(value: RiskLevel) -> TextKey {
    match value {
        RiskLevel::Low => TextKey::RiskLevelLow,
        RiskLevel::Medium => TextKey::RiskLevelMedium,
        RiskLevel::High => TextKey::RiskLevelHigh,
    }
}

#[must_use]
pub fn disruption_level(value: DisruptionLevel) -> TextKey {
    match value {
        DisruptionLevel::None => TextKey::DisruptionNone,
        DisruptionLevel::Brief => TextKey::DisruptionBrief,
        DisruptionLevel::ConnectionInterrupting => TextKey::DisruptionConnectionInterrupting,
        DisruptionLevel::DeviceReenumeration => TextKey::DisruptionDeviceReenumeration,
    }
}

#[must_use]
pub fn rollback_outcome(value: RollbackOutcome) -> TextKey {
    match value {
        RollbackOutcome::NotRequired => TextKey::RollbackNotRequired,
        RollbackOutcome::Applied => TextKey::RollbackApplied,
        RollbackOutcome::Failed { .. } => TextKey::RollbackFailed,
        RollbackOutcome::NotAttempted => TextKey::RollbackNotAttempted,
    }
}

#[must_use]
pub fn operation_phase(value: OperationPhase) -> TextKey {
    match value {
        OperationPhase::Revalidating => TextKey::OperationRevalidating,
        OperationPhase::AwaitingElevation => TextKey::OperationAwaitingElevation,
        OperationPhase::Executing => TextKey::OperationExecuting,
        OperationPhase::Verifying => TextKey::OperationVerifying,
    }
}

#[must_use]
pub fn diagnostic_id(value: DiagnosticCheckId) -> TextKey {
    match value {
        DiagnosticCheckId::UsbDevice => TextKey::FieldUsbIdentity,
        DiagnosticCheckId::AtControl => TextKey::FieldAtPort,
        DiagnosticCheckId::Cellular => TextKey::IssueLayerCellular,
        DiagnosticCheckId::WindowsAdapter => TextKey::FieldAdapter,
        DiagnosticCheckId::BoundGateway => TextKey::FieldBoundGatewayProbe,
        DiagnosticCheckId::BoundPublic => TextKey::FieldBoundPublicProbe,
        DiagnosticCheckId::BoundDns => TextKey::FieldBoundDnsProbe,
        DiagnosticCheckId::SystemRoute => TextKey::FieldDefaultRoute,
        DiagnosticCheckId::Hotspot => TextKey::FieldHotspot,
    }
}

#[must_use]
pub fn diagnostic_state(value: &DiagnosticCheckState) -> TextKey {
    match value {
        DiagnosticCheckState::Unexecuted { .. } => TextKey::CheckUnexecuted,
        DiagnosticCheckState::Running { .. } => TextKey::CheckRunning,
        DiagnosticCheckState::Passed => TextKey::CheckPassed,
        DiagnosticCheckState::Failed { .. } => TextKey::CheckFailed,
        DiagnosticCheckState::Unavailable { .. } => TextKey::CheckUnavailable,
        DiagnosticCheckState::Expired => TextKey::CheckExpired,
    }
}

#[must_use]
pub fn unexecuted_reason(value: UnexecutedReason) -> TextKey {
    match value {
        UnexecutedReason::DisabledBySetting => TextKey::UnexecutedDisabledBySetting,
        UnexecutedReason::NotScheduled => TextKey::UnexecutedNotScheduled,
        UnexecutedReason::Superseded => TextKey::UnexecutedSuperseded,
    }
}

#[must_use]
pub fn action_safety_error(value: ActionSafetyError) -> TextKey {
    match value {
        ActionSafetyError::UnsupportedDevice => TextKey::ActionSafetyUnsupportedDevice,
        ActionSafetyError::StaleEpoch => TextKey::ActionSafetyStaleEpoch,
        ActionSafetyError::StaleSnapshot => TextKey::ActionSafetyStaleSnapshot,
        ActionSafetyError::TargetIdentityChanged => TextKey::ActionSafetyTargetIdentityChanged,
        ActionSafetyError::BeforeStateChanged => TextKey::ActionSafetyBeforeStateChanged,
        ActionSafetyError::Expired => TextKey::ActionSafetyExpired,
    }
}

#[must_use]
pub fn confirmation_invalidation_reason(value: ConfirmationInvalidationReason) -> TextKey {
    match value {
        ConfirmationInvalidationReason::Expired => TextKey::ActionSafetyExpired,
        ConfirmationInvalidationReason::SnapshotChanged => TextKey::ActionSafetyStaleSnapshot,
        ConfirmationInvalidationReason::EpochChanged => TextKey::ActionSafetyStaleEpoch,
        ConfirmationInvalidationReason::TargetChanged => TextKey::ActionSafetyTargetIdentityChanged,
        ConfirmationInvalidationReason::BeforeStateChanged => {
            TextKey::ActionSafetyBeforeStateChanged
        }
        ConfirmationInvalidationReason::DeviceRemoved => TextKey::ErrorDeviceRemoved,
        ConfirmationInvalidationReason::Superseded => TextKey::StatusExpired,
    }
}

#[must_use]
pub fn dns_profile(value: DnsProfile) -> TextKey {
    match value {
        DnsProfile::Automatic => TextKey::DnsProfileAutomatic,
        DnsProfile::Static { .. } => TextKey::DnsProfileStatic,
    }
}

#[must_use]
pub const fn log_level(value: LogLevel) -> TextKey {
    match value {
        LogLevel::Error => TextKey::LogLevelError,
        LogLevel::Warn => TextKey::LogLevelWarn,
        LogLevel::Info => TextKey::LogLevelInfo,
        LogLevel::Debug => TextKey::LogLevelDebug,
    }
}

#[must_use]
pub fn sim_state(value: SimState) -> TextKey {
    match value {
        SimState::Ready => TextKey::SimReady,
        SimState::Missing => TextKey::SimMissing,
        SimState::PinRequired => TextKey::SimPinRequired,
        SimState::PukRequired => TextKey::SimPukRequired,
        SimState::Rejected => TextKey::SimRejected,
        SimState::Unknown => TextKey::SimUnknown,
    }
}

#[must_use]
pub fn registration_state(value: RegistrationState) -> TextKey {
    match value {
        RegistrationState::RegisteredHome => TextKey::RegistrationHome,
        RegistrationState::RegisteredRoaming => TextKey::RegistrationRoaming,
        RegistrationState::Searching => TextKey::RegistrationSearching,
        RegistrationState::Denied => TextKey::RegistrationDenied,
        RegistrationState::NotRegistered => TextKey::RegistrationNotRegistered,
        RegistrationState::Unknown => TextKey::RegistrationUnknown,
    }
}

#[must_use]
pub fn attach_state(value: AttachState) -> TextKey {
    match value {
        AttachState::Attached => TextKey::AttachAttached,
        AttachState::Detached => TextKey::AttachDetached,
        AttachState::Unknown => TextKey::AttachUnknown,
    }
}

#[must_use]
pub fn cellular_block(value: CellularBlock) -> TextKey {
    match value {
        CellularBlock::SimRejected => TextKey::CellularBlockSimRejected,
        CellularBlock::RegistrationRejected => TextKey::CellularBlockRegistrationRejected,
    }
}

#[must_use]
pub fn adapter_state(value: dji4g_domain::AdapterState) -> TextKey {
    match value {
        dji4g_domain::AdapterState::UsableAddressAndRoute => TextKey::AdapterUsableAddressAndRoute,
        dji4g_domain::AdapterState::NoUsableAddressOrRoute => {
            TextKey::AdapterNoUsableAddressOrRoute
        }
    }
}

#[must_use]
pub fn bound_public_status(value: BoundPublicStatus) -> TextKey {
    match value {
        BoundPublicStatus::Succeeded => TextKey::BoundPublicSucceeded,
        BoundPublicStatus::Failed { .. } => TextKey::BoundPublicFailed,
        BoundPublicStatus::Incomplete => TextKey::BoundPublicIncomplete,
    }
}

#[must_use]
pub fn bound_dns_status(value: BoundDnsStatus) -> TextKey {
    match value {
        BoundDnsStatus::Succeeded => TextKey::BoundDnsSucceeded,
        BoundDnsStatus::Failed => TextKey::BoundDnsFailed,
        BoundDnsStatus::Incomplete => TextKey::BoundDnsIncomplete,
    }
}

#[must_use]
pub fn protocol_coverage(value: ProtocolCoverage) -> TextKey {
    match value {
        ProtocolCoverage::AllRequiredFamilies => TextKey::ProtocolCoverageAllRequired,
        ProtocolCoverage::SingleFamilyOnly => TextKey::ProtocolCoverageSingleFamily,
    }
}

#[must_use]
pub fn at_control_availability(value: AtControlAvailability) -> TextKey {
    match value {
        AtControlAvailability::Available => TextKey::AtControlAvailable,
        AtControlAvailability::Unavailable => TextKey::AtControlUnavailable,
    }
}

#[must_use]
pub fn default_route_owner(value: DefaultRouteOwner) -> TextKey {
    match value {
        DefaultRouteOwner::TargetAdapter => TextKey::DefaultRouteTargetAdapter,
        DefaultRouteOwner::VpnOrTun => TextKey::DefaultRouteVpnOrTun,
        DefaultRouteOwner::Other => TextKey::DefaultRouteOther,
    }
}

#[must_use]
pub fn global_connectivity(value: GlobalConnectivity) -> TextKey {
    match value {
        GlobalConnectivity::Online => TextKey::GlobalConnectivityOnline,
        GlobalConnectivity::Offline => TextKey::GlobalConnectivityOffline,
    }
}

#[must_use]
pub fn device_presence(value: &DevicePresence) -> TextKey {
    match value {
        DevicePresence::Supported(_) => TextKey::DevicePresenceSupported,
        DevicePresence::NotDetected => TextKey::DevicePresenceNotDetected,
        DevicePresence::Unsupported { .. } => TextKey::DevicePresenceUnsupported,
        DevicePresence::PermissionDenied => TextKey::DevicePresencePermissionDenied,
    }
}

#[must_use]
pub fn usb_profile(value: UsbNetworkProfile) -> TextKey {
    match value {
        UsbNetworkProfile::DjiNdis => TextKey::UsbNetworkProfileDjiNdis,
        UsbNetworkProfile::Ecm => TextKey::UsbNetworkProfileEcm,
    }
}

/// A short explanation line for a classified optional-feature probe (research §8.1/§10).
///
/// `Supported` and `Empty` need no note (the value row itself carries the result), and
/// `NotProbed` is a neutral default; only the failure categories produce prose the UI can show
/// next to the identity or wireless rows without ever confusing 「没有数据」 with 「查询失败」.
#[must_use]
pub fn feature_status_note(value: FeatureStatus) -> Option<TextKey> {
    match value {
        FeatureStatus::NotProbed | FeatureStatus::Supported | FeatureStatus::Empty => None,
        FeatureStatus::UnsupportedConfirmed => Some(TextKey::FeatureStatusUnsupportedConfirmed),
        FeatureStatus::FormatMismatch => Some(TextKey::FeatureStatusFormatMismatch),
        FeatureStatus::TransportFailure => Some(TextKey::FeatureStatusTransportFailure),
        FeatureStatus::TemporarilyUnavailable => Some(TextKey::FeatureStatusTemporarilyUnavailable),
    }
}

/// Localized fallback label for one observed timeline transition. The event's own `detail` string
/// is the primary display text (it is a closed, non-sensitive phrase written by the application);
/// this category label is only used when a producer emits an empty detail.
#[must_use]
pub fn timeline_kind_text(value: TimelineEventKind) -> TextKey {
    match value {
        TimelineEventKind::SimChanged => TextKey::TimelineSimChanged,
        TimelineEventKind::RegistrationChanged => TextKey::TimelineRegistrationChanged,
        TimelineEventKind::CellChanged => TextKey::TimelineCellChanged,
        TimelineEventKind::DeviceRemoved => TextKey::TimelineDeviceRemoved,
        TimelineEventKind::DeviceArrived => TextKey::TimelineDeviceArrived,
        TimelineEventKind::AdapterLinkChanged => TextKey::TimelineAdapterLinkChanged,
        TimelineEventKind::DnsChanged => TextKey::TimelineDnsChanged,
    }
}

#[test]
fn cellular_readiness_codes_show_device_state_instead_of_internal_error() {
    for (code, expected) in [
        ("app:sim_missing", TextKey::SimMissing),
        ("app:sim_pin_required", TextKey::SimPinRequired),
        ("app:sim_puk_required", TextKey::SimPukRequired),
        ("app:sim_rejected", TextKey::SimRejected),
        ("app:registration_rejected", TextKey::RegistrationDenied),
    ] {
        assert_eq!(stable_code_text(code), Some(expected));
    }
}
