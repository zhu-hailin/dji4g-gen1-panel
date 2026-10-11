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
    DevicePresence, DeviceProfile, DisruptionLevel, DnsProfile, ErrorCode, EvidenceSource,
    FeatureStatus, Freshness, GlobalConnectivity, HotspotStatus, HotspotUnsupportedReason,
    IssueLayer, IssueSeverity, LimitedReason, ProtocolCoverage, RegistrationState, RiskLevel,
    RollbackOutcome, SimState, TimelineDetail, TimelineEventKind, UnavailableReason,
    UsbNetworkProfile,
};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum Language {
    /// Simplified Chinese. The default, and the language the catalog is authored in.
    #[default]
    ZhCn,
    /// Traditional Chinese with Hong Kong vocabulary.
    ZhTw,
    /// American English.
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
    ActionApplyDnsProfile,
    ActionRestartAdapter,
    ActionReenumerateDevice,
    ActionRestartModule,
    ActionEditApn,
    ActionSetUsbProfileDjiNdis,
    ActionSetUsbProfileEcm,
    ActionSetUsbNetworkProfile,
    ActionEnableHotspot,
    ActionDisableHotspot,
    RiskLevelLow,
    RiskLevelMedium,
    RiskLevelHigh,
    OperationOutcomeApplied,
    OperationUsbConfigurationSaved,
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
    DevicePresenceSupportedQuectelGeneric,
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
    NavWireless,
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
    FieldBoundRouteProbe,
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
    LanguageZhTw,
    LanguageEnUs,
    SettingsAutostart,
    SettingsAutostartDescription,
    SettingsAutostartDisabled,
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
    RepairDhcpDisabled,
    RepairApnInvalid,
    ConfirmationTitle,
    ConfirmationDnsServers,
    ConfirmationNewApn,
    ConfirmationUsbConfigurationOnly,
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
    SmsErrorSimRequired,
    SmsErrorSimUnverified,
    SmsErrorSimChanged,
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
    // ---------------------------------------------------------------- chrome
    /// The sidebar group label above the page list.
    NavGroupModule,
    /// First-run entry footer: skipping is always allowed, and the hint says so.
    EntrySkipHint,
    EntryHiddenHint,
    /// Relative ages, in the three magnitudes the panel prints.
    AgeSeconds,
    AgeMinutes,
    AgeHours,
    /// Rate chart: axis labels, centre notes and the hover readout.
    RateNow,
    RateSecondsAgo,
    RateSamplePaused,
    RateNotSampled,
    RateHoverAgo,
    RateHoverDown,
    RateHoverUp,
    RepairsIntro,
    RepairsAdapterModeHeading,
    RepairsDjiGuideLink,
    RepairsAdapterModeNote,
    RepairsUsbSwitchNote,
    RepairsUsbOnlyNote,
    RepairsLowRiskHeading,
    RepairsInterruptsConnection,
    RepairsViewPlan,
    FieldPdpContext,
    FieldNewApn,

    GuideProbeOff,
    GuideCollecting,
    GuideEvidenceStale,
    GuideCheckDisabled,
    GuideCheckNotRun,
    GuideUsbFailed,
    GuideAdapterFailed,
    GuideCellularFailed,
    GuideBoundProbeFailed,
    GuidePassed,
    GuideStartHeading,
    GuideSteps,
    CheckUsbDetection,
    CheckAdapterInterface,
    CheckAtSerial,
    CheckSimCellular,
    CheckBoundPublic,
    CheckBoundDns,
    GuidePassedCount,
    GuideStuckHint,
    FirstCheckHeading,
    FirstCheckIntro,
    FirstCheckUsb,
    FirstCheckAdapter,
    FirstCheckAt,
    DriverInstallHeading,
    DriverBundledNote,
    DriverElevationNote,
    DriverInstallAction,
    DriverNoneNote,
    DriverDjiCompatibilityLink,
    DriverSeparateNote,
    DriverDjiSupportLink,
    DriverVendorLink,
    DriverVendorLinkNote,
    ValueNotReported,
    WirelessSummary,
    WirelessIntro,
    WirelessServingCell,
    WirelessSampleFresh,
    WirelessSampleWaiting,
    WirelessCopySummary,
    WirelessNoCell,
    WirelessBand,
    WirelessRsrpNote,
    WirelessRsrqNote,
    WirelessRssiNote,
    WirelessSinrNote,
    WirelessSinrUnit,
    WirelessCellDetails,
    WirelessBandwidthLine,
    WirelessTacLine,
    WirelessNoconnNote,
    WirelessSignalHeading,
    WirelessSampleCount,
    WirelessSignalNote,
    WirelessChangeHeading,
    WirelessChangeNote,
    WirelessNoChange,
    WirelessSecondsAgo,
    WirelessPreviousCell,
    WirelessNewCell,
    WirelessWaitingRsrp,
    DeviceModelName,
    DeviceModelNameQuectelGeneric,
    ReadOnlyModuleReason,
    SignalWithGrade,
    PdpActive,
    PdpInactive,
    ServingSearching,
    ServingLimitedService,
    ServingNoCell,
    ServingNotCamped,
    ServingCampedIdle,
    ServingSinr,
    ValuePreviewMore,
    OverviewSummaryLine,
    OverviewTabRate,
    OverviewDeviceHeading,
    FieldModelShort,
    FieldRegistrationShort,
    FieldServingCell,
    OverviewNetworkHeading,
    FieldDefaultRouteShort,
    FieldIpAddresses,
    ValueMoreItems,
    FieldFirmware,
    FieldPdpState,
    DiagNoProblemCode,
    DiagProblemCode,
    DiagPort,
    DiagCarrierNotRead,
    DiagRatNotRead,
    DiagRouteProbeNote,
    DiagProxyInterfaceNote,
    DiagIncomplete,
    DiagFailedRepeatedly,
    DiagResolutionFailed,
    MNCQueued,
    MNCChecking,
    MNCStale,
    MNCAvailable,
    MNCNoDevice,
    MNCAdapterFailed,
    MNCAdapterNoAddress,
    MNCAdapterLinkDown,
    MNCBoundRouteFailed,
    MNCBoundPublicFailed,
    MNCBoundDnsFailed,
    MNCInconclusive,
    MNCEvidenceUnexecuted,
    MNCEvidenceRunning,
    MNCEvidencePassed,
    MNCEvidenceFailed,
    MNCEvidenceUnavailable,
    MNCEvidenceExpired,
    MNCDhcpLease,
    MNCRestartAdapter,
    MNCAutomaticDns,
    MNCHeading,
    MNCRunAction,
    MNCRunningReason,
    MNCOperationResult,
    MNCReadOnlyReverify,
    MNCStepUsb,
    MNCStepPorts,
    MNCStepRoute,
    MNCStepCurrent,
    MNCResultExpired,
    MNCDeviceNoAdapter,
    MNCViewSteps,
    MNCSimHint,
    MNCNoPublicProbe,
    MNCEgressAdapter,
    MNCEgressProxy,
    MNCEgressOther,
    MNCEgressUnknown,
    MNCEgressPath,
    MNCProbeUsb,
    MNCProbeAdapterRead,
    MNCProbeLink,
    MNCProbeAddressRoute,
    MNCProbeBoundRoute,
    MNCProbeBoundPublic,
    MNCProbeBoundDns,
    MNCAdapterLinkLine,
    MNCConnected,
    MNCDisconnected,
    MNCEnabled,
    MNCDisabled,
    MNCProtocolLine,
    MNCStaticAddressNote,
    MNCAtControl,
    MNCSimCellular,
    MNCBoundRouteNote,
    MNCEvidenceHeading,
    MNCIntro,
    MNCProbeConsent,
    MNCProbeCancel,
    MNCLocalOnly,
    MNCAllowProbe,
    MNCSubmitError,
    ArchiveSaveFailed,
    SmsViewModule,
    SmsViewArchive,
    ArchiveExportName,
    ArchiveNoExportDir,
    OnboardingSaveFailed,
    ArchiveBusyTitle,
    ArchiveBusyBody,
    WindowsUpdateFailedTitle,
    WindowsUpdateFailedBody,
    MenuButton,
    NavDialogTitle,
    NavDialogClose,
    OverviewPageIntro,
    MoreMenu,
    ExportDetailedLog,
    ExportDetailedLogHint,
    DiagnosticsPageTitle,
    DiagnosticsPageIntro,
    SettingsReopenOnboarding,
    HelpDialogTitle,
    AboutDialogTitle,
    RestartPanelTitle,
    ExitApp,
    BusyDialogTitle,
    RestartBusyBody,
    RestartConfirmBody,
    RestartFailedTitle,
    RestartFailedBody,
    InstallBusyBody,
    InstallDriverTitle,
    InstallDriverBody,
    InstallFailedTitle,
    InstallFailedBody,
    ConfirmProbeNote,
    AboutWindowTitle,
    GitHubProject,
    UpdateAvailable,
    UpdateTitle,
    UpdateCurrentVersion,
    UpdateCheckNow,
    UpdateChecking,
    UpdateNotChecked,
    UpdateUpToDate,
    UpdateCheckFailed,
    UpdateView,
    UpdateDownload,
    UpdateDownloading,
    UpdateProgress,
    UpdateVerifying,
    UpdateReady,
    UpdatePreparing,
    UpdateInstallRestart,
    UpdateRetry,
    UpdateBusy,
    UpdateRestartHint,
    UpdateFailedNetwork,
    UpdateFailedChecksum,
    UpdateFailedIntegrity,
    UpdateFailedUpdater,
    UpdateUnsupported,
    UpdateRecoveryNotice,
    HostPreparingPlan,
    HostBackingUp,
    HostRestoring,
    HostChecking,
    HostConfigChanged,
    HostNotFinished,
    HostNotChecked,
    HostStale,
    HostModulePassed,
    HostMissingAdapter,
    HostAdapterDown,
    HostOutletUnknown,
    HostProxyInUse,
    HostNoKnownProblem,
    HostHeading,
    HostNoModuleHint,
    HostCheckAgain,
    HostReviewRepair,
    HostProxyOff,
    HostProxyManual,
    HostProxyAutoScript,
    HostProxyAutoDetect,
    HostProxyMixed,
    HostProxyUnknown,
    HostWindowsProxy,
    HostConfiguredAdapter,
    HostProxyBoundAdapter,
    HostUnsupportedConfig,
    HostConfigUnreadable,
    HostRemoveOutletPrefix,
    HostRemoveOutletSuffix,
    HostPlanExpired,
    HostKeepOriginal,
    HostWaitCurrentOperation,
    HostBackupAndRepair,
    HostRestartedCheckAgain,
    HostRestoreOriginal,
    HostStepsHeading,
    HostStepsBody,
    HostConfiguredAdapterRef,
    HostCommandNotSubmitted,
    HostVersionUnknown,
    OnboardingWaiting,
    OnboardingChecking,
    OnboardingNeedsReview,
    OnboardingDisabledUnverified,
    OnboardingStaleRefresh,
    OnboardingCheckUsb,
    OnboardingCheckAdapter,
    OnboardingCheckAt,
    OnboardingCheckCellular,
    OnboardingCheckBoundPublic,
    OnboardingCheckBoundDns,
    OnboardingRestartStep,
    OnboardingEnterAfterRestart,
    OnboardingAutoCheckStep,
    OnboardingStartUsing,
    OnboardingEnterPanel,
    OnboardingPassedNote,
    OnboardingNotPassedNote,
    OnboardingTitle,
    OnboardingIntro,
    OnboardingSetupResultHeading,
    OnboardingStep1Title,
    OnboardingStep1Body,
    OnboardingHostHeading,
    OnboardingCheckHost,
    OnboardingHostNote,
    OnboardingStep3Title,
    OnboardingBundledDriverNote,
    OnboardingUseBundledDriver,
    OnboardingBundledDriverHint,
    OnboardingNoBundledDriverNote,
    OnboardingOpenWindowsUpdate,
    OnboardingDjiCompatibilityLink,
    OnboardingOfficialLinkNote,
    ReportSectionSystem,
    ReportSectionUsb,
    ReportSectionDrivers,
    ReportSectionSerial,
    ReportSectionAdapters,
    ReportSectionNetwork,
    ReportSectionSecurity,
    ReportSectionDriverHistory,
    ReportSectionIntegrity,
    ReportNoLogDirectory,
    ReportPreparing,
    ReportStartFailed,
    ReportExporting,
    ReportExported,
    ReportIncomplete,
    ReportThreadEnded,
    ReportFileName,
    ReportHeader,
    ReportUiSummary,
    ReportMachineSnapshot,
    ReportSnapshotNote,
    ReportTimelineNote,
    ReportCollectLogs,
    ReportHistoryScope,
    ReportHistoryCapped,
    ReportHistoryHeading,
    ReportLogDirectory,
    ReportCollectionScope,
    ExportTitle,
    ExportGeneratedAt,
    ExportPrivacy,
    ExportSectionOverview,
    ExportVerdict,
    ExportEvidenceState,
    ExportSectionDevice,
    ExportDeviceId,
    ExportSectionCellular,
    ExportSectionNetwork,
    ExportSectionSms,
    ExportSendRequest,
    ExportSendError,
    ExportSectionEvidence,
    ExportLabelWithCode,
    ArchiveHeading,
    ArchiveIntro,
    ArchiveRetention,
    ArchiveUnavailable,
    ArchiveToggle,
    ArchiveExportTarget,
    ArchiveSearchHint,
    ArchiveFilterAll,
    ArchiveFilterWeek,
    ArchiveFilterMonth,
    ArchiveExportTxt,
    ArchiveClear,
    ArchiveCount,
    ArchiveEmpty,
    ArchiveNoMatch,
    ArchiveNoTimestamp,
    ArchiveIncompleteTag,
    ArchiveSourceGroup,
    ArchiveCopyBody,
    ArchiveClearDescription,
    ArchiveClearConfirm,
    ArchiveExportTitle,
    ArchiveExportDescription,
    ArchiveExportConfirm,
    ArchiveCancel,
    ArchiveReady,
    ArchivePausedCapture,
    ArchivePausedExport,
    ArchiveExported,
    ArchiveUpdated,
    ArchiveWorkerFailed,
    ArchiveReading,
    ArchiveWorkerStopped,
    ArchiveDemoStatus,
    ArchiveDemoBody,
    ArchiveNoIdentity,
    ArchiveBusy,
    ArchiveOpenFailed,
    ArchiveSizeCheckFailed,
    ArchiveTooLarge,
    ArchiveReadFailed,
    ArchiveInvalidFormat,
    ArchiveDecryptedTooLarge,
    ArchiveCorrupt,
    ArchiveTooManyRecords,
    ArchiveEncodeFailed,
    ArchiveSizeCapSave,
    ArchiveSizeCapEncrypt,
    ArchiveClearFailed,
    ToolStateNotQueried,
    ToolStateAvailable,
    ToolStateNoData,
    ToolStateUnsupported,
    ToolStateTemporarilyUnavailable,
    ToolStateFormatMismatch,
    ToolStateTimeout,
    ToolResultOk,
    ToolResultRejected,
    ToolResultUnsupported,
    ToolResultNoAnswer,
    ToolResultUnrecognized,
    ToolResultCancelled,
    ToolResultMaybeWritten,
    ToolResultInvalidated,
    ToolInputEmpty,
    ToolInputTooLong,
    ToolInputNotAscii,
    ToolInputControlChars,
    ToolInputSemicolon,
    ToolInputMustStartAt,
    ToolInputNotWhitelisted,
    ToolInputNeedsInteractive,
    ToolPresetAttention,
    ToolPresetManufacturer,
    ToolPresetModel,
    ToolPresetFirmware,
    ToolPresetSim,
    ToolPresetSignal,
    ToolPresetCarrier,
    ToolPresetRegistration,
    ToolPresetAttach,
    ToolPresetPdpContexts,
    ToolPresetPdpActive,
    ToolPresetPdpAddress,
    ToolPresetUsbMode,
    ToolPresetTemperature,
    ToolPresetServingCell,
    ToolPresetSmsFormat,
    ToolPresetSmsStorage,
    ToolBatchPresets,
    ToolAdvancedAt,
    ToolTaskIdle,
    ToolTaskQueued,
    ToolTaskRunning,
    ToolTaskCancelling,
    ToolTaskFinished,
    ToolElapsedMinutes,
    ToolElapsedSeconds,
    ToolElapsedMillis,
    ToolUsbModeDjiNdis,
    ToolQueueFull,
    ToolChannelClosed,
    ToolApnContextRange,
    ToolApnInvalid,
    ToolControlledUnavailable,
    ToolTimeUnknown,
    ToolClockAgo,
    ToolsTitle,
    ToolsIntro,
    ToolsDeviceConnected,
    ToolsDeviceIdentity,
    ToolsAtPort,
    ToolsDeviceEpoch,
    ToolsNoDevice,
    ToolsNoDeviceHint,
    ToolsSimSession,
    ToolsTabPresets,
    ToolsTabReadOnly,
    ToolsTabSwitchHint,
    ToolsTaskProgress,
    ToolsBatchFinished,
    ToolsFinishedUnknown,
    ToolsNoTask,
    ToolsCancelNote,
    ToolsLastRejected,
    ToolsProfileHeading,
    ToolsRefreshProfile,
    ToolsRefreshProfileHint,
    FieldManufacturer,
    FieldModel,
    FieldFirmwareVersion,
    FieldUsbNetworkMode,
    ToolNotRecognized,
    FieldCapturedAt,
    ToolsUsbModeUnverified,
    ToolsNoProfileYet,
    ToolsEvidenceHeading,
    ToolsEvidenceNote,
    ToolsQuerying,
    ToolsQueryAgain,
    ToolsQueryItem,
    ToolsQueryingKeepLast,
    ToolsReason,
    ToolsCaptured,
    ToolsStorageNote,
    ToolsNotRunYet,
    ToolsStorageCaution,
    ToolsConnectionHeading,
    ToolsNoPdpYet,
    ToolsNoTemperature,
    ToolsSensor,
    ToolsSensorNote,
    ToolsControlledHeading,
    ToolsControlledNote,
    FieldPdpContextShort,
    ToolsApnExample,
    ToolsEditApn,
    ToolsApnConfirm,
    ToolsApnRange,
    ToolsSwitchTo,
    ToolsSwitchHint,
    ToolsConfirmRuns,
    ToolsCurrentUnrecognized,
    ToolsNotQueriedRefresh,
    ToolsRestartModule,
    ToolsRestartCommand,
    ToolsRestartConfirm,
    ToolsRestartNote,
    ToolsReadOnlyHeading,
    ToolsReadOnlyNote,
    ToolsChoosePreset,
    ToolsRunQuery,
    ToolsWhitelistHint,
    ToolsNotInList,
    ToolsOpenAdvanced,
    ToolsBusyReadOnly,
    ToolsInvalidInput,
    ToolsSessionUnlocked,
    ToolsEnableAtInput,
    ToolsUnlockScope,
    ToolsAdvancedNote,
    ToolsAtHint,
    ToolsExecute,
    ToolsClearInput,
    ToolsLocked,
    ToolsCheckFailed,
    ToolsNormalizedWrite,
    ToolsCommandFrozen,
    ToolsAtPending,
    ToolsFrozenList,
    ToolsUnknownEffect,
    ToolsPlanExpired,
    ToolsPlanRemaining,
    ToolsLogHeading,
    ToolsLogNote,
    ToolsCopySummary,
    ToolsCopySummaryNote,
    ToolsCopyRaw,
    ToolsCopyRawNote,
    ToolsClearLog,
    ToolsClearLogNote,
    ToolsShareCaution,
    ToolsNoLog,
    ToolsEmptyResponse,
    ToolsResponseTruncated,
    ToolsHistoryHeader,
    ToolsTruncatedMark,
    ComposeBusyDraftKept,
    ComposeUnsupportedSender,
    ComposeDemoDraft,
    ComposeBackendBusy,
    ComposeQueueFull,
    ComposeChannelClosed,
    ComposeQueued,
    ComposePreparing,
    ComposeSubmitting,
    ComposeAwaitingModule,
    ComposeSubmittedUnknown,
    ComposeFailedDraftKept,
    ComposeUnknownMaybeSent,
    ComposeFailureHeading,
    ComposeFailureStage,
    ComposeSystemError,
    ComposeTitle,
    ComposeIntro,
    ComposeRecipient,
    ComposeRecipientHint,
    ComposeRecipientNote,
    ComposeBodyLabel,
    ComposeLength,
    ComposeBodyHint,
    ComposeLimits,
    ComposeCostNote,
    ComposeSendingWait,
    ComposeModuleBusy,
    ComposeDraftKept,
    ComposeNeedInput,
    ComposeDraftReady,
    ComposeNextConfirm,
    ComposeConfirmTitle,
    ComposeConfirmIntro,
    ComposeConfirmNote,
    ComposeConfirmAction,
    ComposeKeepDraftTitle,
    ComposeKeepDraftBody,
    ComposeReplaceDraft,
    ComposeKeepDraft,
    ComposeRejected,
    ComposeMaybeSent,
    ComposeNotSubmitted,
    ComposeStageQueued,
    ComposeStagePreparing,
    ComposeStageSubmitting,
    ComposeStageAwaiting,
    ComposeStageDone,
    ComposeSerialBusy,
    ComposeSerialOpenFailed,
    ComposeSerialCloseTimeout,
    ComposeNoDevice,
    ComposeContextChanged,
    ComposeModuleRejected,
    ComposeSerialFailed,
    ComposeTimeout,
    ComposeNoReference,
    ComposeUnexpectedEnd,
    ComposeSerialBusyShort,
    ComposePortBusyShort,
    ComposeTimeoutShort,
    ComposeValidationFailed,
    ComposeDeviceLost,
    ComposeGenericAdvice,
    ComposeCmeError,
    SetupFailedTitle,
    SetupNoPayload,
    SetupConfirmTitle,
    SetupConfirmBody,
    SetupVerifyFailed,
    SetupDoneTitle,
    SetupDoneBody,
    SetupDriverCancelled,
    SetupDriverIncomplete,
    PortableBadResourcePath,
    PortableBadResourceDir,
    PortableWriteFailed,
    PortableReadFailed,
    PortableVerifyFailed,
    PortableLaunchFailed,
    PortableNoPayload,
    PortableNoAppData,
    PortableOpenFailed,
    PortableExitedAbnormally,
    DriverResultTitle,
    DriverResultBody,
    DriverNotStarted,
    DriverPanelRunning,
    DriverInstallTitle,
    DriverInstallBody,
    DriverElevationFailed,
    DriverOpenPanel,
    DriverNoReturn,
    DriverNoExePath,
    DriverNoExeDir,
    DriverCheckComponentFailed,
    DriverClockInvalid,
    DriverLogCreateFailed,
    DriverLogWriteFailed,
    DriverLogAppendFailed,
    DriverLogPath,
    DriverCheckNotStarted,
    DialogActionLine,
    DialogDisruptionLine,
    DialogRiskLine,
    DialogElevationLine,
    DriverInstallResultTitle,
    SettingsGeneral,
    SettingsInterfaceTheme,
    SettingsStartupTray,
    SettingsAutoStart,
    SettingsStartHidden,
    SettingsLogging,
    SettingsAbout,
    SettingsVersion,
    CarrierChinaUnicom,
    CarrierChinaMobile,
    CarrierChinaTelecom,
    CarrierChinaBroadnet,
    RateHeading,
    RatePeakDownload,
    RatePeakUpload,
    ThemeSystem,
    ThemeLight,
    ThemeDark,
    SmsFragmentsRead,
    SmsErrPortBusy,
    SmsErrPortAccess,
    SmsErrNoAtPort,
    SmsErrPduMode,
    SmsErrResponseInvalid,
    SmsErrTimeout,
    SmsErrNoDevice,
    SmsErrDeviceGone,
    SmsErrSimUnknown,
    SmsErrSimChanged,
    SmsErrSimIdentity,
    SmsStopped,
    SmsErrContextChanged,
    SmsErrRestoreUnconfirmed,
    SmsErrUnsupportedLocation,
    SmsErrLimit,
    SmsErrQueryFailed,
    SmsSystemError,
    SmsPageTitle,
    SmsReadDetailsAttention,
    SmsReadDetails,
    SmsRefreshList,
    SmsBusyWait,
    SmsPhaseWaiting,
    SmsPhaseConfirming,
    SmsPhaseQueryingStorage,
    SmsPhaseSelecting,
    SmsPhaseReading,
    SmsPhaseOrganizing,
    SmsPhaseRestoring,
    SmsPhaseReleasing,
    SmsProgressRecords,
    SmsStopReading,
    SmsStopRequested,
    SmsStorageUnconfirmed,
    SmsStorageSim,
    SmsStorageModule,
    SmsStorageModuleArea,
    SmsStorageCurrentArea,
    SmsRecordsRead,
    SmsLastRead,
    SmsRefreshNote,
    SmsOtherLocationsNote,
    SmsReadSim,
    SmsReadModule,
    SmsTaskBusy,
    SmsLocationUnsupported,
    SmsWillConfirm,
    SmsReadOtherLocation,
    SmsReadOtherBody,
    SmsConfirmRead,
    SmsSyncing,
    SmsUnreadCount,
    SmsStorageUsage,
    SmsAutoSyncPaused,
    SmsSyncProblem,
    SmsSyncHistory,
    SmsCacheTrimmed,
    SmsTabInbox,
    SmsTabOutgoing,
    SmsBackToList,
    SmsFooterNote,
    SmsFilterAll,
    SmsFilterIncomplete,
    SmsClearSearch,
    SmsEmptyFilter,
    SmsEmptyFilterHint,
    SmsBalanceQuery,
    SmsBalanceIntro,
    SmsBalanceInstructions,
    SmsBalancePortal,
    SmsBalanceBroadnetHelp,
    SmsBalanceReplyHint,
    SmsLastSynced,
    SmsSyncReadingCache,
    SmsSyncFailedCache,
    SmsSyncPartialCache,
    SmsSyncPausedCache,
    SmsSyncStaleCache,
    SmsSyncStoredList,
    SmsSyncNotCompleted,
    SmsSearchHint,
    SmsEmptySearch,
    SmsEmptySearchHint,
    SmsEmptyOutgoing,
    SmsEmptyOutgoingHint,
    SmsLoading,
    SmsLoadingHint,
    SmsInboxWaiting,
    SmsInboxWaitingHint,
    SmsInboxEmpty,
    SmsInboxEmptyHint,
    SmsUnavailable,
    SmsUnavailableHint,
    SmsRefreshMessages,
    SmsNoTimestamp,
    SmsReaderEmpty,
    SmsReaderEmptyHint,
    SmsKindIncoming,
    SmsKindOutgoing,
    SmsNoTimestampFromModule,
    SmsReply,
    SmsConfirmDeleteFragments,
    SmsDeleteFragments,
    SmsDeleteMessage,
    SmsDeleteBusy,
    SmsDeleteConflict,
    SmsDeleting,
    SmsDeletedAll,
    SmsDeleteUnknown,
    SmsDeletePartial,
    SmsDeleteNone,
    SmsDeleteResultTitle,
    SmsDeleteConfirmed,
    SmsDeleteFailed,
    SmsDeleteUnknownShort,
    SmsDeleteNotRun,
    ArchiveExportDirFailed,
    ArchiveExportCreateFailed,
    ArchiveExportWriteFailed,
    ArchiveInvalidPath,
    ArchiveCreateDirFailed,
    ArchiveTempFileFailed,
    ArchiveWriteFailed,
    ArchiveReplaceFailed,
    ExportSmsHeader,
    ExportSmsRecord,
    ExportSmsIncomplete,
    DriverOutcomeReady,
    DriverOutcomeRestartRequired,
    DriverOutcomeRestartAfterFailure,
    DriverOutcomeCancelled,
    DriverOutcomeNoMatch,
    DriverOutcomeInterfacesAbnormal,
    DriverOutcomeNoModule,
    DriverOutcomePayloadInvalid,
    DriverOutcomeIncomplete,
    DriverWindowsUpdateFailed,
    DriverAdminRequired,
    DriverPanelNotSameDirectory,
    DriverPanelExitTimeout,
    TimelineCellChangedDetail,
    TimelineAdapterLinkChangedDetail,
    TimelineRegistrationChangedDetail,
    TimelineDnsChangedDetail,
    TimelineRegistrationHomeDetail,
    TimelineRegistrationRoamingDetail,
    TimelineRegistrationSearchingDetail,
    TimelineRegistrationDeniedDetail,
    TimelineRegistrationNotRegisteredDetail,
    TimelineRegistrationUnknownDetail,
    TimelineDnsPassedDetail,
    TimelineDnsFailedDetail,
    TimelineDnsIncompleteDetail,
    ArchiveCryptoUnsupported,
    ArchiveCryptoTooLarge,
    ArchiveCryptoFailed,
    ArchiveCryptoDecryptFailed,
    ToolUrcLine,
    SmsFailureWithCode,
    ComposeCmsError,
    SmsDeleteMessageOne,
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
        Self::ActionApplyDnsProfile,
        Self::ActionRestartAdapter,
        Self::ActionReenumerateDevice,
        Self::ActionRestartModule,
        Self::ActionEditApn,
        Self::ActionSetUsbProfileDjiNdis,
        Self::ActionSetUsbProfileEcm,
        Self::ActionSetUsbNetworkProfile,
        Self::ActionEnableHotspot,
        Self::ActionDisableHotspot,
        Self::RiskLevelLow,
        Self::RiskLevelMedium,
        Self::RiskLevelHigh,
        Self::OperationOutcomeApplied,
        Self::OperationUsbConfigurationSaved,
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
        Self::DevicePresenceSupportedQuectelGeneric,
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
        Self::NavWireless,
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
        Self::FieldBoundRouteProbe,
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
        Self::LanguageZhTw,
        Self::LanguageEnUs,
        Self::SettingsAutostart,
        Self::SettingsAutostartDescription,
        Self::SettingsAutostartDisabled,
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
        Self::RepairDhcpDisabled,
        Self::RepairApnInvalid,
        Self::ConfirmationTitle,
        Self::ConfirmationDnsServers,
        Self::ConfirmationNewApn,
        Self::ConfirmationUsbConfigurationOnly,
        Self::ConfirmationOperation,
        Self::ConfirmationTarget,
        Self::ConfirmationExpectedEffect,
        Self::ConfirmationInterruption,
        Self::ConfirmationRisk,
        Self::ConfirmationElevation,
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
        Self::SmsErrorSimRequired,
        Self::SmsErrorSimUnverified,
        Self::SmsErrorSimChanged,
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
        Self::NavGroupModule,
        Self::EntrySkipHint,
        Self::EntryHiddenHint,
        Self::AgeSeconds,
        Self::AgeMinutes,
        Self::AgeHours,
        Self::RateNow,
        Self::RateSecondsAgo,
        Self::RateSamplePaused,
        Self::RateNotSampled,
        Self::RateHoverAgo,
        Self::RateHoverDown,
        Self::RateHoverUp,
        Self::RepairsIntro,
        Self::RepairsAdapterModeHeading,
        Self::RepairsDjiGuideLink,
        Self::RepairsAdapterModeNote,
        Self::RepairsUsbSwitchNote,
        Self::RepairsUsbOnlyNote,
        Self::RepairsLowRiskHeading,
        Self::RepairsInterruptsConnection,
        Self::RepairsViewPlan,
        Self::FieldPdpContext,
        Self::FieldNewApn,
        Self::GuideProbeOff,
        Self::GuideCollecting,
        Self::GuideEvidenceStale,
        Self::GuideCheckDisabled,
        Self::GuideCheckNotRun,
        Self::GuideUsbFailed,
        Self::GuideAdapterFailed,
        Self::GuideCellularFailed,
        Self::GuideBoundProbeFailed,
        Self::GuidePassed,
        Self::GuideStartHeading,
        Self::GuideSteps,
        Self::CheckUsbDetection,
        Self::CheckAdapterInterface,
        Self::CheckAtSerial,
        Self::CheckSimCellular,
        Self::CheckBoundPublic,
        Self::CheckBoundDns,
        Self::GuidePassedCount,
        Self::GuideStuckHint,
        Self::FirstCheckHeading,
        Self::FirstCheckIntro,
        Self::FirstCheckUsb,
        Self::FirstCheckAdapter,
        Self::FirstCheckAt,
        Self::DriverInstallHeading,
        Self::DriverBundledNote,
        Self::DriverElevationNote,
        Self::DriverInstallAction,
        Self::DriverNoneNote,
        Self::DriverDjiCompatibilityLink,
        Self::DriverSeparateNote,
        Self::DriverDjiSupportLink,
        Self::DriverVendorLink,
        Self::DriverVendorLinkNote,
        Self::ValueNotReported,
        Self::WirelessSummary,
        Self::WirelessIntro,
        Self::WirelessServingCell,
        Self::WirelessSampleFresh,
        Self::WirelessSampleWaiting,
        Self::WirelessCopySummary,
        Self::WirelessNoCell,
        Self::WirelessBand,
        Self::WirelessRsrpNote,
        Self::WirelessRsrqNote,
        Self::WirelessRssiNote,
        Self::WirelessSinrNote,
        Self::WirelessSinrUnit,
        Self::WirelessCellDetails,
        Self::WirelessBandwidthLine,
        Self::WirelessTacLine,
        Self::WirelessNoconnNote,
        Self::WirelessSignalHeading,
        Self::WirelessSampleCount,
        Self::WirelessSignalNote,
        Self::WirelessChangeHeading,
        Self::WirelessChangeNote,
        Self::WirelessNoChange,
        Self::WirelessSecondsAgo,
        Self::WirelessPreviousCell,
        Self::WirelessNewCell,
        Self::WirelessWaitingRsrp,
        Self::DeviceModelName,
        Self::DeviceModelNameQuectelGeneric,
        Self::ReadOnlyModuleReason,
        Self::SignalWithGrade,
        Self::PdpActive,
        Self::PdpInactive,
        Self::ServingSearching,
        Self::ServingLimitedService,
        Self::ServingNoCell,
        Self::ServingNotCamped,
        Self::ServingCampedIdle,
        Self::ServingSinr,
        Self::ValuePreviewMore,
        Self::OverviewSummaryLine,
        Self::OverviewTabRate,
        Self::OverviewDeviceHeading,
        Self::FieldModelShort,
        Self::FieldRegistrationShort,
        Self::FieldServingCell,
        Self::OverviewNetworkHeading,
        Self::FieldDefaultRouteShort,
        Self::FieldIpAddresses,
        Self::ValueMoreItems,
        Self::FieldFirmware,
        Self::FieldPdpState,
        Self::DiagNoProblemCode,
        Self::DiagProblemCode,
        Self::DiagPort,
        Self::DiagCarrierNotRead,
        Self::DiagRatNotRead,
        Self::DiagRouteProbeNote,
        Self::DiagProxyInterfaceNote,
        Self::DiagIncomplete,
        Self::DiagFailedRepeatedly,
        Self::DiagResolutionFailed,
        Self::MNCQueued,
        Self::MNCChecking,
        Self::MNCStale,
        Self::MNCAvailable,
        Self::MNCNoDevice,
        Self::MNCAdapterFailed,
        Self::MNCAdapterNoAddress,
        Self::MNCAdapterLinkDown,
        Self::MNCBoundRouteFailed,
        Self::MNCBoundPublicFailed,
        Self::MNCBoundDnsFailed,
        Self::MNCInconclusive,
        Self::MNCEvidenceUnexecuted,
        Self::MNCEvidenceRunning,
        Self::MNCEvidencePassed,
        Self::MNCEvidenceFailed,
        Self::MNCEvidenceUnavailable,
        Self::MNCEvidenceExpired,
        Self::MNCDhcpLease,
        Self::MNCRestartAdapter,
        Self::MNCAutomaticDns,
        Self::MNCHeading,
        Self::MNCRunAction,
        Self::MNCRunningReason,
        Self::MNCOperationResult,
        Self::MNCReadOnlyReverify,
        Self::MNCStepUsb,
        Self::MNCStepPorts,
        Self::MNCStepRoute,
        Self::MNCStepCurrent,
        Self::MNCResultExpired,
        Self::MNCDeviceNoAdapter,
        Self::MNCViewSteps,
        Self::MNCSimHint,
        Self::MNCNoPublicProbe,
        Self::MNCEgressAdapter,
        Self::MNCEgressProxy,
        Self::MNCEgressOther,
        Self::MNCEgressUnknown,
        Self::MNCEgressPath,
        Self::MNCProbeUsb,
        Self::MNCProbeAdapterRead,
        Self::MNCProbeLink,
        Self::MNCProbeAddressRoute,
        Self::MNCProbeBoundRoute,
        Self::MNCProbeBoundPublic,
        Self::MNCProbeBoundDns,
        Self::MNCAdapterLinkLine,
        Self::MNCConnected,
        Self::MNCDisconnected,
        Self::MNCEnabled,
        Self::MNCDisabled,
        Self::MNCProtocolLine,
        Self::MNCStaticAddressNote,
        Self::MNCAtControl,
        Self::MNCSimCellular,
        Self::MNCBoundRouteNote,
        Self::MNCEvidenceHeading,
        Self::MNCIntro,
        Self::MNCProbeConsent,
        Self::MNCProbeCancel,
        Self::MNCLocalOnly,
        Self::MNCAllowProbe,
        Self::MNCSubmitError,
        Self::ArchiveSaveFailed,
        Self::SmsViewModule,
        Self::SmsViewArchive,
        Self::ArchiveExportName,
        Self::ArchiveNoExportDir,
        Self::OnboardingSaveFailed,
        Self::ArchiveBusyTitle,
        Self::ArchiveBusyBody,
        Self::WindowsUpdateFailedTitle,
        Self::WindowsUpdateFailedBody,
        Self::MenuButton,
        Self::NavDialogTitle,
        Self::NavDialogClose,
        Self::OverviewPageIntro,
        Self::MoreMenu,
        Self::ExportDetailedLog,
        Self::ExportDetailedLogHint,
        Self::DiagnosticsPageTitle,
        Self::DiagnosticsPageIntro,
        Self::SettingsReopenOnboarding,
        Self::HelpDialogTitle,
        Self::AboutDialogTitle,
        Self::RestartPanelTitle,
        Self::ExitApp,
        Self::BusyDialogTitle,
        Self::RestartBusyBody,
        Self::RestartConfirmBody,
        Self::RestartFailedTitle,
        Self::RestartFailedBody,
        Self::InstallBusyBody,
        Self::InstallDriverTitle,
        Self::InstallDriverBody,
        Self::InstallFailedTitle,
        Self::InstallFailedBody,
        Self::ConfirmProbeNote,
        Self::AboutWindowTitle,
        Self::GitHubProject,
        Self::UpdateAvailable,
        Self::UpdateTitle,
        Self::UpdateCurrentVersion,
        Self::UpdateCheckNow,
        Self::UpdateChecking,
        Self::UpdateNotChecked,
        Self::UpdateUpToDate,
        Self::UpdateCheckFailed,
        Self::UpdateView,
        Self::UpdateDownload,
        Self::UpdateDownloading,
        Self::UpdateProgress,
        Self::UpdateVerifying,
        Self::UpdateReady,
        Self::UpdatePreparing,
        Self::UpdateInstallRestart,
        Self::UpdateRetry,
        Self::UpdateBusy,
        Self::UpdateRestartHint,
        Self::UpdateFailedNetwork,
        Self::UpdateFailedChecksum,
        Self::UpdateFailedIntegrity,
        Self::UpdateFailedUpdater,
        Self::UpdateUnsupported,
        Self::UpdateRecoveryNotice,
        Self::HostPreparingPlan,
        Self::HostBackingUp,
        Self::HostRestoring,
        Self::HostChecking,
        Self::HostConfigChanged,
        Self::HostNotFinished,
        Self::HostNotChecked,
        Self::HostStale,
        Self::HostModulePassed,
        Self::HostMissingAdapter,
        Self::HostAdapterDown,
        Self::HostOutletUnknown,
        Self::HostProxyInUse,
        Self::HostNoKnownProblem,
        Self::HostHeading,
        Self::HostNoModuleHint,
        Self::HostCheckAgain,
        Self::HostReviewRepair,
        Self::HostProxyOff,
        Self::HostProxyManual,
        Self::HostProxyAutoScript,
        Self::HostProxyAutoDetect,
        Self::HostProxyMixed,
        Self::HostProxyUnknown,
        Self::HostWindowsProxy,
        Self::HostConfiguredAdapter,
        Self::HostProxyBoundAdapter,
        Self::HostUnsupportedConfig,
        Self::HostConfigUnreadable,
        Self::HostRemoveOutletPrefix,
        Self::HostRemoveOutletSuffix,
        Self::HostPlanExpired,
        Self::HostKeepOriginal,
        Self::HostWaitCurrentOperation,
        Self::HostBackupAndRepair,
        Self::HostRestartedCheckAgain,
        Self::HostRestoreOriginal,
        Self::HostStepsHeading,
        Self::HostStepsBody,
        Self::HostConfiguredAdapterRef,
        Self::HostCommandNotSubmitted,
        Self::HostVersionUnknown,
        Self::OnboardingWaiting,
        Self::OnboardingChecking,
        Self::OnboardingNeedsReview,
        Self::OnboardingDisabledUnverified,
        Self::OnboardingStaleRefresh,
        Self::OnboardingCheckUsb,
        Self::OnboardingCheckAdapter,
        Self::OnboardingCheckAt,
        Self::OnboardingCheckCellular,
        Self::OnboardingCheckBoundPublic,
        Self::OnboardingCheckBoundDns,
        Self::OnboardingRestartStep,
        Self::OnboardingEnterAfterRestart,
        Self::OnboardingAutoCheckStep,
        Self::OnboardingStartUsing,
        Self::OnboardingEnterPanel,
        Self::OnboardingPassedNote,
        Self::OnboardingNotPassedNote,
        Self::OnboardingTitle,
        Self::OnboardingIntro,
        Self::OnboardingSetupResultHeading,
        Self::OnboardingStep1Title,
        Self::OnboardingStep1Body,
        Self::OnboardingHostHeading,
        Self::OnboardingCheckHost,
        Self::OnboardingHostNote,
        Self::OnboardingStep3Title,
        Self::OnboardingBundledDriverNote,
        Self::OnboardingUseBundledDriver,
        Self::OnboardingBundledDriverHint,
        Self::OnboardingNoBundledDriverNote,
        Self::OnboardingOpenWindowsUpdate,
        Self::OnboardingDjiCompatibilityLink,
        Self::OnboardingOfficialLinkNote,
        Self::ReportSectionSystem,
        Self::ReportSectionUsb,
        Self::ReportSectionDrivers,
        Self::ReportSectionSerial,
        Self::ReportSectionAdapters,
        Self::ReportSectionNetwork,
        Self::ReportSectionSecurity,
        Self::ReportSectionDriverHistory,
        Self::ReportSectionIntegrity,
        Self::ReportNoLogDirectory,
        Self::ReportPreparing,
        Self::ReportStartFailed,
        Self::ReportExporting,
        Self::ReportExported,
        Self::ReportIncomplete,
        Self::ReportThreadEnded,
        Self::ReportFileName,
        Self::ReportHeader,
        Self::ReportUiSummary,
        Self::ReportMachineSnapshot,
        Self::ReportSnapshotNote,
        Self::ReportTimelineNote,
        Self::ReportCollectLogs,
        Self::ReportHistoryScope,
        Self::ReportHistoryCapped,
        Self::ReportHistoryHeading,
        Self::ReportLogDirectory,
        Self::ReportCollectionScope,
        Self::ExportTitle,
        Self::ExportGeneratedAt,
        Self::ExportPrivacy,
        Self::ExportSectionOverview,
        Self::ExportVerdict,
        Self::ExportEvidenceState,
        Self::ExportSectionDevice,
        Self::ExportDeviceId,
        Self::ExportSectionCellular,
        Self::ExportSectionNetwork,
        Self::ExportSectionSms,
        Self::ExportSendRequest,
        Self::ExportSendError,
        Self::ExportSectionEvidence,
        Self::ExportLabelWithCode,
        Self::ArchiveHeading,
        Self::ArchiveIntro,
        Self::ArchiveRetention,
        Self::ArchiveUnavailable,
        Self::ArchiveToggle,
        Self::ArchiveExportTarget,
        Self::ArchiveSearchHint,
        Self::ArchiveFilterAll,
        Self::ArchiveFilterWeek,
        Self::ArchiveFilterMonth,
        Self::ArchiveExportTxt,
        Self::ArchiveClear,
        Self::ArchiveCount,
        Self::ArchiveEmpty,
        Self::ArchiveNoMatch,
        Self::ArchiveNoTimestamp,
        Self::ArchiveIncompleteTag,
        Self::ArchiveSourceGroup,
        Self::ArchiveCopyBody,
        Self::ArchiveClearDescription,
        Self::ArchiveClearConfirm,
        Self::ArchiveExportTitle,
        Self::ArchiveExportDescription,
        Self::ArchiveExportConfirm,
        Self::ArchiveCancel,
        Self::ArchiveReady,
        Self::ArchivePausedCapture,
        Self::ArchivePausedExport,
        Self::ArchiveExported,
        Self::ArchiveUpdated,
        Self::ArchiveWorkerFailed,
        Self::ArchiveReading,
        Self::ArchiveWorkerStopped,
        Self::ArchiveDemoStatus,
        Self::ArchiveDemoBody,
        Self::ArchiveNoIdentity,
        Self::ArchiveBusy,
        Self::ArchiveOpenFailed,
        Self::ArchiveSizeCheckFailed,
        Self::ArchiveTooLarge,
        Self::ArchiveReadFailed,
        Self::ArchiveInvalidFormat,
        Self::ArchiveDecryptedTooLarge,
        Self::ArchiveCorrupt,
        Self::ArchiveTooManyRecords,
        Self::ArchiveEncodeFailed,
        Self::ArchiveSizeCapSave,
        Self::ArchiveSizeCapEncrypt,
        Self::ArchiveClearFailed,
        Self::ToolStateNotQueried,
        Self::ToolStateAvailable,
        Self::ToolStateNoData,
        Self::ToolStateUnsupported,
        Self::ToolStateTemporarilyUnavailable,
        Self::ToolStateFormatMismatch,
        Self::ToolStateTimeout,
        Self::ToolResultOk,
        Self::ToolResultRejected,
        Self::ToolResultUnsupported,
        Self::ToolResultNoAnswer,
        Self::ToolResultUnrecognized,
        Self::ToolResultCancelled,
        Self::ToolResultMaybeWritten,
        Self::ToolResultInvalidated,
        Self::ToolInputEmpty,
        Self::ToolInputTooLong,
        Self::ToolInputNotAscii,
        Self::ToolInputControlChars,
        Self::ToolInputSemicolon,
        Self::ToolInputMustStartAt,
        Self::ToolInputNotWhitelisted,
        Self::ToolInputNeedsInteractive,
        Self::ToolPresetAttention,
        Self::ToolPresetManufacturer,
        Self::ToolPresetModel,
        Self::ToolPresetFirmware,
        Self::ToolPresetSim,
        Self::ToolPresetSignal,
        Self::ToolPresetCarrier,
        Self::ToolPresetRegistration,
        Self::ToolPresetAttach,
        Self::ToolPresetPdpContexts,
        Self::ToolPresetPdpActive,
        Self::ToolPresetPdpAddress,
        Self::ToolPresetUsbMode,
        Self::ToolPresetTemperature,
        Self::ToolPresetServingCell,
        Self::ToolPresetSmsFormat,
        Self::ToolPresetSmsStorage,
        Self::ToolBatchPresets,
        Self::ToolAdvancedAt,
        Self::ToolTaskIdle,
        Self::ToolTaskQueued,
        Self::ToolTaskRunning,
        Self::ToolTaskCancelling,
        Self::ToolTaskFinished,
        Self::ToolElapsedMinutes,
        Self::ToolElapsedSeconds,
        Self::ToolElapsedMillis,
        Self::ToolUsbModeDjiNdis,
        Self::ToolQueueFull,
        Self::ToolChannelClosed,
        Self::ToolApnContextRange,
        Self::ToolApnInvalid,
        Self::ToolControlledUnavailable,
        Self::ToolTimeUnknown,
        Self::ToolClockAgo,
        Self::ToolsTitle,
        Self::ToolsIntro,
        Self::ToolsDeviceConnected,
        Self::ToolsDeviceIdentity,
        Self::ToolsAtPort,
        Self::ToolsDeviceEpoch,
        Self::ToolsNoDevice,
        Self::ToolsNoDeviceHint,
        Self::ToolsSimSession,
        Self::ToolsTabPresets,
        Self::ToolsTabReadOnly,
        Self::ToolsTabSwitchHint,
        Self::ToolsTaskProgress,
        Self::ToolsBatchFinished,
        Self::ToolsFinishedUnknown,
        Self::ToolsNoTask,
        Self::ToolsCancelNote,
        Self::ToolsLastRejected,
        Self::ToolsProfileHeading,
        Self::ToolsRefreshProfile,
        Self::ToolsRefreshProfileHint,
        Self::FieldManufacturer,
        Self::FieldModel,
        Self::FieldFirmwareVersion,
        Self::FieldUsbNetworkMode,
        Self::ToolNotRecognized,
        Self::FieldCapturedAt,
        Self::ToolsUsbModeUnverified,
        Self::ToolsNoProfileYet,
        Self::ToolsEvidenceHeading,
        Self::ToolsEvidenceNote,
        Self::ToolsQuerying,
        Self::ToolsQueryAgain,
        Self::ToolsQueryItem,
        Self::ToolsQueryingKeepLast,
        Self::ToolsReason,
        Self::ToolsCaptured,
        Self::ToolsStorageNote,
        Self::ToolsNotRunYet,
        Self::ToolsStorageCaution,
        Self::ToolsConnectionHeading,
        Self::ToolsNoPdpYet,
        Self::ToolsNoTemperature,
        Self::ToolsSensor,
        Self::ToolsSensorNote,
        Self::ToolsControlledHeading,
        Self::ToolsControlledNote,
        Self::FieldPdpContextShort,
        Self::ToolsApnExample,
        Self::ToolsEditApn,
        Self::ToolsApnConfirm,
        Self::ToolsApnRange,
        Self::ToolsSwitchTo,
        Self::ToolsSwitchHint,
        Self::ToolsConfirmRuns,
        Self::ToolsCurrentUnrecognized,
        Self::ToolsNotQueriedRefresh,
        Self::ToolsRestartModule,
        Self::ToolsRestartCommand,
        Self::ToolsRestartConfirm,
        Self::ToolsRestartNote,
        Self::ToolsReadOnlyHeading,
        Self::ToolsReadOnlyNote,
        Self::ToolsChoosePreset,
        Self::ToolsRunQuery,
        Self::ToolsWhitelistHint,
        Self::ToolsNotInList,
        Self::ToolsOpenAdvanced,
        Self::ToolsBusyReadOnly,
        Self::ToolsInvalidInput,
        Self::ToolsSessionUnlocked,
        Self::ToolsEnableAtInput,
        Self::ToolsUnlockScope,
        Self::ToolsAdvancedNote,
        Self::ToolsAtHint,
        Self::ToolsExecute,
        Self::ToolsClearInput,
        Self::ToolsLocked,
        Self::ToolsCheckFailed,
        Self::ToolsNormalizedWrite,
        Self::ToolsCommandFrozen,
        Self::ToolsAtPending,
        Self::ToolsFrozenList,
        Self::ToolsUnknownEffect,
        Self::ToolsPlanExpired,
        Self::ToolsPlanRemaining,
        Self::ToolsLogHeading,
        Self::ToolsLogNote,
        Self::ToolsCopySummary,
        Self::ToolsCopySummaryNote,
        Self::ToolsCopyRaw,
        Self::ToolsCopyRawNote,
        Self::ToolsClearLog,
        Self::ToolsClearLogNote,
        Self::ToolsShareCaution,
        Self::ToolsNoLog,
        Self::ToolsEmptyResponse,
        Self::ToolsResponseTruncated,
        Self::ToolsHistoryHeader,
        Self::ToolsTruncatedMark,
        Self::ComposeBusyDraftKept,
        Self::ComposeUnsupportedSender,
        Self::ComposeDemoDraft,
        Self::ComposeBackendBusy,
        Self::ComposeQueueFull,
        Self::ComposeChannelClosed,
        Self::ComposeQueued,
        Self::ComposePreparing,
        Self::ComposeSubmitting,
        Self::ComposeAwaitingModule,
        Self::ComposeSubmittedUnknown,
        Self::ComposeFailedDraftKept,
        Self::ComposeUnknownMaybeSent,
        Self::ComposeFailureHeading,
        Self::ComposeFailureStage,
        Self::ComposeSystemError,
        Self::ComposeTitle,
        Self::ComposeIntro,
        Self::ComposeRecipient,
        Self::ComposeRecipientHint,
        Self::ComposeRecipientNote,
        Self::ComposeBodyLabel,
        Self::ComposeLength,
        Self::ComposeBodyHint,
        Self::ComposeLimits,
        Self::ComposeCostNote,
        Self::ComposeSendingWait,
        Self::ComposeModuleBusy,
        Self::ComposeDraftKept,
        Self::ComposeNeedInput,
        Self::ComposeDraftReady,
        Self::ComposeNextConfirm,
        Self::ComposeConfirmTitle,
        Self::ComposeConfirmIntro,
        Self::ComposeConfirmNote,
        Self::ComposeConfirmAction,
        Self::ComposeKeepDraftTitle,
        Self::ComposeKeepDraftBody,
        Self::ComposeReplaceDraft,
        Self::ComposeKeepDraft,
        Self::ComposeRejected,
        Self::ComposeMaybeSent,
        Self::ComposeNotSubmitted,
        Self::ComposeStageQueued,
        Self::ComposeStagePreparing,
        Self::ComposeStageSubmitting,
        Self::ComposeStageAwaiting,
        Self::ComposeStageDone,
        Self::ComposeSerialBusy,
        Self::ComposeSerialOpenFailed,
        Self::ComposeSerialCloseTimeout,
        Self::ComposeNoDevice,
        Self::ComposeContextChanged,
        Self::ComposeModuleRejected,
        Self::ComposeSerialFailed,
        Self::ComposeTimeout,
        Self::ComposeNoReference,
        Self::ComposeUnexpectedEnd,
        Self::ComposeSerialBusyShort,
        Self::ComposePortBusyShort,
        Self::ComposeTimeoutShort,
        Self::ComposeValidationFailed,
        Self::ComposeDeviceLost,
        Self::ComposeGenericAdvice,
        Self::ComposeCmeError,
        Self::SetupFailedTitle,
        Self::SetupNoPayload,
        Self::SetupConfirmTitle,
        Self::SetupConfirmBody,
        Self::SetupVerifyFailed,
        Self::SetupDoneTitle,
        Self::SetupDoneBody,
        Self::SetupDriverCancelled,
        Self::SetupDriverIncomplete,
        Self::PortableBadResourcePath,
        Self::PortableBadResourceDir,
        Self::PortableWriteFailed,
        Self::PortableReadFailed,
        Self::PortableVerifyFailed,
        Self::PortableLaunchFailed,
        Self::PortableNoPayload,
        Self::PortableNoAppData,
        Self::PortableOpenFailed,
        Self::PortableExitedAbnormally,
        Self::DriverResultTitle,
        Self::DriverResultBody,
        Self::DriverNotStarted,
        Self::DriverPanelRunning,
        Self::DriverInstallTitle,
        Self::DriverInstallBody,
        Self::DriverElevationFailed,
        Self::DriverOpenPanel,
        Self::DriverNoReturn,
        Self::DriverNoExePath,
        Self::DriverNoExeDir,
        Self::DriverCheckComponentFailed,
        Self::DriverClockInvalid,
        Self::DriverLogCreateFailed,
        Self::DriverLogWriteFailed,
        Self::DriverLogAppendFailed,
        Self::DriverLogPath,
        Self::DriverCheckNotStarted,
        Self::DialogActionLine,
        Self::DialogDisruptionLine,
        Self::DialogRiskLine,
        Self::DialogElevationLine,
        Self::DriverInstallResultTitle,
        Self::SettingsGeneral,
        Self::SettingsInterfaceTheme,
        Self::SettingsStartupTray,
        Self::SettingsAutoStart,
        Self::SettingsStartHidden,
        Self::SettingsLogging,
        Self::SettingsAbout,
        Self::SettingsVersion,
        Self::CarrierChinaUnicom,
        Self::CarrierChinaMobile,
        Self::CarrierChinaTelecom,
        Self::CarrierChinaBroadnet,
        Self::RateHeading,
        Self::RatePeakDownload,
        Self::RatePeakUpload,
        Self::ThemeSystem,
        Self::ThemeLight,
        Self::ThemeDark,
        Self::SmsFragmentsRead,
        Self::SmsErrPortBusy,
        Self::SmsErrPortAccess,
        Self::SmsErrNoAtPort,
        Self::SmsErrPduMode,
        Self::SmsErrResponseInvalid,
        Self::SmsErrTimeout,
        Self::SmsErrNoDevice,
        Self::SmsErrDeviceGone,
        Self::SmsErrSimUnknown,
        Self::SmsErrSimChanged,
        Self::SmsErrSimIdentity,
        Self::SmsStopped,
        Self::SmsErrContextChanged,
        Self::SmsErrRestoreUnconfirmed,
        Self::SmsErrUnsupportedLocation,
        Self::SmsErrLimit,
        Self::SmsErrQueryFailed,
        Self::SmsSystemError,
        Self::SmsPageTitle,
        Self::SmsReadDetailsAttention,
        Self::SmsReadDetails,
        Self::SmsRefreshList,
        Self::SmsBusyWait,
        Self::SmsPhaseWaiting,
        Self::SmsPhaseConfirming,
        Self::SmsPhaseQueryingStorage,
        Self::SmsPhaseSelecting,
        Self::SmsPhaseReading,
        Self::SmsPhaseOrganizing,
        Self::SmsPhaseRestoring,
        Self::SmsPhaseReleasing,
        Self::SmsProgressRecords,
        Self::SmsStopReading,
        Self::SmsStopRequested,
        Self::SmsStorageUnconfirmed,
        Self::SmsStorageSim,
        Self::SmsStorageModule,
        Self::SmsStorageModuleArea,
        Self::SmsStorageCurrentArea,
        Self::SmsRecordsRead,
        Self::SmsLastRead,
        Self::SmsRefreshNote,
        Self::SmsOtherLocationsNote,
        Self::SmsReadSim,
        Self::SmsReadModule,
        Self::SmsTaskBusy,
        Self::SmsLocationUnsupported,
        Self::SmsWillConfirm,
        Self::SmsReadOtherLocation,
        Self::SmsReadOtherBody,
        Self::SmsConfirmRead,
        Self::SmsSyncing,
        Self::SmsUnreadCount,
        Self::SmsStorageUsage,
        Self::SmsAutoSyncPaused,
        Self::SmsSyncProblem,
        Self::SmsSyncHistory,
        Self::SmsCacheTrimmed,
        Self::SmsTabInbox,
        Self::SmsTabOutgoing,
        Self::SmsBackToList,
        Self::SmsFooterNote,
        Self::SmsFilterAll,
        Self::SmsFilterIncomplete,
        Self::SmsClearSearch,
        Self::SmsEmptyFilter,
        Self::SmsEmptyFilterHint,
        Self::SmsBalanceQuery,
        Self::SmsBalanceIntro,
        Self::SmsBalanceInstructions,
        Self::SmsBalancePortal,
        Self::SmsBalanceBroadnetHelp,
        Self::SmsBalanceReplyHint,
        Self::SmsLastSynced,
        Self::SmsSyncReadingCache,
        Self::SmsSyncFailedCache,
        Self::SmsSyncPartialCache,
        Self::SmsSyncPausedCache,
        Self::SmsSyncStaleCache,
        Self::SmsSyncStoredList,
        Self::SmsSyncNotCompleted,
        Self::SmsSearchHint,
        Self::SmsEmptySearch,
        Self::SmsEmptySearchHint,
        Self::SmsEmptyOutgoing,
        Self::SmsEmptyOutgoingHint,
        Self::SmsLoading,
        Self::SmsLoadingHint,
        Self::SmsInboxWaiting,
        Self::SmsInboxWaitingHint,
        Self::SmsInboxEmpty,
        Self::SmsInboxEmptyHint,
        Self::SmsUnavailable,
        Self::SmsUnavailableHint,
        Self::SmsRefreshMessages,
        Self::SmsNoTimestamp,
        Self::SmsReaderEmpty,
        Self::SmsReaderEmptyHint,
        Self::SmsKindIncoming,
        Self::SmsKindOutgoing,
        Self::SmsNoTimestampFromModule,
        Self::SmsReply,
        Self::SmsConfirmDeleteFragments,
        Self::SmsDeleteFragments,
        Self::SmsDeleteMessage,
        Self::SmsDeleteBusy,
        Self::SmsDeleteConflict,
        Self::SmsDeleting,
        Self::SmsDeletedAll,
        Self::SmsDeleteUnknown,
        Self::SmsDeletePartial,
        Self::SmsDeleteNone,
        Self::SmsDeleteResultTitle,
        Self::SmsDeleteConfirmed,
        Self::SmsDeleteFailed,
        Self::SmsDeleteUnknownShort,
        Self::SmsDeleteNotRun,
        Self::ArchiveExportDirFailed,
        Self::ArchiveExportCreateFailed,
        Self::ArchiveExportWriteFailed,
        Self::ArchiveInvalidPath,
        Self::ArchiveCreateDirFailed,
        Self::ArchiveTempFileFailed,
        Self::ArchiveWriteFailed,
        Self::ArchiveReplaceFailed,
        Self::ExportSmsHeader,
        Self::ExportSmsRecord,
        Self::ExportSmsIncomplete,
        Self::DriverOutcomeReady,
        Self::DriverOutcomeRestartRequired,
        Self::DriverOutcomeRestartAfterFailure,
        Self::DriverOutcomeCancelled,
        Self::DriverOutcomeNoMatch,
        Self::DriverOutcomeInterfacesAbnormal,
        Self::DriverOutcomeNoModule,
        Self::DriverOutcomePayloadInvalid,
        Self::DriverOutcomeIncomplete,
        Self::DriverWindowsUpdateFailed,
        Self::DriverAdminRequired,
        Self::DriverPanelNotSameDirectory,
        Self::DriverPanelExitTimeout,
        Self::TimelineCellChangedDetail,
        Self::TimelineAdapterLinkChangedDetail,
        Self::TimelineRegistrationChangedDetail,
        Self::TimelineDnsChangedDetail,
        Self::TimelineRegistrationHomeDetail,
        Self::TimelineRegistrationRoamingDetail,
        Self::TimelineRegistrationSearchingDetail,
        Self::TimelineRegistrationDeniedDetail,
        Self::TimelineRegistrationNotRegisteredDetail,
        Self::TimelineRegistrationUnknownDetail,
        Self::TimelineDnsPassedDetail,
        Self::TimelineDnsFailedDetail,
        Self::TimelineDnsIncompleteDetail,
        Self::ArchiveCryptoUnsupported,
        Self::ArchiveCryptoTooLarge,
        Self::ArchiveCryptoFailed,
        Self::ArchiveCryptoDecryptFailed,
        Self::ToolUrcLine,
        Self::SmsFailureWithCode,
        Self::ComposeCmsError,
        Self::SmsDeleteMessageOne,
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

    /// Mark the APN argument as present but not showable: [`format_text_in`] substitutes the
    /// catalog's 「已隐藏」 for it instead of the value.
    #[must_use]
    pub fn apn_masked() -> Self {
        Self {
            apn_masked: Some(String::new()),
            ..Self::default()
        }
    }
}

/// The language a persisted settings code means.
///
/// One mapping, kept next to the catalog, so the settings page, the panel and the standalone
/// binaries cannot disagree about it.
#[must_use]
pub const fn language_of(code: dji4g_application::LanguageCode) -> Language {
    match code {
        dji4g_application::LanguageCode::ZhCn => Language::ZhCn,
        dji4g_application::LanguageCode::ZhTw => Language::ZhTw,
        dji4g_application::LanguageCode::EnUs => Language::EnUs,
    }
}

/// The persisted form of a language choice — the inverse of [`language_of`].
#[must_use]
pub const fn language_code(language: Language) -> dji4g_application::LanguageCode {
    match language {
        Language::ZhCn => dji4g_application::LanguageCode::ZhCn,
        Language::ZhTw => dji4g_application::LanguageCode::ZhTw,
        Language::EnUs => dji4g_application::LanguageCode::EnUs,
    }
}

/// The languages the settings page offers, in the order it lists them.
///
/// This list is the one place that decides what a user can pick. A language appears here only once
/// its catalog is complete — not mostly complete — because a half-translated panel is worse than
/// one that stays in a language the user can read.
#[must_use]
pub fn available_languages() -> &'static [Language] {
    &[Language::ZhCn, Language::ZhTw, Language::EnUs]
}

/// The key that names `language` in every catalog.
///
/// A language picker labels each option with that option's *own* name, so the three entries are the
/// same string in all three catalogs: someone who cannot read the current language must still be
/// able to find their own.
#[must_use]
pub const fn language_key(language: Language) -> TextKey {
    match language {
        Language::ZhCn => TextKey::LanguageZhCn,
        Language::ZhTw => TextKey::LanguageZhTw,
        Language::EnUs => TextKey::LanguageEnUs,
    }
}

/// Fill a catalog template that uses bare `{}` slots, in order.
///
/// `format!` cannot take its template as a value, and a technical line such as the radio summary has
/// more values than [`TextArgs`] names. Both languages agree on the order of the slots, and
/// `placeholders_match_across_languages` proves it.
#[must_use]
pub fn format_positional(language: Language, key: TextKey, args: &[&str]) -> String {
    let template = template(language, key);
    let mut output = String::with_capacity(template.len() + 32);
    let mut rest = template;
    for value in args {
        match rest.split_once("{}") {
            Some((before, after)) => {
                output.push_str(before);
                output.push_str(value);
                rest = after;
            }
            None => break,
        }
    }
    output.push_str(rest);
    output
}

/// Resolve one key in one language.
///
/// The three catalogs below are keyed identically and in the same order, so they can be read side
/// by side; `the_three_catalogs_stay_in_step` is what keeps them that way.
/// The interface language a standalone executable should use: the saved setting when a config file
/// exists, simplified Chinese otherwise. The installers run before (or beside) the panel, so this is
/// the only signal available to them, and it matches the panel's own default.
#[must_use]
pub fn configured_language() -> Language {
    let Ok(paths) = crate::config::ConfigPaths::from_environment() else {
        return Language::ZhCn;
    };
    match crate::config::ConfigStore::new(paths).load() {
        Ok(crate::config::ConfigLoadOutcome::Loaded { config }) => language_of(config.language),
        _ => Language::ZhCn,
    }
}

pub fn template(language: Language, key: TextKey) -> &'static str {
    match language {
        Language::ZhCn => zh_cn(key),
        Language::ZhTw => zh_tw(key),
        Language::EnUs => en_us(key),
    }
}

/// Simplified Chinese. This is the catalog the other two are authored against, and its wording is
/// frozen: moving a string out of a page and into this table must never change a single character,
/// which is what keeps the panel's existing behaviour and tests intact.
fn zh_cn(key: TextKey) -> &'static str {
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
        TextKey::EvidenceSourceBoundGatewayProbe => "模块绑定路由查询（历史来源标记）",
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
        TextKey::RepairDhcpDisabled => "模块网卡未启用 IPv4 DHCP，无法续租。",
        TextKey::RepairApnInvalid => "APN 含不允许的字符。",
        TextKey::ActionRenewDhcp => "更新模块网卡的 DHCP 租约",
        TextKey::ActionApplyDnsAutomatic => "恢复自动获取 DNS",
        TextKey::ActionApplyDnsStatic => "应用静态 DNS（{server_count} 个服务器）",
        TextKey::ActionApplyDnsProfile => "修改 DNS 配置",
        TextKey::ActionRestartAdapter => "重启模块网卡",
        TextKey::ActionReenumerateDevice => "重新枚举模块设备",
        TextKey::ActionRestartModule => "重启蜂窝模块",
        TextKey::ActionEditApn => "修改 PDP 上下文 {cid} 的 APN",
        TextKey::ActionSetUsbProfileDjiNdis => "切换为电脑网卡（DJI NDIS）",
        TextKey::ActionSetUsbProfileEcm => "切换为 ECM 网卡",
        TextKey::ActionSetUsbNetworkProfile => "切换 USB 网络配置",
        TextKey::ActionEnableHotspot => "开启移动热点",
        TextKey::ActionDisableHotspot => "关闭移动热点",
        TextKey::RiskLevelLow => "低风险",
        TextKey::RiskLevelMedium => "中等风险",
        TextKey::RiskLevelHigh => "高风险",
        TextKey::OperationOutcomeApplied => "操作已应用，并已完成状态回读。",
        TextKey::OperationUsbConfigurationSaved => {
            "USB 网络配置已保存；需手动重启模块后复检，当前网卡模式尚未验证。"
        }
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
        TextKey::DevicePresenceSupportedQuectelGeneric => {
            "已检测到 Quectel 通用模组（VID 2C7C、PID 0125），仅支持只读检查"
        }
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
        TextKey::NavWireless => "无线",
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
        TextKey::FieldBoundRouteProbe => "模块绑定路由查询",
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
        // The three language names are endonyms: the same string in every catalog, so a language
        // picker can be read by someone who cannot read the language the panel is currently in.
        TextKey::LanguageZhCn => "简体中文",
        TextKey::LanguageZhTw => "繁體中文",
        TextKey::LanguageEnUs => "English",
        TextKey::SettingsAutostart => "登录 Windows 时启动",
        TextKey::SettingsAutostartDescription => "默认关闭；启用后将直接启动到系统托盘。",
        TextKey::SettingsAutostartDisabled => "已关闭",
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
        TextKey::ConfirmationDnsServers => "DNS 服务器",
        TextKey::ConfirmationNewApn => "新 APN",
        TextKey::ConfirmationUsbConfigurationOnly => {
            "此操作只保存 USB 网络配置；需手动重启模块后复检，当前网卡模式尚未验证。"
        }
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
        TextKey::SmsErrorSimRequired => "缺少 SIM 身份信息，未执行短信操作；请先刷新。",
        TextKey::SmsErrorSimUnverified => "无法核实当前 SIM，未执行短信操作。",
        TextKey::SmsErrorSimChanged => "SIM 已变化，未执行短信操作；请刷新后重新确认。",
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
        TextKey::NavGroupModule => "模块管理",
        TextKey::EntrySkipHint => "可以直接进入，稍后继续检查。",
        TextKey::EntryHiddenHint => "进入后不再自动显示；可在设置中重新打开。",
        TextKey::AgeSeconds => "{count} 秒",
        TextKey::AgeMinutes => "{count} 分钟",
        TextKey::AgeHours => "{count} 小时",
        TextKey::RateNow => "现在",
        TextKey::RateSecondsAgo => "{count} 秒前",
        TextKey::RateSamplePaused => "速率采样暂停，等待新读数",
        TextKey::RateNotSampled => "暂未获取速率",
        TextKey::RateHoverAgo => "{age} 秒前 · 实际采样",
        TextKey::RateHoverDown => "↓ 下载  {detail}",
        TextKey::RateHoverUp => "↑ 上传  {detail}",
        TextKey::RepairsIntro => "先检查原因，再确认需要执行的操作",
        TextKey::RepairsAdapterModeHeading => "电脑网卡模式",
        TextKey::RepairsDjiGuideLink => "查看大疆官方使用说明",
        TextKey::RepairsAdapterModeNote => {
            "部分一代模块保留原厂固件即可用作电脑网卡。先检查驱动和当前网络状态；已经能上网时无需切换。"
        }
        TextKey::RepairsUsbSwitchNote => {
            "下方操作只切换 USB 网络配置，不刷写固件。DJI NDIS 配置需要匹配的 Windows 驱动；ECM 配置的兼容性取决于系统与驱动。"
        }
        TextKey::RepairsUsbOnlyNote => {
            "此操作只保存 USB 配置；需手动重启模块后验证模式。重启会中断连接。"
        }
        TextKey::RepairsLowRiskHeading => "低风险与网络恢复",
        TextKey::RepairsInterruptsConnection => "会中断连接",
        TextKey::RepairsViewPlan => "查看方案",
        TextKey::FieldPdpContext => "PDP 上下文",
        TextKey::FieldNewApn => "新 APN",

        TextKey::GuideProbeOff => "主动联网检查已关闭，公网与 DNS 尚未验证；可在设置中开启。",
        TextKey::GuideCollecting => "正在采集连接证据，请等待本轮检查完成；此时无需修改网络设置。",
        TextKey::GuideEvidenceStale => "连接证据已过期，请刷新后再判断；旧结果不代表当前连接状态。",
        TextKey::GuideCheckDisabled => "此项检查已关闭，可在设置中开启；未检查不代表网络失败。",
        TextKey::GuideCheckNotRun => "尚未完成此项检查，请先刷新。未识别设备不等于缺驱动。",
        TextKey::GuideUsbFailed => {
            "USB 检查未通过，请核对数据线、接口与设备。未识别设备不等于缺驱动。"
        }
        TextKey::GuideAdapterFailed => {
            "模块接口检查未通过，请查看串口或网卡的具体原因；正常接口无需重装驱动。"
        }
        TextKey::GuideCellularFailed => {
            "SIM 或蜂窝注册检查未通过，请查看原因并核对卡状态、信号与运营商注册。"
        }
        TextKey::GuideBoundProbeFailed => {
            "模块绑定的公网或 DNS 检查未通过，请按具体失败项排查；无需反复切换 USB 模式。"
        }
        TextKey::GuidePassed => {
            "模块公网与 DNS 检查已通过。系统实际出口仍可能由 Wi-Fi 或 VPN 决定。"
        }
        TextKey::GuideStartHeading => "开始使用模块",
        TextKey::GuideSteps => {
            "1. 连接模块 → 2. 检查驱动与串口 → 3. 检查 SIM / 网络 → 4. 上网或短信"
        }
        TextKey::CheckUsbDetection => "USB 识别",
        TextKey::CheckAdapterInterface => "网卡接口",
        TextKey::CheckAtSerial => "AT 串口",
        TextKey::CheckSimCellular => "SIM 与蜂窝网络",
        TextKey::CheckBoundPublic => "模块公网",
        TextKey::CheckBoundDns => "模块 DNS",
        TextKey::GuidePassedCount => "已通过 {count} 项：{detail}",
        TextKey::GuideStuckHint => {
            "长时间停在检测中：点击顶部“导出详细日志”，完成后“打开所在文件夹”，把该 TXT 文件交给协助排查的人。日志包含设备、驱动和网络信息，不包含短信正文。"
        }
        TextKey::FirstCheckHeading => "首次连接检查",
        TextKey::FirstCheckIntro => {
            "插入模块后，分别检查网卡和 AT 通信。无法上网或未获得 IP 并不一定是缺少驱动。"
        }
        TextKey::FirstCheckUsb => "1. USB 设备识别",
        TextKey::FirstCheckAdapter => "2. Windows 网卡",
        TextKey::FirstCheckAt => "3. AT 通信（短信与模块查询）",
        TextKey::DriverInstallHeading => "驱动安装",
        TextKey::DriverBundledNote => {
            "此离线版附带原始驱动资源，安装前会校验文件和签名。当前包不能覆盖所有接口（包括未匹配的 MI_04）；任何缺驱动接口无法匹配时，将在安装前停止。Windows 可能同时更新其他匹配该包的设备，不强制覆盖更优驱动。"
        }
        TextKey::DriverElevationNote => {
            "确认后面板会自动退出，再显示 Windows 管理员授权。安装结束或取消授权后会自动返回面板并显示结果；如提示重启，请先重启电脑。"
        }
        TextKey::DriverInstallAction => "退出面板并安装驱动",
        TextKey::DriverNoneNote => {
            "此版本没有完整的离线驱动资源。请打开 Windows 设置 → Windows 更新 → 可选更新检查驱动，或联系 DJI 官方支持取得此模块的适配驱动；安装后点击“立即刷新”。"
        }
        TextKey::DriverDjiCompatibilityLink => "大疆官方兼容说明（第 21 项）",
        TextKey::DriverSeparateNote => {
            "网卡与 AT 串口可能需要不同驱动。安装结果返回后仍需验证 AT 与网络；已有功能正常时无需重复安装。Windows 更新不保证提供该模块的全部驱动。"
        }
        TextKey::DriverDjiSupportLink => "联系 DJI 官方支持",
        TextKey::DriverVendorLink => "移远官方驱动获取说明",
        TextKey::DriverVendorLinkNote => {
            "该链接提供厂商获取渠道，不代表其中所有驱动都兼容大疆定制模块。"
        }
        TextKey::ValueNotReported => "未报告",
        TextKey::WirelessSummary => {
            "无线观测（模块 AT+QENG 报告）\n{}\n制式 {} / {}\nRSRP {}\nRSRQ {}\nRSSI {}\nSINR 原始值 {}（单位未确认）\n上行带宽 {} MHz / 下行带宽 {} MHz\nTAC {}"
        }
        TextKey::WirelessIntro => {
            "查看模块的无线参数与小区变化，并控制由模块供网的 Windows 移动热点"
        }
        TextKey::WirelessServingCell => "当前服务小区",
        TextKey::WirelessSampleFresh => "最近一次 AT 查询已完成",
        TextKey::WirelessSampleWaiting => "等待有效采样 / 已有数据仅供回看",
        TextKey::WirelessCopySummary => "复制无线摘要",
        TextKey::WirelessNoCell => {
            "暂未获取服务小区数据。连接模块后随后台监测自动采样；短信发送期间暂停。未报告的字段保持为空。"
        }
        TextKey::WirelessBand => "频段 B{}",
        TextKey::WirelessRsrpNote => "参考信号功率",
        TextKey::WirelessRsrqNote => "参考信号质量",
        TextKey::WirelessRssiNote => "接收总功率",
        TextKey::WirelessSinrNote => "SINR · 原始值",
        TextKey::WirelessSinrUnit => "单位尚未确认",
        TextKey::WirelessCellDetails => "小区与带宽详情",
        TextKey::WirelessBandwidthLine => "上行带宽 {} MHz  ·  下行带宽 {} MHz",
        TextKey::WirelessTacLine => "TAC {}  ·  模块状态 {}",
        TextKey::WirelessNoconnNote => {
            "NOCONN 表示注册后空闲；不单独据此判定网络断开。信号数值不能代替实际吞吐测试。"
        }
        TextKey::WirelessSignalHeading => "信号变化 · RSRP",
        TextKey::WirelessSampleCount => "最近 {} / 120 次采样",
        TextKey::WirelessSignalNote => {
            "按 AT 采样顺序显示；缺失值断开曲线，未收到新回执时不重复造点。设备或 SIM 更换后重新记录。"
        }
        TextKey::WirelessChangeHeading => "小区变化记录 · {}",
        TextKey::WirelessChangeNote => {
            "仅记录观测到的小区标识变化，不将它直接解释为切换失败或断线。最近保留 20 条。"
        }
        TextKey::WirelessNoChange => "当前会话尚未观测到小区变化。",
        TextKey::WirelessSecondsAgo => "{} 秒前",
        TextKey::WirelessPreviousCell => "原小区：{}",
        TextKey::WirelessNewCell => "新小区：{}",
        TextKey::WirelessWaitingRsrp => "等待有效 RSRP 采样",
        TextKey::DeviceModelName => "DJI 一代 4G 模块",
        TextKey::DeviceModelNameQuectelGeneric => "Quectel 通用模组",
        TextKey::ReadOnlyModuleReason => {
            "通用模组仅支持只读检查；驱动安装与受控写入仅适用于 DJI 一代模组"
        }
        TextKey::SignalWithGrade => "{} dBm（速度：{}）",
        TextKey::PdpActive => "已激活",
        TextKey::PdpInactive => "未激活",
        TextKey::ServingSearching => "正在搜索",
        TextKey::ServingLimitedService => "受限服务",
        TextKey::ServingNoCell => "无小区",
        TextKey::ServingNotCamped => "未驻留（搜索中）",
        TextKey::ServingCampedIdle => "已驻留（空闲）",
        TextKey::ServingSinr => "SINR {}（单位待确认）",
        TextKey::ValuePreviewMore => "{}（另有 {} 项）",
        TextKey::OverviewSummaryLine => "运营商 {}  ·  信号 {}  ·  温度 {}",
        TextKey::OverviewTabRate => "收发速率",
        TextKey::OverviewDeviceHeading => "设备与 SIM 详情",
        TextKey::FieldModelShort => "型号",
        TextKey::FieldRegistrationShort => "注册",
        TextKey::FieldServingCell => "服务小区",
        TextKey::OverviewNetworkHeading => "Windows 网络详情",
        TextKey::FieldDefaultRouteShort => "默认路由",
        TextKey::FieldIpAddresses => "IP 地址",
        TextKey::ValueMoreItems => "另有 {count} 项（未展开）",
        TextKey::FieldFirmware => "固件",
        TextKey::FieldPdpState => "PDP 状态",
        TextKey::DiagNoProblemCode => "无问题代码",
        TextKey::DiagProblemCode => "问题代码 {}",
        TextKey::DiagPort => "端口 {}",
        TextKey::DiagCarrierNotRead => "运营商未获取",
        TextKey::DiagRatNotRead => "制式未获取",
        TextKey::DiagRouteProbeNote => "仅查询匹配路由，不证明网关可达；配置网关：{}",
        TextKey::DiagProxyInterfaceNote => "；VPN 或代理可能正常使用此接口，模块通路另行验证",
        TextKey::DiagIncomplete => "尚未完成",
        TextKey::DiagFailedRepeatedly => "未通过（连续 {} 次）",
        TextKey::DiagResolutionFailed => "解析失败",
        TextKey::MNCQueued => "已排队，等待当前任务结束后检查",
        TextKey::MNCChecking => "正在检查模块网络…",
        TextKey::MNCStale => "结果已过期或设备已变化，请重新检查",
        TextKey::MNCAvailable => "模块网络可用：本次公网与 DNS 验证通过",
        TextKey::MNCNoDevice => "未识别到模块，请检查数据线和 USB 接口",
        TextKey::MNCAdapterFailed => "模块网卡检查未通过，请查看具体证据",
        TextKey::MNCAdapterNoAddress => "已识别网卡，但缺少可用地址或路由",
        TextKey::MNCAdapterLinkDown => "模块网卡链路未连接，请检查 USB 连接与设备状态",
        TextKey::MNCBoundRouteFailed => "模块绑定路由查询未通过，请查看地址与路由配置",
        TextKey::MNCBoundPublicFailed => "模块公网测试未通过，请查看蜂窝与公网证据",
        TextKey::MNCBoundDnsFailed => "模块公网可达，但 DNS 解析未通过",
        TextKey::MNCInconclusive => "尚不能判断模块能否上网，请查看未完成的检查",
        TextKey::MNCEvidenceUnexecuted => "未执行",
        TextKey::MNCEvidenceRunning => "检查中",
        TextKey::MNCEvidencePassed => "通过",
        TextKey::MNCEvidenceFailed => "未通过",
        TextKey::MNCEvidenceUnavailable => "无法获取",
        TextKey::MNCEvidenceExpired => "已过期",
        TextKey::MNCDhcpLease => "更新模块网卡 DHCP 租约",
        TextKey::MNCRestartAdapter => "重启模块网卡",
        TextKey::MNCAutomaticDns => "恢复自动 DNS",
        TextKey::MNCHeading => "模块网络检查",
        TextKey::MNCRunAction => "检查模块网络",
        TextKey::MNCRunningReason => "本轮检查尚未结束",
        TextKey::MNCOperationResult => "操作结果：{}",
        TextKey::MNCReadOnlyReverify => "下方为操作后的只读复检；不会自动重复修复。",
        TextKey::MNCStepUsb => "识别 USB 模块",
        TextKey::MNCStepPorts => "读取串口、蜂窝与模块网卡",
        TextKey::MNCStepRoute => "查询模块绑定路由，验证公网、DNS 与电脑出口",
        TextKey::MNCStepCurrent => "当前步骤：{}",
        TextKey::MNCResultExpired => "检查结果已过期，请重新检查",
        TextKey::MNCDeviceNoAdapter => {
            "模块已识别，但尚未核实网卡接口。驱动、USB 网络模式或读取失败都可能有关。"
        }
        TextKey::MNCViewSteps => "查看驱动与接口检查步骤",
        TextKey::MNCSimHint => "请确认 SIM 卡可用、蜂窝注册与套餐状态。一次超时不能说明驱动损坏。",
        TextKey::MNCNoPublicProbe => {
            "本次未验证公网连接。点击检查并允许一次联网探测，长期设置保持不变。"
        }
        TextKey::MNCEgressAdapter => "选择模块网卡",
        TextKey::MNCEgressProxy => "选择代理或 VPN；这本身不是故障",
        TextKey::MNCEgressOther => "选择其他网卡（例如 Wi-Fi / 有线）；这本身不是故障",
        TextKey::MNCEgressUnknown => "无法确认出口",
        TextKey::MNCEgressPath => "{} 对本次测试目标的路径：{}。",
        TextKey::MNCProbeUsb => "USB 模块",
        TextKey::MNCProbeAdapterRead => "网卡读取",
        TextKey::MNCProbeLink => "链路",
        TextKey::MNCProbeAddressRoute => "地址与路由",
        TextKey::MNCProbeBoundRoute => "模块绑定路由查询",
        TextKey::MNCProbeBoundPublic => "模块公网",
        TextKey::MNCProbeBoundDns => "模块 DNS",
        TextKey::MNCAdapterLinkLine => "链路：{}；IPv4 DHCP：{}",
        TextKey::MNCConnected => "已连接",
        TextKey::MNCDisconnected => "未连接",
        TextKey::MNCEnabled => "启用",
        TextKey::MNCDisabled => "未启用",
        TextKey::MNCProtocolLine => "可用协议：IPv4 {} / IPv6 {}",
        TextKey::MNCStaticAddressNote => {
            "检测到非 DHCP 配置，不会自动覆盖静态地址。请确认原有网络设置。"
        }
        TextKey::MNCAtControl => "串口通信",
        TextKey::MNCSimCellular => "SIM 与蜂窝",
        TextKey::MNCBoundRouteNote => {
            "绑定路由查询只证明找到了匹配路由，不证明网关可达。公网探测绑定模块网卡；电脑路径只代表本次固定测试目标，不能代表所有应用。未执行、无法获取和过期均不等于故障。"
        }
        TextKey::MNCEvidenceHeading => "本轮证据与处理说明",
        TextKey::MNCIntro => "单独检查模块网卡能否上网，并说明电脑对测试目标选择的出口。",
        TextKey::MNCProbeConsent => {
            "后台联网探测已关闭。本次检查将向内置固定验证端点发送少量请求（公网与 DNS），不会修改长期设置。"
        }
        TextKey::MNCProbeCancel => "取消",
        TextKey::MNCLocalOnly => "仅检查本地信息",
        TextKey::MNCAllowProbe => "允许本次联网检查",
        TextKey::MNCSubmitError => "检查请求未提交，请稍后重试。",
        TextKey::ArchiveSaveFailed => {
            "保存历史开关失败，本次更改未保存。请检查用户目录权限后重试。"
        }
        TextKey::SmsViewModule => "模块短信",
        TextKey::SmsViewArchive => "本地历史",
        TextKey::ArchiveExportName => "短信历史-{}.txt",
        TextKey::ArchiveNoExportDir => "未导出：用户导出目录不可用。",
        TextKey::OnboardingSaveFailed => {
            "已进入面板，但引导完成状态保存失败（{}）；下次启动可能再次显示。"
        }
        TextKey::ArchiveBusyTitle => "正在完成本地短信历史操作",
        TextKey::ArchiveBusyBody => "保存、清空或导出结束后将自动退出，请稍候。",
        TextKey::WindowsUpdateFailedTitle => "无法打开 Windows 更新",
        TextKey::WindowsUpdateFailedBody => {
            "请从 Windows 设置打开“Windows 更新”，检查可选驱动更新。当前尚未安装任何驱动。"
        }
        TextKey::MenuButton => "菜单",
        TextKey::NavDialogTitle => "导航",
        TextKey::NavDialogClose => "收起菜单",
        TextKey::OverviewPageIntro => "模块与电脑的当前状态 · 本页只读",
        TextKey::MoreMenu => "更多",
        TextKey::ExportDetailedLog => "导出详细日志",
        TextKey::ExportDetailedLogHint => "收集 USB、驱动、串口、网络与检测阶段，不包含短信正文。",
        TextKey::DiagnosticsPageTitle => "网络诊断",
        TextKey::DiagnosticsPageIntro => "分别检查模块通路与电脑网络，按证据定位问题",
        TextKey::SettingsReopenOnboarding => "重新查看首次使用引导",
        TextKey::HelpDialogTitle => "使用说明",
        TextKey::AboutDialogTitle => "关于本应用",
        TextKey::RestartPanelTitle => "重启面板",
        TextKey::ExitApp => "退出应用",
        TextKey::BusyDialogTitle => "请等待当前任务完成",
        TextKey::RestartBusyBody => {
            "正在发送短信、执行修复或运行设备工具。完成或取消后再重启面板，避免中断当前任务。"
        }
        TextKey::RestartConfirmBody => {
            "面板将关闭并立即重新启动。\n设备会在重启后重新识别；正在填写但未发送的短信草稿会丢失。\n\n现在重启？"
        }
        TextKey::RestartFailedTitle => "无法重启面板",
        TextKey::RestartFailedBody => "面板保持运行，未重启。\n{}\n可先退出，再手动打开程序。",
        TextKey::InstallBusyBody => {
            "正在导出日志、发送短信或执行/确认修复。完成后再安装驱动，避免中断当前任务。"
        }
        TextKey::InstallDriverTitle => "安装模块驱动",
        TextKey::InstallDriverBody => {
            "面板将自动退出，随后显示 Windows 管理员授权，请选择“是”。\n仅安装硬件匹配的缺失驱动，正常接口不会强制重装。\n\n完成或取消后会返回普通权限的面板，显示结果和下一步；如提示重启，请先重启电脑。\n\n现在继续？"
        }
        TextKey::InstallFailedTitle => "无法启动安装器",
        TextKey::InstallFailedBody => "面板保持运行，尚未安装驱动。\n{}\n请导出详细日志。",
        TextKey::ConfirmProbeNote => {
            "\n\n完成后将只读复检一次，向固定端点发送少量公网与 DNS 请求；长期探测设置不变。失败或结果未知不会自动再次修复。"
        }
        TextKey::AboutWindowTitle => "关于 {}",
        TextKey::GitHubProject => "GitHub 项目仓库",
        TextKey::UpdateAvailable => "发现新版本 v{}",
        TextKey::UpdateTitle => "软件更新",
        TextKey::UpdateCurrentVersion => "当前版本 v{}",
        TextKey::UpdateCheckNow => "检查更新",
        TextKey::UpdateChecking => "正在检查更新…",
        TextKey::UpdateNotChecked => "尚未检查更新。",
        TextKey::UpdateUpToDate => "当前版本无需更新。",
        TextKey::UpdateCheckFailed => "检查更新失败，可稍后重试。",
        TextKey::UpdateView => "查看更新",
        TextKey::UpdateDownload => "下载更新",
        TextKey::UpdateDownloading => "正在下载",
        TextKey::UpdateProgress => "已下载 {} / {}",
        TextKey::UpdateVerifying => "正在校验更新包",
        TextKey::UpdateReady => "更新包校验通过，可以安装。",
        TextKey::UpdatePreparing => "正在准备更新，请稍候。",
        TextKey::UpdateInstallRestart => "安装并重启",
        TextKey::UpdateRetry => "重新下载",
        TextKey::UpdateBusy => "当前操作完成后即可继续。",
        TextKey::UpdateRestartHint => "安装时软件会退出，完成后自动重启。",
        TextKey::UpdateFailedNetwork => "更新下载失败，请检查网络后重试。",
        TextKey::UpdateFailedChecksum => "无法取得有效的 SHA-256 校验值，禁止安装。",
        TextKey::UpdateFailedIntegrity => "更新包校验失败，已删除无效临时包，禁止安装。",
        TextKey::UpdateFailedUpdater => "更新程序无法启动或准备失败，当前软件继续运行。",
        TextKey::UpdateUnsupported => "当前目录不支持原地更新，请使用正式 portable 包。",
        TextKey::UpdateRecoveryNotice => "更新未完成，已重新启动旧版软件。原有文件仍然保留。",
        TextKey::HostPreparingPlan => "正在准备修复方案…",
        TextKey::HostBackingUp => "正在备份并修复代理配置…",
        TextKey::HostRestoring => "正在恢复原配置…",
        TextKey::HostChecking => "正在检查电脑网络与代理设置…",
        TextKey::HostConfigChanged => "代理配置已修改；重启代理后请重新检查。",
        TextKey::HostNotFinished => "本次电脑网络检查或修复未完成",
        TextKey::HostNotChecked => "电脑网络与代理尚未检查。",
        TextKey::HostStale => "电脑网络信息已过期，请重新检查。",
        TextKey::HostModulePassed => "模块连接正常；",
        TextKey::HostMissingAdapter => "代理软件指定的出口网卡已不存在。",
        TextKey::HostAdapterDown => "代理指定的网卡仍在电脑上，但当前未连接或已禁用。",
        TextKey::HostOutletUnknown => "暂时无法确定代理出口问题，请查看处理步骤。",
        TextKey::HostProxyInUse => "电脑正在通过代理或 VPN 接口联网；模块通路单独检查。",
        TextKey::HostNoKnownProblem => "未发现已知的固定出口网卡问题。",
        TextKey::HostHeading => "电脑网络与代理",
        TextKey::HostNoModuleHint => "未连接模块，仍可检查电脑网络与代理设置。",
        TextKey::HostCheckAgain => "重新检查",
        TextKey::HostReviewRepair => "查看修复方案",
        TextKey::HostProxyOff => "未开启",
        TextKey::HostProxyManual => "手动代理",
        TextKey::HostProxyAutoScript => "自动配置脚本；未执行脚本",
        TextKey::HostProxyAutoDetect => "自动检测；尚未验证",
        TextKey::HostProxyMixed => "多种代理设置；需进一步确认",
        TextKey::HostProxyUnknown => "未获取",
        TextKey::HostWindowsProxy => "Windows 系统代理",
        TextKey::HostConfiguredAdapter => "磁盘配置引用网卡（运行态未核实）",
        TextKey::HostProxyBoundAdapter => "代理指定网卡",
        TextKey::HostUnsupportedConfig => {
            "此配置或版本暂不支持自动修改，请在代理软件中检查出站接口设置。"
        }
        TextKey::HostConfigUnreadable => "代理配置无法可靠读取",
        TextKey::HostRemoveOutletPrefix => "将取消代理软件对",
        TextKey::HostRemoveOutletSuffix => {
            "的固定出口设置。之后可能使用 Wi-Fi、有线网络或其他可用出口。将先备份原配置。请先完整退出 Clash Verge Rev 及相关核心。"
        }
        TextKey::HostPlanExpired => "修复方案已过期，请重新检查后再查看方案。",
        TextKey::HostKeepOriginal => "保留原设置",
        TextKey::HostWaitCurrentOperation => "正在处理，请等待本次操作结束",
        TextKey::HostBackupAndRepair => "备份并修复",
        TextKey::HostRestartedCheckAgain => {
            "配置已修改。重启代理后点击“重新检查”；配置改动不等于互联网已经恢复。"
        }
        TextKey::HostRestoreOriginal => "恢复原配置",
        TextKey::HostStepsHeading => "处理步骤",
        TextKey::HostConfiguredAdapterRef => "磁盘配置引用了网卡“{}”，尚未核实当前运行态。",
        TextKey::HostCommandNotSubmitted => "命令未提交，请稍后重试。",
        TextKey::HostVersionUnknown => "版本未确认",
        TextKey::HostStepsBody => {
            "先检查模块与 SIM；若模块公网检查通过，但代理仍报找不到网卡，请在代理软件中检查“出站接口”是否指向已移除或改名的网卡。多网卡用户可能有意固定出口，修改前确认用途。配置来源不明确时，请在代理软件中手动调整。"
        }
        TextKey::OnboardingWaiting => "等待检查 / 设备就绪",
        TextKey::OnboardingChecking => "正在检查…",
        TextKey::OnboardingNeedsReview => "需要查看原因",
        TextKey::OnboardingDisabledUnverified => "已关闭，尚未验证",
        TextKey::OnboardingStaleRefresh => "结果已过期，请刷新",
        TextKey::OnboardingCheckUsb => "USB 设备识别",
        TextKey::OnboardingCheckAdapter => "Windows 网卡",
        TextKey::OnboardingCheckAt => "AT 串口通信",
        TextKey::OnboardingCheckCellular => "SIM 与蜂窝网络",
        TextKey::OnboardingCheckBoundPublic => "模块公网连接",
        TextKey::OnboardingCheckBoundDns => "模块 DNS 解析",
        TextKey::OnboardingRestartStep => "2. 请先重启电脑",
        TextKey::OnboardingEnterAfterRestart => "进入面板（需重启）",
        TextKey::OnboardingAutoCheckStep => "2. 自动检查模块",
        TextKey::OnboardingStartUsing => "开始使用",
        TextKey::OnboardingEnterPanel => "进入面板",
        TextKey::OnboardingPassedNote => {
            "模块绑定的公网与 DNS 检查通过；电脑实际出口仍可能由 Wi-Fi 或 VPN 决定。"
        }
        TextKey::OnboardingNotPassedNote => {
            "等待检查、未连接、证据过期或关闭主动联网检查，不等于驱动损坏。具体原因可进入面板查看。"
        }
        TextKey::OnboardingTitle => "连接你的 4G 模块",
        TextKey::OnboardingIntro => "连接、检查，然后开始使用。也可以随时进入面板。",
        TextKey::OnboardingSetupResultHeading => "本次驱动安装结果",
        TextKey::OnboardingStep1Title => "1 · 连接模块",
        TextKey::OnboardingStep1Body => {
            "插好 SIM 卡，使用支持数据传输的 USB 线连接电脑。已有驱动可直接使用。"
        }
        TextKey::OnboardingHostHeading => "电脑网络与代理（可选检查）",
        TextKey::OnboardingCheckHost => "检查电脑网络",
        TextKey::OnboardingHostNote => "即使没插模块也能检查；代理配置问题不会当作驱动损坏。",
        TextKey::OnboardingStep3Title => "3. 需要驱动时再安装",
        TextKey::OnboardingBundledDriverNote => {
            "已找到本地驱动资源，安装前还会校验签名和文件。此包不能覆盖所有接口（包括未匹配的 MI_04）；如有缺驱动接口无法匹配，将在安装前停止。已有接口正常时无需重复安装。"
        }
        TextKey::OnboardingUseBundledDriver => "使用内置驱动",
        TextKey::OnboardingBundledDriverHint => {
            "点击后先确认，再退出面板并显示 Windows 授权；安装结束或取消授权后会自动返回面板。需要重启时，请先重启电脑。"
        }
        TextKey::OnboardingNoBundledDriverNote => {
            "此版本未附带完整驱动资源，不能在这里离线安装。请先打开 Windows 设置 → Windows 更新 → 可选更新，查看驱动更新；也可联系 DJI 官方支持取得适配此模块的驱动，按厂商说明安装后点击“重新检查”。"
        }
        TextKey::OnboardingOpenWindowsUpdate => "打开 Windows 更新",
        TextKey::OnboardingDjiCompatibilityLink => "DJI 官方兼容与驱动说明",
        TextKey::OnboardingOfficialLinkNote => {
            "官方网页提供兼容说明与支持入口，不是驱动直达下载。网页不会自动安装；Windows 更新也不保证提供该模块的全部驱动。"
        }
        TextKey::ReportSectionSystem => "系统及程序进程",
        TextKey::ReportSectionUsb => "USB 与异常设备",
        TextKey::ReportSectionDrivers => "已绑定驱动与 INF",
        TextKey::ReportSectionSerial => "串口枚举",
        TextKey::ReportSectionAdapters => "网卡与驱动",
        TextKey::ReportSectionNetwork => "IP、DNS 与默认路由",
        TextKey::ReportSectionSecurity => "安全软件状态",
        TextKey::ReportSectionDriverHistory => "Windows 驱动安装记录",
        TextKey::ReportSectionIntegrity => "程序及驱动文件完整性",
        TextKey::ReportNoLogDirectory => "无法导出：当前用户的日志目录不可用",
        TextKey::ReportPreparing => "正在准备详细日志…",
        TextKey::ReportStartFailed => "无法启动导出：{error}",
        TextKey::ReportExporting => "正在导出详细日志 · {text}",
        TextKey::ReportExported => "详细日志已导出：{}",
        TextKey::ReportIncomplete => "导出未完成：{}（已生成的部分报告保留在导出目录）",
        TextKey::ReportThreadEnded => "导出线程提前结束，部分报告保留在导出目录",
        TextKey::ReportFileName => "大疆4G详细诊断-{}-{}.txt",
        TextKey::ReportHeader => {
            "DJI 4G SUPPORT REPORT schema=1\nREPORT_STARTED unix_ms={}\napp_version={} exe={} pid={} architecture={}\n包含设备实例 ID、硬件 ID、驱动、网络配置及本程序日志。请仅发给排障人员。\n不读取短信正文、通讯录、SIM 号码或口令；不安装驱动、不修改网络、不发送 AT 命令。\n某节失败或超时不会阻止其他节导出。文件末尾 REPORT_COMPLETE 表示收集流程结束，不代表设备正常。\n"
        }
        TextKey::ReportUiSummary => "界面诊断摘要",
        TextKey::ReportMachineSnapshot => "机器可读快照",
        TextKey::ReportSnapshotNote => "检测阶段、时间、错误码和设备绑定",
        TextKey::ReportTimelineNote => "最近检测状态变化（仅本次进程已观察到的变化）",
        TextKey::ReportCollectLogs => "收集程序及驱动安装日志",
        TextKey::ReportHistoryScope => "历史安装日志范围",
        TextKey::ReportHistoryCapped => "目录超过 12 个，仅收集最近 12 个版本",
        TextKey::ReportHistoryHeading => "历史安装日志",
        TextKey::ReportLogDirectory => "日志目录",
        TextKey::ReportCollectionScope => "日志收集范围",
        TextKey::ExportTitle => "DJI 一代 4G 面板 · 诊断信息导出\n",
        TextKey::ExportGeneratedAt => "生成时间（UTC）：{}\n",
        TextKey::ExportPrivacy => "隐私说明：{}\n",
        TextKey::ExportSectionOverview => "\n【概览】\n",
        TextKey::ExportVerdict => "当前判定：{}\n",
        TextKey::ExportEvidenceState => "证据状态：{}\n",
        TextKey::ExportSectionDevice => "\n【设备】\n",
        TextKey::ExportDeviceId => "容器 / 设备实例标识：{}",
        TextKey::ExportSectionCellular => "\n【蜂窝网络】\n",
        TextKey::ExportSectionNetwork => "\n【Windows 网络】\n",
        TextKey::ExportSectionSms => "\n【短信】\n",
        TextKey::ExportSendRequest => "发送请求：{}；阶段：{:?}；结果：{:?}",
        TextKey::ExportSendError => {
            "发送错误：{}；CMS：{:?}；CME：{:?}；系统错误：{:?}；可能已提交：{}"
        }
        TextKey::ExportSectionEvidence => {
            "\n【连接证据】\n模块绑定路由查询只证明找到了匹配路由，不证明网关可达。\n"
        }
        TextKey::ExportLabelWithCode => "{}（代码 {}）",
        TextKey::ArchiveHeading => "本地历史",
        TextKey::ArchiveIntro => {
            "开启后，已读到的收件短信会保存在这台电脑。断开模块或重启软件后仍可查看；这里不能删除模块中的短信。"
        }
        TextKey::ArchiveRetention => {
            "仅当前 Windows 用户可解密；最多保存 5000 条，超过保存日期 90 天自动清除。关闭保存不会删除已有历史。"
        }
        TextKey::ArchiveUnavailable => "本地历史暂不可用：无法确定用户目录，或当前处于模拟演示。",
        TextKey::ArchiveToggle => "在这台电脑保存短信历史（可随时关闭）",
        TextKey::ArchiveExportTarget => "导出目标：{}",
        TextKey::ArchiveSearchHint => "搜索号码或正文",
        TextKey::ArchiveFilterAll => "全部历史",
        TextKey::ArchiveFilterWeek => "近 7 天保存",
        TextKey::ArchiveFilterMonth => "近 30 天保存",
        TextKey::ArchiveExportTxt => "导出 TXT",
        TextKey::ArchiveClear => "清空本地历史",
        TextKey::ArchiveCount => "显示 {} / {} 条 · 按保存顺序排列",
        TextKey::ArchiveEmpty => {
            "暂无本地历史。开启保存后，在“模块短信”中读取短信即可；无法找回模块中已被删除且未保存的消息。"
        }
        TextKey::ArchiveNoMatch => "没有符合筛选条件的记录。",
        TextKey::ArchiveNoTimestamp => "发送时间未提供",
        TextKey::ArchiveIncompleteTag => " · 分片未齐",
        TextKey::ArchiveSourceGroup => "来源分组：{} · 历史副本",
        TextKey::ArchiveCopyBody => "复制正文",
        TextKey::ArchiveClearDescription => {
            "将删除这台电脑保存的全部短信历史，并关闭后续保存。模块中的短信不受影响。此操作无法撤销。"
        }
        TextKey::ArchiveClearConfirm => "确认清空",
        TextKey::ArchiveExportTitle => "导出短信明文",
        TextKey::ArchiveExportDescription => {
            "将全部已保存历史（包含号码和正文）导出为未加密 TXT。请妥善保管，分享前检查隐私。"
        }
        TextKey::ArchiveExportConfirm => "确认导出全部",
        TextKey::ArchiveCancel => "取消",
        TextKey::ArchiveReady => "本地历史已就绪；仅保留最近 90 天、最多 5000 条",
        TextKey::ArchivePausedCapture => "档案读取或保存失败，已暂停采集；请检查或清空本地历史",
        TextKey::ArchivePausedExport => "档案读取或保存失败，已暂停导出；请先处理档案错误",
        TextKey::ArchiveExported => "已导出明文 TXT，请妥善保管该文件",
        TextKey::ArchiveUpdated => "本地历史已更新；仅保留最近 90 天、最多 5000 条",
        TextKey::ArchiveWorkerFailed => "无法启动短信档案后台任务",
        TextKey::ArchiveReading => "正在读取本地历史…",
        TextKey::ArchiveWorkerStopped => "短信档案后台任务已停止",
        TextKey::ArchiveDemoStatus => "模拟历史，仅用于界面验收，没有读取或保存真实短信",
        TextKey::ArchiveDemoBody => {
            "【模拟短信】这是一条本地历史示例，仅用于检查阅读、筛选和导出提示。"
        }
        TextKey::ArchiveNoIdentity => "未取得稳定设备和 SIM 身份，本次不写入本地历史",
        TextKey::ArchiveBusy => "本地历史正在处理，请稍后重试",
        TextKey::ArchiveOpenFailed => "无法打开本地短信档案；原文件已保留",
        TextKey::ArchiveSizeCheckFailed => "无法检查档案大小",
        TextKey::ArchiveTooLarge => "本地短信档案超过 32 MiB，未加载或覆盖",
        TextKey::ArchiveReadFailed => "读取短信档案失败",
        TextKey::ArchiveInvalidFormat => "短信档案格式无效；原文件已保留",
        TextKey::ArchiveDecryptedTooLarge => "解密后的短信档案超过大小限制",
        TextKey::ArchiveCorrupt => "短信档案内容损坏；原文件已保留",
        TextKey::ArchiveTooManyRecords => "短信档案记录数超过上限；原文件已保留",
        TextKey::ArchiveEncodeFailed => "无法编码短信档案",
        TextKey::ArchiveSizeCapSave => "短信档案已达大小上限，本次未保存",
        TextKey::ArchiveSizeCapEncrypt => "加密档案已达大小上限，本次未保存",
        TextKey::ArchiveClearFailed => "无法清空本地短信档案；未删除现有记录",
        TextKey::ToolStateNotQueried => "未查询",
        TextKey::ToolStateAvailable => "可用",
        TextKey::ToolStateNoData => "无数据",
        TextKey::ToolStateUnsupported => "固件不支持",
        TextKey::ToolStateTemporarilyUnavailable => "暂时不可用",
        TextKey::ToolStateFormatMismatch => "格式不匹配",
        TextKey::ToolStateTimeout => "查询超时",
        TextKey::ToolResultOk => "模块返回 OK；配置是否生效需另行确认",
        TextKey::ToolResultRejected => "模块明确拒绝了本次命令",
        TextKey::ToolResultUnsupported => "模块表示不支持此命令",
        TextKey::ToolResultNoAnswer => "没有收到可用应答（超时、串口错误或已断开）",
        TextKey::ToolResultUnrecognized => "模块有应答，但响应格式未被识别",
        TextKey::ToolResultCancelled => "未执行的命令已取消；已执行项见终端记录",
        TextKey::ToolResultMaybeWritten => "可能已写入但未收到最终应答；不会自动重试",
        TextKey::ToolResultInvalidated => "设备或 SIM 已变化，本次结果已作废",
        TextKey::ToolInputEmpty => "请输入一条 AT 命令",
        TextKey::ToolInputTooLong => "命令超过 256 个字符",
        TextKey::ToolInputNotAscii => "命令只能包含 ASCII 字符",
        TextKey::ToolInputControlChars => "命令不能包含控制字符或换行",
        TextKey::ToolInputSemicolon => "命令不能包含分号链式调用",
        TextKey::ToolInputMustStartAt => "命令必须以 AT 开头",
        TextKey::ToolInputNotWhitelisted => "该命令不在只读白名单内",
        TextKey::ToolInputNeedsInteractive => "此命令族需要交互式会话，文本终端无法安全驱动",
        TextKey::ToolPresetAttention => "模块响应（AT）",
        TextKey::ToolPresetManufacturer => "制造商（AT+CGMI）",
        TextKey::ToolPresetModel => "型号（AT+CGMM）",
        TextKey::ToolPresetFirmware => "固件版本（AT+CGMR）",
        TextKey::ToolPresetSim => "SIM 状态（AT+CPIN?）",
        TextKey::ToolPresetSignal => "信号质量（AT+CSQ）",
        TextKey::ToolPresetCarrier => "运营商（AT+COPS?）",
        TextKey::ToolPresetRegistration => "网络注册（AT+CEREG?）",
        TextKey::ToolPresetAttach => "分组附着（AT+CGATT?）",
        TextKey::ToolPresetPdpContexts => "PDP 上下文（AT+CGDCONT?）",
        TextKey::ToolPresetPdpActive => "PDP 激活状态（AT+CGACT?）",
        TextKey::ToolPresetPdpAddress => "PDP 地址（AT+CGPADDR）",
        TextKey::ToolPresetUsbMode => "USB 网络模式（AT+QCFG=\"usbnet\"）",
        TextKey::ToolPresetTemperature => "温度（AT+QTEMP）",
        TextKey::ToolPresetServingCell => "服务小区（AT+QENG=\"servingcell\"）",
        TextKey::ToolPresetSmsFormat => "短信格式（AT+CMGF?）",
        TextKey::ToolPresetSmsStorage => "短信存储（AT+CPMS?）",
        TextKey::ToolBatchPresets => "全部预设查询（批量）",
        TextKey::ToolAdvancedAt => "AT 命令（高级）",
        TextKey::ToolTaskIdle => "空闲",
        TextKey::ToolTaskQueued => "排队中",
        TextKey::ToolTaskRunning => "执行中",
        TextKey::ToolTaskCancelling => "正在取消",
        TextKey::ToolTaskFinished => "已结束",
        TextKey::ToolElapsedMinutes => "{} 分 {} 秒",
        TextKey::ToolElapsedSeconds => "{} 秒",
        TextKey::ToolElapsedMillis => "{} 毫秒",
        TextKey::ToolUsbModeDjiNdis => "DJI NDIS（电脑网卡）",
        TextKey::ToolQueueFull => "命令队列已满，请稍后重试",
        TextKey::ToolChannelClosed => "后台连接已关闭，请稍后重试",
        TextKey::ToolApnContextRange => "PDP 上下文编号必须在 1 到 16 之间。",
        TextKey::ToolApnInvalid => {
            "APN 不合法：不能为空、不能超过 100 字节，且不能包含引号、逗号、分号或控制字符。"
        }
        TextKey::ToolControlledUnavailable => "该受控操作当前不可用。",
        TextKey::ToolTimeUnknown => "时间未知",
        TextKey::ToolClockAgo => "{}（{}前）",
        TextKey::ToolsTitle => "设备工具",
        TextKey::ToolsIntro => "读取模块信息，按需执行经过确认的操作",
        TextKey::ToolsDeviceConnected => "设备已连接",
        TextKey::ToolsDeviceIdentity => "VID {} · PID {} · 设备标识 {}",
        TextKey::ToolsAtPort => "AT 端口 {}",
        TextKey::ToolsDeviceEpoch => "设备代次 {} · SIM 会话 {}",
        TextKey::ToolsNoDevice => "未检测到设备",
        TextKey::ToolsNoDeviceHint => "连接模块后才能执行查询与受控操作",
        TextKey::ToolsSimSession => "SIM 会话 {}",
        TextKey::ToolsTabPresets => "预设",
        TextKey::ToolsTabReadOnly => "只读查询",
        TextKey::ToolsTabSwitchHint => "任务执行期间仍可切换标签页，但写入按钮会被禁用",
        TextKey::ToolsTaskProgress => "任务进度",
        TextKey::ToolsBatchFinished => "批量查询已结束，请查看逐项结果",
        TextKey::ToolsFinishedUnknown => "已结束，结果未知",
        TextKey::ToolsNoTask => "当前没有设备工具任务",
        TextKey::ToolsCancelNote => {
            "取消只会停止等待，不能撤销已经写入模块的改动；写入超时后不会自动重试。"
        }
        TextKey::ToolsLastRejected => "最近一次请求被拒绝：{}（{}）",
        TextKey::ToolsProfileHeading => "模块资料",
        TextKey::ToolsRefreshProfile => "刷新模块资料",
        TextKey::ToolsRefreshProfileHint => "按顺序运行全部只读预设查询；不会写入模块",
        TextKey::FieldManufacturer => "制造商",
        TextKey::FieldModel => "型号",
        TextKey::FieldFirmwareVersion => "固件版本",
        TextKey::FieldUsbNetworkMode => "USB 网络模式",
        TextKey::ToolNotRecognized => "未识别",
        TextKey::FieldCapturedAt => "采集时间",
        TextKey::ToolsUsbModeUnverified => {
            "模块报告的 USB 网络模式不是本版本已验证的值；不会自动切换。"
        }
        TextKey::ToolsNoProfileYet => "尚未读取到模块资料；点击「刷新模块资料」运行一次只读查询。",
        TextKey::ToolsEvidenceHeading => "能力证据",
        TextKey::ToolsEvidenceNote => "每一行只反映一次真实查询的结果",
        TextKey::ToolsQuerying => "本项查询中",
        TextKey::ToolsQueryAgain => "重新查询此项",
        TextKey::ToolsQueryItem => "查询此项",
        TextKey::ToolsQueryingKeepLast => "本项查询中；下方保留上次结果与采集时间",
        TextKey::ToolsReason => "原因：{}（{}）",
        TextKey::ToolsCaptured => "采集：{} · 设备代次 {} · SIM 会话 {}",
        TextKey::ToolsStorageNote => "存储查询可用不代表模块支持发送短信。",
        TextKey::ToolsNotRunYet => "尚未执行此查询。",
        TextKey::ToolsStorageCaution => {
            "注意：「短信存储」查询成功仅表示存储查询可用，不代表模块支持发送短信。"
        }
        TextKey::ToolsConnectionHeading => "连接配置",
        TextKey::ToolsNoPdpYet => "尚未读取到 PDP 上下文；点击「刷新模块资料」。",
        TextKey::ToolsNoTemperature => "尚未读取到温度传感器。",
        TextKey::ToolsSensor => "传感器{}",
        TextKey::ToolsSensorNote => "传感器定义以固件为准。",
        TextKey::ToolsControlledHeading => "受控操作",
        TextKey::ToolsControlledNote => {
            "以下写入沿用修复页的受控流程：提交后仍需复核目标与风险，且超时不会自动重试。"
        }
        TextKey::FieldPdpContextShort => "PDP 上下文",
        TextKey::ToolsApnExample => "例如 internet",
        TextKey::ToolsEditApn => "修改 APN",
        TextKey::ToolsApnConfirm => "已弹出确认窗口；确认后执行：AT+CGDCONT={},\"IP\",\"{}\"",
        TextKey::ToolsApnRange => "PDP 上下文编号必须是 1 到 16 的整数。",
        TextKey::ToolsSwitchTo => "切换为{}",
        TextKey::ToolsSwitchHint => "通过受控修复流程切换；会重枚举模块",
        TextKey::ToolsConfirmRuns => "已弹出确认窗口；确认后执行：{}",
        TextKey::ToolsCurrentUnrecognized => "当前值未识别，本版本不提供切换。",
        TextKey::ToolsNotQueriedRefresh => "未查询；请先刷新模块资料。",
        TextKey::ToolsRestartModule => "重启模块",
        TextKey::ToolsRestartCommand => "AT+CFUN=1,1；会中断当前连接",
        TextKey::ToolsRestartConfirm => "已弹出确认窗口；确认后执行：AT+CFUN=1,1",
        TextKey::ToolsRestartNote => "重启会暂时中断模块连接。",
        TextKey::ToolsReadOnlyHeading => "只读 AT 查询",
        TextKey::ToolsReadOnlyNote => "只读白名单内的查询不需要逐条确认",
        TextKey::ToolsChoosePreset => "选择预设查询",
        TextKey::ToolsRunQuery => "运行查询",
        TextKey::ToolsWhitelistHint => "或输入白名单内的只读命令，例如 AT+CSQ",
        TextKey::ToolsNotInList => {
            "这条命令不在只读查询列表内。若了解其作用，可到 AT 命令（高级）检查并逐条确认；不确定时请使用预设查询。"
        }
        TextKey::ToolsOpenAdvanced => "打开 AT 命令（高级）",
        TextKey::ToolsBusyReadOnly => "任务执行期间只能读取已有结果，查询按钮已禁用。",
        TextKey::ToolsInvalidInput => "输入无效：{}（{}）",
        TextKey::ToolsSessionUnlocked => "本次会话已解锁",
        TextKey::ToolsEnableAtInput => "启用 AT 命令输入",
        TextKey::ToolsUnlockScope => "解锁只在本次会话内有效，设备或 SIM 变化后会自动重新锁定。",
        TextKey::ToolsAdvancedNote => {
            "供了解 AT 命令的用户排查问题。每次只发送一条经校验的命令；确认前会显示完整内容。命令可能修改配置或中断连接。"
        }
        TextKey::ToolsAtHint => "输入一条 AT 命令，例如 AT+CSQ",
        TextKey::ToolsExecute => "执行",
        TextKey::ToolsClearInput => "清空输入",
        TextKey::ToolsLocked => "终端处于锁定状态：解锁前不会显示输入框与执行按钮。",
        TextKey::ToolsCheckFailed => "命令未通过校验：{}（{}）",
        TextKey::ToolsNormalizedWrite => "已识别为受控写入，将执行规范化命令：{}",
        TextKey::ToolsCommandFrozen => "命令已冻结，请在下方逐条确认后才会写入模块。",
        TextKey::ToolsAtPending => "AT 命令待确认",
        TextKey::ToolsFrozenList => "以下命令已冻结，确认后才会写入模块：",
        TextKey::ToolsUnknownEffect => "效果未知，可能改变配置或中断连接。",
        TextKey::ToolsPlanExpired => "已过期；需要重新准备同一条命令。",
        TextKey::ToolsPlanRemaining => "剩余 {} 秒内有效，过期后需要重新准备。",
        TextKey::ToolsLogHeading => "命令记录",
        TextKey::ToolsLogNote => "只保存在本机内存中的最近任务记录",
        TextKey::ToolsCopySummary => "复制诊断摘要",
        TextKey::ToolsCopySummaryNote => "仅包含操作类型、耗时和稳定结果码，不含响应内容",
        TextKey::ToolsCopyRaw => "复制原始响应",
        TextKey::ToolsCopyRawNote => "响应可能包含设备标识、号码或账户信息",
        TextKey::ToolsClearLog => "清空",
        TextKey::ToolsClearLogNote => "清空现有内存记录；正在运行的任务完成后仍可能产生新记录。",
        TextKey::ToolsShareCaution => "「复制原始响应」可能包含设备或账户信息，请谨慎粘贴分享。",
        TextKey::ToolsNoLog => "暂无任务记录；运行任意查询后在此查看响应。",
        TextKey::ToolsEmptyResponse => "（无响应内容）",
        TextKey::ToolsResponseTruncated => "响应过长，已截断",
        TextKey::ToolsHistoryHeader => "设备工具历史摘要（不含响应内容）\n",
        TextKey::ToolsTruncatedMark => "[响应过长，已截断]\n",
        TextKey::ComposeBusyDraftKept => "当前任务或发送确认尚未结束，草稿已保留。",
        TextKey::ComposeUnsupportedSender => {
            "此发件人不是受支持的短信号码，无法直接回复。原号码未被修改。"
        }
        TextKey::ComposeDemoDraft => {
            "【模拟数据·界面验收】这是一条仅用于截图的短信草稿，请勿实际发送。"
        }
        TextKey::ComposeBackendBusy => {
            "后台正忙，本条短信未提交。请等待当前任务结束后重试；草稿已保留。"
        }
        TextKey::ComposeQueueFull => "操作队列已满，未提交。请稍后重试；草稿已保留。",
        TextKey::ComposeChannelClosed => "后台连接已关闭，未提交。请恢复连接后重试；草稿已保留。",
        TextKey::ComposeQueued => "短信已排队，请勿重复发送",
        TextKey::ComposePreparing => "正在准备短信",
        TextKey::ComposeSubmitting => "正在提交短信",
        TextKey::ComposeAwaitingModule => "正在等待模块确认",
        TextKey::ComposeSubmittedUnknown => "已提交给模块，尚不能确认对方收到。",
        TextKey::ComposeFailedDraftKept => "发送失败，草稿已保留。",
        TextKey::ComposeUnknownMaybeSent => {
            "发送结果未知，可能已提交。请先核实，避免重复发送；草稿已保留。"
        }
        TextKey::ComposeFailureHeading => "失败原因与处理建议",
        TextKey::ComposeFailureStage => "失败阶段：{} · 错误码：{}",
        TextKey::ComposeSystemError => "系统错误：{}",
        TextKey::ComposeTitle => "新建短信",
        TextKey::ComposeIntro => "通过当前连接的 4G 模块发送",
        TextKey::ComposeRecipient => "收件人",
        TextKey::ComposeRecipientHint => "国际号码或客服短号",
        TextKey::ComposeRecipientNote => {
            "输入 +8613800138000 等完整号码，或 10086 / 10010 / 10001。"
        }
        TextKey::ComposeBodyLabel => "短信内容",
        TextKey::ComposeLength => "{} / 70 字",
        TextKey::ComposeBodyHint => "在这里输入短信内容…",
        TextKey::ComposeLimits => "单条短信 · 最多 70 字 · 不支持 Emoji",
        TextKey::ComposeCostNote => "可能产生运营商费用；下一步将核对号码和正文。",
        TextKey::ComposeSendingWait => "正在发送，请等待结果",
        TextKey::ComposeModuleBusy => "模块正在处理其他任务，请稍候",
        TextKey::ComposeDraftKept => "草稿已保留",
        TextKey::ComposeNeedInput => "填写有效号码和内容后即可继续",
        TextKey::ComposeDraftReady => "草稿已就绪",
        TextKey::ComposeNextConfirm => "下一步：确认发送",
        TextKey::ComposeConfirmTitle => "确认发送短信",
        TextKey::ComposeConfirmIntro => "请核对以下完整号码和正文：",
        TextKey::ComposeConfirmNote => {
            "本次发送 1 条短信，可能产生运营商费用。模块接受不代表对方收到。"
        }
        TextKey::ComposeConfirmAction => "确认发送这条短信",
        TextKey::ComposeKeepDraftTitle => "保留当前草稿？",
        TextKey::ComposeKeepDraftBody => {
            "已有未发送草稿。可保留当前草稿，或替换为新草稿后继续编辑。"
        }
        TextKey::ComposeReplaceDraft => "替换为新草稿",
        TextKey::ComposeKeepDraft => "保留草稿",
        TextKey::ComposeRejected => {
            "模块已明确拒绝本条短信，未接受提交。请根据错误码排查后再手动发送。"
        }
        TextKey::ComposeMaybeSent => "可能已经提交，请勿直接重发。",
        TextKey::ComposeNotSubmitted => "本次未提交，可检查连接、SIM 卡和短信服务后重试。",
        TextKey::ComposeStageQueued => "排队",
        TextKey::ComposeStagePreparing => "准备",
        TextKey::ComposeStageSubmitting => "提交",
        TextKey::ComposeStageAwaiting => "等待模块结果",
        TextKey::ComposeStageDone => "完成",
        TextKey::ComposeSerialBusy => "串口正被其他任务占用。请等待任务结束，再手动重试。",
        TextKey::ComposeSerialOpenFailed => {
            "无法打开短信串口。请检查设备连接及其他串口程序是否占用。"
        }
        TextKey::ComposeSerialCloseTimeout => {
            "串口关闭超时，后台未能确认资源已释放。请恢复设备连接后再操作。"
        }
        TextKey::ComposeNoDevice => "当前没有可用设备。请连接模块并刷新设备状态。",
        TextKey::ComposeContextChanged => {
            "发送过程中设备或 SIM 卡发生变化。请核对当前设备及发送记录。"
        }
        TextKey::ComposeModuleRejected => {
            "模块拒绝了短信。请结合 CMS/CME 错误码检查 SIM 卡、余额及运营商短信服务。"
        }
        TextKey::ComposeSerialFailed => "串口通信失败。请检查 USB 连接和模块供电。",
        TextKey::ComposeTimeout => "等待模块响应超时。请核实发送记录及连接状态，避免重复发送。",
        TextKey::ComposeNoReference => {
            "模块没有返回短信提交编号，无法确认提交结果。请先核实是否已发送。"
        }
        TextKey::ComposeUnexpectedEnd => {
            "模块返回了非预期的结束响应，无法确认提交结果。请保留错误码并核实发送情况。"
        }
        TextKey::ComposeSerialBusyShort => "串口正在被其他任务占用，请等待任务结束。",
        TextKey::ComposePortBusyShort => "无法打开串口，请检查设备连接及端口占用。",
        TextKey::ComposeTimeoutShort => "等待模块响应超时，请检查连接及模块状态。",
        TextKey::ComposeValidationFailed => "号码或正文未通过校验，请检查支持的号码和正文长度。",
        TextKey::ComposeDeviceLost => "设备连接中断，请重新连接设备并刷新。",
        TextKey::ComposeGenericAdvice => {
            "请检查设备连接、SIM 卡状态和运营商短信服务；保留错误码以便排查。"
        }
        TextKey::ComposeCmeError => "CME 错误：{}",
        TextKey::SetupFailedTitle => "安装未完成",
        TextKey::SetupNoPayload => "此构建未包含安装资源，请使用完整安装包。",
        TextKey::SetupConfirmTitle => "安装大疆 4G 面板",
        TextKey::SetupConfirmBody => {
            "将为当前用户安装程序、离线驱动资源，并创建桌面快捷方式。\n\n安装程序本身不会修改系统驱动，完成后可选择安装驱动。\n\n是否继续？"
        }
        TextKey::SetupVerifyFailed => "安装文件验证失败。",
        TextKey::SetupDoneTitle => "安装完成",
        TextKey::SetupDoneBody => {
            "程序和离线驱动已安装，桌面快捷方式已创建。\n\n现在安装模块驱动吗？需要管理员确认。\n已有驱动可选“否”，直接打开程序。"
        }
        TextKey::SetupDriverCancelled => "驱动安装已取消。程序文件已安装，但未确认模块驱动可用。",
        TextKey::SetupDriverIncomplete => {
            "驱动安装未完成（退出码：{}）。程序文件已安装，但不会自动打开面板。\n\n若安全软件有拦截，请保留报告中的检测名称与文件路径。不要关闭防护；请将报告交给开发者核查。"
        }
        TextKey::PortableBadResourcePath => "安装资源路径无效：{}",
        TextKey::PortableBadResourceDir => "资源目录无效",
        TextKey::PortableWriteFailed => "无法释放资源 {}：{}",
        TextKey::PortableReadFailed => "无法读取资源 {}：{}",
        TextKey::PortableVerifyFailed => "资源校验失败：{}。请保留安全软件报告，不要关闭防护。",
        TextKey::PortableLaunchFailed => "大疆 4G 面板启动失败",
        TextKey::PortableNoPayload => "此构建未包含独立运行资源。",
        TextKey::PortableNoAppData => "无法读取当前用户的应用数据目录",
        TextKey::PortableOpenFailed => "无法打开面板：{}。如有安全软件拦截，请保留报告。",
        TextKey::PortableExitedAbnormally => "面板异常退出：{}。请保留安全软件报告及程序日志。",
        TextKey::DriverResultTitle => "模块驱动检查结果",
        TextKey::DriverResultBody => "{}\n\n{}\n\n点击“确定”返回面板。",
        TextKey::DriverNotStarted => "尚未开始安装",
        TextKey::DriverPanelRunning => {
            "面板尚未完全退出，或无法核实正在运行的面板。请返回原面板；关闭托盘中的面板后再尝试安装。未执行驱动安装。"
        }
        TextKey::DriverInstallTitle => "安装模块驱动",
        TextKey::DriverInstallBody => {
            "将校验随程序附带的驱动，仅为缺驱动接口选择匹配包。Windows 可能更新其他匹配同一驱动包的设备，不强制覆盖更优驱动。\n\n请先退出大疆 4G 面板（含托盘）。点击“是”后申请管理员授权，结束后自动返回面板。"
        }
        TextKey::DriverElevationFailed => "管理员授权未完成",
        TextKey::DriverOpenPanel => "请打开面板继续",
        TextKey::DriverNoReturn => {
            "{}\n\n未能自动返回。请从桌面正常打开“大疆 4G 面板”，在设置中打开首次连接引导。"
        }
        TextKey::DriverNoExePath => "无法确定程序位置，请重新打开完整程序。",
        TextKey::DriverNoExeDir => "无法确定程序目录。",
        TextKey::DriverCheckComponentFailed => "无法启动 Windows 驱动检查组件，请联系技术支持。",
        TextKey::DriverClockInvalid => "系统时间异常，未开始安装。",
        TextKey::DriverLogCreateFailed => {
            "无法创建安装日志，未开始安装。请将完整程序放在可写目录后重试。"
        }
        TextKey::DriverLogWriteFailed => "无法写入日志，未开始安装。",
        TextKey::DriverLogAppendFailed => "安装日志写入失败，请检查设备实际状态。日志位置：{}",
        TextKey::DriverLogPath => "详细安装日志：{}",
        TextKey::DriverCheckNotStarted => "Windows 驱动检查未能启动。{}",
        TextKey::DialogActionLine => "操作：{}\n",
        TextKey::DialogDisruptionLine => "中断：{}\n",
        TextKey::DialogRiskLine => "风险：{}\n",
        TextKey::DialogElevationLine => "提权：{}\n",
        TextKey::DriverInstallResultTitle => "模块驱动安装结果",
        TextKey::SettingsGeneral => "常规",
        TextKey::SettingsInterfaceTheme => "界面主题",
        TextKey::SettingsStartupTray => "启动与托盘",
        TextKey::SettingsAutoStart => "登录后自动启动",
        TextKey::SettingsStartHidden => "启动后隐藏到托盘",
        TextKey::SettingsLogging => "日志",
        TextKey::SettingsAbout => "关于",
        TextKey::SettingsVersion => "DJI 一代 4G 面板 · v{}",
        TextKey::CarrierChinaUnicom => "中国联通",
        TextKey::CarrierChinaMobile => "中国移动",
        TextKey::CarrierChinaTelecom => "中国电信",
        TextKey::CarrierChinaBroadnet => "中国广电",
        TextKey::RateHeading => "实时速率",
        TextKey::RatePeakDownload => "下载峰值",
        TextKey::RatePeakUpload => "上传峰值",
        TextKey::ThemeSystem => "跟随系统",
        TextKey::ThemeLight => "浅色",
        TextKey::ThemeDark => "深色",
        TextKey::SmsFragmentsRead => "已读取 {} / {} 个分片",
        TextKey::SmsErrPortBusy => "串口上一个操作尚未结束，请稍后刷新",
        TextKey::SmsErrPortAccess => "串口访问失败",
        TextKey::SmsErrNoAtPort => "未找到可验证的短信 AT 串口",
        TextKey::SmsErrPduMode => "短信 PDU 模式未确认",
        TextKey::SmsErrResponseInvalid => "模块响应未通过验证",
        TextKey::SmsErrTimeout => "短信查询超时",
        TextKey::SmsErrNoDevice => "无已验证的设备，请先检查概览中的模块连接状态",
        TextKey::SmsErrDeviceGone => "设备已断开",
        TextKey::SmsErrSimUnknown => "尚未识别 SIM 卡，请先返回概览刷新连接状态",
        TextKey::SmsErrSimChanged => "SIM 卡已更换，本次短信未加入列表；请返回概览重新检测",
        TextKey::SmsErrSimIdentity => "无法确认 SIM 卡身份，本次未读取短信；请检查 SIM 后重新检测",
        TextKey::SmsStopped => "已停止读取，原有列表已保留",
        TextKey::SmsErrContextChanged => "模块或 SIM 已变化，本次结果已丢弃，请重新检测",
        TextKey::SmsErrRestoreUnconfirmed => {
            "无法确认已恢复原读取位置；请先重新检测模块，暂不要发送或删除短信"
        }
        TextKey::SmsErrUnsupportedLocation => "模块不支持读取此位置，请使用当前存储位置",
        TextKey::SmsErrLimit => "短信数量或返回内容超过安全上限，本次列表未更新",
        TextKey::SmsErrQueryFailed => "短信查询失败",
        TextKey::SmsSystemError => "；系统错误 {}",
        TextKey::SmsPageTitle => "短信",
        TextKey::SmsReadDetailsAttention => "读取详情 · 需注意",
        TextKey::SmsReadDetails => "读取详情",
        TextKey::SmsRefreshList => "刷新列表",
        TextKey::SmsBusyWait => "当前通信任务尚未结束，请稍后刷新",
        TextKey::SmsPhaseWaiting => "等候读取",
        TextKey::SmsPhaseConfirming => "正在确认模块与 SIM",
        TextKey::SmsPhaseQueryingStorage => "正在查询短信存储位置",
        TextKey::SmsPhaseSelecting => "正在选择读取位置",
        TextKey::SmsPhaseReading => "正在读取历史短信",
        TextKey::SmsPhaseOrganizing => "正在整理短信",
        TextKey::SmsPhaseRestoring => "正在恢复原读取位置，请稍候",
        TextKey::SmsPhaseReleasing => "正在释放连接，请稍候",
        TextKey::SmsProgressRecords => "{} · 已读取 {} 个存储记录",
        TextKey::SmsStopReading => "停止读取",
        TextKey::SmsStopRequested => {
            "已请求停止，正在等待连接释放；自动刷新已暂停，可手动刷新继续。"
        }
        TextKey::SmsStorageUnconfirmed => "未确认",
        TextKey::SmsStorageSim => "SIM 卡",
        TextKey::SmsStorageModule => "模块",
        TextKey::SmsStorageModuleArea => "模块当前汇总区",
        TextKey::SmsStorageCurrentArea => "当前存储区",
        TextKey::SmsRecordsRead => "已读取 {} 个记录 · 存储位置与读取详情",
        TextKey::SmsLastRead => "最近读取：{} · {} 个短信分片，{} 个其他记录未展示",
        TextKey::SmsRefreshNote => {
            "刷新会读取当前位置中仍保存的全部短信（最多 1000 个存储记录）。长短信可能占多个记录；已被模块删除的内容无法重新读回。可开启“本地历史”保存之后读到的短信。"
        }
        TextKey::SmsOtherLocationsNote => {
            "其他位置可能还有短信。选择前会再次确认；读取期间暂时切换读取位置，完成后恢复，可能将未读短信标为已读。"
        }
        TextKey::SmsReadSim => "读取 SIM 卡短信",
        TextKey::SmsReadModule => "读取模块短信",
        TextKey::SmsTaskBusy => "当前通信任务尚未结束",
        TextKey::SmsLocationUnsupported => "模块尚未确认支持此存储位置",
        TextKey::SmsWillConfirm => "读取前将再次确认",
        TextKey::SmsReadOtherLocation => "读取其他位置的短信",
        TextKey::SmsReadOtherBody => {
            "将读取{}中保存的短信，期间暂停其他模块操作。读取可能改变短信已读状态；完成后会恢复原读取位置，不改变短信写入和接收位置。"
        }
        TextKey::SmsConfirmRead => "确认读取",
        TextKey::SmsSyncing => "正在同步模块短信…",
        TextKey::SmsUnreadCount => "{} 条未读",
        TextKey::SmsStorageUsage => "存储 {} / {}",
        TextKey::SmsAutoSyncPaused => "自动同步已暂停 · 点击刷新列表继续",
        TextKey::SmsSyncProblem => "短信同步遇到问题",
        TextKey::SmsSyncHistory => "同步提示与历史记录",
        TextKey::SmsCacheTrimmed => "本地缓存已移出 {} 条较早短信；模块存储可能仍有记录。",
        TextKey::SmsTabInbox => "收件箱  {}",
        TextKey::SmsTabOutgoing => "发送记录  {}",
        TextKey::SmsBackToList => "返回消息列表",
        TextKey::SmsFooterNote => "模块接受发送 ≠ 收件人已收到  ·  短信内容不会写入诊断日志",
        TextKey::SmsFilterAll => "全部",
        TextKey::SmsFilterIncomplete => "长短信缺片",
        TextKey::SmsClearSearch => "清空搜索",
        TextKey::SmsEmptyFilter => "没有符合筛选条件的短信",
        TextKey::SmsEmptyFilterHint => "切换为“全部”可查看其他短信",
        TextKey::SmsBalanceQuery => "查询话费",
        TextKey::SmsBalanceIntro => "选择 SIM 所属运营商，生成查询草稿后确认发送。",
        TextKey::SmsBalanceInstructions => "指令说明",
        TextKey::SmsBalancePortal => "打开广电营业厅查询",
        TextKey::SmsBalanceBroadnetHelp => "登录后进入“个人中心 → 我的账户”查询余额。",
        TextKey::SmsBalanceReplyHint => "短信回复显示在收件箱。查询指令及短信服务以当地套餐为准。",
        TextKey::SmsLastSynced => "上次完整读取 {}：{}前",
        TextKey::SmsSyncReadingCache => "正在读取模块，当前显示已有记录",
        TextKey::SmsSyncFailedCache => "同步失败，当前显示缓存",
        TextKey::SmsSyncPartialCache => "读取不完整，当前显示已有记录",
        TextKey::SmsSyncPausedCache => "同步已暂停，当前显示已有记录",
        TextKey::SmsSyncStaleCache => "记录较旧，等待同步",
        TextKey::SmsSyncStoredList => "模块存储短信 · 本页每 15 秒同步",
        TextKey::SmsSyncNotCompleted => "尚未完成模块存储读取",
        TextKey::SmsSearchHint => "搜索号码或短信内容",
        TextKey::SmsEmptySearch => "没有找到相关短信",
        TextKey::SmsEmptySearchHint => "试试其他号码或关键词",
        TextKey::SmsEmptyOutgoing => "还没有发送记录",
        TextKey::SmsEmptyOutgoingHint => "点击右上角「新建短信」开始写信",
        TextKey::SmsLoading => "正在读取短信",
        TextKey::SmsLoadingHint => "正在与模块同步，请稍候",
        TextKey::SmsInboxWaiting => "收件箱等待同步",
        TextKey::SmsInboxWaitingHint => "连接模块后会自动读取已保存的短信",
        TextKey::SmsInboxEmpty => "暂无收到的短信",
        TextKey::SmsInboxEmptyHint => "模块中暂时没有可显示的短信",
        TextKey::SmsUnavailable => "暂时无法读取短信",
        TextKey::SmsUnavailableHint => "请查看上方的具体错误，检查连接后重试",
        TextKey::SmsRefreshMessages => "刷新短信",
        TextKey::SmsNoTimestamp => "时间未提供",
        TextKey::SmsReaderEmpty => "消息阅读区",
        TextKey::SmsReaderEmptyHint => "从左侧选择一条短信，即可在这里查看完整内容",
        TextKey::SmsKindIncoming => "收到的短信",
        TextKey::SmsKindOutgoing => "发送记录",
        TextKey::SmsNoTimestampFromModule => "模块未提供时间",
        TextKey::SmsReply => "回复",
        TextKey::SmsConfirmDeleteFragments => "确认删除 {} 个已读取分片",
        TextKey::SmsDeleteFragments => "删除已读取 {} 个分片",
        TextKey::SmsDeleteMessage => "删除短信（{} 个分片）",
        TextKey::SmsDeleteBusy => "请等待通信任务结束，并重新核对短信分片",
        TextKey::SmsDeleteConflict => "分片身份存在冲突，暂不能删除；请重新读取并核对。",
        TextKey::SmsDeleting => "正在删除：已确认 {} / {} 个分片",
        TextKey::SmsDeletedAll => "已确认删除全部 {} 个已读取分片",
        TextKey::SmsDeleteUnknown => {
            "删除结果未知：已确认 {} / {} 个，{} 个未能确认。请刷新核对，不会自动重试。"
        }
        TextKey::SmsDeletePartial => "部分删除：已确认 {} / {} 个分片，其余失败或未执行。",
        TextKey::SmsDeleteNone => "未确认删除任何分片；请查看原因并重新读取。",
        TextKey::SmsDeleteResultTitle => "删除分片结果",
        TextKey::SmsDeleteConfirmed => "已确认删除",
        TextKey::SmsDeleteFailed => "失败",
        TextKey::SmsDeleteUnknownShort => "结果未知",
        TextKey::SmsDeleteNotRun => "未执行",
        TextKey::ArchiveExportDirFailed => "无法创建导出目录，请选择可写位置",
        TextKey::ArchiveExportCreateFailed => "无法导出：请选择可写位置和一个不存在的新文件名",
        TextKey::ArchiveExportWriteFailed => "导出写入失败，未保留不完整文件",
        TextKey::ArchiveInvalidPath => "短信档案路径无效",
        TextKey::ArchiveCreateDirFailed => "无法创建短信档案目录",
        TextKey::ArchiveTempFileFailed => "无法创建加密档案临时文件",
        TextKey::ArchiveWriteFailed => "写入加密档案失败",
        TextKey::ArchiveReplaceFailed => "替换加密档案失败；原文件已保留",
        TextKey::ExportSmsHeader => {
            "DJI 4G 本地短信历史（明文导出）\n采集时间为 Unix UTC 秒；原报时间保留模块原文。\n"
        }
        TextKey::ExportSmsRecord => {
            "设备/SIM 分区：{}\n发件人：{}\n原报时间：{}\n采集时间：{}\n内容{}：\n{}\n--------"
        }
        TextKey::ExportSmsIncomplete => "（分片不完整）",
        TextKey::DriverOutcomeReady => {
            "Windows 中的驱动接口检查已通过。面板将重新检查 USB、网卡和 AT 通信；短信及上网是否可用，请以新的检查结果为准。"
        }
        TextKey::DriverOutcomeRestartRequired => {
            "Windows 要求重启电脑，本次还不能确认驱动可用。请先保存工作并重启电脑，再打开面板检查模块。不要重复安装。"
        }
        TextKey::DriverOutcomeRestartAfterFailure => {
            "驱动安装的部分操作失败，且 Windows 已要求重启电脑；本次不能确认驱动可用。请保留安装日志，先保存工作并重启电脑，再打开面板检查。不要重复安装；重启后仍异常时，把日志交给技术支持。"
        }
        TextKey::DriverOutcomeCancelled => {
            "已取消驱动安装或 Windows 管理员授权，未开始安装。你可以继续使用面板；确实需要安装时，点击“使用内置驱动”，并在 Windows 授权窗口选择“是”。"
        }
        TextKey::DriverOutcomeNoMatch => {
            "当前驱动包没有为某个缺驱动接口找到唯一匹配项（例如 MI_04），本次未安装任何驱动。请先通过 Windows 更新查找适配驱动，或联系 DJI 官方支持提供该模块的匹配驱动。重复安装本包不能补齐该接口。"
        }
        TextKey::DriverOutcomeInterfacesAbnormal => {
            "驱动检查后仍有接口异常，暂时不能确认可用。请查看面板的新检查结果；若仍异常，打开设备管理器查看原因，并把安装日志交给技术支持。不要反复安装。"
        }
        TextKey::DriverOutcomeNoModule => {
            "没有检测到已连接的大疆一代模块，或模块在安装后暂时断开。请插稳支持数据传输的 USB 线，等待模块识别，再点击“重新检查”。"
        }
        TextKey::DriverOutcomePayloadInvalid => {
            "安装资源缺失、校验未通过，或与当前 Windows 不兼容，本次未开始安装。请重新取得完整的原始驱动版程序，或联系 DJI 官方支持；不要修改驱动文件。"
        }
        TextKey::DriverOutcomeIncomplete => {
            "安装未完成，不能确认设备状态。请查看本次安装日志，并在面板中重新检查；如需协助，把日志交给技术支持。"
        }
        TextKey::DriverWindowsUpdateFailed => "无法打开 Windows 更新，请从系统设置中打开。",
        TextKey::DriverAdminRequired => "安装器以管理员身份运行，请从桌面正常打开面板。",
        TextKey::DriverPanelNotSameDirectory => "等待对象不是同目录面板，未安装驱动",
        TextKey::DriverPanelExitTimeout => {
            "面板未能在 30 秒内退出，未安装驱动。请退出托盘中的面板后重试。"
        }
        TextKey::TimelineCellChangedDetail => "服务小区已变化",
        TextKey::TimelineAdapterLinkChangedDetail => "网卡链路状态变化",
        TextKey::TimelineRegistrationChangedDetail => "注册状态：{} → {}",
        TextKey::TimelineDnsChangedDetail => "DNS 探测：{} → {}",
        TextKey::TimelineRegistrationHomeDetail => "已注册到本地网络",
        TextKey::TimelineRegistrationRoamingDetail => "已注册到漫游网络",
        TextKey::TimelineRegistrationSearchingDetail => "正在搜索",
        TextKey::TimelineRegistrationDeniedDetail => "注册被拒绝",
        TextKey::TimelineRegistrationNotRegisteredDetail => "未注册",
        TextKey::TimelineRegistrationUnknownDetail => "未知",
        TextKey::TimelineDnsPassedDetail => "通过",
        TextKey::TimelineDnsFailedDetail => "失败",
        TextKey::TimelineDnsIncompleteDetail => "未完成",
        TextKey::ArchiveCryptoUnsupported => "此系统不支持 Windows 用户加密，未写入短信",
        TextKey::ArchiveCryptoTooLarge => "短信档案超过加密大小限制",
        TextKey::ArchiveCryptoFailed => "Windows 用户加密失败，未写入短信",
        TextKey::ArchiveCryptoDecryptFailed => {
            "无法解密本地短信档案：用户不匹配或文件损坏；原文件已保留"
        }
        TextKey::ToolUrcLine => "[模块主动上报] {}",
        TextKey::SmsFailureWithCode => "{}（{}）",
        TextKey::ComposeCmsError => "CMS：{}",
        TextKey::SmsDeleteMessageOne => "删除短信（{} 个分片）",
    }
}

/// Traditional Chinese with Hong Kong vocabulary.
///
/// GENERATED — do not hand-edit the arms. `docs/i18n-20260930/generate-zh-tw.py` derives them from
/// the simplified catalog above through the Windows character conversion plus the Hong Kong term
/// glossary and override table stored beside that script. Edit those, regenerate, and the
/// `no_simplified_forms_survive_in_traditional` test checks the result.
fn zh_tw(key: TextKey) -> &'static str {
    // ZH-TW-CATALOG-START
    match key {
        TextKey::AvailabilityDetectingTitle => "正在偵測",
        TextKey::AvailabilityDetectingReason => "正在收集並驗證當前裝置的連接證據。",
        TextKey::AvailabilityAvailableTitle => "可用",
        TextKey::AvailabilityAvailableReason => {
            "已確認此模組的網絡地址、路由、公共網絡連接和 DNS 均可用。"
        }
        TextKey::AvailabilityLimitedTitle => "受限",
        TextKey::AvailabilityUnavailableTitle => "不可用",
        TextKey::AvailabilityNotDetectedTitle => "未偵測到",
        TextKey::AvailabilityNotDetectedReason => {
            "當前枚舉未發現受支援的 DJI 一代 4G 模組（VID 2CA3、PID 4006）。"
        }
        TextKey::AvailabilityUnsupportedTitle => "不受支援",
        TextKey::AvailabilityUnsupportedReason => {
            "偵測到相關裝置，但它不是受支援的一代模組（PID 4006）；不會執行寫入或修復。"
        }
        TextKey::LimitedReasonDnsFailure => "模組資料通道可達，但通過該介面的 DNS 解析失敗。",
        TextKey::LimitedReasonSingleProtocolFamily => {
            "模組僅通過一種所需的 IP 協議族完成連接驗證。"
        }
        TextKey::LimitedReasonCompetingDefaultRoute => {
            "模組通道已通過驗證，但系統預設路由由 VPN 或 TUN 介面佔用。"
        }
        TextKey::LimitedReasonAtControlUnavailable => {
            "裝置已識別，但 AT 連接埠不可用（序列埠異常），無法讀取流動狀態。"
        }
        TextKey::LimitedReasonIncompleteEvidence => "現有證據不足，暫不能確認全部連接能力。",
        TextKey::UnavailableReasonCellularRejected => "SIM 或流動網絡註冊被明確拒絕。",
        TextKey::UnavailableReasonNoUsableAddressOrRoute => "模組網絡卡沒有可用的地址或路由。",
        TextKey::UnavailableReasonBoundPublicProbeFailed => {
            "經模組網絡卡綁定的公共網絡探測已連續失敗。"
        }
        TextKey::UnavailableReasonNoBoundReachability => {
            "Windows 可通過其他網絡聯網，但尚無證據證明此模組通道可達。"
        }
        TextKey::HotspotUnsupportedTitle => "熱點不可用",
        TextKey::HotspotOff => "已關閉",
        TextKey::HotspotStarting => "正在開啟",
        TextKey::HotspotOnWithClients => "已開啟（{client_count} 臺裝置已連接）",
        TextKey::HotspotOnClientsUnknown => "已開啟（連接裝置數未知）",
        TextKey::HotspotStopping => "正在關閉",
        TextKey::HotspotFailed => "熱點操作失敗。",
        TextKey::HotspotUnsupportedMissingPackageIdentity => {
            "當前執行方式沒有熱點功能所需的應用程式包身份。"
        }
        TextKey::HotspotUnsupportedMissingWifiControlCapability => {
            "當前安裝包未獲得控制流動熱點所需的系統能力。"
        }
        TextKey::HotspotUnsupportedNoWifiAdapter => "未偵測到可用於共享網絡的 Wi‑Fi 適配器。",
        TextKey::HotspotUnsupportedPolicyDisabled => "流動熱點已被系統或組織策略禁用。",
        TextKey::HotspotUnsupportedOperatingSystem => "當前 Windows 版本不支援此熱點控制方式。",
        TextKey::HotspotUnsupportedSourceProfileUnavailable => {
            "無法把流動熱點的上行連接安全地綁定到此模組。"
        }
        TextKey::IssueSeverityInfo => "提示",
        TextKey::IssueSeverityWarning => "警告",
        TextKey::IssueSeverityError => "錯誤",
        TextKey::IssueLayerDevice => "USB 裝置",
        TextKey::IssueLayerCellular => "流動網絡",
        TextKey::IssueLayerNetwork => "Windows 網絡",
        TextKey::IssueLayerBoundProbe => "模組綁定探測",
        TextKey::IssueLayerHotspot => "流動熱點",
        TextKey::IssueLayerOperation => "操作與修復",
        TextKey::EvidenceSourcePnp => "Windows 裝置枚舉",
        TextKey::EvidenceSourceAtControl => "AT 控制通道",
        TextKey::EvidenceSourceWindowsAdapter => "Windows 網絡卡狀態",
        TextKey::EvidenceSourceBoundGatewayProbe => "模組綁定路由查詢（歷史來源標記）",
        TextKey::EvidenceSourceBoundDnsProbe => "模組 DNS 綁定探測",
        TextKey::EvidenceSourceBoundPublicProbe => "模組公共網絡綁定探測",
        TextKey::EvidenceSourceGlobalRoute => "系統預設路由",
        TextKey::EvidenceSourceGlobalConnectivity => "Windows 全局聯網狀態",
        TextKey::EvidenceSourceHotspot => "Windows 流動熱點",
        TextKey::ClassificationPhaseStartup => "正在啟動偵測",
        TextKey::ClassificationPhaseRecentInsertion => "已發現新插入的裝置，正在偵測",
        TextKey::ClassificationPhaseReenumerating => "裝置正在重新枚舉",
        TextKey::ClassificationPhasePostWriteVerification => "正在驗證操作後的狀態",
        TextKey::ClassificationPhaseStable => "偵測完成",
        TextKey::ActionRefresh => "重新整理並重新偵測",
        TextKey::RepairDhcpDisabled => "模組網絡卡未啟用 IPv4 DHCP，無法續租。",
        TextKey::RepairApnInvalid => "APN 含不允許的字符。",
        TextKey::ActionRenewDhcp => "更新模組網絡卡的 DHCP 租約",
        TextKey::ActionApplyDnsAutomatic => "恢復自動取得 DNS",
        TextKey::ActionApplyDnsStatic => "應用程式靜態 DNS（{server_count} 個服務器）",
        TextKey::ActionApplyDnsProfile => "修改 DNS 配置",
        TextKey::ActionRestartAdapter => "重新啟動模組網絡卡",
        TextKey::ActionReenumerateDevice => "重新枚舉模組裝置",
        TextKey::ActionRestartModule => "重新啟動流動模組",
        TextKey::ActionEditApn => "修改 PDP 上下文 {cid} 的 APN",
        TextKey::ActionSetUsbProfileDjiNdis => "切換為電腦網絡卡（DJI NDIS）",
        TextKey::ActionSetUsbProfileEcm => "切換為 ECM 網絡卡",
        TextKey::ActionSetUsbNetworkProfile => "切換 USB 網絡配置",
        TextKey::ActionEnableHotspot => "開啟流動熱點",
        TextKey::ActionDisableHotspot => "關閉流動熱點",
        TextKey::RiskLevelLow => "低風險",
        TextKey::RiskLevelMedium => "中等風險",
        TextKey::RiskLevelHigh => "高風險",
        TextKey::OperationOutcomeApplied => "操作已應用程式，並已完成狀態回讀。",
        TextKey::OperationUsbConfigurationSaved => {
            "USB 網絡配置已儲存；需手動重新啟動模組後複檢，當前網絡卡模式尚未驗證。"
        }
        TextKey::OperationOutcomeFailed => "操作失敗。",
        TextKey::OperationOutcomeUnknown => {
            "無法確認操作結果；系統不會自動重試。請重新整理後核對裝置狀態。"
        }
        TextKey::DnsProfileAutomatic => "自動取得 DNS",
        TextKey::DnsProfileStatic => "靜態 DNS",
        TextKey::UsbNetworkProfileDjiNdis => "項目已驗證的 DJI NDIS 配置",
        TextKey::UsbNetworkProfileEcm => "項目已驗證的 ECM 配置",
        TextKey::DisruptionNone => "不會中斷連接",
        TextKey::DisruptionBrief => "連接可能短暫波動",
        TextKey::DisruptionConnectionInterrupting => "將暫時中斷網絡連接",
        TextKey::DisruptionDeviceReenumeration => "裝置將斷開並重新出現",
        TextKey::ActionSafetyUnsupportedDevice => "目標不是受支援的 DJI 一代 4G 模組，操作已阻止。",
        TextKey::ActionSafetyStaleEpoch => "裝置已重新連接或重新枚舉，請重新準備操作。",
        TextKey::ActionSafetyStaleSnapshot => "裝置狀態已經變化，請重新整理後重試。",
        TextKey::ActionSafetyTargetIdentityChanged => "目標裝置身份已經變化，操作已阻止。",
        TextKey::ActionSafetyBeforeStateChanged => "操作前狀態已經變化，請重新確認。",
        TextKey::ActionSafetyExpired => "此確認已過期，請重新準備操作。",
        TextKey::RollbackNotRequired => "無需回滾",
        TextKey::RollbackApplied => "已恢復原狀態",
        TextKey::RollbackFailed => "回滾失敗，請檢查當前狀態",
        TextKey::RollbackNotAttempted => "未執行回滾",
        TextKey::FreshnessFresh => "狀態為最新",
        TextKey::FreshnessStale => "狀態已過期，正在重新偵測",
        TextKey::FreshnessUnknown => "尚無有效的更新時間",
        TextKey::LastObservedAt => "上次偵測：{time}",
        TextKey::ObservedAgo => "更新於 {age}前",
        TextKey::ErrorPermissionDenied => "權限不足，無法完成此操作。",
        TextKey::ErrorDeviceRemoved => "操作期間裝置已斷開。",
        TextKey::ErrorDeviceIdentityChanged => "裝置身份已經變化，操作已停止。",
        TextKey::ErrorEvidenceExpired => "偵測依據已過期，請重新整理狀態。",
        TextKey::ErrorProbeFailed => "網絡探測未成功完成。",
        TextKey::ErrorDnsFailed => "通過模組介面的 DNS 解析失敗。",
        TextKey::ErrorTimeout => "操作等待超時。",
        TextKey::ErrorUnsupported => "當前裝置、系統或操作不受支援。",
        TextKey::ErrorCapabilityUnavailable => "所需的系統能力當前不可用。",
        TextKey::ErrorOperationCancelled => "操作已取消。",
        TextKey::ErrorVerificationFailed => "操作後的狀態驗證未通過。",
        TextKey::ErrorRollbackFailed => "未能恢復操作前的狀態，請檢查當前配置。",
        TextKey::ErrorInternal => {
            "應用程式發生內部錯誤。請重新整理狀態；如仍出現，請匯出診斷資訊。"
        }
        TextKey::ErrorHelperUnsigned => "helper 未簽名；特權修復保持關閉（開發候選不包含簽名）。",
        TextKey::ErrorHelperUnverified => "無法執行簽名驗證；特權修復保持關閉。",
        TextKey::SimReady => "SIM 已就緒",
        TextKey::SimMissing => "未偵測到 SIM",
        TextKey::SimPinRequired => "SIM 需要 PIN（本應用程式不會提交 PIN）",
        TextKey::SimPukRequired => "SIM 需要 PUK（本應用程式不會提交 PUK）",
        TextKey::SimRejected => "SIM 被拒絕",
        TextKey::SimUnknown => "SIM 狀態未知",
        TextKey::RegistrationHome => "已註冊到本地網絡",
        TextKey::RegistrationRoaming => "已註冊到漫游網絡",
        TextKey::RegistrationSearching => "正在搜索網絡",
        TextKey::RegistrationDenied => "網絡註冊被拒絕",
        TextKey::RegistrationNotRegistered => "尚未註冊到網絡",
        TextKey::RegistrationUnknown => "註冊狀態未知",
        TextKey::AttachAttached => "分組資料已附著",
        TextKey::AttachDetached => "分組資料未附著",
        TextKey::AttachUnknown => "分組資料附著狀態未知",
        TextKey::CellularBlockSimRejected => "SIM 被明確拒絕",
        TextKey::CellularBlockRegistrationRejected => "流動網絡註冊被明確拒絕",
        TextKey::DevicePresenceSupported => "已偵測到受支援的 DJI 一代 4G 模組",
        TextKey::DevicePresenceSupportedQuectelGeneric => {
            "已偵測到 Quectel 通用模組（VID 2C7C、PID 0125），僅支援唯讀檢查"
        }
        TextKey::DevicePresenceNotDetected => "未偵測到受支援的模組",
        TextKey::DevicePresenceUnsupported => "偵測到相關但不受支援的 USB 裝置",
        TextKey::DevicePresencePermissionDenied => "無法讀取裝置資訊：權限不足",
        TextKey::AdapterUsableAddressAndRoute => "網絡卡具有可用地址和路由",
        TextKey::AdapterNoUsableAddressOrRoute => "網絡卡沒有可用地址或路由",
        TextKey::BoundPublicSucceeded => "模組綁定的公共網絡探測通過",
        TextKey::BoundPublicFailed => "模組綁定的公共網絡探測失敗（連續 {count} 次）",
        TextKey::BoundPublicIncomplete => "公共網絡探測尚未完成",
        TextKey::BoundDnsSucceeded => "模組綁定的 DNS 解析通過",
        TextKey::BoundDnsFailed => "模組綁定的 DNS 解析失敗",
        TextKey::BoundDnsIncomplete => "DNS 探測尚未完成",
        TextKey::ProtocolCoverageAllRequired => "所需 IP 協議族均通過驗證",
        TextKey::ProtocolCoverageSingleFamily => "僅一種所需 IP 協議族通過驗證",
        TextKey::AtControlAvailable => "AT 控制通道可用",
        TextKey::AtControlUnavailable => "AT 控制通道不可用",
        TextKey::DefaultRouteTargetAdapter => "系統預設路由由模組網絡卡提供",
        TextKey::DefaultRouteVpnOrTun => "系統預設路由由 VPN 或 TUN 介面提供",
        TextKey::DefaultRouteOther => "系統預設路由由其他網絡介面提供",
        TextKey::GlobalConnectivityOnline => "Windows 當前可通過某個網絡聯網",
        TextKey::GlobalConnectivityOffline => "Windows 當前未偵測到全局聯網",
        TextKey::ProtocolApnEmpty => "APN 不能為空。",
        TextKey::ProtocolApnTooLong => "APN 不能超過 100 個 ASCII 字節。",
        TextKey::ProtocolApnUnsafeCharacter => {
            "APN 含有不允許的字符；請使用不含引號、逗號、分號或控制字符的 ASCII 文本。"
        }
        TextKey::ProtocolPdpContextIdOutOfRange => "PDP 上下文編號必須在 1 到 16 之間。",
        TextKey::ProtocolWrongPortData => "當前序列埠返回了非 AT 資料，已停止使用該連接埠。",
        TextKey::ProtocolLineTooLong => "模組返回的資料行超過安全長度限制。",
        TextKey::ProtocolResponseTooLarge => "模組響應超過安全大小限制。",
        TextKey::ProtocolTimeout => "等待模組響應超時。",
        TextKey::ProtocolDeviceRemoved => "等待響應時裝置已斷開。",
        TextKey::ProtocolUnexpectedData => "模組返回了無法安全解析的資料。",
        TextKey::AtFinalOk => "模組已確認命令",
        TextKey::AtFinalError => "模組拒絕了命令",
        TextKey::AtFinalCmeError => "模組返回 CME 錯誤（{detail}）",
        TextKey::AtFinalCmsError => "模組返回 CMS 錯誤（{detail}）",
        TextKey::AtFinalNoCarrier => "未建立載波連接",
        TextKey::AtFinalNoAnswer => "對端無響應",
        TextKey::AtFinalBusy => "模組當前忙",
        TextKey::AtFinalNoDialTone => "未偵測到撥號音",
        TextKey::PlatformNoSafeAtPort => "未找到可安全使用的 AT 連接埠；不會嘗試未知連接埠。",
        TextKey::PlatformAmbiguousAtPort => {
            "找到多個同等候選的 AT 連接埠，為避免誤操作已禁用寫入。"
        }
        TextKey::PlatformAtPortUnverified => {
            "找到多個候選連接埠，安全握手均未確認 AT 協議，為避免誤操作已禁用寫入。"
        }
        TextKey::PlatformUnsupportedPlatform => "當前平臺不支援 Windows 裝置枚舉。",
        TextKey::PlatformPnpEnumerateFailed => "無法完成 Windows 裝置枚舉。",
        TextKey::PlatformInterfaceEnumerateFailed => "無法枚舉裝置介面。",
        TextKey::PlatformPnpPermissionDenied => "Windows 拒絕讀取裝置資訊。",
        TextKey::PlatformPnpOpenFailed => "無法開啟 Windows 裝置資訊集。",
        TextKey::SerialQueueFull => "AT 請求佇列已滿，請稍後重試。",
        TextKey::SerialSessionClosed => "AT 工作階段已關閉。",
        TextKey::SerialIoFailed => "與模組序列埠通信失敗。",
        TextKey::SerialAtFinalError => "模組未接受該 AT 命令。",
        TextKey::NavOverview => "概覽",
        TextKey::NavDiagnostics => "診斷",
        TextKey::NavRepairs => "修復",
        TextKey::NavWireless => "無線",
        TextKey::NavSettings => "設定",
        TextKey::DiagnosticsTitle => "連接證據",
        TextKey::DiagnosticsIntro => {
            "以下檢查按裝置、流動網絡、Windows 網絡卡和模組綁定探測分層顯示。"
        }
        TextKey::FieldDeviceIdentity => "裝置身份",
        TextKey::FieldDeviceModel => "裝置型號",
        TextKey::FieldUsbIdentity => "USB 識別碼",
        TextKey::FieldProblemCode => "Windows 問題代碼",
        TextKey::FieldAtPort => "AT 連接埠",
        TextKey::FieldAdapter => "模組網絡卡",
        TextKey::FieldCarrier => "網絡供應商",
        TextKey::FieldRadioAccessTechnology => "接入制式",
        TextKey::FieldSignal => "訊號",
        TextKey::FieldSimState => "SIM 狀態",
        TextKey::FieldRegistration => "網絡註冊",
        TextKey::FieldAttachState => "分組資料附著",
        TextKey::FieldApn => "接入點（APN）",
        TextKey::FieldPdpAddress => "PDP 地址",
        TextKey::FieldWindowsAddresses => "Windows 地址",
        TextKey::FieldGateway => "網關",
        TextKey::FieldDnsServers => "DNS 服務器",
        TextKey::FieldDefaultRoute => "系統預設路由",
        TextKey::FieldBoundRouteProbe => "模組綁定路由查詢",
        TextKey::FieldBoundPublicProbe => "模組公共網絡探測",
        TextKey::FieldBoundDnsProbe => "模組 DNS 探測",
        TextKey::FieldProtocolCoverage => "IP 協議覆蓋",
        TextKey::FieldGlobalConnectivity => "Windows 全局聯網狀態",
        TextKey::FieldHotspot => "流動熱點",
        TextKey::FieldEvidenceSource => "證據來源",
        TextKey::FieldObservedAt => "偵測時間",
        TextKey::FieldPhoneNumber => "本機號碼",
        TextKey::FieldNumberSource => "來源",
        TextKey::FieldVerificationState => "驗證狀態",
        TextKey::FieldCaptureTime => "擷取時間",
        TextKey::FieldIccid => "SIM 卡 ICCID",
        TextKey::ValueUnknown => "未知",
        TextKey::ValueNotAvailable => "未取得",
        TextKey::ValueRedacted => "已隱藏",
        TextKey::ValueNotApplicable => "不適用",
        TextKey::ValueNumberNotProvided => "SIM/裝置未提供本機號碼",
        TextKey::ValuePhoneNumberNotRead => "未讀取到本機號碼",
        TextKey::ValueNumberSourceSimReport => "SIM/裝置報告",
        TextKey::ValueVerificationNotCarrierChecked => "未通過網絡供應商賬戶核驗",
        TextKey::ValueCaptureTimeSimSession => "本次 SIM 工作階段內",
        TextKey::ValueIccidNotRead => "未讀取到 ICCID",
        TextKey::IdentityHeading => "身份資訊",
        TextKey::ButtonShow => "顯示",
        TextKey::ButtonCopy => "複製",
        TextKey::ButtonCopied => "已複製",
        TextKey::ServingCellLayoutProvisional => "服務小區欄位佈局為候選方案，待實機確認",
        TextKey::FeatureStatusUnsupportedConfirmed => {
            "韌體不支援此查詢（裝置已返回明確的不支援錯誤）"
        }
        TextKey::FeatureStatusFormatMismatch => "格式不匹配：裝置有應答，但響應格式未被識別",
        TextKey::FeatureStatusTransportFailure => "本次超時：查詢未完成（裝置無應答或已斷開）",
        TextKey::FeatureStatusTemporarilyUnavailable => "暫時不可用：本次查詢未成功，原因尚未確認",
        TextKey::CheckPassed => "已通過",
        TextKey::CheckFailed => "未通過",
        TextKey::CheckUnavailable => "不可用",
        TextKey::CheckUnexecuted => "未執行",
        TextKey::CheckRunning => "偵測中",
        TextKey::CheckExpired => "已過期",
        TextKey::UnexecutedDisabledBySetting => "未執行：已被設定關閉",
        TextKey::UnexecutedNotScheduled => "未執行：尚未排程",
        TextKey::UnexecutedSuperseded => "未執行：已被更新的偵測取代",
        TextKey::AppTitle => "DJI 一代 4G 面板",
        TextKey::UnofficialNotice => {
            "非官方開源工具，與 DJI、白旺、Quectel、Microsoft 或網絡供應商無隸屬或認可關係。"
        }
        TextKey::OverviewQuestion => "此模組當前能否作為 Windows 的可用網絡上行？",
        TextKey::OverviewLastObservation => "上次偵測：{time}",
        TextKey::RateCaptionDown => "下載",
        TextKey::RateCaptionUp => "上傳",
        TextKey::RateWindow => "最近 {age}",
        TextKey::RatePeak => "峰值 {detail}",
        TextKey::RateSampling => "正在取樣網速……（每秒一個點）",
        TextKey::RateGradeChip => "速度：{detail}",
        TextKey::RateGradePending => "待測速",
        TextKey::RateGradeIdle => "空閑",
        TextKey::RateGradeBasic => "基礎",
        TextKey::RateGradeGood => "良好",
        TextKey::RateGradeExcellent => "優秀",
        TextKey::RateGradeVeryFast => "極速",
        TextKey::ButtonRefresh => "重新整理",
        TextKey::ButtonDiagnostics => "診斷",
        TextKey::ButtonRepair => "修復",
        TextKey::ButtonConfirm => "確認",
        TextKey::ButtonCancel => "取消",
        TextKey::ButtonClose => "關閉",
        TextKey::ButtonBack => "返回",
        TextKey::ButtonRetry => "重試",
        TextKey::ButtonDone => "完成",
        TextKey::ButtonViewDiagnostics => "查看診斷",
        TextKey::ButtonCopyAddress => "複製地址",
        TextKey::ButtonExportDiagnostics => "匯出診斷資訊",
        TextKey::ButtonOpenReleases => "開啟發布頁面",
        TextKey::StatusLoading => "正在載入",
        TextKey::StatusNoActiveOperation => "當前沒有正在執行的操作",
        TextKey::StatusExpired => "狀態已過期",
        TextKey::StatusQueueFull => "請求佇列已滿，請稍後重試",
        TextKey::CommandFeedbackBusy => "已有操作正在執行，請稍候再試。",
        TextKey::CommandFeedbackConfirmRejected => "確認未生效：操作計劃已失效，請重新準備。",
        TextKey::CommandFeedbackRejected => "操作未執行，請重新整理後重試。",
        TextKey::StatusBackendUnavailable => "後臺暫不可用，請稍後重試",
        TextKey::UnknownBackendError => {
            "發生未識別的錯誤。請重新整理狀態；如仍出現，請匯出診斷資訊。"
        }
        TextKey::SystemErrorNumber => "系統錯誤編號：{detail}",
        TextKey::TrayOpen => "開啟面板",
        TextKey::TrayRefreshNow => "立即重新整理",
        TextKey::TrayHotspotStatus => "熱點狀態",
        TextKey::TrayExit => "結束",
        TextKey::TrayUnavailableFallback => "無法建立系統匣圖標，視窗將保持顯示。",
        TextKey::CloseToTrayHint => "視窗已隱藏到系統匣。",
        TextKey::SettingsTitle => "設定",
        TextKey::SettingsLanguage => "介面語言",
        TextKey::LanguageZhCn => "简体中文",
        TextKey::LanguageZhTw => "繁體中文",
        TextKey::LanguageEnUs => "English",
        TextKey::SettingsAutostart => "登入 Windows 時啟動",
        TextKey::SettingsAutostartDescription => "預設關閉；啟用後將直接啟動到系統匣。",
        TextKey::SettingsAutostartDisabled => "已關閉",
        TextKey::SettingsStartMinimized => "啟動時隱藏到系統匣",
        TextKey::SettingsActiveProbe => "允許主動連接探測",
        TextKey::SettingsActiveProbeDescription => {
            "使用模組網絡卡進行小流量、嚴格綁定的聯網與 DNS 檢查。"
        }
        TextKey::SettingsLogLevel => "日誌詳細程度",
        TextKey::SettingsLogLevelRestart => "更改將在下次啟動時生效。",
        TextKey::LogLevelError => "僅錯誤",
        TextKey::LogLevelWarn => "警告及以上",
        TextKey::LogLevelInfo => "一般",
        TextKey::LogLevelDebug => "調試",
        TextKey::SettingsPrivacy => "私隱",
        TextKey::SettingsPrivacyDescription => "診斷資訊預設去識別化，不會自動上傳。",
        TextKey::SettingsConfigDrift => "啟動項與當前程式位置不一致，請重新啟用自動啟動。",
        TextKey::SettingsSaved => "設定已儲存",
        TextKey::SettingsSaveFailed => "無法儲存設定。",
        TextKey::SettingsCorruptConfig => "設定檔已損壞，已保留原檔案並恢復安全預設值。",
        TextKey::SettingsAutostartLoading => "正在讀取啟動設定。",
        TextKey::SettingsAutostartSaving => "正在儲存啟動設定。",
        TextKey::SettingsAutostartNotOwned => "啟動項內容與當前程式不一致，未自動刪除。",
        TextKey::SettingsPathUnavailable => "無法確定設定目錄，設定不會持久化。",
        TextKey::SettingsReadFailed => "無法讀取設定，已使用安全預設值；原檔案未覆蓋。",
        TextKey::SingleInstanceActivationFailed => "已有面板正在執行，但無法喚醒它。",
        TextKey::LoggingInitFailed => "無法啟用本地日誌；應用程式仍可執行。",
        TextKey::LoggingRotationFailed => "日誌輪換失敗；應用程式仍可執行。",
        TextKey::RepairsTitle => "修復操作",
        TextKey::RepairsReadOnlyNotice => "只有在目標身份和當前證據均有效時，才會啟用相應操作。",
        TextKey::RepairsDriverNotIncluded => "驅動程式安裝需單獨確認；正常工作的介面無需重裝。",
        TextKey::ConfirmationTitle => "確認執行",
        TextKey::ConfirmationDnsServers => "DNS 服務器",
        TextKey::ConfirmationNewApn => "新 APN",
        TextKey::ConfirmationUsbConfigurationOnly => {
            "此操作只儲存 USB 網絡配置；需手動重新啟動模組後複檢，當前網絡卡模式尚未驗證。"
        }
        TextKey::ConfirmationOperation => "操作：{operation}",
        TextKey::ConfirmationTarget => "目標：DJI 一代 4G 模組（VID 2CA3、PID 4006）",
        TextKey::ConfirmationExpectedEffect => "預期效果",
        TextKey::ConfirmationInterruption => "連接影響",
        TextKey::ConfirmationElevation => "提權要求",
        TextKey::ConfirmationRisk => "風險級別",
        TextKey::ConfirmationElevationRequired => "此操作需要 Windows 管理員授權。",
        TextKey::ConfirmationElevationNotRequired => "此操作不需要管理員授權。",
        TextKey::ConfirmationStateRecheck => "執行前將再次核對裝置身份和當前狀態。",
        TextKey::ConfirmationNoAutomaticRetry => "寫入操作只執行一次；超時後不會自動重試。",
        TextKey::ConfirmationApnContext => "PDP 上下文：{cid}",
        TextKey::ConfirmationApnNewValue => "新 APN：{apn_masked}",
        TextKey::OperationPreparing => "正在準備操作",
        TextKey::OperationRevalidating => "正在重新核對目標狀態",
        TextKey::OperationAwaitingElevation => "等待管理員授權",
        TextKey::OperationExecuting => "正在執行：{operation}",
        TextKey::OperationVerifying => "正在重新偵測並驗證結果",
        TextKey::OperationUacCancelled => "管理員授權已取消，未執行操作。",
        TextKey::OperationDeviceRemoved => "裝置已斷開，操作已停止。",
        TextKey::OperationAuditRecorded => "操作結果已記錄到本地審計日誌。",
        TextKey::NoPreparedAction => "當前沒有可確認的操作計劃。",
        TextKey::PreparedActionAwaitingConfirmation => "操作已準備，等待你的確認。",
        TextKey::PlanExpired => "此操作計劃已過期，請重新準備。",
        TextKey::OperationResultTitle => "操作結果",
        TextKey::ConfirmationDevModeWarning => {
            "開發構建：dji4g-helper.exe 未簽名，僅供開發測試，請謹慎操作。"
        }
        TextKey::DiagnosticsExportTitle => "匯出診斷資訊",
        TextKey::DiagnosticsExportDescription => {
            "將生成一份便於閱讀的報告和一份結構化資料檔案；預設隱藏敏感識別碼。"
        }
        TextKey::DiagnosticsExportRedactionNotice => {
            "完整 IMEI、IMSI、ICCID、電話號碼、PIN/PUK、原始序列埠資料和完整配置不會寫入匯出。"
        }
        TextKey::DiagnosticsExportSuccess => {
            "診斷資訊已匯出到 %LOCALAPPDATA%\\Dji4GPanel\\exports。"
        }
        TextKey::DiagnosticsExportFailed => "無法匯出診斷資訊。",
        TextKey::BuildDevelopmentUnsigned => "開發版（未簽名）",
        TextKey::BuildStableSigned => "穩定版（已簽名）",
        TextKey::FeatureUnavailablePortable => "當前執行方式不提供此功能。",
        TextKey::UiCjkFontUnavailable => {
            "未找到可用的 Windows 中文字體；介面文字可能無法完整顯示。"
        }
        TextKey::NoAutomaticUpdate => "本應用程式不會自動更新。",
        TextKey::DemoUsage => "調試演示：available、limited、unavailable、absent 或 detecting",
        TextKey::DemoRejectedRelease => "發布版本不允許使用演示模式。",
        TextKey::DemoInvalidScenario => {
            "未知演示場景，請使用 available、limited、unavailable、absent 或 detecting。"
        }
        TextKey::NavSms => "短訊",
        TextKey::NavDeviceTools => "裝置工具",
        TextKey::SmsTitle => "短訊",
        TextKey::SmsIntro => {
            "短訊功能首次啟用會把模組短訊格式設為 PDU；讀取訊息可能將未讀標記為已讀。"
        }
        TextKey::ButtonSmsRefresh => "重新整理短訊",
        TextKey::FieldSmsStatus => "狀態",
        TextKey::FieldSmsMessageCount => "訊息數",
        TextKey::FieldSmsUnreadCount => "未讀數",
        TextKey::FieldSmsCapacity => "容量",
        TextKey::SmsCapacityUsed => "已用 {used} / 總數 {total}",
        TextKey::SmsStatusNotQueried => "尚未查詢",
        TextKey::SmsStatusRead => "已讀取",
        TextKey::SmsIncompleteWarning => "存在未完整接收的長短訊",
        TextKey::SmsEmpty => "暫無短訊（或尚未重新整理）",
        TextKey::SmsListPending => "點擊「重新整理短訊」讀取收件匣。",
        TextKey::SmsUnread => "未讀",
        TextKey::SmsRead => "已讀",
        TextKey::FieldSmsSender => "發送方",
        TextKey::FieldSmsTime => "時間",
        TextKey::FieldSmsEncoding => "編碼",
        TextKey::FieldSmsParts => "分段",
        TextKey::FieldSmsBody => "正文",
        TextKey::SmsEncodingOther => "其他",
        TextKey::SmsReadNote => "讀取可能已將其標記為已讀",
        TextKey::ButtonSmsDelete => "刪除",
        TextKey::ButtonSmsDeleteConfirm => "確認刪除",
        TextKey::ButtonSmsSend => "發送短訊",
        TextKey::ButtonSmsSendConfirm => "確認發送（可能產生費用）",
        TextKey::FieldSmsRecipient => "收件人",
        TextKey::SmsEvictedWarning => {
            "本地快取已滿，較早的 {count} 條訊息已從本地視圖移除（模組中可能仍存在）。"
        }
        TextKey::SmsSendNotice => {
            "發送可能產生費用；提交成功不代表對方收到。失敗或超時不會自動重試。"
        }
        TextKey::SmsIncompleteTag => "未完整",
        TextKey::ButtonSmsExpand => "展開",
        TextKey::ButtonSmsCollapse => "收起",
        TextKey::SmsInboxHeading => "收件匣",
        TextKey::SmsOutgoingSubmitted => "已提交",
        TextKey::SmsOutgoingFailed => "發送失敗",
        TextKey::SmsOutgoingUnknown => "結果未知",
        TextKey::SmsBodyCharCount => "字數 {count} / 70",
        TextKey::SmsErrorPduModeRequired => {
            "短訊需要 PDU 模式：請先在短訊頁點擊「重新整理短訊」啟用（首次會切換模組短訊格式）。"
        }
        TextKey::SmsErrorPduConfirmFailed => "切換 PDU 模式後未能確認，請重試重新整理短訊。",
        TextKey::SmsErrorInvalidMessage => {
            "短訊內容或收件人不符合要求（收件人需為 + 開頭的國際格式，正文 ≤140 字節且僅限 BMP 字符）。"
        }
        TextKey::SmsErrorSendFailed => "模組拒絕了本次短訊提交。",
        TextKey::SmsErrorTimeout => "短訊操作超時：結果可能未知，不會自動重試。",
        TextKey::SmsErrorDeviceRemoved => "短訊操作期間裝置已斷開。",
        TextKey::SmsErrorUnsupported => "該韌體不支援短訊 AT 命令。",
        TextKey::SmsErrorVerificationFailed => "短訊響應格式未被識別。",
        TextKey::SmsErrorInternal => "短訊內部錯誤，請重新整理後重試。",
        TextKey::SmsErrorSimRequired => "缺少 SIM 身份資訊，未執行短訊操作；請先重新整理。",
        TextKey::SmsErrorSimUnverified => "無法核實當前 SIM，未執行短訊操作。",
        TextKey::SmsErrorSimChanged => "SIM 已變化，未執行短訊操作；請重新整理後重新確認。",
        TextKey::SmsErrorGeneric => "短訊操作失敗。",
        TextKey::FieldTemperature => "溫度",
        TextKey::TemperatureNotRead => "未讀取到",
        TextKey::TemperatureSensorNote => "感應器定義以韌體為準",
        TextKey::TemperatureSectionHeading => "模組溫度",
        TextKey::TemperatureTrendWindow => "最近 {age}",
        TextKey::TemperatureTrendNote => "折線為第 1 個報告值 · 每 {age}一個取樣點",
        TextKey::TemperatureTrendSampling => "正在取樣模組溫度……（每個重新整理周期一個點）",
        TextKey::TemperatureDeltaUp => "較上次 +{detail} °C",
        TextKey::TemperatureDeltaDown => "較上次 -{detail} °C",
        TextKey::TemperatureDeltaFlat => "與上次相同",
        TextKey::TemperatureSensorsReported => {
            "裝置報告 {count} 個感應器值：{detail} °C · 順序與含義以韌體為準"
        }
        TextKey::FieldAdapterErrors => "介面錯誤",
        TextKey::FieldAdapterDiscards => "介面丟棄",
        TextKey::FieldAdapterLinkRate => "鏈路速率",
        TextKey::AdapterRxTx => "收 {rx} / 發 {tx}",
        TextKey::AdapterLinkRateNote => "介面鏈路速率，不是實測吞吐",
        TextKey::TimelineHeading => "網絡變化記錄",
        TextKey::TimelineEmpty => "暫無記錄（僅記錄觀察到的變化）",
        TextKey::TimelineSimChanged => "SIM 已更換",
        TextKey::TimelineRegistrationChanged => "網絡註冊變化",
        TextKey::TimelineCellChanged => "服務小區變化",
        TextKey::TimelineDeviceRemoved => "裝置已斷開",
        TextKey::TimelineDeviceArrived => "裝置已重新枚舉",
        TextKey::TimelineAdapterLinkChanged => "網絡卡鏈路變化",
        TextKey::TimelineDnsChanged => "DNS 探測變化",
        TextKey::NavGroupModule => "模組管理",
        TextKey::EntrySkipHint => "可以直接進入，稍後繼續檢查。",
        TextKey::EntryHiddenHint => "進入後不再自動顯示；可在設定中重新開啟。",
        TextKey::AgeSeconds => "{count} 秒",
        TextKey::AgeMinutes => "{count} 分鐘",
        TextKey::AgeHours => "{count} 小時",
        TextKey::RateNow => "現在",
        TextKey::RateSecondsAgo => "{count} 秒前",
        TextKey::RateSamplePaused => "速率取樣暫停，等待新讀數",
        TextKey::RateNotSampled => "暫未取得速率",
        TextKey::RateHoverAgo => "{age} 秒前 · 實際取樣",
        TextKey::RateHoverDown => "↓ 下載  {detail}",
        TextKey::RateHoverUp => "↑ 上傳  {detail}",
        TextKey::RepairsIntro => "先檢查原因，再確認需要執行的操作",
        TextKey::RepairsAdapterModeHeading => "電腦網絡卡模式",
        TextKey::RepairsDjiGuideLink => "查看大疆官方使用說明",
        TextKey::RepairsAdapterModeNote => {
            "部分一代模組保留原廠韌體即可用作電腦網絡卡。先檢查驅動程式和當前網絡狀態；已經能上網時無需切換。"
        }
        TextKey::RepairsUsbSwitchNote => {
            "下方操作只切換 USB 網絡配置，不刷寫韌體。DJI NDIS 配置需要匹配的 Windows 驅動程式；ECM 配置的相容性取決於系統與驅動程式。"
        }
        TextKey::RepairsUsbOnlyNote => {
            "此操作只儲存 USB 配置；需手動重新啟動模組後驗證模式。重新啟動會中斷連接。"
        }
        TextKey::RepairsLowRiskHeading => "低風險與網絡恢復",
        TextKey::RepairsInterruptsConnection => "會中斷連接",
        TextKey::RepairsViewPlan => "查看方案",
        TextKey::FieldPdpContext => "PDP 上下文",
        TextKey::FieldNewApn => "新 APN",
        TextKey::GuideProbeOff => "主動聯網檢查已關閉，公網與 DNS 尚未驗證；可在設定中開啟。",
        TextKey::GuideCollecting => "正在擷取連接證據，請等待本輪檢查完成；此時無需修改網絡設定。",
        TextKey::GuideEvidenceStale => {
            "連接證據已過期，請重新整理後再判斷；舊結果不代表當前連接狀態。"
        }
        TextKey::GuideCheckDisabled => "此項檢查已關閉，可在設定中開啟；未檢查不代表網絡失敗。",
        TextKey::GuideCheckNotRun => "尚未完成此項檢查，請先重新整理。未識別裝置不等於缺驅動程式。",
        TextKey::GuideUsbFailed => {
            "USB 檢查未通過，請核對資料線、介面與裝置。未識別裝置不等於缺驅動程式。"
        }
        TextKey::GuideAdapterFailed => {
            "模組介面檢查未通過，請查看序列埠或網絡卡的具體原因；正常介面無需重裝驅動程式。"
        }
        TextKey::GuideCellularFailed => {
            "SIM 或流動註冊檢查未通過，請查看原因並核對卡狀態、訊號與網絡供應商註冊。"
        }
        TextKey::GuideBoundProbeFailed => {
            "模組綁定的公網或 DNS 檢查未通過，請按具體失敗項排查；無需反復切換 USB 模式。"
        }
        TextKey::GuidePassed => {
            "模組公網與 DNS 檢查已通過。系統實際出口仍可能由 Wi-Fi 或 VPN 決定。"
        }
        TextKey::GuideStartHeading => "開始使用模組",
        TextKey::GuideSteps => {
            "1. 連接模組 → 2. 檢查驅動程式與序列埠 → 3. 檢查 SIM / 網絡 → 4. 上網或短訊"
        }
        TextKey::CheckUsbDetection => "USB 識別",
        TextKey::CheckAdapterInterface => "網絡卡介面",
        TextKey::CheckAtSerial => "AT 序列埠",
        TextKey::CheckSimCellular => "SIM 與流動網絡",
        TextKey::CheckBoundPublic => "模組公網",
        TextKey::CheckBoundDns => "模組 DNS",
        TextKey::GuidePassedCount => "已通過 {count} 項：{detail}",
        TextKey::GuideStuckHint => {
            "長時間停在偵測中：點擊頂部“匯出詳細日誌”，完成後“開啟所在資料夾”，把該 TXT 檔案交給協助排查的人。日誌包含裝置、驅動程式和網絡資訊，不包含短訊正文。"
        }
        TextKey::FirstCheckHeading => "首次連接檢查",
        TextKey::FirstCheckIntro => {
            "插入模組後，分別檢查網絡卡和 AT 通信。無法上網或未獲得 IP 並不一定是缺少驅動程式。"
        }
        TextKey::FirstCheckUsb => "1. USB 裝置識別",
        TextKey::FirstCheckAdapter => "2. Windows 網絡卡",
        TextKey::FirstCheckAt => "3. AT 通信（短訊與模組查詢）",
        TextKey::DriverInstallHeading => "驅動程式安裝",
        TextKey::DriverBundledNote => {
            "此離線版附帶原始驅動程式資源，安裝前會驗證檔案和簽名。當前包不能覆蓋所有介面（包括未匹配的 MI_04）；任何缺驅動程式介面無法匹配時，將在安裝前停止。Windows 可能同時更新其他匹配該包的裝置，不強制覆蓋更優驅動程式。"
        }
        TextKey::DriverElevationNote => {
            "確認後面板會自動結束，再顯示 Windows 管理員授權。安裝結束或取消授權後會自動返回面板並顯示結果；如提示重新啟動，請先重新啟動電腦。"
        }
        TextKey::DriverInstallAction => "結束面板並安裝驅動程式",
        TextKey::DriverNoneNote => {
            "此版本沒有完整的離線驅動程式資源。請開啟 Windows 設定 → Windows 更新 → 可選更新檢查驅動程式，或聯系 DJI 官方支援取得此模組的適配驅動程式；安裝後點擊“立即重新整理”。"
        }
        TextKey::DriverDjiCompatibilityLink => "大疆官方相容說明（第 21 項）",
        TextKey::DriverSeparateNote => {
            "網絡卡與 AT 序列埠可能需要不同驅動程式。安裝結果返回後仍需驗證 AT 與網絡；已有功能正常時無需重復安裝。Windows 更新不保證提供該模組的全部驅動程式。"
        }
        TextKey::DriverDjiSupportLink => "聯系 DJI 官方支援",
        TextKey::DriverVendorLink => "移遠官方驅動程式取得說明",
        TextKey::DriverVendorLinkNote => {
            "該鏈接提供廠商取得渠道，不代表其中所有驅動程式都相容大疆定制模組。"
        }
        TextKey::ValueNotReported => "未報告",
        TextKey::WirelessSummary => {
            "無線觀測（模組 AT+QENG 報告）\n{}\n制式 {} / {}\nRSRP {}\nRSRQ {}\nRSSI {}\nSINR 原始值 {}（單位未確認）\n上行頻寬 {} MHz / 下行頻寬 {} MHz\nTAC {}"
        }
        TextKey::WirelessIntro => {
            "查看模組的無線參數與小區變化，並控制由模組供網的 Windows 流動熱點"
        }
        TextKey::WirelessServingCell => "當前服務小區",
        TextKey::WirelessSampleFresh => "最近一次 AT 查詢已完成",
        TextKey::WirelessSampleWaiting => "等待有效取樣 / 已有資料僅供回看",
        TextKey::WirelessCopySummary => "複製無線摘要",
        TextKey::WirelessNoCell => {
            "暫未取得服務小區資料。連接模組後隨後臺監測自動取樣；短訊發送期間暫停。未報告的欄位保持為空。"
        }
        TextKey::WirelessBand => "頻段 B{}",
        TextKey::WirelessRsrpNote => "參考訊號功率",
        TextKey::WirelessRsrqNote => "參考訊號質素",
        TextKey::WirelessRssiNote => "接收總功率",
        TextKey::WirelessSinrNote => "SINR · 原始值",
        TextKey::WirelessSinrUnit => "單位尚未確認",
        TextKey::WirelessCellDetails => "小區與頻寬詳情",
        TextKey::WirelessBandwidthLine => "上行頻寬 {} MHz  ·  下行頻寬 {} MHz",
        TextKey::WirelessTacLine => "TAC {}  ·  模組狀態 {}",
        TextKey::WirelessNoconnNote => {
            "NOCONN 表示註冊後空閑；不單獨據此判定網絡斷開。訊號數值不能代替實際吞吐測試。"
        }
        TextKey::WirelessSignalHeading => "訊號變化 · RSRP",
        TextKey::WirelessSampleCount => "最近 {} / 120 次取樣",
        TextKey::WirelessSignalNote => {
            "按 AT 取樣順序顯示；缺失值斷開曲線，未收到新回執時不重復造點。裝置或 SIM 更換後重新記錄。"
        }
        TextKey::WirelessChangeHeading => "小區變化記錄 · {}",
        TextKey::WirelessChangeNote => {
            "僅記錄觀測到的小區識別碼變化，不將它直接解釋為切換失敗或斷線。最近保留 20 條。"
        }
        TextKey::WirelessNoChange => "當前工作階段尚未觀測到小區變化。",
        TextKey::WirelessSecondsAgo => "{} 秒前",
        TextKey::WirelessPreviousCell => "原小區：{}",
        TextKey::WirelessNewCell => "新小區：{}",
        TextKey::WirelessWaitingRsrp => "等待有效 RSRP 取樣",
        TextKey::DeviceModelName => "DJI 一代 4G 模組",
        TextKey::DeviceModelNameQuectelGeneric => "Quectel 通用模組",
        TextKey::ReadOnlyModuleReason => {
            "通用模組僅支援唯讀檢查；驅動程式安裝與受控寫入僅適用於 DJI 一代模組"
        }
        TextKey::SignalWithGrade => "{} dBm（速度：{}）",
        TextKey::PdpActive => "已啟用",
        TextKey::PdpInactive => "未啟用",
        TextKey::ServingSearching => "正在搜索",
        TextKey::ServingLimitedService => "受限服務",
        TextKey::ServingNoCell => "無小區",
        TextKey::ServingNotCamped => "未駐留（搜索中）",
        TextKey::ServingCampedIdle => "已駐留（空閑）",
        TextKey::ServingSinr => "SINR {}（單位待確認）",
        TextKey::ValuePreviewMore => "{}（另有 {} 項）",
        TextKey::OverviewSummaryLine => "網絡供應商 {}  ·  訊號 {}  ·  溫度 {}",
        TextKey::OverviewTabRate => "收發速率",
        TextKey::OverviewDeviceHeading => "裝置與 SIM 詳情",
        TextKey::FieldModelShort => "型號",
        TextKey::FieldRegistrationShort => "註冊",
        TextKey::FieldServingCell => "服務小區",
        TextKey::OverviewNetworkHeading => "Windows 網絡詳情",
        TextKey::FieldDefaultRouteShort => "預設路由",
        TextKey::FieldIpAddresses => "IP 地址",
        TextKey::ValueMoreItems => "另有 {count} 項（未展開）",
        TextKey::FieldFirmware => "韌體",
        TextKey::FieldPdpState => "PDP 狀態",
        TextKey::DiagNoProblemCode => "無問題代碼",
        TextKey::DiagProblemCode => "問題代碼 {}",
        TextKey::DiagPort => "連接埠 {}",
        TextKey::DiagCarrierNotRead => "網絡供應商未取得",
        TextKey::DiagRatNotRead => "制式未取得",
        TextKey::DiagRouteProbeNote => "僅查詢匹配路由，不證明網關可達；配置網關：{}",
        TextKey::DiagProxyInterfaceNote => "；VPN 或代理可能正常使用此介面，模組通道另行驗證",
        TextKey::DiagIncomplete => "尚未完成",
        TextKey::DiagFailedRepeatedly => "未通過（連續 {} 次）",
        TextKey::DiagResolutionFailed => "解析失敗",
        TextKey::MNCQueued => "已排隊，等待當前任務結束後檢查",
        TextKey::MNCChecking => "正在檢查模組網絡…",
        TextKey::MNCStale => "結果已過期或裝置已變化，請重新檢查",
        TextKey::MNCAvailable => "模組網絡可用：本次公網與 DNS 驗證通過",
        TextKey::MNCNoDevice => "未識別到模組，請檢查資料線和 USB 介面",
        TextKey::MNCAdapterFailed => "模組網絡卡檢查未通過，請查看具體證據",
        TextKey::MNCAdapterNoAddress => "已識別網絡卡，但缺少可用地址或路由",
        TextKey::MNCAdapterLinkDown => "模組網絡卡鏈路未連接，請檢查 USB 連接與裝置狀態",
        TextKey::MNCBoundRouteFailed => "模組綁定路由查詢未通過，請查看地址與路由配置",
        TextKey::MNCBoundPublicFailed => "模組公網測試未通過，請查看流動與公網證據",
        TextKey::MNCBoundDnsFailed => "模組公網可達，但 DNS 解析未通過",
        TextKey::MNCInconclusive => "尚不能判斷模組能否上網，請查看未完成的檢查",
        TextKey::MNCEvidenceUnexecuted => "未執行",
        TextKey::MNCEvidenceRunning => "檢查中",
        TextKey::MNCEvidencePassed => "通過",
        TextKey::MNCEvidenceFailed => "未通過",
        TextKey::MNCEvidenceUnavailable => "無法取得",
        TextKey::MNCEvidenceExpired => "已過期",
        TextKey::MNCDhcpLease => "更新模組網絡卡 DHCP 租約",
        TextKey::MNCRestartAdapter => "重新啟動模組網絡卡",
        TextKey::MNCAutomaticDns => "恢復自動 DNS",
        TextKey::MNCHeading => "模組網絡檢查",
        TextKey::MNCRunAction => "檢查模組網絡",
        TextKey::MNCRunningReason => "本輪檢查尚未結束",
        TextKey::MNCOperationResult => "操作結果：{}",
        TextKey::MNCReadOnlyReverify => "下方為操作後的唯讀複檢；不會自動重復修復。",
        TextKey::MNCStepUsb => "識別 USB 模組",
        TextKey::MNCStepPorts => "讀取序列埠、流動與模組網絡卡",
        TextKey::MNCStepRoute => "查詢模組綁定路由，驗證公網、DNS 與電腦出口",
        TextKey::MNCStepCurrent => "當前步驟：{}",
        TextKey::MNCResultExpired => "檢查結果已過期，請重新檢查",
        TextKey::MNCDeviceNoAdapter => {
            "模組已識別，但尚未核實網絡卡介面。驅動程式、USB 網絡模式或讀取失敗都可能有關。"
        }
        TextKey::MNCViewSteps => "查看驅動程式與介面檢查步驟",
        TextKey::MNCSimHint => {
            "請確認 SIM 卡可用、流動註冊與套餐狀態。一次超時不能說明驅動程式損壞。"
        }
        TextKey::MNCNoPublicProbe => {
            "本次未驗證公網連接。點擊檢查並允許一次聯網探測，長期設定保持不變。"
        }
        TextKey::MNCEgressAdapter => "選擇模組網絡卡",
        TextKey::MNCEgressProxy => "選擇代理或 VPN；這本身不是故障",
        TextKey::MNCEgressOther => "選擇其他網絡卡（例如 Wi-Fi / 有線）；這本身不是故障",
        TextKey::MNCEgressUnknown => "無法確認出口",
        TextKey::MNCEgressPath => "{} 對本次測試目標的路徑：{}。",
        TextKey::MNCProbeUsb => "USB 模組",
        TextKey::MNCProbeAdapterRead => "網絡卡讀取",
        TextKey::MNCProbeLink => "鏈路",
        TextKey::MNCProbeAddressRoute => "地址與路由",
        TextKey::MNCProbeBoundRoute => "模組綁定路由查詢",
        TextKey::MNCProbeBoundPublic => "模組公網",
        TextKey::MNCProbeBoundDns => "模組 DNS",
        TextKey::MNCAdapterLinkLine => "鏈路：{}；IPv4 DHCP：{}",
        TextKey::MNCConnected => "已連接",
        TextKey::MNCDisconnected => "未連接",
        TextKey::MNCEnabled => "啟用",
        TextKey::MNCDisabled => "未啟用",
        TextKey::MNCProtocolLine => "可用協議：IPv4 {} / IPv6 {}",
        TextKey::MNCStaticAddressNote => {
            "偵測到非 DHCP 配置，不會自動覆蓋靜態地址。請確認原有網絡設定。"
        }
        TextKey::MNCAtControl => "序列埠通信",
        TextKey::MNCSimCellular => "SIM 與流動",
        TextKey::MNCBoundRouteNote => {
            "綁定路由查詢只證明找到了匹配路由，不證明網關可達。公網探測綁定模組網絡卡；電腦路徑只代表本次固定測試目標，不能代表所有應用程式。未執行、無法取得和過期均不等於故障。"
        }
        TextKey::MNCEvidenceHeading => "本輪證據與處理說明",
        TextKey::MNCIntro => "單獨檢查模組網絡卡能否上網，並說明電腦對測試目標選擇的出口。",
        TextKey::MNCProbeConsent => {
            "後臺聯網探測已關閉。本次檢查將向內置固定驗證端點發送少量請求（公網與 DNS），不會修改長期設定。"
        }
        TextKey::MNCProbeCancel => "取消",
        TextKey::MNCLocalOnly => "僅檢查本地資訊",
        TextKey::MNCAllowProbe => "允許本次聯網檢查",
        TextKey::MNCSubmitError => "檢查請求未提交，請稍後重試。",
        TextKey::ArchiveSaveFailed => {
            "儲存歷史開關失敗，本次更改未儲存。請檢查用戶目錄權限後重試。"
        }
        TextKey::SmsViewModule => "模組短訊",
        TextKey::SmsViewArchive => "本地歷史",
        TextKey::ArchiveExportName => "短訊歷史-{}.txt",
        TextKey::ArchiveNoExportDir => "未匯出：用戶匯出目錄不可用。",
        TextKey::OnboardingSaveFailed => {
            "已進入面板，但引導完成狀態儲存失敗（{}）；下次啟動可能再次顯示。"
        }
        TextKey::ArchiveBusyTitle => "正在完成本地短訊歷史操作",
        TextKey::ArchiveBusyBody => "儲存、清空或匯出結束後將自動結束，請稍候。",
        TextKey::WindowsUpdateFailedTitle => "無法開啟 Windows 更新",
        TextKey::WindowsUpdateFailedBody => {
            "請從 Windows 設定開啟“Windows 更新”，檢查可選驅動程式更新。當前尚未安裝任何驅動程式。"
        }
        TextKey::MenuButton => "菜單",
        TextKey::NavDialogTitle => "導航",
        TextKey::NavDialogClose => "收起菜單",
        TextKey::OverviewPageIntro => "模組與電腦的當前狀態 · 本頁唯讀",
        TextKey::MoreMenu => "更多",
        TextKey::ExportDetailedLog => "匯出詳細日誌",
        TextKey::ExportDetailedLogHint => {
            "收集 USB、驅動程式、序列埠、網絡與偵測階段，不包含短訊正文。"
        }
        TextKey::DiagnosticsPageTitle => "網絡診斷",
        TextKey::DiagnosticsPageIntro => "分別檢查模組通道與電腦網絡，按證據定位問題",
        TextKey::SettingsReopenOnboarding => "重新查看首次使用引導",
        TextKey::HelpDialogTitle => "使用說明",
        TextKey::AboutDialogTitle => "關於本應用程式",
        TextKey::RestartPanelTitle => "重新啟動面板",
        TextKey::ExitApp => "結束應用程式",
        TextKey::BusyDialogTitle => "請等待當前任務完成",
        TextKey::RestartBusyBody => {
            "正在發送短訊、執行修復或執行裝置工具。完成或取消後再重新啟動面板，避免中斷當前任務。"
        }
        TextKey::RestartConfirmBody => {
            "面板將關閉並立即重新啟動。\n裝置會在重新啟動後重新識別；正在填寫但未發送的短訊草稿會丟失。\n\n現在重新啟動？"
        }
        TextKey::RestartFailedTitle => "無法重新啟動面板",
        TextKey::RestartFailedBody => "面板保持執行，未重新啟動。\n{}\n可先結束，再手動開啟程式。",
        TextKey::InstallBusyBody => {
            "正在匯出日誌、發送短訊或執行/確認修復。完成後再安裝驅動程式，避免中斷當前任務。"
        }
        TextKey::InstallDriverTitle => "安裝模組驅動程式",
        TextKey::InstallDriverBody => {
            "面板將自動結束，隨後顯示 Windows 管理員授權，請選擇“是”。\n僅安裝硬件匹配的缺失驅動程式，正常介面不會強制重裝。\n\n完成或取消後會返回普通權限的面板，顯示結果和下一步；如提示重新啟動，請先重新啟動電腦。\n\n現在繼續？"
        }
        TextKey::InstallFailedTitle => "無法啟動安裝器",
        TextKey::InstallFailedBody => "面板保持執行，尚未安裝驅動程式。\n{}\n請匯出詳細日誌。",
        TextKey::ConfirmProbeNote => {
            "\n\n完成後將唯讀複檢一次，向固定端點發送少量公網與 DNS 請求；長期探測設定不變。失敗或結果未知不會自動再次修復。"
        }
        TextKey::AboutWindowTitle => "關於 {}",
        TextKey::GitHubProject => "GitHub 專案儲存庫",
        TextKey::UpdateAvailable => "發現新版本 v{}",
        TextKey::UpdateTitle => "軟體更新",
        TextKey::UpdateCurrentVersion => "目前版本 v{}",
        TextKey::UpdateCheckNow => "檢查更新",
        TextKey::UpdateChecking => "正在檢查更新…",
        TextKey::UpdateNotChecked => "尚未檢查更新。",
        TextKey::UpdateUpToDate => "目前版本無需更新。",
        TextKey::UpdateCheckFailed => "檢查更新失敗，可稍後重試。",
        TextKey::UpdateView => "查看更新",
        TextKey::UpdateDownload => "下載更新",
        TextKey::UpdateDownloading => "正在下載",
        TextKey::UpdateProgress => "已下載 {} / {}",
        TextKey::UpdateVerifying => "正在驗證更新套件",
        TextKey::UpdateReady => "更新套件驗證通過，可以安裝。",
        TextKey::UpdatePreparing => "正在準備更新，請稍候。",
        TextKey::UpdateInstallRestart => "安裝並重新啟動",
        TextKey::UpdateRetry => "重新下載",
        TextKey::UpdateBusy => "目前操作完成後即可繼續。",
        TextKey::UpdateRestartHint => "安裝時軟體會退出，完成後自動重新啟動。",
        TextKey::UpdateFailedNetwork => "更新下載失敗，請檢查網路後重試。",
        TextKey::UpdateFailedChecksum => "無法取得有效的 SHA-256 驗證值，禁止安裝。",
        TextKey::UpdateFailedIntegrity => "更新套件驗證失敗，已刪除無效暫存套件，禁止安裝。",
        TextKey::UpdateFailedUpdater => "更新程式無法啟動或準備失敗，目前軟體繼續執行。",
        TextKey::UpdateUnsupported => "目前目錄不支援原地更新，請使用正式 portable 套件。",
        TextKey::UpdateRecoveryNotice => "更新未完成，已重新啟動舊版軟體。原有檔案仍然保留。",
        TextKey::HostPreparingPlan => "正在準備修復方案…",
        TextKey::HostBackingUp => "正在備份並修復代理配置…",
        TextKey::HostRestoring => "正在恢復原配置…",
        TextKey::HostChecking => "正在檢查電腦網絡與代理設定…",
        TextKey::HostConfigChanged => "代理配置已修改；重新啟動代理後請重新檢查。",
        TextKey::HostNotFinished => "本次電腦網絡檢查或修復未完成",
        TextKey::HostNotChecked => "電腦網絡與代理尚未檢查。",
        TextKey::HostStale => "電腦網絡資訊已過期，請重新檢查。",
        TextKey::HostModulePassed => "模組連接正常；",
        TextKey::HostMissingAdapter => "代理軟件指定的出口網絡卡已不存在。",
        TextKey::HostAdapterDown => "代理指定的網絡卡仍在電腦上，但當前未連接或已禁用。",
        TextKey::HostOutletUnknown => "暫時無法確定代理出口問題，請查看處理步驟。",
        TextKey::HostProxyInUse => "電腦正在通過代理或 VPN 介面聯網；模組通道單獨檢查。",
        TextKey::HostNoKnownProblem => "未發現已知的固定出口網絡卡問題。",
        TextKey::HostHeading => "電腦網絡與代理",
        TextKey::HostNoModuleHint => "未連接模組，仍可檢查電腦網絡與代理設定。",
        TextKey::HostCheckAgain => "重新檢查",
        TextKey::HostReviewRepair => "查看修復方案",
        TextKey::HostProxyOff => "未開啟",
        TextKey::HostProxyManual => "手動代理",
        TextKey::HostProxyAutoScript => "自動配置腳本；未執行腳本",
        TextKey::HostProxyAutoDetect => "自動偵測；尚未驗證",
        TextKey::HostProxyMixed => "多種代理設定；需進一步確認",
        TextKey::HostProxyUnknown => "未取得",
        TextKey::HostWindowsProxy => "Windows 系統代理",
        TextKey::HostConfiguredAdapter => "磁盤配置引用網絡卡（執行態未核實）",
        TextKey::HostProxyBoundAdapter => "代理指定網絡卡",
        TextKey::HostUnsupportedConfig => {
            "此配置或版本暫不支援自動修改，請在代理軟件中檢查出站介面設定。"
        }
        TextKey::HostConfigUnreadable => "代理配置無法可靠讀取",
        TextKey::HostRemoveOutletPrefix => "將取消代理軟件對",
        TextKey::HostRemoveOutletSuffix => {
            "的固定出口設定。之後可能使用 Wi-Fi、有線網絡或其他可用出口。將先備份原配置。請先完整結束 Clash Verge Rev 及相關核心。"
        }
        TextKey::HostPlanExpired => "修復方案已過期，請重新檢查後再查看方案。",
        TextKey::HostKeepOriginal => "保留原設定",
        TextKey::HostWaitCurrentOperation => "正在處理，請等待本次操作結束",
        TextKey::HostBackupAndRepair => "備份並修復",
        TextKey::HostRestartedCheckAgain => {
            "配置已修改。重新啟動代理後點擊“重新檢查”；配置改動不等於互聯網已經恢復。"
        }
        TextKey::HostRestoreOriginal => "恢復原配置",
        TextKey::HostStepsHeading => "處理步驟",
        TextKey::HostConfiguredAdapterRef => "磁盤配置引用了網絡卡“{}”，尚未核實當前執行態。",
        TextKey::HostCommandNotSubmitted => "命令未提交，請稍後重試。",
        TextKey::HostVersionUnknown => "版本未確認",
        TextKey::HostStepsBody => {
            "先檢查模組與 SIM；若模組公網檢查通過，但代理仍報找不到網絡卡，請在代理軟件中檢查“出站介面”是否指向已移除或改名的網絡卡。多網絡卡用戶可能有意固定出口，修改前確認用途。配置來源不明確時，請在代理軟件中手動調整。"
        }
        TextKey::OnboardingWaiting => "等待檢查 / 裝置就緒",
        TextKey::OnboardingChecking => "正在檢查…",
        TextKey::OnboardingNeedsReview => "需要查看原因",
        TextKey::OnboardingDisabledUnverified => "已關閉，尚未驗證",
        TextKey::OnboardingStaleRefresh => "結果已過期，請重新整理",
        TextKey::OnboardingCheckUsb => "USB 裝置識別",
        TextKey::OnboardingCheckAdapter => "Windows 網絡卡",
        TextKey::OnboardingCheckAt => "AT 序列埠通信",
        TextKey::OnboardingCheckCellular => "SIM 與流動網絡",
        TextKey::OnboardingCheckBoundPublic => "模組公網連接",
        TextKey::OnboardingCheckBoundDns => "模組 DNS 解析",
        TextKey::OnboardingRestartStep => "2. 請先重新啟動電腦",
        TextKey::OnboardingEnterAfterRestart => "進入面板（需重新啟動）",
        TextKey::OnboardingAutoCheckStep => "2. 自動檢查模組",
        TextKey::OnboardingStartUsing => "開始使用",
        TextKey::OnboardingEnterPanel => "進入面板",
        TextKey::OnboardingPassedNote => {
            "模組綁定的公網與 DNS 檢查通過；電腦實際出口仍可能由 Wi-Fi 或 VPN 決定。"
        }
        TextKey::OnboardingNotPassedNote => {
            "等待檢查、未連接、證據過期或關閉主動聯網檢查，不等於驅動程式損壞。具體原因可進入面板查看。"
        }
        TextKey::OnboardingTitle => "連接你的 4G 模組",
        TextKey::OnboardingIntro => "連接、檢查，然後開始使用。也可以隨時進入面板。",
        TextKey::OnboardingSetupResultHeading => "本次驅動程式安裝結果",
        TextKey::OnboardingStep1Title => "1 · 連接模組",
        TextKey::OnboardingStep1Body => {
            "插好 SIM 卡，使用支援資料傳輸的 USB 線連接電腦。已有驅動程式可直接使用。"
        }
        TextKey::OnboardingHostHeading => "電腦網絡與代理（可選檢查）",
        TextKey::OnboardingCheckHost => "檢查電腦網絡",
        TextKey::OnboardingHostNote => "即使沒插模組也能檢查；代理配置問題不會當作驅動程式損壞。",
        TextKey::OnboardingStep3Title => "3. 需要驅動程式時再安裝",
        TextKey::OnboardingBundledDriverNote => {
            "已找到本地驅動程式資源，安裝前還會驗證簽名和檔案。此包不能覆蓋所有介面（包括未匹配的 MI_04）；如有缺驅動程式介面無法匹配，將在安裝前停止。已有介面正常時無需重復安裝。"
        }
        TextKey::OnboardingUseBundledDriver => "使用內置驅動程式",
        TextKey::OnboardingBundledDriverHint => {
            "點擊後先確認，再結束面板並顯示 Windows 授權；安裝結束或取消授權後會自動返回面板。需要重新啟動時，請先重新啟動電腦。"
        }
        TextKey::OnboardingNoBundledDriverNote => {
            "此版本未附帶完整驅動程式資源，不能在這里離線安裝。請先開啟 Windows 設定 → Windows 更新 → 可選更新，查看驅動程式更新；也可聯系 DJI 官方支援取得適配此模組的驅動程式，按廠商說明安裝後點擊“重新檢查”。"
        }
        TextKey::OnboardingOpenWindowsUpdate => "開啟 Windows 更新",
        TextKey::OnboardingDjiCompatibilityLink => "DJI 官方相容與驅動程式說明",
        TextKey::OnboardingOfficialLinkNote => {
            "官方網頁提供相容說明與支援入口，不是驅動程式直達下載。網頁不會自動安裝；Windows 更新也不保證提供該模組的全部驅動程式。"
        }
        TextKey::ReportSectionSystem => "系統及程式進程",
        TextKey::ReportSectionUsb => "USB 與異常裝置",
        TextKey::ReportSectionDrivers => "已綁定驅動程式與 INF",
        TextKey::ReportSectionSerial => "序列埠枚舉",
        TextKey::ReportSectionAdapters => "網絡卡與驅動程式",
        TextKey::ReportSectionNetwork => "IP、DNS 與預設路由",
        TextKey::ReportSectionSecurity => "安全軟件狀態",
        TextKey::ReportSectionDriverHistory => "Windows 驅動程式安裝記錄",
        TextKey::ReportSectionIntegrity => "程式及驅動程式檔案完整性",
        TextKey::ReportNoLogDirectory => "無法匯出：當前用戶的日誌目錄不可用",
        TextKey::ReportPreparing => "正在準備詳細日誌…",
        TextKey::ReportStartFailed => "無法啟動匯出：{error}",
        TextKey::ReportExporting => "正在匯出詳細日誌 · {text}",
        TextKey::ReportExported => "詳細日誌已匯出：{}",
        TextKey::ReportIncomplete => "匯出未完成：{}（已生成的部分報告保留在匯出目錄）",
        TextKey::ReportThreadEnded => "匯出線程提前結束，部分報告保留在匯出目錄",
        TextKey::ReportFileName => "大疆4G詳細診斷-{}-{}.txt",
        TextKey::ReportHeader => {
            "DJI 4G SUPPORT REPORT schema=1\nREPORT_STARTED unix_ms={}\napp_version={} exe={} pid={} architecture={}\n包含裝置實例 ID、硬件 ID、驅動程式、網絡配置及本程式日誌。請僅發給排障人員。\n不讀取短訊正文、通訊錄、SIM 號碼或口令；不安裝驅動程式、不修改網絡、不發送 AT 命令。\n某節失敗或超時不會阻止其他節匯出。檔案末尾 REPORT_COMPLETE 表示收集流程結束，不代表裝置正常。\n"
        }
        TextKey::ReportUiSummary => "介面診斷摘要",
        TextKey::ReportMachineSnapshot => "機器可讀快照",
        TextKey::ReportSnapshotNote => "偵測階段、時間、錯誤碼和裝置綁定",
        TextKey::ReportTimelineNote => "最近偵測狀態變化（僅本次進程已觀察到的變化）",
        TextKey::ReportCollectLogs => "收集程式及驅動程式安裝日誌",
        TextKey::ReportHistoryScope => "歷史安裝日誌范圍",
        TextKey::ReportHistoryCapped => "目錄超過 12 個，僅收集最近 12 個版本",
        TextKey::ReportHistoryHeading => "歷史安裝日誌",
        TextKey::ReportLogDirectory => "日誌目錄",
        TextKey::ReportCollectionScope => "日誌收集范圍",
        TextKey::ExportTitle => "DJI 一代 4G 面板 · 診斷資訊匯出\n",
        TextKey::ExportGeneratedAt => "生成時間（UTC）：{}\n",
        TextKey::ExportPrivacy => "私隱說明：{}\n",
        TextKey::ExportSectionOverview => "\n【概覽】\n",
        TextKey::ExportVerdict => "當前判定：{}\n",
        TextKey::ExportEvidenceState => "證據狀態：{}\n",
        TextKey::ExportSectionDevice => "\n【裝置】\n",
        TextKey::ExportDeviceId => "容器 / 裝置實例識別碼：{}",
        TextKey::ExportSectionCellular => "\n【流動網絡】\n",
        TextKey::ExportSectionNetwork => "\n【Windows 網絡】\n",
        TextKey::ExportSectionSms => "\n【短訊】\n",
        TextKey::ExportSendRequest => "發送請求：{}；階段：{:?}；結果：{:?}",
        TextKey::ExportSendError => {
            "發送錯誤：{}；CMS：{:?}；CME：{:?}；系統錯誤：{:?}；可能已提交：{}"
        }
        TextKey::ExportSectionEvidence => {
            "\n【連接證據】\n模組綁定路由查詢只證明找到了匹配路由，不證明網關可達。\n"
        }
        TextKey::ExportLabelWithCode => "{}（代碼 {}）",
        TextKey::ArchiveHeading => "本地歷史",
        TextKey::ArchiveIntro => {
            "開啟後，已讀到的收件短訊會儲存在這臺電腦。斷開模組或重新啟動軟件後仍可查看；這里不能刪除模組中的短訊。"
        }
        TextKey::ArchiveRetention => {
            "僅當前 Windows 用戶可解密；最多儲存 5000 條，超過儲存日期 90 天自動清除。關閉儲存不會刪除已有歷史。"
        }
        TextKey::ArchiveUnavailable => "本地歷史暫不可用：無法確定用戶目錄，或當前處於模擬演示。",
        TextKey::ArchiveToggle => "在這臺電腦儲存短訊歷史（可隨時關閉）",
        TextKey::ArchiveExportTarget => "匯出目標：{}",
        TextKey::ArchiveSearchHint => "搜索號碼或正文",
        TextKey::ArchiveFilterAll => "全部歷史",
        TextKey::ArchiveFilterWeek => "近 7 天儲存",
        TextKey::ArchiveFilterMonth => "近 30 天儲存",
        TextKey::ArchiveExportTxt => "匯出 TXT",
        TextKey::ArchiveClear => "清空本地歷史",
        TextKey::ArchiveCount => "顯示 {} / {} 條 · 按儲存順序排列",
        TextKey::ArchiveEmpty => {
            "暫無本地歷史。開啟儲存後，在“模組短訊”中讀取短訊即可；無法找回模組中已被刪除且未儲存的訊息。"
        }
        TextKey::ArchiveNoMatch => "沒有符合篩選條件的記錄。",
        TextKey::ArchiveNoTimestamp => "發送時間未提供",
        TextKey::ArchiveIncompleteTag => " · 分段未齊",
        TextKey::ArchiveSourceGroup => "來源分組：{} · 歷史副本",
        TextKey::ArchiveCopyBody => "複製正文",
        TextKey::ArchiveClearDescription => {
            "將刪除這臺電腦儲存的全部短訊歷史，並關閉後續儲存。模組中的短訊不受影響。此操作無法撤銷。"
        }
        TextKey::ArchiveClearConfirm => "確認清空",
        TextKey::ArchiveExportTitle => "匯出短訊明文",
        TextKey::ArchiveExportDescription => {
            "將全部已儲存歷史（包含號碼和正文）匯出為未加密 TXT。請妥善保管，分享前檢查私隱。"
        }
        TextKey::ArchiveExportConfirm => "確認匯出全部",
        TextKey::ArchiveCancel => "取消",
        TextKey::ArchiveReady => "本地歷史已就緒；僅保留最近 90 天、最多 5000 條",
        TextKey::ArchivePausedCapture => "檔案讀取或儲存失敗，已暫停擷取；請檢查或清空本地歷史",
        TextKey::ArchivePausedExport => "檔案讀取或儲存失敗，已暫停匯出；請先處理檔案錯誤",
        TextKey::ArchiveExported => "已匯出明文 TXT，請妥善保管該檔案",
        TextKey::ArchiveUpdated => "本地歷史已更新；僅保留最近 90 天、最多 5000 條",
        TextKey::ArchiveWorkerFailed => "無法啟動短訊檔案後臺任務",
        TextKey::ArchiveReading => "正在讀取本地歷史…",
        TextKey::ArchiveWorkerStopped => "短訊檔案後臺任務已停止",
        TextKey::ArchiveDemoStatus => "模擬歷史，僅用於介面驗收，沒有讀取或儲存真實短訊",
        TextKey::ArchiveDemoBody => {
            "【模擬短訊】這是一條本地歷史示例，僅用於檢查閱讀、篩選和匯出提示。"
        }
        TextKey::ArchiveNoIdentity => "未取得穩定裝置和 SIM 身份，本次不寫入本地歷史",
        TextKey::ArchiveBusy => "本地歷史正在處理，請稍後重試",
        TextKey::ArchiveOpenFailed => "無法開啟本地短訊檔案；原檔案已保留",
        TextKey::ArchiveSizeCheckFailed => "無法檢查檔案大小",
        TextKey::ArchiveTooLarge => "本地短訊檔案超過 32 MiB，未載入或覆蓋",
        TextKey::ArchiveReadFailed => "讀取短訊檔案失敗",
        TextKey::ArchiveInvalidFormat => "短訊檔案格式無效；原檔案已保留",
        TextKey::ArchiveDecryptedTooLarge => "解密後的短訊檔案超過大小限制",
        TextKey::ArchiveCorrupt => "短訊檔案內容損壞；原檔案已保留",
        TextKey::ArchiveTooManyRecords => "短訊檔案記錄數超過上限；原檔案已保留",
        TextKey::ArchiveEncodeFailed => "無法編碼短訊檔案",
        TextKey::ArchiveSizeCapSave => "短訊檔案已達大小上限，本次未儲存",
        TextKey::ArchiveSizeCapEncrypt => "加密檔案已達大小上限，本次未儲存",
        TextKey::ArchiveClearFailed => "無法清空本地短訊檔案；未刪除現有記錄",
        TextKey::ToolStateNotQueried => "未查詢",
        TextKey::ToolStateAvailable => "可用",
        TextKey::ToolStateNoData => "無資料",
        TextKey::ToolStateUnsupported => "韌體不支援",
        TextKey::ToolStateTemporarilyUnavailable => "暫時不可用",
        TextKey::ToolStateFormatMismatch => "格式不匹配",
        TextKey::ToolStateTimeout => "查詢超時",
        TextKey::ToolResultOk => "模組返回 OK；配置是否生效需另行確認",
        TextKey::ToolResultRejected => "模組明確拒絕了本次命令",
        TextKey::ToolResultUnsupported => "模組表示不支援此命令",
        TextKey::ToolResultNoAnswer => "沒有收到可用應答（超時、序列埠錯誤或已斷開）",
        TextKey::ToolResultUnrecognized => "模組有應答，但響應格式未被識別",
        TextKey::ToolResultCancelled => "未執行的命令已取消；已執行項見終端記錄",
        TextKey::ToolResultMaybeWritten => "可能已寫入但未收到最終應答；不會自動重試",
        TextKey::ToolResultInvalidated => "裝置或 SIM 已變化，本次結果已作廢",
        TextKey::ToolInputEmpty => "請輸入一條 AT 命令",
        TextKey::ToolInputTooLong => "命令超過 256 個字符",
        TextKey::ToolInputNotAscii => "命令只能包含 ASCII 字符",
        TextKey::ToolInputControlChars => "命令不能包含控制字符或換行",
        TextKey::ToolInputSemicolon => "命令不能包含分號鏈式調用",
        TextKey::ToolInputMustStartAt => "命令必須以 AT 開頭",
        TextKey::ToolInputNotWhitelisted => "該命令不在唯讀白名單內",
        TextKey::ToolInputNeedsInteractive => {
            "此命令族需要交互式工作階段，文本終端無法安全驅動程式"
        }
        TextKey::ToolPresetAttention => "模組響應（AT）",
        TextKey::ToolPresetManufacturer => "制造商（AT+CGMI）",
        TextKey::ToolPresetModel => "型號（AT+CGMM）",
        TextKey::ToolPresetFirmware => "韌體版本（AT+CGMR）",
        TextKey::ToolPresetSim => "SIM 狀態（AT+CPIN?）",
        TextKey::ToolPresetSignal => "訊號質素（AT+CSQ）",
        TextKey::ToolPresetCarrier => "網絡供應商（AT+COPS?）",
        TextKey::ToolPresetRegistration => "網絡註冊（AT+CEREG?）",
        TextKey::ToolPresetAttach => "分組附著（AT+CGATT?）",
        TextKey::ToolPresetPdpContexts => "PDP 上下文（AT+CGDCONT?）",
        TextKey::ToolPresetPdpActive => "PDP 啟用狀態（AT+CGACT?）",
        TextKey::ToolPresetPdpAddress => "PDP 地址（AT+CGPADDR）",
        TextKey::ToolPresetUsbMode => "USB 網絡模式（AT+QCFG=\"usbnet\"）",
        TextKey::ToolPresetTemperature => "溫度（AT+QTEMP）",
        TextKey::ToolPresetServingCell => "服務小區（AT+QENG=\"servingcell\"）",
        TextKey::ToolPresetSmsFormat => "短訊格式（AT+CMGF?）",
        TextKey::ToolPresetSmsStorage => "短訊儲存（AT+CPMS?）",
        TextKey::ToolBatchPresets => "全部預設查詢（批量）",
        TextKey::ToolAdvancedAt => "AT 命令（高級）",
        TextKey::ToolTaskIdle => "空閑",
        TextKey::ToolTaskQueued => "排隊中",
        TextKey::ToolTaskRunning => "執行中",
        TextKey::ToolTaskCancelling => "正在取消",
        TextKey::ToolTaskFinished => "已結束",
        TextKey::ToolElapsedMinutes => "{} 分 {} 秒",
        TextKey::ToolElapsedSeconds => "{} 秒",
        TextKey::ToolElapsedMillis => "{} 毫秒",
        TextKey::ToolUsbModeDjiNdis => "DJI NDIS（電腦網絡卡）",
        TextKey::ToolQueueFull => "命令佇列已滿，請稍後重試",
        TextKey::ToolChannelClosed => "後臺連接已關閉，請稍後重試",
        TextKey::ToolApnContextRange => "PDP 上下文編號必須在 1 到 16 之間。",
        TextKey::ToolApnInvalid => {
            "APN 不合法：不能為空、不能超過 100 字節，且不能包含引號、逗號、分號或控制字符。"
        }
        TextKey::ToolControlledUnavailable => "該受控操作當前不可用。",
        TextKey::ToolTimeUnknown => "時間未知",
        TextKey::ToolClockAgo => "{}（{}前）",
        TextKey::ToolsTitle => "裝置工具",
        TextKey::ToolsIntro => "讀取模組資訊，按需執行經過確認的操作",
        TextKey::ToolsDeviceConnected => "裝置已連接",
        TextKey::ToolsDeviceIdentity => "VID {} · PID {} · 裝置識別碼 {}",
        TextKey::ToolsAtPort => "AT 連接埠 {}",
        TextKey::ToolsDeviceEpoch => "裝置代次 {} · SIM 工作階段 {}",
        TextKey::ToolsNoDevice => "未偵測到裝置",
        TextKey::ToolsNoDeviceHint => "連接模組後才能執行查詢與受控操作",
        TextKey::ToolsSimSession => "SIM 工作階段 {}",
        TextKey::ToolsTabPresets => "預設",
        TextKey::ToolsTabReadOnly => "唯讀查詢",
        TextKey::ToolsTabSwitchHint => "任務執行期間仍可切換標簽頁，但寫入按鈕會被禁用",
        TextKey::ToolsTaskProgress => "任務進度",
        TextKey::ToolsBatchFinished => "批量查詢已結束，請查看逐項結果",
        TextKey::ToolsFinishedUnknown => "已結束，結果未知",
        TextKey::ToolsNoTask => "當前沒有裝置工具任務",
        TextKey::ToolsCancelNote => {
            "取消只會停止等待，不能撤銷已經寫入模組的改動；寫入超時後不會自動重試。"
        }
        TextKey::ToolsLastRejected => "最近一次請求被拒絕：{}（{}）",
        TextKey::ToolsProfileHeading => "模組資料",
        TextKey::ToolsRefreshProfile => "重新整理模組資料",
        TextKey::ToolsRefreshProfileHint => "按順序執行全部唯讀預設查詢；不會寫入模組",
        TextKey::FieldManufacturer => "制造商",
        TextKey::FieldModel => "型號",
        TextKey::FieldFirmwareVersion => "韌體版本",
        TextKey::FieldUsbNetworkMode => "USB 網絡模式",
        TextKey::ToolNotRecognized => "未識別",
        TextKey::FieldCapturedAt => "擷取時間",
        TextKey::ToolsUsbModeUnverified => {
            "模組報告的 USB 網絡模式不是本版本已驗證的值；不會自動切換。"
        }
        TextKey::ToolsNoProfileYet => {
            "尚未讀取到模組資料；點擊「重新整理模組資料」執行一次唯讀查詢。"
        }
        TextKey::ToolsEvidenceHeading => "能力證據",
        TextKey::ToolsEvidenceNote => "每一行只反映一次真實查詢的結果",
        TextKey::ToolsQuerying => "本項查詢中",
        TextKey::ToolsQueryAgain => "重新查詢此項",
        TextKey::ToolsQueryItem => "查詢此項",
        TextKey::ToolsQueryingKeepLast => "本項查詢中；下方保留上次結果與擷取時間",
        TextKey::ToolsReason => "原因：{}（{}）",
        TextKey::ToolsCaptured => "擷取：{} · 裝置代次 {} · SIM 工作階段 {}",
        TextKey::ToolsStorageNote => "儲存查詢可用不代表模組支援發送短訊。",
        TextKey::ToolsNotRunYet => "尚未執行此查詢。",
        TextKey::ToolsStorageCaution => {
            "注意：「短訊儲存」查詢成功僅表示儲存查詢可用，不代表模組支援發送短訊。"
        }
        TextKey::ToolsConnectionHeading => "連接配置",
        TextKey::ToolsNoPdpYet => "尚未讀取到 PDP 上下文；點擊「重新整理模組資料」。",
        TextKey::ToolsNoTemperature => "尚未讀取到溫度感應器。",
        TextKey::ToolsSensor => "感應器{}",
        TextKey::ToolsSensorNote => "感應器定義以韌體為準。",
        TextKey::ToolsControlledHeading => "受控操作",
        TextKey::ToolsControlledNote => {
            "以下寫入沿用修復頁的受控流程：提交後仍需復核目標與風險，且超時不會自動重試。"
        }
        TextKey::FieldPdpContextShort => "PDP 上下文",
        TextKey::ToolsApnExample => "例如 internet",
        TextKey::ToolsEditApn => "修改 APN",
        TextKey::ToolsApnConfirm => "已彈出確認視窗；確認後執行：AT+CGDCONT={},\"IP\",\"{}\"",
        TextKey::ToolsApnRange => "PDP 上下文編號必須是 1 到 16 的整數。",
        TextKey::ToolsSwitchTo => "切換為{}",
        TextKey::ToolsSwitchHint => "通過受控修復流程切換；會重枚舉模組",
        TextKey::ToolsConfirmRuns => "已彈出確認視窗；確認後執行：{}",
        TextKey::ToolsCurrentUnrecognized => "當前值未識別，本版本不提供切換。",
        TextKey::ToolsNotQueriedRefresh => "未查詢；請先重新整理模組資料。",
        TextKey::ToolsRestartModule => "重新啟動模組",
        TextKey::ToolsRestartCommand => "AT+CFUN=1,1；會中斷當前連接",
        TextKey::ToolsRestartConfirm => "已彈出確認視窗；確認後執行：AT+CFUN=1,1",
        TextKey::ToolsRestartNote => "重新啟動會暫時中斷模組連接。",
        TextKey::ToolsReadOnlyHeading => "唯讀 AT 查詢",
        TextKey::ToolsReadOnlyNote => "唯讀白名單內的查詢不需要逐條確認",
        TextKey::ToolsChoosePreset => "選擇預設查詢",
        TextKey::ToolsRunQuery => "執行查詢",
        TextKey::ToolsWhitelistHint => "或輸入白名單內的唯讀命令，例如 AT+CSQ",
        TextKey::ToolsNotInList => {
            "這條命令不在唯讀查詢清單內。若了解其作用，可到 AT 命令（高級）檢查並逐條確認；不確定時請使用預設查詢。"
        }
        TextKey::ToolsOpenAdvanced => "開啟 AT 命令（高級）",
        TextKey::ToolsBusyReadOnly => "任務執行期間只能讀取已有結果，查詢按鈕已禁用。",
        TextKey::ToolsInvalidInput => "輸入無效：{}（{}）",
        TextKey::ToolsSessionUnlocked => "本次工作階段已解鎖",
        TextKey::ToolsEnableAtInput => "啟用 AT 命令輸入",
        TextKey::ToolsUnlockScope => {
            "解鎖只在本次工作階段內有效，裝置或 SIM 變化後會自動重新鎖定。"
        }
        TextKey::ToolsAdvancedNote => {
            "供了解 AT 命令的用戶排查問題。每次只發送一條經驗證的命令；確認前會顯示完整內容。命令可能修改配置或中斷連接。"
        }
        TextKey::ToolsAtHint => "輸入一條 AT 命令，例如 AT+CSQ",
        TextKey::ToolsExecute => "執行",
        TextKey::ToolsClearInput => "清空輸入",
        TextKey::ToolsLocked => "終端處於鎖定狀態：解鎖前不會顯示輸入框與執行按鈕。",
        TextKey::ToolsCheckFailed => "命令未通過驗證：{}（{}）",
        TextKey::ToolsNormalizedWrite => "已識別為受控寫入，將執行規范化命令：{}",
        TextKey::ToolsCommandFrozen => "命令已凍結，請在下方逐條確認後才會寫入模組。",
        TextKey::ToolsAtPending => "AT 命令待確認",
        TextKey::ToolsFrozenList => "以下命令已凍結，確認後才會寫入模組：",
        TextKey::ToolsUnknownEffect => "效果未知，可能改變配置或中斷連接。",
        TextKey::ToolsPlanExpired => "已過期；需要重新準備同一條命令。",
        TextKey::ToolsPlanRemaining => "剩餘 {} 秒內有效，過期後需要重新準備。",
        TextKey::ToolsLogHeading => "命令記錄",
        TextKey::ToolsLogNote => "只儲存在本機內存中的最近任務記錄",
        TextKey::ToolsCopySummary => "複製診斷摘要",
        TextKey::ToolsCopySummaryNote => "僅包含操作類型、耗時和穩定結果碼，不含響應內容",
        TextKey::ToolsCopyRaw => "複製原始響應",
        TextKey::ToolsCopyRawNote => "響應可能包含裝置識別碼、號碼或賬戶資訊",
        TextKey::ToolsClearLog => "清空",
        TextKey::ToolsClearLogNote => "清空現有內存記錄；正在執行的任務完成後仍可能產生新記錄。",
        TextKey::ToolsShareCaution => "「複製原始響應」可能包含裝置或賬戶資訊，請謹慎粘貼分享。",
        TextKey::ToolsNoLog => "暫無任務記錄；執行任意查詢後在此查看響應。",
        TextKey::ToolsEmptyResponse => "（無響應內容）",
        TextKey::ToolsResponseTruncated => "響應過長，已截斷",
        TextKey::ToolsHistoryHeader => "裝置工具歷史摘要（不含響應內容）\n",
        TextKey::ToolsTruncatedMark => "[響應過長，已截斷]\n",
        TextKey::ComposeBusyDraftKept => "當前任務或發送確認尚未結束，草稿已保留。",
        TextKey::ComposeUnsupportedSender => {
            "此發件人不是受支援的短訊號碼，無法直接回復。原號碼未被修改。"
        }
        TextKey::ComposeDemoDraft => {
            "【模擬資料·介面驗收】這是一條僅用於截圖的短訊草稿，請勿實際發送。"
        }
        TextKey::ComposeBackendBusy => {
            "後臺正忙，本條短訊未提交。請等待當前任務結束後重試；草稿已保留。"
        }
        TextKey::ComposeQueueFull => "操作佇列已滿，未提交。請稍後重試；草稿已保留。",
        TextKey::ComposeChannelClosed => "後臺連接已關閉，未提交。請恢復連接後重試；草稿已保留。",
        TextKey::ComposeQueued => "短訊已排隊，請勿重復發送",
        TextKey::ComposePreparing => "正在準備短訊",
        TextKey::ComposeSubmitting => "正在提交短訊",
        TextKey::ComposeAwaitingModule => "正在等待模組確認",
        TextKey::ComposeSubmittedUnknown => "已提交給模組，尚不能確認對方收到。",
        TextKey::ComposeFailedDraftKept => "發送失敗，草稿已保留。",
        TextKey::ComposeUnknownMaybeSent => {
            "發送結果未知，可能已提交。請先核實，避免重復發送；草稿已保留。"
        }
        TextKey::ComposeFailureHeading => "失敗原因與處理建議",
        TextKey::ComposeFailureStage => "失敗階段：{} · 錯誤碼：{}",
        TextKey::ComposeSystemError => "系統錯誤：{}",
        TextKey::ComposeTitle => "新建短訊",
        TextKey::ComposeIntro => "通過當前連接的 4G 模組發送",
        TextKey::ComposeRecipient => "收件人",
        TextKey::ComposeRecipientHint => "國際號碼或客服短號",
        TextKey::ComposeRecipientNote => {
            "輸入 +8613800138000 等完整號碼，或 10086 / 10010 / 10001。"
        }
        TextKey::ComposeBodyLabel => "短訊內容",
        TextKey::ComposeLength => "{} / 70 字",
        TextKey::ComposeBodyHint => "在這里輸入短訊內容…",
        TextKey::ComposeLimits => "單條短訊 · 最多 70 字 · 不支援 Emoji",
        TextKey::ComposeCostNote => "可能產生網絡供應商費用；下一步將核對號碼和正文。",
        TextKey::ComposeSendingWait => "正在發送，請等待結果",
        TextKey::ComposeModuleBusy => "模組正在處理其他任務，請稍候",
        TextKey::ComposeDraftKept => "草稿已保留",
        TextKey::ComposeNeedInput => "填寫有效號碼和內容後即可繼續",
        TextKey::ComposeDraftReady => "草稿已就緒",
        TextKey::ComposeNextConfirm => "下一步：確認發送",
        TextKey::ComposeConfirmTitle => "確認發送短訊",
        TextKey::ComposeConfirmIntro => "請核對以下完整號碼和正文：",
        TextKey::ComposeConfirmNote => {
            "本次發送 1 條短訊，可能產生網絡供應商費用。模組接受不代表對方收到。"
        }
        TextKey::ComposeConfirmAction => "確認發送這條短訊",
        TextKey::ComposeKeepDraftTitle => "保留當前草稿？",
        TextKey::ComposeKeepDraftBody => {
            "已有未發送草稿。可保留當前草稿，或替換為新草稿後繼續編輯。"
        }
        TextKey::ComposeReplaceDraft => "替換為新草稿",
        TextKey::ComposeKeepDraft => "保留草稿",
        TextKey::ComposeRejected => {
            "模組已明確拒絕本條短訊，未接受提交。請根據錯誤碼排查後再手動發送。"
        }
        TextKey::ComposeMaybeSent => "可能已經提交，請勿直接重發。",
        TextKey::ComposeNotSubmitted => "本次未提交，可檢查連接、SIM 卡和短訊服務後重試。",
        TextKey::ComposeStageQueued => "排隊",
        TextKey::ComposeStagePreparing => "準備",
        TextKey::ComposeStageSubmitting => "提交",
        TextKey::ComposeStageAwaiting => "等待模組結果",
        TextKey::ComposeStageDone => "完成",
        TextKey::ComposeSerialBusy => "序列埠正被其他任務佔用。請等待任務結束，再手動重試。",
        TextKey::ComposeSerialOpenFailed => {
            "無法開啟短訊序列埠。請檢查裝置連接及其他序列埠程式是否佔用。"
        }
        TextKey::ComposeSerialCloseTimeout => {
            "序列埠關閉超時，後臺未能確認資源已釋放。請恢復裝置連接後再操作。"
        }
        TextKey::ComposeNoDevice => "當前沒有可用裝置。請連接模組並重新整理裝置狀態。",
        TextKey::ComposeContextChanged => {
            "發送過程中裝置或 SIM 卡發生變化。請核對當前裝置及發送記錄。"
        }
        TextKey::ComposeModuleRejected => {
            "模組拒絕了短訊。請結合 CMS/CME 錯誤碼檢查 SIM 卡、餘額及網絡供應商短訊服務。"
        }
        TextKey::ComposeSerialFailed => "序列埠通信失敗。請檢查 USB 連接和模組供電。",
        TextKey::ComposeTimeout => "等待模組響應超時。請核實發送記錄及連接狀態，避免重復發送。",
        TextKey::ComposeNoReference => {
            "模組沒有返回短訊提交編號，無法確認提交結果。請先核實是否已發送。"
        }
        TextKey::ComposeUnexpectedEnd => {
            "模組返回了非預期的結束響應，無法確認提交結果。請保留錯誤碼並核實發送情況。"
        }
        TextKey::ComposeSerialBusyShort => "序列埠正在被其他任務佔用，請等待任務結束。",
        TextKey::ComposePortBusyShort => "無法開啟序列埠，請檢查裝置連接及連接埠佔用。",
        TextKey::ComposeTimeoutShort => "等待模組響應超時，請檢查連接及模組狀態。",
        TextKey::ComposeValidationFailed => "號碼或正文未通過校驗，請檢查支援的號碼和正文長度。",
        TextKey::ComposeDeviceLost => "裝置連接中斷，請重新連接裝置並重新整理。",
        TextKey::ComposeGenericAdvice => {
            "請檢查裝置連接、SIM 卡狀態和網絡供應商短訊服務；保留錯誤碼以便排查。"
        }
        TextKey::ComposeCmeError => "CME 錯誤：{}",
        TextKey::SetupFailedTitle => "安裝未完成",
        TextKey::SetupNoPayload => "此構建未包含安裝資源，請使用完整安裝包。",
        TextKey::SetupConfirmTitle => "安裝大疆 4G 面板",
        TextKey::SetupConfirmBody => {
            "將為當前用戶安裝程式、離線驅動程式資源，並建立桌面快捷方式。\n\n安裝程式本身不會修改系統驅動程式，完成後可選擇安裝驅動程式。\n\n是否繼續？"
        }
        TextKey::SetupVerifyFailed => "安裝檔案驗證失敗。",
        TextKey::SetupDoneTitle => "安裝完成",
        TextKey::SetupDoneBody => {
            "程式和離線驅動程式已安裝，桌面快捷方式已建立。\n\n現在安裝模組驅動程式嗎？需要管理員確認。\n已有驅動程式可選“否”，直接開啟程式。"
        }
        TextKey::SetupDriverCancelled => {
            "驅動程式安裝已取消。程式檔案已安裝，但未確認模組驅動程式可用。"
        }
        TextKey::SetupDriverIncomplete => {
            "驅動程式安裝未完成（結束碼：{}）。程式檔案已安裝，但不會自動開啟面板。\n\n若安全軟件有攔截，請保留報告中的偵測名稱與檔案路徑。不要關閉防護；請將報告交給開發者核查。"
        }
        TextKey::PortableBadResourcePath => "安裝資源路徑無效：{}",
        TextKey::PortableBadResourceDir => "資源目錄無效",
        TextKey::PortableWriteFailed => "無法釋放資源 {}：{}",
        TextKey::PortableReadFailed => "無法讀取資源 {}：{}",
        TextKey::PortableVerifyFailed => "資源驗證失敗：{}。請保留安全軟件報告，不要關閉防護。",
        TextKey::PortableLaunchFailed => "大疆 4G 面板啟動失敗",
        TextKey::PortableNoPayload => "此構建未包含獨立執行資源。",
        TextKey::PortableNoAppData => "無法讀取當前用戶的應用程式資料目錄",
        TextKey::PortableOpenFailed => "無法開啟面板：{}。如有安全軟件攔截，請保留報告。",
        TextKey::PortableExitedAbnormally => "面板異常結束：{}。請保留安全軟件報告及程式日誌。",
        TextKey::DriverResultTitle => "模組驅動程式檢查結果",
        TextKey::DriverResultBody => "{}\n\n{}\n\n點擊“確定”返回面板。",
        TextKey::DriverNotStarted => "尚未開始安裝",
        TextKey::DriverPanelRunning => {
            "面板尚未完全結束，或無法核實正在執行的面板。請返回原面板；關閉系統匣中的面板後再嘗試安裝。未執行驅動程式安裝。"
        }
        TextKey::DriverInstallTitle => "安裝模組驅動程式",
        TextKey::DriverInstallBody => {
            "將驗證隨程式附帶的驅動程式，僅為缺驅動程式介面選擇匹配包。Windows 可能更新其他匹配同一驅動程式包的裝置，不強制覆蓋更優驅動程式。\n\n請先結束大疆 4G 面板（含系統匣）。點擊“是”後申請管理員授權，結束後自動返回面板。"
        }
        TextKey::DriverElevationFailed => "管理員授權未完成",
        TextKey::DriverOpenPanel => "請開啟面板繼續",
        TextKey::DriverNoReturn => {
            "{}\n\n未能自動返回。請從桌面正常開啟“大疆 4G 面板”，在設定中開啟首次連接引導。"
        }
        TextKey::DriverNoExePath => "無法確定程式位置，請重新開啟完整程式。",
        TextKey::DriverNoExeDir => "無法確定程式目錄。",
        TextKey::DriverCheckComponentFailed => {
            "無法啟動 Windows 驅動程式檢查組件，請聯系技術支援。"
        }
        TextKey::DriverClockInvalid => "系統時間異常，未開始安裝。",
        TextKey::DriverLogCreateFailed => {
            "無法建立安裝日誌，未開始安裝。請將完整程式放在可寫目錄後重試。"
        }
        TextKey::DriverLogWriteFailed => "無法寫入日誌，未開始安裝。",
        TextKey::DriverLogAppendFailed => "安裝日誌寫入失敗，請檢查裝置實際狀態。日誌位置：{}",
        TextKey::DriverLogPath => "詳細安裝日誌：{}",
        TextKey::DriverCheckNotStarted => "Windows 驅動程式檢查未能啟動。{}",
        TextKey::DialogActionLine => "操作：{}\n",
        TextKey::DialogDisruptionLine => "中斷：{}\n",
        TextKey::DialogRiskLine => "風險：{}\n",
        TextKey::DialogElevationLine => "提權：{}\n",
        TextKey::DriverInstallResultTitle => "模組驅動程式安裝結果",
        TextKey::SettingsGeneral => "一般",
        TextKey::SettingsInterfaceTheme => "介面主題",
        TextKey::SettingsStartupTray => "啟動與系統匣",
        TextKey::SettingsAutoStart => "登入後自動啟動",
        TextKey::SettingsStartHidden => "啟動後隱藏到系統匣",
        TextKey::SettingsLogging => "日誌",
        TextKey::SettingsAbout => "關於",
        TextKey::SettingsVersion => "DJI 一代 4G 面板 · v{}",
        TextKey::CarrierChinaUnicom => "中國聯通",
        TextKey::CarrierChinaMobile => "中國流動",
        TextKey::CarrierChinaTelecom => "中國電信",
        TextKey::CarrierChinaBroadnet => "中國廣電",
        TextKey::RateHeading => "實時速率",
        TextKey::RatePeakDownload => "下載峰值",
        TextKey::RatePeakUpload => "上傳峰值",
        TextKey::ThemeSystem => "跟隨系統",
        TextKey::ThemeLight => "淺色",
        TextKey::ThemeDark => "深色",
        TextKey::SmsFragmentsRead => "已讀取 {} / {} 個分段",
        TextKey::SmsErrPortBusy => "序列埠上一個操作尚未結束，請稍後重新整理",
        TextKey::SmsErrPortAccess => "序列埠訪問失敗",
        TextKey::SmsErrNoAtPort => "未找到可驗證的短訊 AT 序列埠",
        TextKey::SmsErrPduMode => "短訊 PDU 模式未確認",
        TextKey::SmsErrResponseInvalid => "模組響應未通過驗證",
        TextKey::SmsErrTimeout => "短訊查詢超時",
        TextKey::SmsErrNoDevice => "無已驗證的裝置，請先檢查概覽中的模組連接狀態",
        TextKey::SmsErrDeviceGone => "裝置已斷開",
        TextKey::SmsErrSimUnknown => "尚未識別 SIM 卡，請先返回概覽重新整理連接狀態",
        TextKey::SmsErrSimChanged => "SIM 卡已更換，本次短訊未加入清單；請返回概覽重新偵測",
        TextKey::SmsErrSimIdentity => "無法確認 SIM 卡身份，本次未讀取短訊；請檢查 SIM 後重新偵測",
        TextKey::SmsStopped => "已停止讀取，原有清單已保留",
        TextKey::SmsErrContextChanged => "模組或 SIM 已變化，本次結果已丟棄，請重新偵測",
        TextKey::SmsErrRestoreUnconfirmed => {
            "無法確認已恢復原讀取位置；請先重新偵測模組，暫不要發送或刪除短訊"
        }
        TextKey::SmsErrUnsupportedLocation => "模組不支援讀取此位置，請使用當前儲存位置",
        TextKey::SmsErrLimit => "短訊數量或返回內容超過安全上限，本次清單未更新",
        TextKey::SmsErrQueryFailed => "短訊查詢失敗",
        TextKey::SmsSystemError => "；系統錯誤 {}",
        TextKey::SmsPageTitle => "短訊",
        TextKey::SmsReadDetailsAttention => "讀取詳情 · 需注意",
        TextKey::SmsReadDetails => "讀取詳情",
        TextKey::SmsRefreshList => "重新整理清單",
        TextKey::SmsBusyWait => "當前通信任務尚未結束，請稍後重新整理",
        TextKey::SmsPhaseWaiting => "等候讀取",
        TextKey::SmsPhaseConfirming => "正在確認模組與 SIM",
        TextKey::SmsPhaseQueryingStorage => "正在查詢短訊儲存位置",
        TextKey::SmsPhaseSelecting => "正在選擇讀取位置",
        TextKey::SmsPhaseReading => "正在讀取歷史短訊",
        TextKey::SmsPhaseOrganizing => "正在整理短訊",
        TextKey::SmsPhaseRestoring => "正在恢復原讀取位置，請稍候",
        TextKey::SmsPhaseReleasing => "正在釋放連接，請稍候",
        TextKey::SmsProgressRecords => "{} · 已讀取 {} 個儲存記錄",
        TextKey::SmsStopReading => "停止讀取",
        TextKey::SmsStopRequested => {
            "已請求停止，正在等待連接釋放；自動重新整理已暫停，可手動重新整理繼續。"
        }
        TextKey::SmsStorageUnconfirmed => "未確認",
        TextKey::SmsStorageSim => "SIM 卡",
        TextKey::SmsStorageModule => "模組",
        TextKey::SmsStorageModuleArea => "模組當前匯總區",
        TextKey::SmsStorageCurrentArea => "當前儲存區",
        TextKey::SmsRecordsRead => "已讀取 {} 個記錄 · 儲存位置與讀取詳情",
        TextKey::SmsLastRead => "最近讀取：{} · {} 個短訊分段，{} 個其他記錄未展示",
        TextKey::SmsRefreshNote => {
            "重新整理會讀取當前位置中仍儲存的全部短訊（最多 1000 個儲存記錄）。長短訊可能佔多個記錄；已被模組刪除的內容無法重新讀回。可開啟“本地歷史”儲存之後讀到的短訊。"
        }
        TextKey::SmsOtherLocationsNote => {
            "其他位置可能還有短訊。選擇前會再次確認；讀取期間暫時切換讀取位置，完成後恢復，可能將未讀短訊標為已讀。"
        }
        TextKey::SmsReadSim => "讀取 SIM 卡短訊",
        TextKey::SmsReadModule => "讀取模組短訊",
        TextKey::SmsTaskBusy => "當前通信任務尚未結束",
        TextKey::SmsLocationUnsupported => "模組尚未確認支援此儲存位置",
        TextKey::SmsWillConfirm => "讀取前將再次確認",
        TextKey::SmsReadOtherLocation => "讀取其他位置的短訊",
        TextKey::SmsReadOtherBody => {
            "將讀取{}中儲存的短訊，期間暫停其他模組操作。讀取可能改變短訊已讀狀態；完成後會恢復原讀取位置，不改變短訊寫入和接收位置。"
        }
        TextKey::SmsConfirmRead => "確認讀取",
        TextKey::SmsSyncing => "正在同步模組短訊…",
        TextKey::SmsUnreadCount => "{} 條未讀",
        TextKey::SmsStorageUsage => "儲存 {} / {}",
        TextKey::SmsAutoSyncPaused => "自動同步已暫停 · 點擊重新整理清單繼續",
        TextKey::SmsSyncProblem => "短訊同步遇到問題",
        TextKey::SmsSyncHistory => "同步提示與歷史記錄",
        TextKey::SmsCacheTrimmed => "本地快取已移出 {} 條較早短訊；模組儲存可能仍有記錄。",
        TextKey::SmsTabInbox => "收件匣  {}",
        TextKey::SmsTabOutgoing => "發送記錄  {}",
        TextKey::SmsBackToList => "返回訊息清單",
        TextKey::SmsFooterNote => "模組接受發送 ≠ 收件人已收到  ·  短訊內容不會寫入診斷日誌",
        TextKey::SmsFilterAll => "全部",
        TextKey::SmsFilterIncomplete => "長短訊缺片",
        TextKey::SmsClearSearch => "清空搜索",
        TextKey::SmsEmptyFilter => "沒有符合篩選條件的短訊",
        TextKey::SmsEmptyFilterHint => "切換為「全部」可查看其他短訊",
        TextKey::SmsBalanceQuery => "查詢話費",
        TextKey::SmsBalanceIntro => "選擇 SIM 所屬網絡供應商，產生查詢草稿後確認發送。",
        TextKey::SmsBalanceInstructions => "指令說明",
        TextKey::SmsBalancePortal => "開啟廣電營業廳查詢",
        TextKey::SmsBalanceBroadnetHelp => "登入後進入「個人中心 → 我的帳戶」查詢餘額。",
        TextKey::SmsBalanceReplyHint => "短訊回復顯示在收件箱。查詢指令及短訊服務以當地套餐為準。",
        TextKey::SmsLastSynced => "上次完整讀取 {}：{}前",
        TextKey::SmsSyncReadingCache => "正在讀取模組，目前顯示已有記錄",
        TextKey::SmsSyncFailedCache => "同步失敗，目前顯示快取",
        TextKey::SmsSyncPartialCache => "讀取不完整，目前顯示已有記錄",
        TextKey::SmsSyncPausedCache => "同步已暫停，目前顯示已有記錄",
        TextKey::SmsSyncStaleCache => "記錄較舊，等待同步",
        TextKey::SmsSyncStoredList => "模組儲存短訊 · 本頁每 15 秒同步",
        TextKey::SmsSyncNotCompleted => "尚未完成模組儲存讀取",
        TextKey::SmsSearchHint => "搜索號碼或短訊內容",
        TextKey::SmsEmptySearch => "沒有找到相關短訊",
        TextKey::SmsEmptySearchHint => "試試其他號碼或關鍵詞",
        TextKey::SmsEmptyOutgoing => "還沒有發送記錄",
        TextKey::SmsEmptyOutgoingHint => "點擊右上角「新建短訊」開始寫信",
        TextKey::SmsLoading => "正在讀取短訊",
        TextKey::SmsLoadingHint => "正在與模組同步，請稍候",
        TextKey::SmsInboxWaiting => "收件匣等待同步",
        TextKey::SmsInboxWaitingHint => "連接模組後會自動讀取已儲存的短訊",
        TextKey::SmsInboxEmpty => "暫無收到的短訊",
        TextKey::SmsInboxEmptyHint => "模組中暫時沒有可顯示的短訊",
        TextKey::SmsUnavailable => "暫時無法讀取短訊",
        TextKey::SmsUnavailableHint => "請查看上方的具體錯誤，檢查連接後重試",
        TextKey::SmsRefreshMessages => "重新整理短訊",
        TextKey::SmsNoTimestamp => "時間未提供",
        TextKey::SmsReaderEmpty => "訊息閱讀區",
        TextKey::SmsReaderEmptyHint => "從左側選擇一條短訊，即可在這里查看完整內容",
        TextKey::SmsKindIncoming => "收到的短訊",
        TextKey::SmsKindOutgoing => "發送記錄",
        TextKey::SmsNoTimestampFromModule => "模組未提供時間",
        TextKey::SmsReply => "回復",
        TextKey::SmsConfirmDeleteFragments => "確認刪除 {} 個已讀取分段",
        TextKey::SmsDeleteFragments => "刪除已讀取 {} 個分段",
        TextKey::SmsDeleteMessage => "刪除短訊（{} 個分段）",
        TextKey::SmsDeleteBusy => "請等待通信任務結束，並重新核對短訊分段",
        TextKey::SmsDeleteConflict => "分段身份存在沖突，暫不能刪除；請重新讀取並核對。",
        TextKey::SmsDeleting => "正在刪除：已確認 {} / {} 個分段",
        TextKey::SmsDeletedAll => "已確認刪除全部 {} 個已讀取分段",
        TextKey::SmsDeleteUnknown => {
            "刪除結果未知：已確認 {} / {} 個，{} 個未能確認。請重新整理核對，不會自動重試。"
        }
        TextKey::SmsDeletePartial => "部分刪除：已確認 {} / {} 個分段，其餘失敗或未執行。",
        TextKey::SmsDeleteNone => "未確認刪除任何分段；請查看原因並重新讀取。",
        TextKey::SmsDeleteResultTitle => "刪除分段結果",
        TextKey::SmsDeleteConfirmed => "已確認刪除",
        TextKey::SmsDeleteFailed => "失敗",
        TextKey::SmsDeleteUnknownShort => "結果未知",
        TextKey::SmsDeleteNotRun => "未執行",
        TextKey::ArchiveExportDirFailed => "無法建立匯出目錄，請選擇可寫位置",
        TextKey::ArchiveExportCreateFailed => "無法匯出：請選擇可寫位置和一個不存在的新檔案名",
        TextKey::ArchiveExportWriteFailed => "匯出寫入失敗，未保留不完整檔案",
        TextKey::ArchiveInvalidPath => "短訊檔案路徑無效",
        TextKey::ArchiveCreateDirFailed => "無法建立短訊檔案目錄",
        TextKey::ArchiveTempFileFailed => "無法建立加密檔案臨時檔案",
        TextKey::ArchiveWriteFailed => "寫入加密檔案失敗",
        TextKey::ArchiveReplaceFailed => "替換加密檔案失敗；原檔案已保留",
        TextKey::ExportSmsHeader => {
            "DJI 4G 本地短訊歷史（明文匯出）\n擷取時間為 Unix UTC 秒；原報時間保留模組原文。\n"
        }
        TextKey::ExportSmsRecord => {
            "裝置/SIM 分區：{}\n發件人：{}\n原報時間：{}\n擷取時間：{}\n內容{}：\n{}\n--------"
        }
        TextKey::ExportSmsIncomplete => "（分段不完整）",
        TextKey::DriverOutcomeReady => {
            "Windows 中的驅動程式介面檢查已通過。面板將重新檢查 USB、網絡卡和 AT 通信；短訊及上網是否可用，請以新的檢查結果為準。"
        }
        TextKey::DriverOutcomeRestartRequired => {
            "Windows 要求重新啟動電腦，本次還不能確認驅動程式可用。請先儲存工作並重新啟動電腦，再開啟面板檢查模組。不要重復安裝。"
        }
        TextKey::DriverOutcomeRestartAfterFailure => {
            "驅動程式安裝的部分操作失敗，且 Windows 已要求重新啟動電腦；本次不能確認驅動程式可用。請保留安裝日誌，先儲存工作並重新啟動電腦，再開啟面板檢查。不要重復安裝；重新啟動後仍異常時，把日誌交給技術支援。"
        }
        TextKey::DriverOutcomeCancelled => {
            "已取消驅動程式安裝或 Windows 管理員授權，未開始安裝。你可以繼續使用面板；確實需要安裝時，點擊“使用內置驅動程式”，並在 Windows 授權視窗選擇“是”。"
        }
        TextKey::DriverOutcomeNoMatch => {
            "當前驅動程式包沒有為某個缺驅動程式介面找到唯一匹配項（例如 MI_04），本次未安裝任何驅動程式。請先通過 Windows 更新查找適配驅動程式，或聯系 DJI 官方支援提供該模組的匹配驅動程式。重復安裝本包不能補齊該介面。"
        }
        TextKey::DriverOutcomeInterfacesAbnormal => {
            "驅動程式檢查後仍有介面異常，暫時不能確認可用。請查看面板的新檢查結果；若仍異常，開啟裝置管理器查看原因，並把安裝日誌交給技術支援。不要反復安裝。"
        }
        TextKey::DriverOutcomeNoModule => {
            "沒有偵測到已連接的大疆一代模組，或模組在安裝後暫時斷開。請插穩支援資料傳輸的 USB 線，等待模組識別，再點擊“重新檢查”。"
        }
        TextKey::DriverOutcomePayloadInvalid => {
            "安裝資源缺失、驗證未通過，或與當前 Windows 不相容，本次未開始安裝。請重新取得完整的原始驅動程式版程式，或聯系 DJI 官方支援；不要修改驅動程式檔案。"
        }
        TextKey::DriverOutcomeIncomplete => {
            "安裝未完成，不能確認裝置狀態。請查看本次安裝日誌，並在面板中重新檢查；如需協助，把日誌交給技術支援。"
        }
        TextKey::DriverWindowsUpdateFailed => "無法開啟 Windows 更新，請從系統設定中開啟。",
        TextKey::DriverAdminRequired => "安裝器以管理員身份執行，請從桌面正常開啟面板。",
        TextKey::DriverPanelNotSameDirectory => "等待對象不是同目錄面板，未安裝驅動程式",
        TextKey::DriverPanelExitTimeout => {
            "面板未能在 30 秒內結束，未安裝驅動程式。請結束系統匣中的面板後重試。"
        }
        TextKey::TimelineCellChangedDetail => "服務小區已變化",
        TextKey::TimelineAdapterLinkChangedDetail => "網絡卡鏈路狀態變化",
        TextKey::TimelineRegistrationChangedDetail => "註冊狀態：{} → {}",
        TextKey::TimelineDnsChangedDetail => "DNS 探測：{} → {}",
        TextKey::TimelineRegistrationHomeDetail => "已註冊到本地網絡",
        TextKey::TimelineRegistrationRoamingDetail => "已註冊到漫游網絡",
        TextKey::TimelineRegistrationSearchingDetail => "正在搜索",
        TextKey::TimelineRegistrationDeniedDetail => "註冊被拒絕",
        TextKey::TimelineRegistrationNotRegisteredDetail => "未註冊",
        TextKey::TimelineRegistrationUnknownDetail => "未知",
        TextKey::TimelineDnsPassedDetail => "通過",
        TextKey::TimelineDnsFailedDetail => "失敗",
        TextKey::TimelineDnsIncompleteDetail => "未完成",
        TextKey::ArchiveCryptoUnsupported => "此系統不支援 Windows 用戶加密，未寫入短訊",
        TextKey::ArchiveCryptoTooLarge => "短訊檔案超過加密大小限制",
        TextKey::ArchiveCryptoFailed => "Windows 用戶加密失敗，未寫入短訊",
        TextKey::ArchiveCryptoDecryptFailed => {
            "無法解密本地短訊檔案：用戶不匹配或檔案損壞；原檔案已保留"
        }
        TextKey::ToolUrcLine => "[模組主動上報] {}",
        TextKey::SmsFailureWithCode => "{}（{}）",
        TextKey::ComposeCmsError => "CMS：{}",
        TextKey::SmsDeleteMessageOne => "刪除短訊（{} 個分段）",
    }
    // ZH-TW-CATALOG-END
}

/// American English.
///
/// Authored arm by arm, in the same order as the simplified catalog, so the two can be read side by
/// side. Keys whose batch has not landed yet fall back to the authored Chinese text; the fallback
/// arm is removed by the last batch, which turns any key without an English arm into a compile
/// error rather than a silent Chinese string on an English screen.
fn en_us(key: TextKey) -> &'static str {
    // EN-CATALOG-START
    match key {
        TextKey::AvailabilityDetectingTitle => "Checking",
        TextKey::AvailabilityDetectingReason => {
            "Collecting and verifying this device's connection evidence."
        }
        TextKey::AvailabilityAvailableTitle => "Available",
        TextKey::AvailabilityAvailableReason => {
            "This module's network address, route, public connectivity and DNS all check out."
        }
        TextKey::AvailabilityLimitedTitle => "Limited",
        TextKey::AvailabilityUnavailableTitle => "Unavailable",
        TextKey::AvailabilityNotDetectedTitle => "Not detected",
        TextKey::AvailabilityNotDetectedReason => {
            "Enumeration found no supported first-generation DJI 4G module (VID 2CA3, PID 4006)."
        }
        TextKey::AvailabilityUnsupportedTitle => "Unsupported",
        TextKey::AvailabilityUnsupportedReason => {
            "A related device was detected, but it is not a supported first-generation module \
             (PID 4006); no writes or repairs will run."
        }
        TextKey::LimitedReasonDnsFailure => {
            "The module data path is reachable, but DNS resolution through that interface fails."
        }
        TextKey::LimitedReasonSingleProtocolFamily => {
            "The module verified a connection over only one of the required IP families."
        }
        TextKey::LimitedReasonCompetingDefaultRoute => {
            "The module path passed verification, but the system default route is held by a VPN or \
             TUN interface."
        }
        TextKey::LimitedReasonAtControlUnavailable => {
            "The device was identified, but the AT port is unusable (serial error), so cellular \
             state cannot be read."
        }
        TextKey::LimitedReasonIncompleteEvidence => {
            "The available evidence is not yet enough to confirm every connection capability."
        }
        TextKey::UnavailableReasonCellularRejected => {
            "SIM or cellular registration was explicitly rejected."
        }
        TextKey::UnavailableReasonNoUsableAddressOrRoute => {
            "The module adapter has no usable address or route."
        }
        TextKey::UnavailableReasonBoundPublicProbeFailed => {
            "Public-network probes bound to the module adapter kept failing."
        }
        TextKey::UnavailableReasonNoBoundReachability => {
            "Windows can reach the internet over another network, but nothing yet proves this \
             module's path is reachable."
        }
        TextKey::HotspotUnsupportedTitle => "Hotspot unavailable",
        TextKey::HotspotOff => "Off",
        TextKey::HotspotStarting => "Starting",
        TextKey::HotspotOnWithClients => "On ({client_count} devices connected)",
        TextKey::HotspotOnClientsUnknown => "On (client count unknown)",
        TextKey::HotspotStopping => "Stopping",
        TextKey::HotspotFailed => "The hotspot action failed.",
        TextKey::HotspotUnsupportedMissingPackageIdentity => {
            "This run has no app package identity, which the hotspot feature requires."
        }
        TextKey::HotspotUnsupportedMissingWifiControlCapability => {
            "This build has not been granted the system capability needed to control the mobile \
             hotspot."
        }
        TextKey::HotspotUnsupportedNoWifiAdapter => {
            "No Wi-Fi adapter that can share the network was detected."
        }
        TextKey::HotspotUnsupportedPolicyDisabled => {
            "The mobile hotspot is disabled by system or organization policy."
        }
        TextKey::HotspotUnsupportedOperatingSystem => {
            "This version of Windows does not support this way of controlling the hotspot."
        }
        TextKey::HotspotUnsupportedSourceProfileUnavailable => {
            "The hotspot's uplink cannot be safely bound to this module."
        }
        TextKey::IssueSeverityInfo => "Notice",
        TextKey::IssueSeverityWarning => "Warning",
        TextKey::IssueSeverityError => "Error",
        TextKey::IssueLayerDevice => "USB device",
        TextKey::IssueLayerCellular => "Cellular",
        TextKey::IssueLayerNetwork => "Windows network",
        TextKey::IssueLayerBoundProbe => "Module-bound probes",
        TextKey::IssueLayerHotspot => "Mobile hotspot",
        TextKey::IssueLayerOperation => "Actions and repairs",
        TextKey::EvidenceSourcePnp => "Windows device enumeration",
        TextKey::EvidenceSourceAtControl => "AT control channel",
        TextKey::EvidenceSourceWindowsAdapter => "Windows adapter state",
        TextKey::EvidenceSourceBoundGatewayProbe => {
            "Module-bound route lookup (historical source label)"
        }
        TextKey::EvidenceSourceBoundDnsProbe => "Module-bound DNS probe",
        TextKey::EvidenceSourceBoundPublicProbe => "Module-bound public-network probe",
        TextKey::EvidenceSourceGlobalRoute => "System default route",
        TextKey::EvidenceSourceGlobalConnectivity => "Windows global connectivity",
        TextKey::EvidenceSourceHotspot => "Windows mobile hotspot",
        TextKey::ClassificationPhaseStartup => "Starting detection",
        TextKey::ClassificationPhaseRecentInsertion => {
            "A newly inserted device was found; detecting"
        }
        TextKey::ClassificationPhaseReenumerating => "The device is re-enumerating",
        TextKey::ClassificationPhasePostWriteVerification => "Verifying the state after the action",
        TextKey::ClassificationPhaseStable => "Detection complete",
        TextKey::ActionRefresh => "Refresh and re-check",
        TextKey::RepairDhcpDisabled => {
            "IPv4 DHCP is disabled on the module adapter; its lease cannot be renewed."
        }
        TextKey::RepairApnInvalid => "The APN contains characters that are not allowed.",
        TextKey::ActionRenewDhcp => "Renew the module adapter's DHCP lease",
        TextKey::ActionApplyDnsAutomatic => "Restore automatic DNS",
        TextKey::ActionApplyDnsStatic => "Apply static DNS ({server_count} servers)",
        TextKey::ActionApplyDnsProfile => "Change the DNS configuration",
        TextKey::ActionRestartAdapter => "Restart the module adapter",
        TextKey::ActionReenumerateDevice => "Re-enumerate the module device",
        TextKey::ActionRestartModule => "Restart the cellular module",
        TextKey::ActionEditApn => "Edit the APN of PDP context {cid}",
        TextKey::ActionSetUsbProfileDjiNdis => "Switch to computer adapter (DJI NDIS)",
        TextKey::ActionSetUsbProfileEcm => "Switch to the ECM adapter",
        TextKey::ActionSetUsbNetworkProfile => "Switch the USB network profile",
        TextKey::ActionEnableHotspot => "Turn on the mobile hotspot",
        TextKey::ActionDisableHotspot => "Turn off the mobile hotspot",
        TextKey::RiskLevelLow => "Low risk",
        TextKey::RiskLevelMedium => "Medium risk",
        TextKey::RiskLevelHigh => "High risk",
        TextKey::OperationOutcomeApplied => "The action was applied and the state was read back.",
        TextKey::OperationUsbConfigurationSaved => {
            "The USB network profile was saved; restart the module by hand and re-check — the \
             adapter mode is not verified yet."
        }
        TextKey::OperationOutcomeFailed => "The action failed.",
        TextKey::OperationOutcomeUnknown => {
            "The result could not be confirmed, and nothing is retried automatically. Refresh, then \
             check the device state."
        }
        TextKey::DnsProfileAutomatic => "Automatic DNS",
        TextKey::DnsProfileStatic => "Static DNS",
        TextKey::UsbNetworkProfileDjiNdis => "The DJI NDIS profile this project verified",
        TextKey::UsbNetworkProfileEcm => "The ECM profile this project verified",
        TextKey::DisruptionNone => "No connection interruption",
        TextKey::DisruptionBrief => "The connection may flicker briefly",
        TextKey::DisruptionConnectionInterrupting => {
            "The network connection will be interrupted briefly"
        }
        TextKey::DisruptionDeviceReenumeration => "The device will disconnect and reappear",
        TextKey::ActionSafetyUnsupportedDevice => {
            "The target is not a supported first-generation DJI 4G module; the action was blocked."
        }
        TextKey::ActionSafetyStaleEpoch => {
            "The device reconnected or re-enumerated; prepare the action again."
        }
        TextKey::ActionSafetyStaleSnapshot => {
            "The device state has changed; refresh and try again."
        }
        TextKey::ActionSafetyTargetIdentityChanged => {
            "The target device's identity has changed; the action was blocked."
        }
        TextKey::ActionSafetyBeforeStateChanged => {
            "The state before the action has changed; confirm again."
        }
        TextKey::ActionSafetyExpired => "This confirmation has expired; prepare the action again.",
        TextKey::RollbackNotRequired => "No rollback needed",
        TextKey::RollbackApplied => "The previous state was restored",
        TextKey::RollbackFailed => "The rollback failed; check the current state",
        TextKey::RollbackNotAttempted => "No rollback was attempted",
        TextKey::FreshnessFresh => "Up to date",
        TextKey::FreshnessStale => "Stale; re-checking",
        TextKey::FreshnessUnknown => "No valid update time yet",
        TextKey::LastObservedAt => "Last checked: {time}",
        TextKey::ObservedAgo => "Updated {age} ago",
        TextKey::ErrorPermissionDenied => "Not enough permission to complete this action.",
        TextKey::ErrorDeviceRemoved => "The device disconnected during the action.",
        TextKey::ErrorDeviceIdentityChanged => "The device's identity changed; the action stopped.",
        TextKey::ErrorEvidenceExpired => {
            "The evidence behind this decision has expired; refresh the state."
        }
        TextKey::ErrorProbeFailed => "The network probe did not complete successfully.",
        TextKey::ErrorDnsFailed => "DNS resolution through the module interface failed.",
        TextKey::ErrorTimeout => "The action timed out.",
        TextKey::ErrorUnsupported => "The current device, system or action is not supported.",
        TextKey::ErrorCapabilityUnavailable => {
            "A required system capability is not available right now."
        }
        TextKey::ErrorOperationCancelled => "The action was canceled.",
        TextKey::ErrorVerificationFailed => "The state check after the action did not pass.",
        TextKey::ErrorRollbackFailed => {
            "The state from before the action could not be restored; check the current \
             configuration."
        }
        TextKey::ErrorInternal => {
            "An internal error occurred. Refresh the state; if it persists, export the diagnostics."
        }
        TextKey::ErrorHelperUnsigned => {
            "The helper is unsigned, so privileged repairs stay off (development builds carry no \
             signature)."
        }
        TextKey::ErrorHelperUnverified => {
            "The signature could not be verified, so privileged repairs stay off."
        }
        TextKey::SimReady => "SIM ready",
        TextKey::SimMissing => "No SIM detected",
        TextKey::SimPinRequired => "SIM needs a PIN (this app never submits one)",
        TextKey::SimPukRequired => "SIM needs a PUK (this app never submits one)",
        TextKey::SimRejected => "SIM rejected",
        TextKey::SimUnknown => "SIM state unknown",
        TextKey::RegistrationHome => "Registered on the home network",
        TextKey::RegistrationRoaming => "Registered while roaming",
        TextKey::RegistrationSearching => "Searching for a network",
        TextKey::RegistrationDenied => "Network registration was denied",
        TextKey::RegistrationNotRegistered => "Not registered on a network yet",
        TextKey::RegistrationUnknown => "Registration state unknown",
        TextKey::AttachAttached => "Packet data attached",
        TextKey::AttachDetached => "Packet data not attached",
        TextKey::AttachUnknown => "Packet data attach state unknown",
        TextKey::CellularBlockSimRejected => "The SIM was explicitly rejected",
        TextKey::CellularBlockRegistrationRejected => {
            "Cellular registration was explicitly rejected"
        }
        TextKey::DevicePresenceSupported => {
            "A supported first-generation DJI 4G module was detected"
        }
        TextKey::DevicePresenceSupportedQuectelGeneric => {
            "A Quectel generic module was detected (VID 2C7C, PID 0125); read-only inspection only"
        }
        TextKey::DevicePresenceNotDetected => "No supported module detected",
        TextKey::DevicePresenceUnsupported => "A related but unsupported USB device was detected",
        TextKey::DevicePresencePermissionDenied => {
            "The device information could not be read: not enough permission"
        }
        TextKey::AdapterUsableAddressAndRoute => "The adapter has a usable address and route",
        TextKey::AdapterNoUsableAddressOrRoute => "The adapter has no usable address or route",
        TextKey::BoundPublicSucceeded => "The module-bound public-network probe passed",
        TextKey::BoundPublicFailed => {
            "The module-bound public-network probe failed ({count} times in a row)"
        }
        TextKey::BoundPublicIncomplete => "The public-network probe has not finished",
        TextKey::BoundDnsSucceeded => "Module-bound DNS resolution passed",
        TextKey::BoundDnsFailed => "Module-bound DNS resolution failed",
        TextKey::BoundDnsIncomplete => "The DNS probe has not finished",
        TextKey::ProtocolCoverageAllRequired => "Every required IP family passed verification",
        TextKey::ProtocolCoverageSingleFamily => "Only one required IP family passed verification",
        TextKey::AtControlAvailable => "AT control channel available",
        TextKey::AtControlUnavailable => "AT control channel unavailable",
        TextKey::DefaultRouteTargetAdapter => {
            "The system default route is provided by the module adapter"
        }
        TextKey::DefaultRouteVpnOrTun => {
            "The system default route is provided by a VPN or TUN interface"
        }
        TextKey::DefaultRouteOther => {
            "The system default route is provided by another network interface"
        }
        TextKey::GlobalConnectivityOnline => {
            "Windows can currently reach the internet over some network"
        }
        TextKey::GlobalConnectivityOffline => "Windows currently sees no global connectivity",
        TextKey::ProtocolApnEmpty => "The APN cannot be empty.",
        TextKey::ProtocolApnTooLong => "The APN cannot exceed 100 ASCII bytes.",
        TextKey::ProtocolApnUnsafeCharacter => {
            "The APN contains characters that are not allowed; use ASCII text without quotes, \
             commas, semicolons or control characters."
        }
        TextKey::ProtocolPdpContextIdOutOfRange => {
            "The PDP context number must be between 1 and 16."
        }
        TextKey::ProtocolWrongPortData => {
            "The serial port returned non-AT data, so it is no longer used."
        }
        TextKey::ProtocolLineTooLong => {
            "A data line from the module exceeded the safe length limit."
        }
        TextKey::ProtocolResponseTooLarge => "The module response exceeded the safe size limit.",
        TextKey::ProtocolTimeout => "Timed out waiting for the module to respond.",
        TextKey::ProtocolDeviceRemoved => "The device disconnected while a response was pending.",
        TextKey::ProtocolUnexpectedData => "The module returned data that cannot be parsed safely.",
        TextKey::AtFinalOk => "The module acknowledged the command",
        TextKey::AtFinalError => "The module rejected the command",
        TextKey::AtFinalCmeError => "The module returned a CME error ({detail})",
        TextKey::AtFinalCmsError => "The module returned a CMS error ({detail})",
        TextKey::AtFinalNoCarrier => "No carrier was established",
        TextKey::AtFinalNoAnswer => "The other end did not answer",
        TextKey::AtFinalBusy => "The module is busy",
        TextKey::AtFinalNoDialTone => "No dial tone was detected",
        TextKey::PlatformNoSafeAtPort => {
            "No AT port that can be used safely was found; unknown ports are never tried."
        }
        TextKey::PlatformAmbiguousAtPort => {
            "Several equally plausible AT ports were found; writing is disabled so the wrong one is \
             never used."
        }
        TextKey::PlatformAtPortUnverified => {
            "Several candidate ports were found and no safe handshake confirmed the AT protocol; \
             writing is disabled so the wrong one is never used."
        }
        TextKey::PlatformUnsupportedPlatform => {
            "This platform does not support Windows device enumeration."
        }
        TextKey::PlatformPnpEnumerateFailed => "Windows device enumeration could not complete.",
        TextKey::PlatformInterfaceEnumerateFailed => {
            "The device interfaces could not be enumerated."
        }
        TextKey::PlatformPnpPermissionDenied => "Windows refused to read the device information.",
        TextKey::PlatformPnpOpenFailed => "The Windows device information set could not be opened.",
        TextKey::SerialQueueFull => "The AT request queue is full; try again shortly.",
        TextKey::SerialSessionClosed => "The AT session is closed.",
        TextKey::SerialIoFailed => "Communication with the module's serial port failed.",
        TextKey::SerialAtFinalError => "The module did not accept that AT command.",
        TextKey::NavOverview => "Overview",
        TextKey::NavDiagnostics => "Diagnostics",
        TextKey::NavRepairs => "Repairs",
        TextKey::NavWireless => "Wireless",
        TextKey::NavSettings => "Settings",
        TextKey::DiagnosticsTitle => "Connection evidence",
        TextKey::DiagnosticsIntro => {
            "These checks are grouped by device, cellular network, Windows adapter and \
             module-bound probes."
        }
        TextKey::FieldDeviceIdentity => "Device identity",
        TextKey::FieldDeviceModel => "Device model",
        TextKey::FieldUsbIdentity => "USB identity",
        TextKey::FieldProblemCode => "Windows problem code",
        TextKey::FieldAtPort => "AT port",
        TextKey::FieldAdapter => "Module adapter",
        TextKey::FieldCarrier => "Carrier",
        TextKey::FieldRadioAccessTechnology => "Radio access technology",
        TextKey::FieldSignal => "Signal",
        TextKey::FieldSimState => "SIM state",
        TextKey::FieldRegistration => "Network registration",
        TextKey::FieldAttachState => "Packet data attach",
        TextKey::FieldApn => "Access point (APN)",
        TextKey::FieldPdpAddress => "PDP address",
        TextKey::FieldWindowsAddresses => "Windows addresses",
        TextKey::FieldGateway => "Gateway",
        TextKey::FieldDnsServers => "DNS servers",
        TextKey::FieldDefaultRoute => "System default route",
        TextKey::FieldBoundRouteProbe => "Module-bound route lookup",
        TextKey::FieldBoundPublicProbe => "Module public-network probe",
        TextKey::FieldBoundDnsProbe => "Module DNS probe",
        TextKey::FieldProtocolCoverage => "IP protocol coverage",
        TextKey::FieldGlobalConnectivity => "Windows global connectivity",
        TextKey::FieldHotspot => "Mobile hotspot",
        TextKey::FieldEvidenceSource => "Evidence source",
        TextKey::FieldObservedAt => "Checked at",
        TextKey::FieldPhoneNumber => "Phone number",
        TextKey::FieldNumberSource => "Source",
        TextKey::FieldVerificationState => "Verification",
        TextKey::FieldCaptureTime => "Captured at",
        TextKey::FieldIccid => "SIM ICCID",
        TextKey::ValueUnknown => "Unknown",
        TextKey::ValueNotAvailable => "Unavailable",
        TextKey::ValueRedacted => "Hidden",
        TextKey::ValueNotApplicable => "Not applicable",
        TextKey::ValueNumberNotProvided => "The SIM or device did not provide a phone number",
        TextKey::ValuePhoneNumberNotRead => "No phone number was read",
        TextKey::ValueNumberSourceSimReport => "Reported by the SIM or device",
        TextKey::ValueVerificationNotCarrierChecked => "Not verified against a carrier account",
        TextKey::ValueCaptureTimeSimSession => "Within this SIM session",
        TextKey::ValueIccidNotRead => "No ICCID was read",
        TextKey::IdentityHeading => "Identity",
        TextKey::ButtonShow => "Show",
        TextKey::ButtonCopy => "Copy",
        TextKey::ButtonCopied => "Copied",
        TextKey::ServingCellLayoutProvisional => {
            "The serving-cell field layout is provisional and awaits confirmation on real hardware"
        }
        TextKey::FeatureStatusUnsupportedConfirmed => {
            "The firmware does not support this query (the device returned an explicit unsupported \
             error)"
        }
        TextKey::FeatureStatusFormatMismatch => {
            "Format not recognized: the device answered, but the response shape was not recognized"
        }
        TextKey::FeatureStatusTransportFailure => {
            "Timed out this time: the query did not finish (no answer, or the device disconnected)"
        }
        TextKey::FeatureStatusTemporarilyUnavailable => {
            "Temporarily unavailable: this query did not succeed and the cause is not yet confirmed"
        }
        TextKey::CheckPassed => "Passed",
        TextKey::CheckFailed => "Failed",
        TextKey::CheckUnavailable => "Unavailable",
        TextKey::CheckUnexecuted => "Not run",
        TextKey::CheckRunning => "Running",
        TextKey::CheckExpired => "Expired",
        TextKey::UnexecutedDisabledBySetting => "Not run: turned off in settings",
        TextKey::UnexecutedNotScheduled => "Not run: not scheduled",
        TextKey::UnexecutedSuperseded => "Not run: replaced by a newer check",
        TextKey::AppTitle => "DJI 1st-gen 4G Panel",
        TextKey::UnofficialNotice => {
            "An unofficial open-source tool, not affiliated with or endorsed by DJI, Baiwang, \
             Quectel, Microsoft or any carrier."
        }
        TextKey::OverviewQuestion => {
            "Can this module serve as a usable network uplink for Windows right now?"
        }
        TextKey::OverviewLastObservation => "Last checked: {time}",
        TextKey::RateCaptionDown => "Down",
        TextKey::RateCaptionUp => "Up",
        TextKey::RateWindow => "Last {age}",
        TextKey::RatePeak => "Peak {detail}",
        TextKey::RateSampling => "Sampling throughput… (one point per second)",
        TextKey::RateGradeChip => "Speed: {detail}",
        TextKey::RateGradePending => "Not measured yet",
        TextKey::RateGradeIdle => "Idle",
        TextKey::RateGradeBasic => "Basic",
        TextKey::RateGradeGood => "Good",
        TextKey::RateGradeExcellent => "Excellent",
        TextKey::RateGradeVeryFast => "Very fast",
        TextKey::ButtonRefresh => "Refresh",
        TextKey::ButtonDiagnostics => "Diagnostics",
        TextKey::ButtonRepair => "Repairs",
        TextKey::ButtonConfirm => "Confirm",
        TextKey::ButtonCancel => "Cancel",
        TextKey::ButtonClose => "Close",
        TextKey::ButtonBack => "Back",
        TextKey::ButtonRetry => "Retry",
        TextKey::ButtonDone => "Done",
        TextKey::ButtonViewDiagnostics => "View diagnostics",
        TextKey::ButtonCopyAddress => "Copy address",
        TextKey::ButtonExportDiagnostics => "Export diagnostics",
        TextKey::ButtonOpenReleases => "Open the releases page",
        TextKey::StatusLoading => "Loading",
        TextKey::StatusNoActiveOperation => "No action is running",
        TextKey::StatusExpired => "State is stale",
        TextKey::StatusQueueFull => "The request queue is full; try again shortly",
        TextKey::CommandFeedbackBusy => "Another action is already running; try again shortly.",
        TextKey::CommandFeedbackConfirmRejected => {
            "The confirmation did not take effect: the plan expired. Prepare it again."
        }
        TextKey::CommandFeedbackRejected => "The action did not run; refresh and try again.",
        TextKey::StatusBackendUnavailable => {
            "The backend is not available right now; try again shortly"
        }
        TextKey::UnknownBackendError => {
            "An unrecognized error occurred. Refresh the state; if it persists, export the \
             diagnostics."
        }
        TextKey::SystemErrorNumber => "System error number: {detail}",
        TextKey::TrayOpen => "Open the panel",
        TextKey::TrayRefreshNow => "Refresh now",
        TextKey::TrayHotspotStatus => "Hotspot status",
        TextKey::TrayExit => "Exit",
        TextKey::TrayUnavailableFallback => {
            "The tray icon could not be created; the window will stay visible."
        }
        TextKey::CloseToTrayHint => "The window is hidden in the system tray.",
        TextKey::SettingsTitle => "Settings",
        TextKey::SettingsLanguage => "Interface language",
        // Endonyms: the same three strings in every catalog.
        TextKey::LanguageZhCn => "简体中文",
        TextKey::LanguageZhTw => "繁體中文",
        TextKey::LanguageEnUs => "English",
        TextKey::SettingsAutostart => "Start when Windows starts",
        TextKey::SettingsAutostartDescription => {
            "Off by default; when enabled, the panel starts straight into the system tray."
        }
        TextKey::SettingsAutostartDisabled => "Off",
        TextKey::SettingsStartMinimized => "Hide in the tray at startup",
        TextKey::SettingsActiveProbe => "Allow active connection probes",
        TextKey::SettingsActiveProbeDescription => {
            "Small, strictly bound connectivity and DNS checks over the module adapter."
        }
        TextKey::SettingsLogLevel => "Log detail",
        TextKey::SettingsLogLevelRestart => "The change takes effect on the next start.",
        TextKey::LogLevelError => "Errors only",
        TextKey::LogLevelWarn => "Warnings and above",
        TextKey::LogLevelInfo => "Normal",
        TextKey::LogLevelDebug => "Debug",
        TextKey::SettingsPrivacy => "Privacy",
        TextKey::SettingsPrivacyDescription => {
            "Diagnostics are de-identified by default and are never uploaded automatically."
        }
        TextKey::SettingsConfigDrift => {
            "The startup entry does not match the program's current location; enable autostart \
             again."
        }
        TextKey::SettingsSaved => "Settings saved",
        TextKey::SettingsSaveFailed => "The settings could not be saved.",
        TextKey::SettingsCorruptConfig => {
            "The configuration file is damaged; the original was kept and safe defaults were \
             restored."
        }
        TextKey::SettingsAutostartLoading => "Reading the startup setting.",
        TextKey::SettingsAutostartSaving => "Saving the startup setting.",
        TextKey::SettingsAutostartNotOwned => {
            "The startup entry does not match this program, so it was not removed."
        }
        TextKey::SettingsPathUnavailable => {
            "The settings folder could not be determined, so settings will not persist."
        }
        TextKey::SettingsReadFailed => {
            "The settings could not be read; safe defaults are in use and the original file was \
             left untouched."
        }
        TextKey::SingleInstanceActivationFailed => {
            "A panel is already running, but it could not be brought to the front."
        }
        TextKey::LoggingInitFailed => "Local logging could not be enabled; the app still runs.",
        TextKey::LoggingRotationFailed => "Log rotation failed; the app still runs.",
        TextKey::RepairsTitle => "Repair actions",
        TextKey::RepairsReadOnlyNotice => {
            "An action is enabled only while the target identity and the current evidence both hold."
        }
        TextKey::RepairsDriverNotIncluded => {
            "Driver installation needs its own confirmation; a working interface does not need \
             reinstalling."
        }
        TextKey::ConfirmationTitle => "Confirm this action",
        TextKey::ConfirmationDnsServers => "DNS servers",
        TextKey::ConfirmationNewApn => "New APN",
        TextKey::ConfirmationUsbConfigurationOnly => {
            "This only saves the USB network profile; restart the module by hand and re-check — \
             the adapter mode is not verified yet."
        }
        TextKey::ConfirmationOperation => "Action: {operation}",
        TextKey::ConfirmationTarget => "Target: DJI 1st-gen 4G module (VID 2CA3, PID 4006)",
        TextKey::ConfirmationExpectedEffect => "Expected effect",
        TextKey::ConfirmationInterruption => "Connection impact",
        TextKey::ConfirmationElevation => "Administrator approval",
        TextKey::ConfirmationRisk => "Risk level",
        TextKey::ConfirmationElevationRequired => {
            "This action needs Windows administrator approval."
        }
        TextKey::ConfirmationElevationNotRequired => {
            "This action does not need administrator approval."
        }
        TextKey::ConfirmationStateRecheck => {
            "The device identity and the current state are checked once more before it runs."
        }
        TextKey::ConfirmationNoAutomaticRetry => {
            "A write runs exactly once; a timeout is never retried automatically."
        }
        TextKey::ConfirmationApnContext => "PDP context: {cid}",
        TextKey::ConfirmationApnNewValue => "New APN: {apn_masked}",
        TextKey::OperationPreparing => "Preparing the action",
        TextKey::OperationRevalidating => "Re-checking the target state",
        TextKey::OperationAwaitingElevation => "Waiting for administrator approval",
        TextKey::OperationExecuting => "Running: {operation}",
        TextKey::OperationVerifying => "Re-checking and verifying the result",
        TextKey::OperationUacCancelled => "Administrator approval was cancelled, so nothing ran.",
        TextKey::OperationDeviceRemoved => "The device disconnected; the action stopped.",
        TextKey::OperationAuditRecorded => "The result was written to the local audit log.",
        TextKey::NoPreparedAction => "There is no action waiting for confirmation.",
        TextKey::PreparedActionAwaitingConfirmation => {
            "The action is ready and waiting for your confirmation."
        }
        TextKey::PlanExpired => "This action plan has expired; prepare it again.",
        TextKey::OperationResultTitle => "Result",
        TextKey::ConfirmationDevModeWarning => {
            "Development build: dji4g-helper.exe is unsigned and meant for testing only. Proceed \
             with care."
        }
        TextKey::DiagnosticsExportTitle => "Export diagnostics",
        TextKey::DiagnosticsExportDescription => {
            "Creates a readable report and a structured data file; sensitive identifiers are hidden \
             by default."
        }
        TextKey::DiagnosticsExportRedactionNotice => {
            "Full IMEI, IMSI, ICCID, phone numbers, PIN/PUK, raw serial data and the complete \
             configuration are never written to the export."
        }
        TextKey::DiagnosticsExportSuccess => {
            "Diagnostics were exported to %LOCALAPPDATA%\\Dji4GPanel\\exports."
        }
        TextKey::DiagnosticsExportFailed => "The diagnostics could not be exported.",
        TextKey::BuildDevelopmentUnsigned => "Development build (unsigned)",
        TextKey::BuildStableSigned => "Stable build (signed)",
        TextKey::FeatureUnavailablePortable => {
            "This feature is not available in the way the panel is currently running."
        }
        TextKey::UiCjkFontUnavailable => {
            "No usable Windows Chinese font was found; some interface text may not display fully."
        }
        TextKey::NoAutomaticUpdate => "This app never updates itself.",
        TextKey::DemoUsage => "Debug demo: available, limited, unavailable, absent or detecting",
        TextKey::DemoRejectedRelease => "Demo mode is not allowed in a release build.",
        TextKey::DemoInvalidScenario => {
            "Unknown demo scenario; use available, limited, unavailable, absent or detecting."
        }
        TextKey::NavSms => "Messages",
        TextKey::NavDeviceTools => "Device tools",
        TextKey::SmsTitle => "Messages",
        TextKey::SmsIntro => {
            "Turning messaging on for the first time switches the module's SMS format to PDU; \
             reading messages may mark unread ones as read."
        }
        TextKey::ButtonSmsRefresh => "Refresh messages",
        TextKey::FieldSmsStatus => "Status",
        TextKey::FieldSmsMessageCount => "Messages",
        TextKey::FieldSmsUnreadCount => "Unread",
        TextKey::FieldSmsCapacity => "Capacity",
        TextKey::SmsCapacityUsed => "Used {used} / {total} total",
        TextKey::SmsStatusNotQueried => "Not queried yet",
        TextKey::SmsStatusRead => "Read",
        TextKey::SmsIncompleteWarning => "Some long messages were not received in full",
        TextKey::SmsEmpty => "No messages (or nothing refreshed yet)",
        TextKey::SmsListPending => "Use \"Refresh messages\" to read the inbox.",
        TextKey::SmsUnread => "Unread",
        TextKey::SmsRead => "Read",
        TextKey::FieldSmsSender => "Sender",
        TextKey::FieldSmsTime => "Time",
        TextKey::FieldSmsEncoding => "Encoding",
        TextKey::FieldSmsParts => "Parts",
        TextKey::FieldSmsBody => "Body",
        TextKey::SmsEncodingOther => "Other",
        TextKey::SmsReadNote => "Reading may have marked it as read",
        TextKey::ButtonSmsDelete => "Delete",
        TextKey::ButtonSmsDeleteConfirm => "Confirm delete",
        TextKey::ButtonSmsSend => "Send message",
        TextKey::ButtonSmsSendConfirm => "Confirm send (may cost money)",
        TextKey::FieldSmsRecipient => "Recipient",
        TextKey::SmsEvictedWarning => {
            "The local cache is full; the {count} oldest messages were removed from the local view \
             (the module may still hold them)."
        }
        TextKey::SmsSendNotice => {
            "Sending may cost money, and a successful submission does not mean the recipient \
             received it. A failure or timeout is never retried automatically."
        }
        TextKey::SmsIncompleteTag => "Incomplete",
        TextKey::ButtonSmsExpand => "Expand",
        TextKey::ButtonSmsCollapse => "Collapse",
        TextKey::SmsInboxHeading => "Inbox",
        TextKey::SmsOutgoingSubmitted => "Submitted",
        TextKey::SmsOutgoingFailed => "Send failed",
        TextKey::SmsOutgoingUnknown => "Result unknown",
        TextKey::SmsBodyCharCount => "{count} / 70 chars",
        TextKey::SmsErrorPduModeRequired => {
            "Messages need PDU mode: use \"Refresh messages\" on the messages page to turn it on \
             (the module's SMS format changes once)."
        }
        TextKey::SmsErrorPduConfirmFailed => {
            "PDU mode could not be confirmed after switching; refresh the messages again."
        }
        TextKey::SmsErrorInvalidMessage => {
            "The body or the recipient is not acceptable (the recipient must be international \
             format starting with +, the body at most 140 bytes of BMP characters)."
        }
        TextKey::SmsErrorSendFailed => "The module rejected this submission.",
        TextKey::SmsErrorTimeout => {
            "The message action timed out: the result may be unknown, and nothing is retried \
             automatically."
        }
        TextKey::SmsErrorDeviceRemoved => "The device disconnected during the message action.",
        TextKey::SmsErrorUnsupported => "This firmware does not support the SMS AT commands.",
        TextKey::SmsErrorVerificationFailed => "The message response format was not recognized.",
        TextKey::SmsErrorInternal => "Internal message error; refresh and try again.",
        TextKey::SmsErrorSimRequired => {
            "SIM identity is missing, so nothing was sent; refresh first."
        }
        TextKey::SmsErrorSimUnverified => {
            "The current SIM could not be verified, so nothing was sent."
        }
        TextKey::SmsErrorSimChanged => {
            "The SIM changed, so nothing was sent; refresh and confirm again."
        }
        TextKey::SmsErrorGeneric => "The message action failed.",
        TextKey::FieldTemperature => "Temperature",
        TextKey::TemperatureNotRead => "Not read",
        TextKey::TemperatureSensorNote => "The sensor definition belongs to the firmware",
        TextKey::TemperatureSectionHeading => "Module temperature",
        TextKey::TemperatureTrendWindow => "Last {age}",
        TextKey::TemperatureTrendNote => {
            "The line is the first reported value · one sample per {age}"
        }
        TextKey::TemperatureTrendSampling => {
            "Sampling the module temperature… (one point per refresh cycle)"
        }
        TextKey::TemperatureDeltaUp => "+{detail} °C since last time",
        TextKey::TemperatureDeltaDown => "-{detail} °C since last time",
        TextKey::TemperatureDeltaFlat => "Same as last time",
        TextKey::TemperatureSensorsReported => {
            "The device reported {count} sensor values: {detail} °C · the order and meaning follow \
             the firmware"
        }
        TextKey::FieldAdapterErrors => "Interface errors",
        TextKey::FieldAdapterDiscards => "Interface discards",
        TextKey::FieldAdapterLinkRate => "Link rate",
        TextKey::AdapterRxTx => "Rx {rx} / Tx {tx}",
        TextKey::AdapterLinkRateNote => "Negotiated interface link rate, not measured throughput",
        TextKey::TimelineHeading => "Network changes",
        TextKey::TimelineEmpty => "No entries yet (only observed changes are recorded)",
        TextKey::TimelineSimChanged => "SIM replaced",
        TextKey::TimelineRegistrationChanged => "Network registration changed",
        TextKey::TimelineCellChanged => "Serving cell changed",
        TextKey::TimelineDeviceRemoved => "Device disconnected",
        TextKey::TimelineDeviceArrived => "Device re-enumerated",
        TextKey::TimelineAdapterLinkChanged => "Adapter link changed",
        TextKey::TimelineDnsChanged => "DNS probe changed",
        TextKey::NavGroupModule => "Module management",
        TextKey::EntrySkipHint => "You can go straight in and finish the checks later.",
        TextKey::EntryHiddenHint => "It will not appear again on its own; reopen it from Settings.",
        TextKey::AgeSeconds => "{count} s",
        TextKey::AgeMinutes => "{count} min",
        TextKey::AgeHours => "{count} h",
        TextKey::RateNow => "now",
        TextKey::RateSecondsAgo => "{count} s ago",
        TextKey::RateSamplePaused => "Sampling paused, waiting for a new reading",
        TextKey::RateNotSampled => "No rate sampled yet",
        TextKey::RateHoverAgo => "{age} s ago · measured",
        TextKey::RateHoverDown => "↓ Down  {detail}",
        TextKey::RateHoverUp => "↑ Up  {detail}",
        TextKey::RepairsIntro => "Check the cause first, then confirm the action to run",
        TextKey::RepairsAdapterModeHeading => "Computer adapter mode",
        TextKey::RepairsDjiGuideLink => "Open DJI's official user guide",
        TextKey::RepairsAdapterModeNote => {
            "Some first-generation modules work as a computer adapter on their factory firmware. Check the driver and the current network state first; if the network already works, nothing needs switching."
        }
        TextKey::RepairsUsbSwitchNote => {
            "The actions below only switch the USB network profile; no firmware is written. The DJI NDIS profile needs a matching Windows driver, and the ECM profile depends on the system and driver for compatibility."
        }
        TextKey::RepairsUsbOnlyNote => {
            "This only saves the USB profile; restart the module by hand and then verify the mode. A restart interrupts the connection."
        }
        TextKey::RepairsLowRiskHeading => "Low risk and network recovery",
        TextKey::RepairsInterruptsConnection => "Interrupts the connection",
        TextKey::RepairsViewPlan => "View the plan",
        TextKey::FieldPdpContext => "PDP context",
        TextKey::FieldNewApn => "New APN",

        TextKey::GuideProbeOff => {
            "Active connectivity checks are off, so public reachability and DNS are unverified; turn them on in Settings."
        }
        TextKey::GuideCollecting => {
            "Connection evidence is being collected; wait for this round to finish. There is no need to change network settings now."
        }
        TextKey::GuideEvidenceStale => {
            "The connection evidence is stale; refresh before judging. Old results do not describe the current connection."
        }
        TextKey::GuideCheckDisabled => {
            "This check is turned off; you can enable it in Settings. Leaving it unchecked does not mean the network failed."
        }
        TextKey::GuideCheckNotRun => {
            "This check has not finished; refresh first. An unrecognized device does not mean a driver is missing."
        }
        TextKey::GuideUsbFailed => {
            "The USB check did not pass; check the cable, the port and the device. An unrecognized device does not mean a driver is missing."
        }
        TextKey::GuideAdapterFailed => {
            "The module interface check did not pass; look at the specific serial-port or adapter reason. A working interface does not need its driver reinstalled."
        }
        TextKey::GuideCellularFailed => {
            "The SIM or cellular registration check did not pass; check the SIM state, the signal and carrier registration."
        }
        TextKey::GuideBoundProbeFailed => {
            "The module-bound public-reachability or DNS check did not pass; work from the specific failure. Switching USB modes repeatedly will not help."
        }
        TextKey::GuidePassed => {
            "The module's public-reachability and DNS checks passed. The system's actual route may still be decided by Wi-Fi or a VPN."
        }
        TextKey::GuideStartHeading => "Getting started",
        TextKey::GuideSteps => {
            "1. Connect the module → 2. Check the driver and serial port → 3. Check the SIM and network → 4. Get online or send messages"
        }
        TextKey::CheckUsbDetection => "USB detection",
        TextKey::CheckAdapterInterface => "Adapter interface",
        TextKey::CheckAtSerial => "AT serial port",
        TextKey::CheckSimCellular => "SIM and cellular",
        TextKey::CheckBoundPublic => "Module public reachability",
        TextKey::CheckBoundDns => "Module DNS",
        TextKey::GuidePassedCount => "Passed {count} checks: {detail}",
        TextKey::GuideStuckHint => {
            "Stuck in detection for a long time: go back to the overview and use “Export detailed log” under “More” in the top right to write a TXT, then “Open containing folder” and hand that file to whoever is helping. The log covers the device, driver and network and contains no message bodies."
        }
        TextKey::FirstCheckHeading => "First connection check",
        TextKey::FirstCheckIntro => {
            "With the module plugged in, the adapter and the AT link are checked separately. Being offline or having no address does not necessarily mean a missing driver."
        }
        TextKey::FirstCheckUsb => "1. USB device detection",
        TextKey::FirstCheckAdapter => "2. Windows adapter",
        TextKey::FirstCheckAt => "3. AT link (messages and module queries)",
        TextKey::DriverInstallHeading => "Driver installation",
        TextKey::DriverBundledNote => {
            "This offline build carries the original driver package and verifies its files and signature before installing. The package cannot cover every interface (including a MI_04 that does not match); if a driverless interface cannot be matched, installation stops before it starts. Windows may also update other devices that match the package, and a better driver is never force-replaced."
        }
        TextKey::DriverElevationNote => {
            "After you confirm, the panel exits and Windows asks for administrator approval. When the install finishes, or you cancel the approval, the panel returns and shows the result. If Windows asks for a restart, restart the computer first."
        }
        TextKey::DriverInstallAction => "Exit the panel and install the driver",
        TextKey::DriverNoneNote => {
            "This build carries no complete offline driver package. Open Windows Settings → Windows Update → Optional updates to look for a driver, or contact DJI support for a driver matched to this module; after installing, click “Refresh”."
        }
        TextKey::DriverDjiCompatibilityLink => "DJI's official compatibility note (item 21)",
        TextKey::DriverSeparateNote => {
            "The adapter and the AT serial port may need different drivers. After the install returns, the AT link and the network still have to be verified; a working setup does not need reinstalling. Windows Update is not guaranteed to carry every driver for this module."
        }
        TextKey::DriverDjiSupportLink => "Contact DJI support",
        TextKey::DriverVendorLink => "Quectel's official driver page",
        TextKey::DriverVendorLinkNote => {
            "That link leads to the vendor's own download page; it does not mean every driver there works with DJI's customized module."
        }
        TextKey::ValueNotReported => "Not reported",
        TextKey::WirelessSummary => {
            "Radio observation (module AT+QENG report)\n{}\nRAT {} / {}\nRSRP {}\nRSRQ {}\nRSSI {}\nSINR raw value {} (unit unconfirmed)\nUplink {} MHz / Downlink {} MHz\nTAC {}"
        }
        TextKey::WirelessIntro => {
            "See the module's radio measurements and cell changes, and control the Windows hotspot the module feeds."
        }
        TextKey::WirelessServingCell => "Current serving cell",
        TextKey::WirelessSampleFresh => "The latest AT query completed",
        TextKey::WirelessSampleWaiting => {
            "Waiting for a valid sample / existing data is for reference only"
        }
        TextKey::WirelessCopySummary => "Copy the radio summary",
        TextKey::WirelessNoCell => {
            "No serving-cell data yet. Once the module is connected, background monitoring samples it, and sending messages pauses it. Fields the module does not report stay empty."
        }
        TextKey::WirelessBand => "Band B{}",
        TextKey::WirelessRsrpNote => "Reference signal power",
        TextKey::WirelessRsrqNote => "Reference signal quality",
        TextKey::WirelessRssiNote => "Received signal strength",
        TextKey::WirelessSinrNote => "SINR · raw value",
        TextKey::WirelessSinrUnit => "unit not confirmed",
        TextKey::WirelessCellDetails => "Cell and bandwidth details",
        TextKey::WirelessBandwidthLine => "Uplink {} MHz  ·  Downlink {} MHz",
        TextKey::WirelessTacLine => "TAC {}  ·  Module state {}",
        TextKey::WirelessNoconnNote => {
            "NOCONN means idle after registration; it alone does not prove the network dropped. Signal numbers cannot replace a real throughput test."
        }
        TextKey::WirelessSignalHeading => "Signal history · RSRP",
        TextKey::WirelessSampleCount => "Last {} of 120 samples",
        TextKey::WirelessSignalNote => {
            "Shown in AT sampling order; a missing value breaks the line, and no point is invented while no new reply arrives. Recording restarts after a device or SIM change."
        }
        TextKey::WirelessChangeHeading => "Cell changes · {}",
        TextKey::WirelessChangeNote => {
            "Only observed cell-identity changes are recorded, and they are not read as a handover failure or a dropped line. The last 20 are kept."
        }
        TextKey::WirelessNoChange => "No cell change has been observed in this session yet.",
        TextKey::WirelessSecondsAgo => "{} s ago",
        TextKey::WirelessPreviousCell => "Previous cell: {}",
        TextKey::WirelessNewCell => "New cell: {}",
        TextKey::WirelessWaitingRsrp => "Waiting for a valid RSRP sample",
        TextKey::DeviceModelName => "DJI 1st-gen 4G module",
        TextKey::DeviceModelNameQuectelGeneric => "Quectel generic module",
        TextKey::ReadOnlyModuleReason => {
            "The generic module supports read-only inspection only; driver installation and controlled writes apply to the DJI 1st-gen module alone"
        }
        TextKey::SignalWithGrade => "{} dBm (speed: {})",
        TextKey::PdpActive => "Active",
        TextKey::PdpInactive => "Inactive",
        TextKey::ServingSearching => "Searching",
        TextKey::ServingLimitedService => "Limited service",
        TextKey::ServingNoCell => "No cell",
        TextKey::ServingNotCamped => "Not camped (searching)",
        TextKey::ServingCampedIdle => "Camped (idle)",
        TextKey::ServingSinr => "SINR {} (unit unconfirmed)",
        TextKey::ValuePreviewMore => "{} (plus {} more)",
        TextKey::OverviewSummaryLine => "Carrier {}  ·  Signal {}  ·  Temperature {}",
        TextKey::OverviewTabRate => "Rate",
        TextKey::OverviewDeviceHeading => "Device and SIM",
        TextKey::FieldModelShort => "Model",
        TextKey::FieldRegistrationShort => "Registration",
        TextKey::FieldServingCell => "Serving cell",
        TextKey::OverviewNetworkHeading => "Windows network",
        TextKey::FieldDefaultRouteShort => "Default route",
        TextKey::FieldIpAddresses => "IP addresses",
        TextKey::ValueMoreItems => "{count} more not expanded",
        TextKey::FieldFirmware => "Firmware",
        TextKey::FieldPdpState => "PDP state",
        TextKey::DiagNoProblemCode => "No problem code",
        TextKey::DiagProblemCode => "Problem code {}",
        TextKey::DiagPort => "Port {}",
        TextKey::DiagCarrierNotRead => "Carrier not read",
        TextKey::DiagRatNotRead => "RAT not read",
        TextKey::DiagRouteProbeNote => {
            "Only the matching route is looked up, which does not prove the gateway is reachable. Configured gateway: {}"
        }
        TextKey::DiagProxyInterfaceNote => {
            " · a VPN or proxy may use this interface normally; the module path is verified separately"
        }
        TextKey::DiagIncomplete => "Not finished",
        TextKey::DiagFailedRepeatedly => "Failed ({} times in a row)",
        TextKey::DiagResolutionFailed => "Resolution failed",
        TextKey::MNCQueued => "Queued; it will be checked once the current task finishes",
        TextKey::MNCChecking => "Checking the module network…",
        TextKey::MNCStale => "The result is stale or the device changed; check again",
        TextKey::MNCAvailable => {
            "Module network available: this round's public-reachability and DNS checks passed"
        }
        TextKey::MNCNoDevice => "No module recognized; check the cable and the USB port",
        TextKey::MNCAdapterFailed => {
            "The module adapter check did not pass; look at the specific evidence"
        }
        TextKey::MNCAdapterNoAddress => {
            "The adapter was recognized, but it has no usable address or route"
        }
        TextKey::MNCAdapterLinkDown => {
            "The module adapter link is down; check the USB connection and the device state"
        }
        TextKey::MNCBoundRouteFailed => {
            "The module-bound route lookup did not pass; check the address and route configuration"
        }
        TextKey::MNCBoundPublicFailed => {
            "The module public-reachability test did not pass; check the cellular and public-network evidence"
        }
        TextKey::MNCBoundDnsFailed => {
            "The module reaches the public network, but DNS resolution did not pass"
        }
        TextKey::MNCInconclusive => {
            "It is not yet possible to say whether the module can get online; look at the unfinished checks"
        }
        TextKey::MNCEvidenceUnexecuted => "Not run",
        TextKey::MNCEvidenceRunning => "Checking",
        TextKey::MNCEvidencePassed => "Passed",
        TextKey::MNCEvidenceFailed => "Failed",
        TextKey::MNCEvidenceUnavailable => "Unavailable",
        TextKey::MNCEvidenceExpired => "Expired",
        TextKey::MNCDhcpLease => "Renew the module adapter's DHCP lease",
        TextKey::MNCRestartAdapter => "Restart the module adapter",
        TextKey::MNCAutomaticDns => "Restore automatic DNS",
        TextKey::MNCHeading => "Module network check",
        TextKey::MNCRunAction => "Check the module network",
        TextKey::MNCRunningReason => "This round has not finished",
        TextKey::MNCOperationResult => "Result: {}",
        TextKey::MNCReadOnlyReverify => {
            "Below is the read-only re-check after the action; no repair is repeated automatically."
        }
        TextKey::MNCStepUsb => "Recognize the USB module",
        TextKey::MNCStepPorts => "Read the serial port, cellular state and module adapter",
        TextKey::MNCStepRoute => {
            "Look up the module-bound route and verify public reachability, DNS and the computer's egress"
        }
        TextKey::MNCStepCurrent => "Current step: {}",
        TextKey::MNCResultExpired => "The result has expired; check again",
        TextKey::MNCDeviceNoAdapter => {
            "The module was recognized, but its adapter interface is not verified yet. A driver, the USB network mode or a failed read could all be responsible."
        }
        TextKey::MNCViewSteps => "See the driver and interface check steps",
        TextKey::MNCSimHint => {
            "Check that the SIM works and look at the cellular registration and plan state. A single timeout does not mean the driver is broken."
        }
        TextKey::MNCNoPublicProbe => {
            "Public connectivity was not verified this time. Use the check and allow one network probe; the long-term setting stays unchanged."
        }
        TextKey::MNCEgressAdapter => "The module adapter is chosen",
        TextKey::MNCEgressProxy => "A proxy or VPN is chosen; that alone is not a fault",
        TextKey::MNCEgressOther => {
            "Another adapter (Wi-Fi or wired) is chosen; that alone is not a fault"
        }
        TextKey::MNCEgressUnknown => "The egress cannot be confirmed",
        TextKey::MNCEgressPath => "{} path to this test target: {}.",
        TextKey::MNCProbeUsb => "USB module",
        TextKey::MNCProbeAdapterRead => "Adapter read",
        TextKey::MNCProbeLink => "Link",
        TextKey::MNCProbeAddressRoute => "Address and route",
        TextKey::MNCProbeBoundRoute => "Module-bound route lookup",
        TextKey::MNCProbeBoundPublic => "Module public reachability",
        TextKey::MNCProbeBoundDns => "Module DNS",
        TextKey::MNCAdapterLinkLine => "Link: {}; IPv4 DHCP: {}",
        TextKey::MNCConnected => "Connected",
        TextKey::MNCDisconnected => "Not connected",
        TextKey::MNCEnabled => "on",
        TextKey::MNCDisabled => "off",
        TextKey::MNCProtocolLine => "Available protocols: IPv4 {} / IPv6 {}",
        TextKey::MNCStaticAddressNote => {
            "A non-DHCP configuration was found, so a static address is never overwritten. Check the existing network settings."
        }
        TextKey::MNCAtControl => "Serial communication",
        TextKey::MNCSimCellular => "SIM and cellular",
        TextKey::MNCBoundRouteNote => {
            "A bound route lookup only proves a matching route was found; it does not prove the gateway is reachable. The public probe is bound to the module adapter, and the computer path only represents this fixed test target rather than every application. Not run, unavailable and expired all mean something other than a fault."
        }
        TextKey::MNCEvidenceHeading => "This round's evidence and what to do",
        TextKey::MNCIntro => {
            "Checks the module adapter on its own and explains which egress the computer picks for the test target."
        }
        TextKey::MNCProbeConsent => {
            "Background network probing is off. This check sends a few requests to a built-in fixed endpoint (public reachability and DNS) and changes no long-term setting."
        }
        TextKey::MNCProbeCancel => "Cancel",
        TextKey::MNCLocalOnly => "Check local information only",
        TextKey::MNCAllowProbe => "Allow this network check",
        TextKey::MNCSubmitError => "The check was not submitted; try again shortly.",
        TextKey::ArchiveSaveFailed => {
            "The history switch could not be saved, so this change was not kept. Check write access to your user folder and try again."
        }
        TextKey::SmsViewModule => "Module messages",
        TextKey::SmsViewArchive => "Local history",
        TextKey::ArchiveExportName => "sms-history-{}.txt",
        TextKey::ArchiveNoExportDir => "Not exported: the user export folder is unavailable.",
        TextKey::OnboardingSaveFailed => {
            "The panel is open, but the onboarding-complete state could not be saved ({}); it may appear again on the next start."
        }
        TextKey::ArchiveBusyTitle => "Finishing a local message-history action",
        TextKey::ArchiveBusyBody => {
            "The panel exits by itself once the save, clear or export finishes. Please wait."
        }
        TextKey::WindowsUpdateFailedTitle => "Windows Update could not be opened",
        TextKey::WindowsUpdateFailedBody => {
            "Open “Windows Update” from Windows Settings and look for optional driver updates. No driver is installed yet."
        }
        TextKey::MenuButton => "Menu",
        TextKey::NavDialogTitle => "Navigation",
        TextKey::NavDialogClose => "Hide the menu",
        TextKey::OverviewPageIntro => "The module and the computer right now · read-only",
        TextKey::MoreMenu => "More",
        TextKey::ExportDetailedLog => "Export a detailed log",
        TextKey::ExportDetailedLogHint => {
            "Collects the USB, driver, serial, network and detection stages; it contains no message bodies."
        }
        TextKey::DiagnosticsPageTitle => "Diagnostics",
        TextKey::DiagnosticsPageIntro => {
            "Checks the module path and the computer network separately, and locates the problem from evidence"
        }
        TextKey::SettingsReopenOnboarding => "Show the first-run guide again",
        TextKey::HelpDialogTitle => "How to use",
        TextKey::AboutDialogTitle => "About",
        TextKey::RestartPanelTitle => "Restart the panel",
        TextKey::ExitApp => "Exit",
        TextKey::BusyDialogTitle => "Wait for the current task to finish",
        TextKey::RestartBusyBody => {
            "A message is being sent, a repair is running or the device tools are busy. Finish or cancel it before restarting so the task is not interrupted."
        }
        TextKey::RestartConfirmBody => {
            "The panel closes and starts again immediately.\nThe device is re-detected after the restart; an unsent draft is lost.\n\nRestart now?"
        }
        TextKey::RestartFailedTitle => "The panel could not restart",
        TextKey::RestartFailedBody => {
            "The panel is still running and did not restart.\n{}\nYou can exit and open the program again by hand."
        }
        TextKey::InstallBusyBody => {
            "A log is being exported, a message sent, or a repair run or confirmed. Finish it before installing the driver so the task is not interrupted."
        }
        TextKey::InstallDriverTitle => "Install the module driver",
        TextKey::InstallDriverBody => {
            "The panel exits by itself and Windows then asks for administrator approval; choose “Yes”.\nOnly a missing driver for matching hardware is installed, and a working interface is never force-reinstalled.\n\nWhen it finishes, or you cancel, the panel returns with normal rights and shows the result and the next step. If Windows asks for a restart, restart the computer first.\n\nContinue now?"
        }
        TextKey::InstallFailedTitle => "The installer could not start",
        TextKey::InstallFailedBody => {
            "The panel is still running and no driver was installed.\n{}\nExport a detailed log."
        }
        TextKey::ConfirmProbeNote => {
            "\n\nWhen it finishes, the panel re-checks read-only once and sends a few public-reachability and DNS requests to a fixed endpoint; the long-term probe setting is unchanged. A failure or an unknown result is never repaired again automatically."
        }
        TextKey::AboutWindowTitle => "About {}",
        TextKey::GitHubProject => "GitHub project repository",
        TextKey::UpdateAvailable => "New version v{} available",
        TextKey::UpdateTitle => "Software update",
        TextKey::UpdateCurrentVersion => "Current version v{}",
        TextKey::UpdateCheckNow => "Check for updates",
        TextKey::UpdateChecking => "Checking for updates…",
        TextKey::UpdateNotChecked => "Updates have not been checked yet.",
        TextKey::UpdateUpToDate => "No update is needed for this version.",
        TextKey::UpdateCheckFailed => "Could not check for updates. Try again later.",
        TextKey::UpdateView => "View update",
        TextKey::UpdateDownload => "Download update",
        TextKey::UpdateDownloading => "Downloading",
        TextKey::UpdateProgress => "Downloaded {} / {}",
        TextKey::UpdateVerifying => "Verifying update package",
        TextKey::UpdateReady => "Update verified and ready to install.",
        TextKey::UpdatePreparing => "Preparing update. Please wait.",
        TextKey::UpdateInstallRestart => "Install and restart",
        TextKey::UpdateRetry => "Download again",
        TextKey::UpdateBusy => "Wait for the current operation to finish.",
        TextKey::UpdateRestartHint => {
            "The app exits during installation and restarts when it is complete."
        }
        TextKey::UpdateFailedNetwork => "Update download failed. Check your connection and retry.",
        TextKey::UpdateFailedChecksum => {
            "A valid SHA-256 checksum is unavailable. Installation is blocked."
        }
        TextKey::UpdateFailedIntegrity => {
            "Update verification failed. The invalid temporary package was deleted. Installation is blocked."
        }
        TextKey::UpdateFailedUpdater => {
            "The updater could not start or prepare. The current app remains running."
        }
        TextKey::UpdateUnsupported => {
            "This directory cannot update in place. Use the official portable package."
        }
        TextKey::UpdateRecoveryNotice => {
            "The update did not complete. The old app has restarted and its original files are preserved."
        }
        TextKey::HostPreparingPlan => "Preparing the repair plan…",
        TextKey::HostBackingUp => "Backing up and repairing proxy configuration…",
        TextKey::HostRestoring => "Restoring the original configuration…",
        TextKey::HostChecking => "Checking computer network and proxy settings…",
        TextKey::HostConfigChanged => {
            "Proxy configuration changed. Restart the client, then check again."
        }
        TextKey::HostNotFinished => "Computer network check or repair did not finish",
        TextKey::HostNotChecked => "Computer network and proxy have not been checked.",
        TextKey::HostStale => "Computer network information is stale. Check again.",
        TextKey::HostModulePassed => "The module connection passed; ",
        TextKey::HostMissingAdapter => "the proxy client references a missing network adapter.",
        TextKey::HostAdapterDown => "The proxy-bound adapter exists but is down or disabled.",
        TextKey::HostOutletUnknown => "The proxy outlet could not be determined; see guidance.",
        TextKey::HostProxyInUse => {
            "A proxy or VPN interface is in use; the module path is checked separately."
        }
        TextKey::HostNoKnownProblem => "No known fixed-outlet adapter problem was found.",
        TextKey::HostHeading => "Computer network and proxy",
        TextKey::HostNoModuleHint => {
            "No module is connected. Computer and proxy checks remain available."
        }
        TextKey::HostCheckAgain => "Check again",
        TextKey::HostReviewRepair => "Review repair",
        TextKey::HostProxyOff => "Off",
        TextKey::HostProxyManual => "Manual",
        TextKey::HostProxyAutoScript => "Automatic script; script not executed",
        TextKey::HostProxyAutoDetect => "Auto-detect; not verified",
        TextKey::HostProxyMixed => "Multiple proxy settings; needs review",
        TextKey::HostProxyUnknown => "Unavailable",
        TextKey::HostWindowsProxy => "Windows system proxy",
        TextKey::HostConfiguredAdapter => "Configured adapter (runtime unverified)",
        TextKey::HostProxyBoundAdapter => "Proxy-bound adapter",
        TextKey::HostUnsupportedConfig => {
            "This configuration cannot be changed automatically. Review the outbound interface in the proxy client."
        }
        TextKey::HostConfigUnreadable => "Proxy configuration could not be read reliably",
        TextKey::HostRemoveOutletPrefix => "Remove the proxy client's fixed outlet for ",
        TextKey::HostRemoveOutletSuffix => {
            ". It may then use Wi-Fi, Ethernet or another available outlet. The original will be backed up. Exit Clash Verge Rev and its core first."
        }
        TextKey::HostPlanExpired => {
            "This repair plan expired. Check again before preparing a new plan."
        }
        TextKey::HostKeepOriginal => "Keep original settings",
        TextKey::HostWaitCurrentOperation => "Wait for the current operation to finish",
        TextKey::HostBackupAndRepair => "Back up and repair",
        TextKey::HostRestartedCheckAgain => {
            "Configuration changed. Restart the proxy and check again; this does not prove Internet access."
        }
        TextKey::HostRestoreOriginal => "Restore original configuration",
        TextKey::HostStepsHeading => "Troubleshooting steps",
        TextKey::HostStepsBody => {
            "Check the module and SIM. If the module path passes but the proxy reports a missing interface, review its outbound-interface setting. A fixed outlet may be intentional on computers with multiple adapters. Use the proxy client to edit ambiguous configurations."
        }
        TextKey::HostConfiguredAdapterRef => {
            "Disk configuration references adapter “{}”; the current runtime is unverified."
        }
        TextKey::HostCommandNotSubmitted => "The command was not submitted; try again shortly.",
        TextKey::HostVersionUnknown => "Version unconfirmed",
        TextKey::OnboardingWaiting => "Waiting for checks / device ready",
        TextKey::OnboardingChecking => "Checking…",
        TextKey::OnboardingNeedsReview => "Needs a look",
        TextKey::OnboardingDisabledUnverified => "Turned off, not verified",
        TextKey::OnboardingStaleRefresh => "The result is stale; refresh",
        TextKey::OnboardingCheckUsb => "USB device detection",
        TextKey::OnboardingCheckAdapter => "Windows adapter",
        TextKey::OnboardingCheckAt => "AT serial communication",
        TextKey::OnboardingCheckCellular => "SIM and cellular",
        TextKey::OnboardingCheckBoundPublic => "Module public reachability",
        TextKey::OnboardingCheckBoundDns => "Module DNS resolution",
        TextKey::OnboardingRestartStep => "2. Restart the computer first",
        TextKey::OnboardingEnterAfterRestart => "Open the panel (restart needed)",
        TextKey::OnboardingAutoCheckStep => "2. The module is checked automatically",
        TextKey::OnboardingStartUsing => "Start using it",
        TextKey::OnboardingEnterPanel => "Open the panel",
        TextKey::OnboardingPassedNote => {
            "The module-bound public-reachability and DNS checks passed; the computer's actual route may still be decided by Wi-Fi or a VPN."
        }
        TextKey::OnboardingNotPassedNote => {
            "Waiting, not connected, stale evidence or active probing turned off do not mean the driver is broken. Open the panel to see the specific reason."
        }
        TextKey::OnboardingTitle => "Connect your 4G module",
        TextKey::OnboardingIntro => {
            "Connect it, check it, then start using it. You can open the panel at any time."
        }
        TextKey::OnboardingSetupResultHeading => "This driver installation's result",
        TextKey::OnboardingStep1Title => "1 · Connect the module",
        TextKey::OnboardingStep1Body => {
            "Insert the SIM card and connect the module with a USB cable that supports data transfer. An existing driver can be used as it is."
        }
        TextKey::OnboardingHostHeading => "Computer network and proxy (optional check)",
        TextKey::OnboardingCheckHost => "Check the computer network",
        TextKey::OnboardingHostNote => {
            "This works without a module attached, and a proxy misconfiguration is never treated as a broken driver."
        }
        TextKey::OnboardingStep3Title => "3. Install a driver only if one is needed",
        TextKey::OnboardingBundledDriverNote => {
            "A local driver package was found; its signature and files are verified before installing. The package cannot cover every interface (including a MI_04 that does not match); if a driverless interface cannot be matched, installation stops before it starts. A working interface does not need reinstalling."
        }
        TextKey::OnboardingUseBundledDriver => "Use the bundled driver",
        TextKey::OnboardingBundledDriverHint => {
            "You confirm first, then the panel exits and Windows asks for approval. The panel returns by itself when the install finishes or you cancel the approval. If Windows asks for a restart, restart the computer first."
        }
        TextKey::OnboardingNoBundledDriverNote => {
            "This build carries no complete offline driver package, so it cannot be installed from here. Open Windows Settings → Windows Update → Optional updates to look for a driver, or contact DJI support for one matched to this module; after installing it as the vendor describes, click “Check again”."
        }
        TextKey::OnboardingOpenWindowsUpdate => "Open Windows Update",
        TextKey::OnboardingDjiCompatibilityLink => "DJI's official compatibility and driver notes",
        TextKey::OnboardingOfficialLinkNote => {
            "The official page offers compatibility notes and a support entry point; it is not a direct driver download. Nothing installs by itself, and Windows Update is not guaranteed to carry every driver for this module."
        }
        TextKey::ReportSectionSystem => "System and program processes",
        TextKey::ReportSectionUsb => "USB and problem devices",
        TextKey::ReportSectionDrivers => "Bound drivers and INF",
        TextKey::ReportSectionSerial => "Serial port enumeration",
        TextKey::ReportSectionAdapters => "Adapters and drivers",
        TextKey::ReportSectionNetwork => "IP, DNS and the default route",
        TextKey::ReportSectionSecurity => "Security software state",
        TextKey::ReportSectionDriverHistory => "Windows driver installation history",
        TextKey::ReportSectionIntegrity => "Program and driver file integrity",
        TextKey::ReportNoLogDirectory => {
            "Not exported: the current user's log folder is unavailable"
        }
        TextKey::ReportPreparing => "Preparing the detailed log…",
        TextKey::ReportStartFailed => "Export could not start: {error}",
        TextKey::ReportExporting => "Exporting the detailed log · {text}",
        TextKey::ReportExported => "Detailed log exported: {}",
        TextKey::ReportIncomplete => {
            "Export did not finish: {} (the partial report stays in the export folder)"
        }
        TextKey::ReportThreadEnded => {
            "The export thread ended early; the partial report stays in the export folder"
        }
        TextKey::ReportFileName => "dji4g-detailed-diagnostics-{}-{}.txt",
        TextKey::ReportHeader => {
            "DJI 4G SUPPORT REPORT schema=1\nREPORT_STARTED unix_ms={}\napp_version={} exe={} pid={} architecture={}\nIt contains device instance IDs, hardware IDs, drivers, network configuration and this program's logs. Send it only to whoever is troubleshooting.\nIt never reads message bodies, contacts, SIM numbers or passwords; it installs no driver, changes no network setting and sends no AT command.\nA section that fails or times out does not stop the others. REPORT_COMPLETE at the end marks the end of collection, not a healthy device.\n"
        }
        TextKey::ReportUiSummary => "Interface diagnostics summary",
        TextKey::ReportMachineSnapshot => "Machine-readable snapshot",
        TextKey::ReportSnapshotNote => "Detection phase, time, error codes and device binding",
        TextKey::ReportTimelineNote => {
            "Recent check state changes (only those this process observed)"
        }
        TextKey::ReportCollectLogs => "Collects program and driver installation logs",
        TextKey::ReportHistoryScope => "Installation-log history scope",
        TextKey::ReportHistoryCapped => "More than 12 folders; only the 12 newest are collected",
        TextKey::ReportHistoryHeading => "Installation-log history",
        TextKey::ReportLogDirectory => "Log folder",
        TextKey::ReportCollectionScope => "Log collection scope",
        TextKey::ExportTitle => "DJI 1st-gen 4G Panel · diagnostics export",
        TextKey::ExportGeneratedAt => "Generated (UTC): {}",
        TextKey::ExportPrivacy => "Privacy: {}",
        TextKey::ExportSectionOverview => "\n[Overview]\n",
        TextKey::ExportVerdict => "Current verdict: {}",
        TextKey::ExportEvidenceState => "Evidence state: {}",
        TextKey::ExportSectionDevice => "\n[Device]\n",
        TextKey::ExportDeviceId => "Container / device instance ID: {}",
        TextKey::ExportSectionCellular => "\n[Cellular]\n",
        TextKey::ExportSectionNetwork => "\n[Windows network]\n",
        TextKey::ExportSectionSms => "\n[Messages]\n",
        TextKey::ExportSendRequest => "Send request: {}; phase: {:?}; result: {:?}",
        TextKey::ExportSendError => {
            "Send error: {}; CMS: {:?}; CME: {:?}; system error: {:?}; possibly submitted: {}"
        }
        TextKey::ExportSectionEvidence => {
            "\n[Connection evidence]\nA module-bound route lookup only proves a matching route was found, not that the gateway is reachable.\n"
        }
        TextKey::ExportLabelWithCode => "{} (code {})",
        TextKey::ArchiveHeading => "Local history",
        TextKey::ArchiveIntro => {
            "When this is on, received messages that have been read are kept on this computer. They stay readable after the module is unplugged or the app restarts, and messages inside the module cannot be deleted from here."
        }
        TextKey::ArchiveRetention => {
            "Only the current Windows user can decrypt them; at most 5000 messages are kept, and anything older than 90 days is removed automatically. Turning saving off never deletes existing history."
        }
        TextKey::ArchiveUnavailable => {
            "Local history is unavailable right now: the user folder could not be determined, or the panel is in demo mode."
        }
        TextKey::ArchiveToggle => {
            "Keep message history on this computer (can be turned off at any time)"
        }
        TextKey::ArchiveExportTarget => "Export target: {}",
        TextKey::ArchiveSearchHint => "Search number or body",
        TextKey::ArchiveFilterAll => "All history",
        TextKey::ArchiveFilterWeek => "Saved in the last 7 days",
        TextKey::ArchiveFilterMonth => "Saved in the last 30 days",
        TextKey::ArchiveExportTxt => "Export TXT",
        TextKey::ArchiveClear => "Clear local history",
        TextKey::ArchiveCount => "Showing {} of {} · in the order they were saved",
        TextKey::ArchiveEmpty => {
            "No local history yet. Turn saving on and read messages under “Module messages”; anything already deleted from the module and never saved cannot be recovered."
        }
        TextKey::ArchiveNoMatch => "No record matches the filter.",
        TextKey::ArchiveNoTimestamp => "Send time not provided",
        TextKey::ArchiveIncompleteTag => " · parts incomplete",
        TextKey::ArchiveSourceGroup => "Source group: {} · historical copy",
        TextKey::ArchiveCopyBody => "Copy body",
        TextKey::ArchiveClearDescription => {
            "This deletes every message this computer has saved and turns further saving off. Messages inside the module are not affected, and the action cannot be undone."
        }
        TextKey::ArchiveClearConfirm => "Clear it",
        TextKey::ArchiveExportTitle => "Export messages as plain text",
        TextKey::ArchiveExportDescription => {
            "Every saved message (numbers and bodies included) is exported as an unencrypted TXT file. Keep it safe and check it for private content before sharing."
        }
        TextKey::ArchiveExportConfirm => "Export everything",
        TextKey::ArchiveCancel => "Cancel",
        TextKey::ArchiveReady => {
            "Local history is ready; only the last 90 days and at most 5000 messages are kept"
        }
        TextKey::ArchivePausedCapture => {
            "Reading or saving the archive failed, so collection is paused; check or clear the local history"
        }
        TextKey::ArchivePausedExport => {
            "Reading or saving the archive failed, so export is paused; deal with the archive error first"
        }
        TextKey::ArchiveExported => "A plain-text TXT was exported; keep that file safe",
        TextKey::ArchiveUpdated => {
            "Local history was updated; only the last 90 days and at most 5000 messages are kept"
        }
        TextKey::ArchiveWorkerFailed => "The message-archive worker could not be started",
        TextKey::ArchiveReading => "Reading the local history…",
        TextKey::ArchiveWorkerStopped => "The message-archive worker has stopped",
        TextKey::ArchiveDemoStatus => {
            "Simulated history for interface review only; no real message was read or saved"
        }
        TextKey::ArchiveDemoBody => {
            "[Simulated message] This is a local-history sample, used only to check reading, filtering and the export note."
        }
        TextKey::ArchiveNoIdentity => {
            "No stable device and SIM identity was obtained, so nothing was written to the local history"
        }
        TextKey::ArchiveBusy => "Local history is busy; try again shortly",
        TextKey::ArchiveOpenFailed => {
            "The local message archive could not be opened; the original file was kept"
        }
        TextKey::ArchiveSizeCheckFailed => "The archive size could not be checked",
        TextKey::ArchiveTooLarge => {
            "The local message archive exceeds 32 MiB, so it was neither loaded nor overwritten"
        }
        TextKey::ArchiveReadFailed => "Reading the message archive failed",
        TextKey::ArchiveInvalidFormat => {
            "The message archive format is invalid; the original file was kept"
        }
        TextKey::ArchiveDecryptedTooLarge => "The decrypted message archive exceeds the size limit",
        TextKey::ArchiveCorrupt => {
            "The message archive content is damaged; the original file was kept"
        }
        TextKey::ArchiveTooManyRecords => {
            "The message archive holds more records than allowed; the original file was kept"
        }
        TextKey::ArchiveEncodeFailed => "The message archive could not be encoded",
        TextKey::ArchiveSizeCapSave => {
            "The message archive reached its size limit, so nothing was saved this time"
        }
        TextKey::ArchiveSizeCapEncrypt => {
            "The encrypted archive reached its size limit, so nothing was saved this time"
        }
        TextKey::ArchiveClearFailed => {
            "The local message archive could not be cleared; existing records were not deleted"
        }
        TextKey::ToolStateNotQueried => "Not queried",
        TextKey::ToolStateAvailable => "Available",
        TextKey::ToolStateNoData => "No data",
        TextKey::ToolStateUnsupported => "Firmware unsupported",
        TextKey::ToolStateTemporarilyUnavailable => "Temporarily unavailable",
        TextKey::ToolStateFormatMismatch => "Format not recognized",
        TextKey::ToolStateTimeout => "Query timed out",
        TextKey::ToolResultOk => {
            "The module answered OK; whether the setting took effect still has to be confirmed separately"
        }
        TextKey::ToolResultRejected => "The module explicitly rejected this command",
        TextKey::ToolResultUnsupported => "The module says this command is not supported",
        TextKey::ToolResultNoAnswer => {
            "No usable answer arrived (timeout, serial error or disconnected)"
        }
        TextKey::ToolResultUnrecognized => {
            "The module answered, but the response format was not recognized"
        }
        TextKey::ToolResultCancelled => {
            "Commands that had not run were cancelled; see the terminal log for the ones that did"
        }
        TextKey::ToolResultMaybeWritten => {
            "It may have been written, but no final answer arrived; nothing is retried automatically"
        }
        TextKey::ToolResultInvalidated => "The device or SIM changed, so this result is void",
        TextKey::ToolInputEmpty => "Type an AT command",
        TextKey::ToolInputTooLong => "The command exceeds 256 characters",
        TextKey::ToolInputNotAscii => "The command may only contain ASCII characters",
        TextKey::ToolInputControlChars => {
            "The command may not contain control characters or line breaks"
        }
        TextKey::ToolInputSemicolon => "The command may not chain with semicolons",
        TextKey::ToolInputMustStartAt => "The command must start with AT",
        TextKey::ToolInputNotWhitelisted => "That command is not on the read-only whitelist",
        TextKey::ToolInputNeedsInteractive => {
            "This command family needs an interactive session, which a text terminal cannot drive safely"
        }
        TextKey::ToolPresetAttention => "Module response (AT)",
        TextKey::ToolPresetManufacturer => "Manufacturer (AT+CGMI)",
        TextKey::ToolPresetModel => "Model (AT+CGMM)",
        TextKey::ToolPresetFirmware => "Firmware version (AT+CGMR)",
        TextKey::ToolPresetSim => "SIM state (AT+CPIN?)",
        TextKey::ToolPresetSignal => "Signal quality (AT+CSQ)",
        TextKey::ToolPresetCarrier => "Carrier (AT+COPS?)",
        TextKey::ToolPresetRegistration => "Network registration (AT+CEREG?)",
        TextKey::ToolPresetAttach => "Packet attach (AT+CGATT?)",
        TextKey::ToolPresetPdpContexts => "PDP contexts (AT+CGDCONT?)",
        TextKey::ToolPresetPdpActive => "PDP activation (AT+CGACT?)",
        TextKey::ToolPresetPdpAddress => "PDP address (AT+CGPADDR)",
        TextKey::ToolPresetUsbMode => "USB network mode (AT+QCFG=\"usbnet\")",
        TextKey::ToolPresetTemperature => "Temperature (AT+QTEMP)",
        TextKey::ToolPresetServingCell => "Serving cell (AT+QENG=\"servingcell\")",
        TextKey::ToolPresetSmsFormat => "Message format (AT+CMGF?)",
        TextKey::ToolPresetSmsStorage => "Message storage (AT+CPMS?)",
        TextKey::ToolBatchPresets => "All preset queries (batch)",
        TextKey::ToolAdvancedAt => "AT commands (advanced)",
        TextKey::ToolTaskIdle => "Idle",
        TextKey::ToolTaskQueued => "Queued",
        TextKey::ToolTaskRunning => "Running",
        TextKey::ToolTaskCancelling => "Cancelling",
        TextKey::ToolTaskFinished => "Finished",
        TextKey::ToolElapsedMinutes => "{} min {} s",
        TextKey::ToolElapsedSeconds => "{} s",
        TextKey::ToolElapsedMillis => "{} ms",
        TextKey::ToolUsbModeDjiNdis => "DJI NDIS (computer adapter)",
        TextKey::ToolQueueFull => "The command queue is full; try again shortly",
        TextKey::ToolChannelClosed => "The backend connection is closed; try again shortly",
        TextKey::ToolApnContextRange => "The PDP context number must be between 1 and 16.",
        TextKey::ToolApnInvalid => {
            "The APN is not valid: it cannot be empty, exceed 100 bytes, or contain quotes, commas, semicolons or control characters."
        }
        TextKey::ToolControlledUnavailable => "That controlled action is not available right now.",
        TextKey::ToolTimeUnknown => "Time unknown",
        TextKey::ToolClockAgo => "{} ({} ago)",
        TextKey::ToolsTitle => "Device tools",
        TextKey::ToolsIntro => {
            "Read module information and run confirmed actions when you need them"
        }
        TextKey::ToolsDeviceConnected => "Device connected",
        TextKey::ToolsDeviceIdentity => "VID {} · PID {} · device id {}",
        TextKey::ToolsAtPort => "AT port {}",
        TextKey::ToolsDeviceEpoch => "Device generation {} · SIM session {}",
        TextKey::ToolsNoDevice => "No device detected",
        TextKey::ToolsNoDeviceHint => "Queries and controlled actions need a connected module",
        TextKey::ToolsSimSession => "SIM session {}",
        TextKey::ToolsTabPresets => "Presets",
        TextKey::ToolsTabReadOnly => "Read-only queries",
        TextKey::ToolsTabSwitchHint => {
            "You can switch tabs while a task runs, but write buttons stay disabled"
        }
        TextKey::ToolsTaskProgress => "Task progress",
        TextKey::ToolsBatchFinished => "The batch finished; see the per-item results below",
        TextKey::ToolsFinishedUnknown => "Finished with an unknown result",
        TextKey::ToolsNoTask => "No device-tools task is running",
        TextKey::ToolsCancelNote => {
            "Cancelling only stops the wait; it cannot undo changes already written to the module, and a write that times out is never retried automatically."
        }
        TextKey::ToolsLastRejected => "The last request was rejected: {} ({})",
        TextKey::ToolsProfileHeading => "Module profile",
        TextKey::ToolsRefreshProfile => "Refresh the module profile",
        TextKey::ToolsRefreshProfileHint => {
            "Runs every read-only preset query in order and writes nothing to the module"
        }
        TextKey::FieldManufacturer => "Manufacturer",
        TextKey::FieldModel => "Model",
        TextKey::FieldFirmwareVersion => "Firmware version",
        TextKey::FieldUsbNetworkMode => "USB network mode",
        TextKey::ToolNotRecognized => "Not recognized",
        TextKey::FieldCapturedAt => "Captured at",
        TextKey::ToolsUsbModeUnverified => {
            "The USB network mode the module reports is not one this version verified, so it is never switched automatically."
        }
        TextKey::ToolsNoProfileYet => {
            "The module profile has not been read yet; use “Refresh the module profile” to run one read-only query."
        }
        TextKey::ToolsEvidenceHeading => "Capability evidence",
        TextKey::ToolsEvidenceNote => "Each row reflects the result of one real query",
        TextKey::ToolsQuerying => "Querying this item",
        TextKey::ToolsQueryAgain => "Query this item again",
        TextKey::ToolsQueryItem => "Query this item",
        TextKey::ToolsQueryingKeepLast => {
            "Querying this item; the previous result and capture time stay below"
        }
        TextKey::ToolsReason => "Reason: {} ({})",
        TextKey::ToolsCaptured => "Captured: {} · device generation {} · SIM session {}",
        TextKey::ToolsStorageNote => {
            "A working storage query does not mean the module can send messages."
        }
        TextKey::ToolsNotRunYet => "This query has not run yet.",
        TextKey::ToolsStorageCaution => {
            "Note: a successful “message storage” query only means storage is readable; it does not mean the module can send messages."
        }
        TextKey::ToolsConnectionHeading => "Connection configuration",
        TextKey::ToolsNoPdpYet => {
            "No PDP context has been read yet; use “Refresh the module profile”."
        }
        TextKey::ToolsNoTemperature => "No temperature sensor has been read yet.",
        TextKey::ToolsSensor => "Sensor {}",
        TextKey::ToolsSensorNote => "The sensor definition follows the firmware.",
        TextKey::ToolsControlledHeading => "Controlled actions",
        TextKey::ToolsControlledNote => {
            "These writes use the repair page's controlled flow: after submitting you still confirm the target and the risk, and a timeout is never retried automatically."
        }
        TextKey::FieldPdpContextShort => "PDP context",
        TextKey::ToolsApnExample => "For example internet",
        TextKey::ToolsEditApn => "Change the APN",
        TextKey::ToolsApnConfirm => {
            "The confirmation dialog is open; it runs AT+CGDCONT={},\"IP\",\"{}\" once you confirm"
        }
        TextKey::ToolsApnRange => "The PDP context number must be an integer from 1 to 16.",
        TextKey::ToolsSwitchTo => "Switch to {}",
        TextKey::ToolsSwitchHint => {
            "Switched through the controlled repair flow; the module re-enumerates"
        }
        TextKey::ToolsConfirmRuns => "The confirmation dialog is open; it runs {} once you confirm",
        TextKey::ToolsCurrentUnrecognized => {
            "The current value is not recognized, so this version does not offer a switch."
        }
        TextKey::ToolsNotQueriedRefresh => "Not queried; refresh the module profile first.",
        TextKey::ToolsRestartModule => "Restart the module",
        TextKey::ToolsRestartCommand => "AT+CFUN=1,1; this interrupts the current connection",
        TextKey::ToolsRestartConfirm => {
            "The confirmation dialog is open; it runs AT+CFUN=1,1 once you confirm"
        }
        TextKey::ToolsRestartNote => "A restart interrupts the module connection for a moment.",
        TextKey::ToolsReadOnlyHeading => "Read-only AT queries",
        TextKey::ToolsReadOnlyNote => {
            "Queries on the read-only whitelist need no confirmation one by one"
        }
        TextKey::ToolsChoosePreset => "Choose a preset query",
        TextKey::ToolsRunQuery => "Run the query",
        TextKey::ToolsWhitelistHint => {
            "Or type a read-only command from the whitelist, such as AT+CSQ"
        }
        TextKey::ToolsNotInList => {
            "This command is not on the read-only list. If you know what it does, use AT commands (advanced) to check it and confirm it item by item; when in doubt, use a preset query."
        }
        TextKey::ToolsOpenAdvanced => "Open AT commands (advanced)",
        TextKey::ToolsBusyReadOnly => {
            "While a task runs you can only read existing results; the query buttons are disabled."
        }
        TextKey::ToolsInvalidInput => "Invalid input: {} ({})",
        TextKey::ToolsSessionUnlocked => "Unlocked for this session",
        TextKey::ToolsEnableAtInput => "Enable AT command input",
        TextKey::ToolsUnlockScope => {
            "The unlock lasts only for this session and relocks automatically when the device or SIM changes."
        }
        TextKey::ToolsAdvancedNote => {
            "For users who know AT commands. One checked command is sent at a time, and the full text is shown before you confirm. A command may change configuration or interrupt the connection."
        }
        TextKey::ToolsAtHint => "Type an AT command, for example AT+CSQ",
        TextKey::ToolsExecute => "Run",
        TextKey::ToolsClearInput => "Clear the input",
        TextKey::ToolsLocked => {
            "The terminal is locked: the input and the run button stay hidden until you unlock it."
        }
        TextKey::ToolsCheckFailed => "The command did not pass validation: {} ({})",
        TextKey::ToolsNormalizedWrite => {
            "Recognized as a controlled write; this normalized command will run: {}"
        }
        TextKey::ToolsCommandFrozen => {
            "The command is frozen; nothing is written to the module until you confirm below."
        }
        TextKey::ToolsAtPending => "AT command awaiting confirmation",
        TextKey::ToolsFrozenList => {
            "These commands are frozen and are written to the module only after you confirm:"
        }
        TextKey::ToolsUnknownEffect => {
            "The effect is unknown; it may change configuration or interrupt the connection."
        }
        TextKey::ToolsPlanExpired => "Expired; the same command has to be prepared again.",
        TextKey::ToolsPlanRemaining => {
            "Valid for another {} s; after that the command has to be prepared again."
        }
        TextKey::ToolsLogHeading => "Command log",
        TextKey::ToolsLogNote => "Recent task records, kept in this process's memory only",
        TextKey::ToolsCopySummary => "Copy the diagnostics summary",
        TextKey::ToolsCopySummaryNote => {
            "Contains only the action type, the duration and stable result codes — no response content"
        }
        TextKey::ToolsCopyRaw => "Copy the raw response",
        TextKey::ToolsCopyRawNote => {
            "The response may contain device identifiers, numbers or account information"
        }
        TextKey::ToolsClearLog => "Clear",
        TextKey::ToolsClearLogNote => {
            "Clears the records in memory; a running task can still add new ones when it finishes."
        }
        TextKey::ToolsShareCaution => {
            "“Copy the raw response” may contain device or account information; be careful where you paste it."
        }
        TextKey::ToolsNoLog => "No task records yet; run any query and its response appears here.",
        TextKey::ToolsEmptyResponse => "(no response content)",
        TextKey::ToolsResponseTruncated => "The response was too long and has been truncated",
        TextKey::ToolsHistoryHeader => "Device-tools history summary (no response content)",
        TextKey::ToolsTruncatedMark => "[response too long, truncated]",
        TextKey::ComposeBusyDraftKept => {
            "The current task or send confirmation has not finished; the draft was kept."
        }
        TextKey::ComposeUnsupportedSender => {
            "This sender is not a supported message number, so it cannot be replied to directly. The original number was left unchanged."
        }
        TextKey::ComposeDemoDraft => {
            "[Simulated data · interface review] This draft exists only for screenshots; do not actually send it."
        }
        TextKey::ComposeBackendBusy => {
            "The backend is busy, so this message was not submitted. Wait for the current task to finish and try again; the draft was kept."
        }
        TextKey::ComposeQueueFull => {
            "The operation queue is full, so nothing was submitted. Try again shortly; the draft was kept."
        }
        TextKey::ComposeChannelClosed => {
            "The backend connection is closed, so nothing was submitted. Restore the connection and try again; the draft was kept."
        }
        TextKey::ComposeQueued => "The message is queued; do not send it again",
        TextKey::ComposePreparing => "Preparing the message",
        TextKey::ComposeSubmitting => "Submitting the message",
        TextKey::ComposeAwaitingModule => "Waiting for the module to confirm",
        TextKey::ComposeSubmittedUnknown => {
            "It was submitted to the module; whether the other side received it cannot be confirmed yet."
        }
        TextKey::ComposeFailedDraftKept => "Sending failed; the draft was kept.",
        TextKey::ComposeUnknownMaybeSent => {
            "The result is unknown and it may have been submitted. Verify first to avoid sending twice; the draft was kept."
        }
        TextKey::ComposeFailureHeading => "Why it failed and what to do",
        TextKey::ComposeFailureStage => "Failed at: {} · error code: {}",
        TextKey::ComposeSystemError => "System error: {}",
        TextKey::ComposeTitle => "New message",
        TextKey::ComposeIntro => "Sent through the currently connected 4G module",
        TextKey::ComposeRecipient => "Recipient",
        TextKey::ComposeRecipientHint => "International number or service code",
        TextKey::ComposeRecipientNote => {
            "Enter a full number such as +8613800138000, or 10086 / 10010 / 10001."
        }
        TextKey::ComposeBodyLabel => "Message",
        TextKey::ComposeLength => "{} / 70 characters",
        TextKey::ComposeBodyHint => "Type the message here…",
        TextKey::ComposeLimits => "One message · at most 70 characters · emoji not supported",
        TextKey::ComposeCostNote => {
            "Carrier charges may apply; the next step checks the number and the text."
        }
        TextKey::ComposeSendingWait => "Sending; wait for the result",
        TextKey::ComposeModuleBusy => "The module is handling another task; please wait",
        TextKey::ComposeDraftKept => "The draft was kept",
        TextKey::ComposeNeedInput => "Fill in a valid number and content to continue",
        TextKey::ComposeDraftReady => "The draft is ready",
        TextKey::ComposeNextConfirm => "Next: confirm sending",
        TextKey::ComposeConfirmTitle => "Confirm sending",
        TextKey::ComposeConfirmIntro => "Check the full number and the text below:",
        TextKey::ComposeConfirmNote => {
            "This sends one message and may incur carrier charges. The module accepting it does not mean the other side received it."
        }
        TextKey::ComposeConfirmAction => "Send this message",
        TextKey::ComposeKeepDraftTitle => "Keep the current draft?",
        TextKey::ComposeKeepDraftBody => {
            "An unsent draft exists. Keep it, or replace it with the new draft and continue editing."
        }
        TextKey::ComposeReplaceDraft => "Use the new draft",
        TextKey::ComposeKeepDraft => "Keep the draft",
        TextKey::ComposeRejected => {
            "The module explicitly rejected this message and did not accept the submission. Check the error code and send it by hand afterwards."
        }
        TextKey::ComposeMaybeSent => "It may already have been submitted; do not simply resend it.",
        TextKey::ComposeNotSubmitted => {
            "Nothing was submitted; check the connection, the SIM and the messaging service, then try again."
        }
        TextKey::ComposeStageQueued => "Queued",
        TextKey::ComposeStagePreparing => "Preparing",
        TextKey::ComposeStageSubmitting => "Submitting",
        TextKey::ComposeStageAwaiting => "Waiting for the module",
        TextKey::ComposeStageDone => "Done",
        TextKey::ComposeSerialBusy => {
            "Another task is using the serial port. Wait for it to finish, then try again by hand."
        }
        TextKey::ComposeSerialOpenFailed => {
            "The messaging serial port could not be opened. Check the device connection and whether another program holds the port."
        }
        TextKey::ComposeSerialCloseTimeout => {
            "The serial port timed out while closing, so the backend could not confirm the resource was released. Restore the device connection and try again."
        }
        TextKey::ComposeNoDevice => {
            "No device is available right now. Connect the module and refresh the device state."
        }
        TextKey::ComposeContextChanged => {
            "The device or SIM changed while sending. Check the current device and the send history."
        }
        TextKey::ComposeModuleRejected => {
            "The module rejected the message. Check the SIM, the balance and the carrier's messaging service using the CMS/CME error codes."
        }
        TextKey::ComposeSerialFailed => {
            "Serial communication failed. Check the USB connection and the module's power."
        }
        TextKey::ComposeTimeout => {
            "Waiting for the module timed out. Check the send history and the connection to avoid sending twice."
        }
        TextKey::ComposeNoReference => {
            "The module returned no message reference, so the submission cannot be confirmed. Check whether it was sent first."
        }
        TextKey::ComposeUnexpectedEnd => {
            "The module returned an unexpected final response, so the submission cannot be confirmed. Keep the error code and check whether it was sent."
        }
        TextKey::ComposeSerialBusyShort => {
            "Another task is using the serial port; wait for it to finish."
        }
        TextKey::ComposePortBusyShort => {
            "The serial port could not be opened; check the device connection and whether the port is held."
        }
        TextKey::ComposeTimeoutShort => {
            "Waiting for the module timed out; check the connection and the module state."
        }
        TextKey::ComposeValidationFailed => {
            "The number or message is invalid. Check the supported number format and message length."
        }
        TextKey::ComposeDeviceLost => {
            "The device connection dropped; reconnect the device and refresh."
        }
        TextKey::ComposeGenericAdvice => {
            "Check the device connection, the SIM state and the carrier's messaging service; keep the error code for troubleshooting."
        }
        TextKey::ComposeCmeError => "CME: {}",
        TextKey::SetupFailedTitle => "Installation did not finish",
        TextKey::SetupNoPayload => {
            "This build carries no installation payload; use the complete installer package."
        }
        TextKey::SetupConfirmTitle => "Install DJI 4G Panel",
        TextKey::SetupConfirmBody => {
            "This installs the program and the offline driver payload for the current user and creates a desktop shortcut.\n\nThe installer itself changes no system driver; you can install a driver afterwards.\n\nContinue?"
        }
        TextKey::SetupVerifyFailed => "The installation files did not pass verification.",
        TextKey::SetupDoneTitle => "Installation complete",
        TextKey::SetupDoneBody => {
            "The program and the offline driver are installed, and a desktop shortcut was created.\n\nInstall the module driver now? It needs administrator approval.\nIf a driver already works, choose “No” and open the program directly."
        }
        TextKey::SetupDriverCancelled => {
            "The driver installation was cancelled. The program files are installed, but the module driver was not confirmed to work."
        }
        TextKey::SetupDriverIncomplete => {
            "The driver installation did not finish (exit code: {}). The program files are installed, but the panel does not open automatically.\n\nIf security software blocked it, keep the detection name and file path in the report. Do not turn protection off; hand the report to the developer."
        }
        TextKey::PortableBadResourcePath => "The installation resource path is invalid: {}",
        TextKey::PortableBadResourceDir => "The resource folder is invalid",
        TextKey::PortableWriteFailed => "Resource {} could not be written: {}",
        TextKey::PortableReadFailed => "Resource {} could not be read: {}",
        TextKey::PortableVerifyFailed => {
            "Resource verification failed: {}. Keep the security software's report and do not turn protection off."
        }
        TextKey::PortableLaunchFailed => "DJI 4G Panel could not start",
        TextKey::PortableNoPayload => "This build carries no standalone runtime payload.",
        TextKey::PortableNoAppData => {
            "The current user's application-data folder could not be read"
        }
        TextKey::PortableOpenFailed => {
            "The panel could not be opened: {}. If security software blocked it, keep the report."
        }
        TextKey::PortableExitedAbnormally => {
            "The panel exited abnormally: {}. Keep the security software's report and the program log."
        }
        TextKey::DriverResultTitle => "Module driver check result",
        TextKey::DriverResultBody => "{}\n\n{}\n\nClick “OK” to return to the panel.",
        TextKey::DriverNotStarted => "The installation has not started",
        TextKey::DriverPanelRunning => {
            "The panel has not fully exited, or the running panel could not be verified. Return to the original panel; close the panel from the tray and try the installation again. No driver was installed."
        }
        TextKey::DriverInstallTitle => "Install the module driver",
        TextKey::DriverInstallBody => {
            "The driver shipped with the program is verified, and only a package matching a driverless interface is selected. Windows may update other devices matched by the same package, and a better driver is never force-replaced.\n\nExit DJI 4G Panel first (including the tray). After you click “Yes”, Windows asks for administrator approval and the panel returns automatically when it finishes."
        }
        TextKey::DriverElevationFailed => "Administrator approval did not complete",
        TextKey::DriverOpenPanel => "Open the panel to continue",
        TextKey::DriverNoReturn => {
            "{}\n\nThe panel could not return automatically. Open “DJI 4G Panel” normally from the desktop and turn on the first-connection guide in Settings."
        }
        TextKey::DriverNoExePath => {
            "The program location could not be determined; open the complete program again."
        }
        TextKey::DriverNoExeDir => "The program folder could not be determined.",
        TextKey::DriverCheckComponentFailed => {
            "The Windows driver-check component could not be started; contact technical support."
        }
        TextKey::DriverClockInvalid => {
            "The system clock is wrong, so the installation did not start."
        }
        TextKey::DriverLogCreateFailed => {
            "The installation log could not be created, so the installation did not start. Move the complete program to a writable folder and try again."
        }
        TextKey::DriverLogWriteFailed => {
            "The log could not be written, so the installation did not start."
        }
        TextKey::DriverLogAppendFailed => {
            "Writing the installation log failed; check the device's actual state. Log location: {}"
        }
        TextKey::DriverLogPath => "Detailed installation log: {}",
        TextKey::DriverCheckNotStarted => "The Windows driver check did not start. {}",
        TextKey::DialogActionLine => "Action: {}",
        TextKey::DialogDisruptionLine => "Disruption: {}",
        TextKey::DialogRiskLine => "Risk: {}",
        TextKey::DialogElevationLine => "Elevation: {}",
        TextKey::DriverInstallResultTitle => "Module driver installation result",
        TextKey::SettingsGeneral => "General",
        TextKey::SettingsInterfaceTheme => "Interface theme",
        TextKey::SettingsStartupTray => "Startup and tray",
        TextKey::SettingsAutoStart => "Start automatically after sign-in",
        TextKey::SettingsStartHidden => "Start hidden in the tray",
        TextKey::SettingsLogging => "Logging",
        TextKey::SettingsAbout => "About",
        TextKey::SettingsVersion => "DJI 1st-gen 4G Panel · v{}",
        TextKey::CarrierChinaUnicom => "China Unicom",
        TextKey::CarrierChinaMobile => "China Mobile",
        TextKey::CarrierChinaTelecom => "China Telecom",
        TextKey::CarrierChinaBroadnet => "China Broadnet",
        TextKey::RateHeading => "Live rate",
        TextKey::RatePeakDownload => "Download peak",
        TextKey::RatePeakUpload => "Upload peak",
        TextKey::ThemeSystem => "Follow the system",
        TextKey::ThemeLight => "Light",
        TextKey::ThemeDark => "Dark",
        TextKey::SmsFragmentsRead => "Read {} of {} fragments",
        TextKey::SmsErrPortBusy => {
            "The previous operation on the serial port has not finished; refresh shortly"
        }
        TextKey::SmsErrPortAccess => "The serial port could not be accessed",
        TextKey::SmsErrNoAtPort => "No verifiable AT serial port for messaging was found",
        TextKey::SmsErrPduMode => "Message PDU mode is unconfirmed",
        TextKey::SmsErrResponseInvalid => "The module's response did not pass validation",
        TextKey::SmsErrTimeout => "The message query timed out",
        TextKey::SmsErrNoDevice => {
            "No verified device; check the module connection on the overview first"
        }
        TextKey::SmsErrDeviceGone => "The device is disconnected",
        TextKey::SmsErrSimUnknown => {
            "The SIM is not recognized yet; go back to the overview and refresh the connection"
        }
        TextKey::SmsErrSimChanged => {
            "The SIM changed, so these messages were not added to the list; go back to the overview and detect again"
        }
        TextKey::SmsErrSimIdentity => {
            "The SIM identity could not be confirmed, so nothing was read; check the SIM and detect again"
        }
        TextKey::SmsStopped => "Reading stopped; the existing list was kept",
        TextKey::SmsErrContextChanged => {
            "The module or SIM changed, so this result was discarded; detect again"
        }
        TextKey::SmsErrRestoreUnconfirmed => {
            "Restoring the original read location could not be confirmed; detect the module again first and do not send or delete messages for now"
        }
        TextKey::SmsErrUnsupportedLocation => {
            "The module cannot read this location; use the current storage location"
        }
        TextKey::SmsErrLimit => {
            "The message count or the returned content exceeded the safety limit, so the list was not updated"
        }
        TextKey::SmsErrQueryFailed => "The message query failed",
        TextKey::SmsSystemError => " · system error {}",
        TextKey::SmsPageTitle => "Messages",
        TextKey::SmsReadDetailsAttention => "Read details · needs attention",
        TextKey::SmsReadDetails => "Read details",
        TextKey::SmsRefreshList => "Refresh the list",
        TextKey::SmsBusyWait => "The current communication task has not finished; refresh shortly",
        TextKey::SmsPhaseWaiting => "Waiting to read",
        TextKey::SmsPhaseConfirming => "Confirming the module and SIM",
        TextKey::SmsPhaseQueryingStorage => "Querying the message storage location",
        TextKey::SmsPhaseSelecting => "Choosing the read location",
        TextKey::SmsPhaseReading => "Reading stored messages",
        TextKey::SmsPhaseOrganizing => "Organizing the messages",
        TextKey::SmsPhaseRestoring => "Restoring the original read location; please wait",
        TextKey::SmsPhaseReleasing => "Releasing the connection; please wait",
        TextKey::SmsProgressRecords => "{} · {} storage records read",
        TextKey::SmsStopReading => "Stop reading",
        TextKey::SmsStopRequested => {
            "Stop requested; waiting for the connection to be released. Automatic refresh is paused and you can refresh by hand to continue."
        }
        TextKey::SmsStorageUnconfirmed => "Unconfirmed",
        TextKey::SmsStorageSim => "SIM",
        TextKey::SmsStorageModule => "Module",
        TextKey::SmsStorageModuleArea => "The module's current summary area",
        TextKey::SmsStorageCurrentArea => "Current storage area",
        TextKey::SmsRecordsRead => "{} records read · storage location and read details",
        TextKey::SmsLastRead => "Last read: {} · {} message fragments, {} other records not shown",
        TextKey::SmsRefreshNote => {
            "Refreshing reads every message still stored in the current location (up to 1000 storage records). A long message may take several records; anything the module already deleted cannot be read back. Turn on “Local history” to keep messages read from now on."
        }
        TextKey::SmsOtherLocationsNote => {
            "Other locations may hold messages too. Choosing one asks for confirmation again; the read location is switched temporarily and restored afterwards, and it may mark unread messages as read."
        }
        TextKey::SmsReadSim => "Read SIM messages",
        TextKey::SmsReadModule => "Read module messages",
        TextKey::SmsTaskBusy => "The current communication task has not finished",
        TextKey::SmsLocationUnsupported => {
            "The module has not confirmed support for this storage location"
        }
        TextKey::SmsWillConfirm => "You will confirm again before reading",
        TextKey::SmsReadOtherLocation => "Read messages from another location",
        TextKey::SmsReadOtherBody => {
            "This reads the messages stored in {} and pauses other module operations while it runs. It may change read/unread state; the original read location is restored afterwards, and the write and receive locations are not changed."
        }
        TextKey::SmsConfirmRead => "Read them",
        TextKey::SmsSyncing => "Syncing the module's messages…",
        TextKey::SmsUnreadCount => "{} unread",
        TextKey::SmsStorageUsage => "Storage {} / {}",
        TextKey::SmsAutoSyncPaused => {
            "Automatic sync is paused · click Refresh the list to continue"
        }
        TextKey::SmsSyncProblem => "Message sync ran into a problem",
        TextKey::SmsSyncHistory => "Sync notices and history",
        TextKey::SmsCacheTrimmed => {
            "The local cache dropped {} older messages; the module storage may still hold them."
        }
        TextKey::SmsTabInbox => "Inbox  {}",
        TextKey::SmsTabOutgoing => "Sent  {}",
        TextKey::SmsBackToList => "Back to the message list",
        TextKey::SmsFooterNote => {
            "The module accepting a send ≠ the recipient receiving it  ·  message bodies are never written to the diagnostics log"
        }
        TextKey::SmsFilterAll => "All",
        TextKey::SmsFilterIncomplete => "Missing fragments",
        TextKey::SmsClearSearch => "Clear search",
        TextKey::SmsEmptyFilter => "No messages match this filter",
        TextKey::SmsEmptyFilterHint => "Select All to see other messages",
        TextKey::SmsBalanceQuery => "Check balance",
        TextKey::SmsBalanceIntro => "Choose the SIM carrier, then review and send the query draft.",
        TextKey::SmsBalanceInstructions => "Instructions",
        TextKey::SmsBalancePortal => "Open Broadnet account portal",
        TextKey::SmsBalanceBroadnetHelp => {
            "Sign in, then open Personal Center → My Account to check the balance."
        }
        TextKey::SmsBalanceReplyHint => {
            "Replies appear in the inbox. Commands and SMS availability depend on the local plan."
        }
        TextKey::SmsLastSynced => "Last complete read of {}: {} ago",
        TextKey::SmsSyncReadingCache => "Reading the module; showing existing records",
        TextKey::SmsSyncFailedCache => "Sync failed; showing cached records",
        TextKey::SmsSyncPartialCache => "Incomplete read; showing existing records",
        TextKey::SmsSyncPausedCache => "Sync paused; showing existing records",
        TextKey::SmsSyncStaleCache => "Older records; waiting to sync",
        TextKey::SmsSyncStoredList => "Stored messages · syncs every 15 seconds on this page",
        TextKey::SmsSyncNotCompleted => "No complete module-storage read yet",
        TextKey::SmsSearchHint => "Search number or message text",
        TextKey::SmsEmptySearch => "No matching message",
        TextKey::SmsEmptySearchHint => "Try another number or keyword",
        TextKey::SmsEmptyOutgoing => "No sent messages yet",
        TextKey::SmsEmptyOutgoingHint => "Use “New message” in the top right to write one",
        TextKey::SmsLoading => "Reading messages",
        TextKey::SmsLoadingHint => "Syncing with the module; please wait",
        TextKey::SmsInboxWaiting => "The inbox is waiting to sync",
        TextKey::SmsInboxWaitingHint => {
            "Stored messages are read automatically once the module is connected"
        }
        TextKey::SmsInboxEmpty => "No received messages",
        TextKey::SmsInboxEmptyHint => "The module currently has no messages to show",
        TextKey::SmsUnavailable => "Messages cannot be read right now",
        TextKey::SmsUnavailableHint => {
            "Look at the specific error above, check the connection and try again"
        }
        TextKey::SmsRefreshMessages => "Refresh messages",
        TextKey::SmsNoTimestamp => "Time not provided",
        TextKey::SmsReaderEmpty => "Message reader",
        TextKey::SmsReaderEmptyHint => "Pick a message on the left to read it in full here",
        TextKey::SmsKindIncoming => "Received message",
        TextKey::SmsKindOutgoing => "Sent message",
        TextKey::SmsNoTimestampFromModule => "The module provided no time",
        TextKey::SmsReply => "Reply",
        TextKey::SmsConfirmDeleteFragments => "Delete {} read fragments",
        TextKey::SmsDeleteFragments => "Delete {} read fragments",
        TextKey::SmsDeleteMessage => "Delete the message ({} fragments)",
        TextKey::SmsDeleteBusy => {
            "Wait for the communication task to finish and check the message fragments again"
        }
        TextKey::SmsDeleteConflict => {
            "The fragment identities conflict, so nothing can be deleted yet; read again and check them."
        }
        TextKey::SmsDeleting => "Deleting: {} of {} fragments confirmed",
        TextKey::SmsDeletedAll => "All {} read fragments are confirmed deleted",
        TextKey::SmsDeleteUnknown => {
            "The delete result is unknown: {} of {} confirmed, {} unconfirmed. Refresh and check; nothing is retried automatically."
        }
        TextKey::SmsDeletePartial => {
            "Partial delete: {} of {} fragments confirmed, the rest failed or did not run."
        }
        TextKey::SmsDeleteNone => {
            "No fragment was confirmed deleted; look at the reason and read again."
        }
        TextKey::SmsDeleteResultTitle => "Fragment delete result",
        TextKey::SmsDeleteConfirmed => "Confirmed deleted",
        TextKey::SmsDeleteFailed => "Failed",
        TextKey::SmsDeleteUnknownShort => "Result unknown",
        TextKey::SmsDeleteNotRun => "Not run",
        TextKey::ArchiveExportDirFailed => {
            "The export directory could not be created; choose a writable location"
        }
        TextKey::ArchiveExportCreateFailed => {
            "Export failed: choose a writable location and a new file name that does not exist yet"
        }
        TextKey::ArchiveExportWriteFailed => {
            "Writing the export failed; the incomplete file was not kept"
        }
        TextKey::ArchiveInvalidPath => "The message archive path is invalid",
        TextKey::ArchiveCreateDirFailed => "The message archive directory could not be created",
        TextKey::ArchiveTempFileFailed => {
            "The temporary file for the encrypted archive could not be created"
        }
        TextKey::ArchiveWriteFailed => "Writing the encrypted archive failed",
        TextKey::ArchiveReplaceFailed => {
            "Replacing the encrypted archive failed; the original file was kept"
        }
        TextKey::ExportSmsHeader => {
            "DJI 4G local message history (plain-text export)\nCaptured times are Unix UTC seconds; reported times keep the module's own text.\n"
        }
        TextKey::ExportSmsRecord => {
            "Device/SIM partition: {}\nSender: {}\nReported time: {}\nCaptured time: {}\nBody {}:\n{}\n--------"
        }
        TextKey::ExportSmsIncomplete => "(fragments incomplete)",
        TextKey::DriverOutcomeReady => {
            "The driver interface check in Windows has passed. The panel will re-check USB, the network adapter and AT communication; whether SMS and internet access work depends on the new check results."
        }
        TextKey::DriverOutcomeRestartRequired => {
            "Windows requires a restart, so the driver cannot be confirmed as usable yet. Save your work, restart the computer, then open the panel to check the module. Do not reinstall."
        }
        TextKey::DriverOutcomeRestartAfterFailure => {
            "Part of the driver installation failed and Windows requires a restart; the driver cannot be confirmed as usable yet. Keep the installation log, save your work, restart the computer, then open the panel to check. Do not reinstall; if it is still abnormal after the restart, give the log to technical support."
        }
        TextKey::DriverOutcomeCancelled => {
            "The driver installation or the Windows administrator authorization was cancelled, so nothing was installed. You can keep using the panel; if the installation is really needed, choose “Install the bundled driver” and select “Yes” in the Windows authorization prompt."
        }
        TextKey::DriverOutcomeNoMatch => {
            "The current driver package found no single match for one interface that is missing a driver (for example MI_04), so no driver was installed. Look for a suitable driver through Windows Update first, or ask DJI official support for a driver that matches this module. Reinstalling this package cannot fill that interface."
        }
        TextKey::DriverOutcomeInterfacesAbnormal => {
            "Some interfaces are still abnormal after the driver check, so availability cannot be confirmed yet. Read the panel's new check results; if they are still abnormal, open Device Manager to see why and give the installation log to technical support. Do not install repeatedly."
        }
        TextKey::DriverOutcomeNoModule => {
            "No connected DJI first-generation module was detected, or the module is temporarily disconnected after the installation. Plug in a USB cable that supports data transfer, wait for the module to be recognized, then choose “Re-check”."
        }
        TextKey::DriverOutcomePayloadInvalid => {
            "The installation payload is missing, failed verification, or is incompatible with this version of Windows, so nothing was installed. Obtain the complete original driver release again, or contact DJI official support; do not modify the driver files."
        }
        TextKey::DriverOutcomeIncomplete => {
            "The installation did not finish, so the device state cannot be confirmed. Read this installation's log and re-check in the panel; if you need help, give the log to technical support."
        }
        TextKey::DriverWindowsUpdateFailed => {
            "Windows Update could not be opened; open it from the system settings."
        }
        TextKey::DriverAdminRequired => {
            "The installer is running with administrator rights; open the panel normally from the desktop."
        }
        TextKey::DriverPanelNotSameDirectory => {
            "The process being waited for is not the panel in the same directory; no driver was installed."
        }
        TextKey::DriverPanelExitTimeout => {
            "The panel did not exit within 30 seconds, so no driver was installed. Exit the panel from the tray and try again."
        }
        TextKey::TimelineCellChangedDetail => "The serving cell changed",
        TextKey::TimelineAdapterLinkChangedDetail => "The adapter link state changed",
        TextKey::TimelineRegistrationChangedDetail => "Registration: {} → {}",
        TextKey::TimelineDnsChangedDetail => "DNS probe: {} → {}",
        TextKey::TimelineRegistrationHomeDetail => "Registered on the home network",
        TextKey::TimelineRegistrationRoamingDetail => "Registered while roaming",
        TextKey::TimelineRegistrationSearchingDetail => "Searching",
        TextKey::TimelineRegistrationDeniedDetail => "Registration denied",
        TextKey::TimelineRegistrationNotRegisteredDetail => "Not registered",
        TextKey::TimelineRegistrationUnknownDetail => "Unknown",
        TextKey::TimelineDnsPassedDetail => "passed",
        TextKey::TimelineDnsFailedDetail => "failed",
        TextKey::TimelineDnsIncompleteDetail => "incomplete",
        TextKey::ArchiveCryptoUnsupported => {
            "This system does not support Windows user encryption; nothing was written to the SMS archive."
        }
        TextKey::ArchiveCryptoTooLarge => "The SMS archive exceeds the encryption size limit.",
        TextKey::ArchiveCryptoFailed => {
            "Windows user encryption failed; nothing was written to the SMS archive."
        }
        TextKey::ArchiveCryptoDecryptFailed => {
            "The local SMS archive could not be decrypted: a different user account, or a damaged file. The original file was kept."
        }
        TextKey::ToolUrcLine => "[module-initiated report] {}",
        TextKey::SmsFailureWithCode => "{} ({})",
        TextKey::ComposeCmsError => "CMS: {}",
        TextKey::SmsDeleteMessageOne => "Delete the message ({} fragment)",
    }
    // EN-CATALOG-END
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
    // APNs and arbitrary backend details are never displayed raw: a caller that sets the masked
    // flag gets the catalog's own 「已隐藏」 in place of the value, in the caller's language.
    replace(
        &mut output,
        "{apn_masked}",
        args.apn_masked
            .as_ref()
            .map(|_| template(language, TextKey::ValueRedacted).to_owned()),
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
        // The read-only limitation is the same fact the refusal reason states, so it keeps the same
        // wording instead of inventing a second sentence for it.
        LimitedReason::ReadOnlyModule => TextKey::ReadOnlyModuleReason,
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
        "archive:ready" => TextKey::ArchiveReady,
        "archive:paused_capture" => TextKey::ArchivePausedCapture,
        "archive:paused_export" => TextKey::ArchivePausedExport,
        "archive:exported" => TextKey::ArchiveExported,
        "archive:updated" => TextKey::ArchiveUpdated,
        "archive:worker_failed" => TextKey::ArchiveWorkerFailed,
        "archive:reading" => TextKey::ArchiveReading,
        "archive:worker_stopped" => TextKey::ArchiveWorkerStopped,
        "archive:demo_status" => TextKey::ArchiveDemoStatus,
        "archive:demo_body" => TextKey::ArchiveDemoBody,
        "archive:no_identity" => TextKey::ArchiveNoIdentity,
        "archive:busy" => TextKey::ArchiveBusy,
        "archive:open_failed" => TextKey::ArchiveOpenFailed,
        "archive:size_check_failed" => TextKey::ArchiveSizeCheckFailed,
        "archive:too_large" => TextKey::ArchiveTooLarge,
        "archive:read_failed" => TextKey::ArchiveReadFailed,
        "archive:invalid_format" => TextKey::ArchiveInvalidFormat,
        "archive:decrypted_too_large" => TextKey::ArchiveDecryptedTooLarge,
        "archive:corrupt" => TextKey::ArchiveCorrupt,
        "archive:too_many_records" => TextKey::ArchiveTooManyRecords,
        "archive:encode_failed" => TextKey::ArchiveEncodeFailed,
        "archive:size_cap_save" => TextKey::ArchiveSizeCapSave,
        "archive:size_cap_encrypt" => TextKey::ArchiveSizeCapEncrypt,
        "archive:clear_failed" => TextKey::ArchiveClearFailed,
        "archive:export_dir_failed" => TextKey::ArchiveExportDirFailed,
        "archive:export_create_failed" => TextKey::ArchiveExportCreateFailed,
        "archive:export_write_failed" => TextKey::ArchiveExportWriteFailed,
        "archive:invalid_path" => TextKey::ArchiveInvalidPath,
        "archive:create_dir_failed" => TextKey::ArchiveCreateDirFailed,
        "archive:temp_file_failed" => TextKey::ArchiveTempFileFailed,
        "archive:write_failed" => TextKey::ArchiveWriteFailed,
        "archive:replace_failed" => TextKey::ArchiveReplaceFailed,
        "archive:crypto_unsupported" => TextKey::ArchiveCryptoUnsupported,
        "archive:crypto_too_large" => TextKey::ArchiveCryptoTooLarge,
        "archive:crypto_failed" => TextKey::ArchiveCryptoFailed,
        "archive:crypto_decrypt_failed" => TextKey::ArchiveCryptoDecryptFailed,

        // The driver-setup helper and the installers below the panel emit `driver:` codes for
        // every closed result and refusal; the panel owns all of their wording.
        "driver:ready" => TextKey::DriverOutcomeReady,
        "driver:restart_required" => TextKey::DriverOutcomeRestartRequired,
        "driver:restart_after_failure" => TextKey::DriverOutcomeRestartAfterFailure,
        "driver:cancelled" => TextKey::DriverOutcomeCancelled,
        "driver:no_match" => TextKey::DriverOutcomeNoMatch,
        "driver:interfaces_abnormal" => TextKey::DriverOutcomeInterfacesAbnormal,
        "driver:no_module" => TextKey::DriverOutcomeNoModule,
        "driver:payload_invalid" => TextKey::DriverOutcomePayloadInvalid,
        "driver:incomplete" => TextKey::DriverOutcomeIncomplete,
        "driver:windows_update_failed" => TextKey::DriverWindowsUpdateFailed,
        "driver:admin_required" => TextKey::DriverAdminRequired,
        "driver:panel_not_same_directory" => TextKey::DriverPanelNotSameDirectory,
        "driver:panel_exit_timeout" => TextKey::DriverPanelExitTimeout,

        "apn:empty" => TextKey::ProtocolApnEmpty,
        "apn:too_long" => TextKey::ProtocolApnTooLong,
        "apn:unsafe_character" => TextKey::ProtocolApnUnsafeCharacter,
        "pdp_context_id:out_of_range" => TextKey::ProtocolPdpContextIdOutOfRange,
        "repair:dhcp_disabled" => TextKey::RepairDhcpDisabled,
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
        "app:busy" | "host:unavailable_or_busy" | "tool:busy" | "sms:busy" => {
            TextKey::CommandFeedbackBusy
        }
        "app:confirm_rejected" => TextKey::CommandFeedbackConfirmRejected,
        "app:read_only_module" | "sms:read_only_module" => TextKey::ReadOnlyModuleReason,
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
        "sms:sim_identity_required" => TextKey::SmsErrorSimRequired,
        "sms:sim_identity_unverified" => TextKey::SmsErrorSimUnverified,
        "sms:sim_changed" => TextKey::SmsErrorSimChanged,
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

/// Display text for one stable ASCII code produced by a crate that cannot depend on this catalog
/// (the `driver:` results and refusals, the `archive:` status and crypto codes).
///
/// A code nothing maps is shown as-is: a backend code that gains no wording yet must never be
/// mislabelled with a neighbouring feature's sentence.
#[must_use]
pub fn stable_code_display(language: Language, code: &str) -> String {
    stable_code_text(code).map_or_else(
        || code.to_owned(),
        |key| LocalizedText::new(language, key).text,
    )
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
        ActionKindTag::ApplyDnsProfile => TextKey::ActionApplyDnsProfile,
        ActionKindTag::RestartAdapter => TextKey::ActionRestartAdapter,
        ActionKindTag::ReenumerateDevice => TextKey::ActionReenumerateDevice,
        ActionKindTag::RestartModule => TextKey::ActionRestartModule,
        ActionKindTag::EditApn { .. } => TextKey::ActionEditApn,
        ActionKindTag::SetVerifiedUsbNetworkProfile => TextKey::ActionSetUsbNetworkProfile,
        ActionKindTag::ToggleHotspot { enabled: true } => TextKey::ActionEnableHotspot,
        ActionKindTag::ToggleHotspot { enabled: false } => TextKey::ActionDisableHotspot,
    }
}

/// A snapshot tag identifies an operation class, but cannot recover its DNS servers or USB
/// profile. Use neutral class labels, and format the CID that an APN tag does retain.
#[must_use]
pub fn action_tag_text(value: ActionKindTag, language: Language) -> LocalizedText {
    match value {
        ActionKindTag::EditApn { cid } => {
            format_text_in(language, TextKey::ActionEditApn, &TextArgs::cid(cid))
        }
        _ => LocalizedText::new(language, action_tag_key(value)),
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
        DiagnosticCheckId::BoundRoute => TextKey::FieldBoundRouteProbe,
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
        DevicePresence::Supported(profile) => device_model_presence(*profile),
        DevicePresence::NotDetected => TextKey::DevicePresenceNotDetected,
        DevicePresence::Unsupported { .. } => TextKey::DevicePresenceUnsupported,
        DevicePresence::PermissionDenied => TextKey::DevicePresencePermissionDenied,
    }
}

/// The 型号 name of one matched module profile, for the overview's model row.
#[must_use]
pub fn device_model_name(profile: DeviceProfile) -> TextKey {
    if profile == dji4g_domain::QUECTEL_GENERIC {
        TextKey::DeviceModelNameQuectelGeneric
    } else {
        TextKey::DeviceModelName
    }
}

/// The presence line for one matched module profile; only a write-capable profile may claim the
/// full-support wording.
fn device_model_presence(profile: DeviceProfile) -> TextKey {
    if profile.allows_controlled_actions() {
        TextKey::DevicePresenceSupported
    } else {
        TextKey::DevicePresenceSupportedQuectelGeneric
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

/// Localized label for one observed timeline transition, the category label of its kind.
///
/// The row text itself comes from [`timeline_detail_text`]; this is the phrase for a transition
/// that carries no values, and the key a caller keys the row on.
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

/// The catalog key of one timeline row.
///
/// Three kinds have a phrase of their own for the row: 「服务小区已变化」 says something the
/// category label 「服务小区变化」 does not, and the registration and DNS rows carry the values of
/// the transition. Every other kind reads as its category label.
#[must_use]
fn timeline_detail_key(value: TimelineEventKind) -> TextKey {
    match value {
        TimelineEventKind::CellChanged => TextKey::TimelineCellChangedDetail,
        TimelineEventKind::AdapterLinkChanged => TextKey::TimelineAdapterLinkChangedDetail,
        TimelineEventKind::RegistrationChanged => TextKey::TimelineRegistrationChangedDetail,
        TimelineEventKind::DnsChanged => TextKey::TimelineDnsChangedDetail,
        other => timeline_kind_text(other),
    }
}

/// One closed registration state as the short form a timeline row uses.
#[must_use]
fn registration_detail_label(language: Language, value: RegistrationState) -> String {
    let key = match value {
        RegistrationState::RegisteredHome => TextKey::TimelineRegistrationHomeDetail,
        RegistrationState::RegisteredRoaming => TextKey::TimelineRegistrationRoamingDetail,
        RegistrationState::Searching => TextKey::TimelineRegistrationSearchingDetail,
        RegistrationState::Denied => TextKey::TimelineRegistrationDeniedDetail,
        RegistrationState::NotRegistered => TextKey::TimelineRegistrationNotRegisteredDetail,
        RegistrationState::Unknown => TextKey::TimelineRegistrationUnknownDetail,
    };
    template(language, key).to_owned()
}

/// One bound-DNS verdict as the short form a timeline row uses.
#[must_use]
fn dns_detail_label(language: Language, value: BoundDnsStatus) -> String {
    let key = match value {
        BoundDnsStatus::Succeeded => TextKey::TimelineDnsPassedDetail,
        BoundDnsStatus::Failed => TextKey::TimelineDnsFailedDetail,
        BoundDnsStatus::Incomplete => TextKey::TimelineDnsIncompleteDetail,
    };
    template(language, key).to_owned()
}

/// The display text of one timeline row.
///
/// The reducer records closed values, never prose; the row's wording is chosen here, in the
/// language the page is rendered with, with the transition's values filled into the `{}` slots in
/// order.
#[must_use]
pub fn timeline_detail_text(
    language: Language,
    kind: TimelineEventKind,
    detail: TimelineDetail,
) -> LocalizedText {
    let key = timeline_detail_key(kind);
    let text = match detail {
        TimelineDetail::Kind => template(language, key).to_owned(),
        TimelineDetail::Registration { from, to } => format_positional(
            language,
            key,
            &[
                &registration_detail_label(language, from),
                &registration_detail_label(language, to),
            ],
        ),
        TimelineDetail::Dns { from, to } => format_positional(
            language,
            key,
            &[
                &dns_detail_label(language, from),
                &dns_detail_label(language, to),
            ],
        ),
    };
    LocalizedText { key, text }
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

/// Every key the panel can show, in every language it offers, must be something a person can read.
#[test]
fn every_key_resolves_in_every_language() {
    for key in TextKey::ALL {
        for language in available_languages() {
            let text = template(*language, *key);
            assert!(!text.trim().is_empty(), "{key:?} is empty in {language:?}");
        }
    }
}

/// `{count}` and its siblings are substituted by name, so a translation that drops or renames one
/// would silently ship a literal `{age}` to the user.
#[test]
fn placeholders_match_across_languages() {
    for key in TextKey::ALL {
        let expected = placeholders(template(Language::ZhCn, *key));
        for language in available_languages() {
            assert_eq!(
                placeholders(template(*language, *key)),
                expected,
                "{key:?} uses different placeholders in {language:?}"
            );
        }
    }
}

#[cfg(test)]
fn placeholders(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        found.push(&rest[start..start + end + 1]);
        rest = &rest[start + end + 1..];
    }
    found
}

/// The traditional catalog is generated from the simplified one, so this is the mechanical half of
/// its review: a simplified-only character left in a traditional string means the generator missed
/// a term. The string below comes from `docs/i18n-20260930/generate-zh-tw.py`, which prints it.
#[test]
fn no_simplified_forms_survive_in_traditional() {
    const SIMPLIFIED_ONLY: &str = "与丢严个临为举义产仅从优会传体侧储关内册写冲决况冻准击划创删别务动区协单厂历参发变台号吗启员响唤围国图场坏块处备复头夹实审宽对导将尝层属带广应废开异弃弹强当录径态总户执护报拟拦拨择损换据数断无旧时显暂术机权条来极构标样档检汇没浅测温滚满点热状独现电监盖盘着码础确离种称稳窝筛签简类约级线组细织终经绑结给络绝统继绪续缓编网联脑脚脱节获营补装见观规视览计认议讯记许设访证识诊词试话询该详语误说请读调谨败账质贴费资轮软载较输达过运还这进远连适选释钟钮链销锁错键长闭问闲间阅队阶际险随隐隶静页顶项顺须预频题额风驱驻验骤齐";
    for key in TextKey::ALL {
        if SAME_IN_EVERY_CATALOG.contains(key) {
            continue;
        }
        let text = template(Language::ZhTw, *key);
        if let Some(found) = text.chars().find(|char| SIMPLIFIED_ONLY.contains(*char)) {
            panic!("{key:?} still carries the simplified form {found:?}: {text}");
        }
    }
}

/// The three language names are endonyms and the panel shows them in their own script everywhere.
#[cfg(test)]
const SAME_IN_EVERY_CATALOG: &[TextKey] = &[
    TextKey::LanguageZhCn,
    TextKey::LanguageZhTw,
    TextKey::LanguageEnUs,
];

/// The English catalog is written arm by arm, so the one thing a test can still prove is that no arm
/// was left as a copy of the authored Chinese text: a translation that never happened looks exactly
/// like a translation that did.
#[test]
fn english_catalog_has_no_chinese_leftovers() {
    let pending: Vec<TextKey> = TextKey::ALL
        .iter()
        .filter(|key| !SAME_IN_EVERY_CATALOG.contains(key) && en_us(**key) == zh_cn(**key))
        .copied()
        .collect();
    assert!(
        pending.is_empty(),
        "these keys still show the Chinese text in English: {pending:?}"
    );
}

/// Adding a key means adding it to the catalog list too: every test above walks `TextKey::ALL`, so
/// a key missing from it would escape all of them. Bump this number with the key.
#[test]
fn the_key_list_covers_the_enum() {
    assert_eq!(TextKey::ALL.len(), 1311);
}
