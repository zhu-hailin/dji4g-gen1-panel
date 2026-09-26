//! eframe shell for the immutable snapshot-driven panel.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use dji4g_application::{
    ActionKindTag, ActionPlanId, ActionRequest, AutostartApplyOutcome, AutostartKnownState,
    AutostartStatus, ControlledRepairRequest, ControllerHandle, ControllerSnapshot, ErrorCode,
    FailureCode, LanguageCode, OperationState, PortError, PreparedActionState, SettingsSaveOutcome,
    UiCommand, UiSendError as ApplicationUiSendError,
};
use dji4g_windows_platform::{
    ActivationRequest, AutostartControl, AutostartObservedState, PlatformError, SingleInstance,
};
use eframe::egui::{self, Color32, RichText};

use crate::config::{ConfigError, ConfigStore, ConfigV1};
use crate::diagnostics_export::ExportError;
use crate::localization::{Language, LocalizedText, TextKey, availability_title};
use crate::native_dialog::{
    DialogBackend, DialogRequest, confirm_message_for_action, result_request,
};
use crate::tray::{
    TrayBackend, TrayCommand, TrayController, TrayError, WindowState, off_ui_command,
};
use crate::ui::{
    StatusTone, availability_reason_with_diagnostics, availability_vm, device_tools, diagnostics,
    overview, repairs, scale, settings, sms, wrapped_label,
};
use dji4g_domain::{ActionKind, Availability, DisruptionLevel, RiskLevel};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Page {
    Overview,
    Diagnostics,
    Repairs,
    Sms,
    DeviceTools,
    Settings,
}

/// The ordered navigation tabs. Kept as one closed list so the strip and its tests can never
/// disagree about which pages exist.
pub(crate) const NAV_ITEMS: [(Page, TextKey); 6] = [
    (Page::Overview, TextKey::NavOverview),
    (Page::Sms, TextKey::NavSms),
    (Page::DeviceTools, TextKey::NavDeviceTools),
    (Page::Diagnostics, TextKey::NavDiagnostics),
    (Page::Repairs, TextKey::NavRepairs),
    (Page::Settings, TextKey::NavSettings),
];

pub trait UiCommandSink: Send + Sync {
    fn try_send(&self, command: UiCommand) -> Result<(), ApplicationUiSendError>;
}

/// The command surface a repair/hotspot button needs on top of the plain controller commands:
/// a click must be able to present its native confirmation box *immediately*, composed from the
/// click's own action metadata, instead of waiting for the controller round-trip to publish a
/// prepared action.  Implemented by [`PanelApp`], which owns the dialog worker machinery; page
/// renderers receive `&dyn PanelCommandSink` so the click path is the only way a confirmation
/// box can open.
pub trait PanelCommandSink {
    /// Queue a plain controller command (e.g. the hotspot retry's `Refresh`).
    fn try_send(&self, command: UiCommand) -> Result<(), ApplicationUiSendError>;

    /// Present the native confirm box for a controlled repair request and prepare it.
    fn prepare_repair_now(&self, request: ControlledRepairRequest);
    fn prepare_network_repair_now(
        &self,
        request_id: u64,
        repair: dji4g_application::NetworkRepairKind,
    ) {
        let _ = self.try_send(UiCommand::PrepareNetworkRepair { request_id, repair });
    }

    /// Present the native confirm box for a plain action request and prepare it.
    fn prepare_action_now(&self, request: ActionRequest);
}

/// UI-facing projection of a tray backend.  The native worker and all platform handles remain
/// outside egui; this trait exposes only bounded polling and stable errors.
pub trait TrayEventSource: Send {
    fn try_recv(&mut self) -> Option<TrayCommand>;
    fn take_error(&mut self) -> Option<TrayError>;

    /// Install a hook the backend calls (on its own thread) whenever a tray event is queued, so
    /// the UI can force a repaint. Needed because a window hidden to the tray stops repainting
    /// and would otherwise never drain queued commands such as Exit.
    fn set_wake_hook(&mut self, _hook: Arc<dyn Fn() + Send + Sync>) {}

    /// Report that the UI thread drained an Exit command and is shutting down gracefully. This
    /// only extends the native worker's hard-exit deadline; it never cancels it, because this
    /// call is never reached when the window is hidden and egui stops repainting.
    fn acknowledge_exit(&mut self) {}
    fn defer_exit_for_local_io(&mut self) {}
}

impl<B> TrayEventSource for TrayController<B>
where
    B: TrayBackend + Send + 'static,
{
    fn try_recv(&mut self) -> Option<TrayCommand> {
        TrayController::try_recv(self)
    }

    fn take_error(&mut self) -> Option<TrayError> {
        TrayController::take_error(self)
    }

    fn set_wake_hook(&mut self, hook: Arc<dyn Fn() + Send + Sync>) {
        TrayController::set_wake_hook(self, hook);
    }

    fn acknowledge_exit(&mut self) {
        TrayController::acknowledge_exit(self);
    }
    fn defer_exit_for_local_io(&mut self) {
        TrayController::defer_exit_for_local_io(self);
    }
}

/// The write half of the tray controller (tooltip and balloon).  Split from
/// [`TrayEventSource`] so the availability updater and the command poller can share one
/// controller; a failed write is swallowed here because it must never take the window down.
pub trait TrayTooltipSink: Send {
    fn set_tooltip(&mut self, _tooltip: &str) {}

    /// Best-effort shell balloon; text comes only from the closed verdict/reason vocabulary.
    fn show_balloon(&mut self, _title: &str, _text: &str) {}
}

/// Shares one [`TrayController`] between the event poller and the tooltip sink.
struct SharedTray<B: TrayBackend>(Arc<Mutex<TrayController<B>>>);

impl<B> TrayEventSource for SharedTray<B>
where
    B: TrayBackend + Send + 'static,
{
    fn try_recv(&mut self) -> Option<TrayCommand> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_recv()
    }

    fn take_error(&mut self) -> Option<TrayError> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take_error()
    }

    fn set_wake_hook(&mut self, hook: Arc<dyn Fn() + Send + Sync>) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_wake_hook(hook);
    }

    fn acknowledge_exit(&mut self) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .acknowledge_exit();
    }
    fn defer_exit_for_local_io(&mut self) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .defer_exit_for_local_io();
    }
}

impl<B> TrayTooltipSink for SharedTray<B>
where
    B: TrayBackend + Send + 'static,
{
    fn set_tooltip(&mut self, tooltip: &str) {
        let mut tray = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = tray.set_tooltip(tooltip);
    }

    fn show_balloon(&mut self, title: &str, text: &str) {
        let mut tray = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = tray.show_balloon(title, text);
    }
}

impl UiCommandSink for ControllerHandle {
    fn try_send(&self, command: UiCommand) -> Result<(), ApplicationUiSendError> {
        ControllerHandle::try_send(self, command).map(|_| ())
    }
}

/// Sink wrapper that fulfils UI-owned commands before dispatch. `ExportDiagnostics` is built
/// from the snapshot the UI already holds, so it must never reach the controller queue; the
/// click is recorded here and completed by [`PanelApp::export_diagnostics`] after layout.
struct UiSideCommands {
    inner: Arc<dyn UiCommandSink>,
    export_requested: Arc<AtomicBool>,
}

impl UiCommandSink for UiSideCommands {
    fn try_send(&self, command: UiCommand) -> Result<(), ApplicationUiSendError> {
        if matches!(command, UiCommand::ExportDiagnostics) {
            self.export_requested.store(true, Ordering::Release);
            return Ok(());
        }
        self.inner.try_send(command)
    }
}

/// Panel-owned settings side effects: the `config.toml` write and the autostart registration.
///
/// Both run on the UI thread against the snapshot the panel already rendered, and their terminal
/// outcomes travel back through the controller queue as closed [`UiCommand`] values — the same
/// interception pattern as `ExportDiagnostics`. The panel never claims success before the real
/// write (with readback verification) has returned.
pub trait SettingsBackend: Send + Sync {
    fn save_config(&self, config: &ConfigV1) -> Result<(), ConfigError>;
    fn set_autostart(&self, enabled: bool) -> Result<AutostartObservedState, PlatformError>;
}

/// Production backend: the panel's atomic config store plus the current process's autostart
/// control. When either handle is unavailable at startup the whole backend is `None` and pending
/// changes are reported as honest stable failures instead of silently never resolving.
pub struct NativeSettingsBackend {
    store: ConfigStore,
    autostart: AutostartControl,
}

impl NativeSettingsBackend {
    #[must_use]
    pub fn new(store: ConfigStore, autostart: AutostartControl) -> Self {
        Self { store, autostart }
    }
}

impl SettingsBackend for NativeSettingsBackend {
    fn save_config(&self, config: &ConfigV1) -> Result<(), ConfigError> {
        self.store.save(config)
    }

    fn set_autostart(&self, enabled: bool) -> Result<AutostartObservedState, PlatformError> {
        self.autostart.set_enabled(enabled)
    }
}

/// Keep the closed `config:*` stable string so the settings page can localise the exact cause.
fn config_failure_code(error: ConfigError) -> FailureCode {
    PortError::new(ErrorCode::Internal, error.stable_code()).code
}

fn platform_failure_code(error: PlatformError) -> FailureCode {
    PortError::new(ErrorCode::Internal, error.code).code
}

/// Map the platform registry observation onto the application's closed autostart state.
#[must_use]
pub fn map_autostart_state(value: AutostartObservedState) -> AutostartKnownState {
    match value {
        AutostartObservedState::Disabled => AutostartKnownState::Disabled,
        AutostartObservedState::Enabled => AutostartKnownState::Enabled,
        AutostartObservedState::Drift => AutostartKnownState::Drift,
    }
}

/// Logical size of the full panel window.
pub const PANEL_WINDOW_SIZE: [f32; 2] = [1100.0, 760.0];

/// Smallest full-panel window.
pub const PANEL_MIN_SIZE: [f32; 2] = [800.0, 600.0];

#[derive(Clone)]
pub struct PanelInputs {
    pub snapshot_rx: dji4g_application::sync::watch::Receiver<Arc<ControllerSnapshot>>,
    pub commands: Arc<dyn UiCommandSink>,
    /// Destination for UI-side diagnostic exports; `None` when the Windows profile variables
    /// were unavailable at startup, which makes exports fail with `export:path_unavailable`.
    pub exports_dir: Option<PathBuf>,
    /// Panel-owned settings side effects; `None` when the config paths or the autostart control
    /// are unavailable (or in a demo session), in which case pending writes fail with a stable
    /// code instead of being claimed as saved.
    pub settings_backend: Option<Arc<dyn SettingsBackend>>,
    /// Native dialog backend for repair confirmations and results; `None` in headless/demo
    /// sessions, where the confirmation state machine simply does not present boxes.
    pub dialog_backend: Option<DialogBackend>,
    /// Optional-feature probe record (CNUM/QCCID/serving-cell classifications) written by the
    /// production AT port each observation cycle and read by the overview for correlated status
    /// rows.  `None` in demo/headless sessions.
    pub feature_probe: Option<Arc<Mutex<crate::feature_probe::FeatureProbeState>>>,
}

impl PanelInputs {
    #[must_use]
    pub fn new(
        snapshot_rx: dji4g_application::sync::watch::Receiver<Arc<ControllerSnapshot>>,
        commands: Arc<dyn UiCommandSink>,
        exports_dir: Option<PathBuf>,
        settings_backend: Option<Arc<dyn SettingsBackend>>,
    ) -> Self {
        Self {
            snapshot_rx,
            commands,
            exports_dir,
            settings_backend,
            dialog_backend: None,
            feature_probe: None,
        }
    }

    /// Attach the production optional-probe record shared with the runtime AT port.
    #[must_use]
    pub fn with_feature_probe(
        mut self,
        feature_probe: Arc<Mutex<crate::feature_probe::FeatureProbeState>>,
    ) -> Self {
        self.feature_probe = Some(feature_probe);
        self
    }

    /// Attach the production native-message-box backend.
    #[must_use]
    pub fn with_dialog_backend(mut self, backend: DialogBackend) -> Self {
        self.dialog_backend = Some(backend);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToastState {
    pub text: LocalizedText,
    pub expires_at: SystemTime,
}

/// The exact title `main.rs` passes to `eframe::run_native`, i.e. the panel window's real title.
/// The window is located by this title (process-verified) once at startup so the tray worker can
/// natively restore/show/foreground it when 「打开面板」 is selected while the window is hidden —
/// a hidden window runs no frames, so the UI thread can never re-show itself. The zh-CN tray
/// tooltip base is the same string; the unit tests pin the two together.
pub const PANEL_WINDOW_TITLE: &str = "DJI 一代 4G 面板";

/// Locate the panel's real viewport window (raw `HWND` value) at startup. Runs in the eframe
/// creation callback, which executes even when the window is created hidden (`--autostart` /
/// `start_minimized`) and `update()` may never run. `None` degrades the tray's Open item to its
/// previous event-queue-only behaviour; nothing else changes.
#[must_use]
fn panel_window_handle() -> Option<isize> {
    dji4g_windows_platform::tray::find_process_window(PANEL_WINDOW_TITLE)
}

pub struct PanelApp {
    onboarding: crate::ui::onboarding::OnboardingState,
    loaded_config: ConfigV1,
    driver_setup_outcome: Option<dji4g_windows_platform::driver_setup::DriverSetupOutcome>,
    archive: Option<crate::sms_archive::ArchiveService>,
    archive_ui: crate::ui::sms_archive::ArchiveUi,
    archive_view: bool,
    exit_archive_snapshot: Option<Arc<ControllerSnapshot>>,
    exit_archive_observed: bool,
    snapshot_rx: dji4g_application::sync::watch::Receiver<Arc<ControllerSnapshot>>,
    commands: Arc<dyn UiCommandSink>,
    snapshot: Arc<ControllerSnapshot>,
    language: Language,
    page: Page,
    sms_compose: sms::SmsComposeState,
    /// UI-local device-tools terminal state (expert unlock, drafts, history visibility). Never
    /// persisted; the page resets it whenever the device or SIM context changes.
    device_tools: device_tools::DeviceToolsState,
    wireless_view: bool,
    wireless_history: crate::ui::wireless::WirelessHistory,
    toast: Option<ToastState>,
    font_warning: Option<LocalizedText>,
    exports_dir: Option<PathBuf>,
    window: WindowState,
    support_report: crate::support_report::ReportState,
    tray: Option<Box<dyn TrayEventSource>>,
    tray_tooltip: Option<Box<dyn TrayTooltipSink>>,
    tray_tooltip_base: Option<String>,
    tray_tooltip_last: Option<String>,
    rate_history: crate::ui::RateHistory,
    /// Wall clock of the last ring sample, so the chart records exactly one point per second
    /// regardless of how often the snapshot's rates change.
    last_rate_sample_at: Option<SystemTime>,
    /// Module-temperature trend of the current device (§7.5): one point per published evidence
    /// cycle, cleared when the device epoch changes so two modules are never charted as one line.
    temperature_history: crate::ui::TemperatureHistory,
    /// Device epoch and observation time of the newest ring sample, so a cycle that produced no
    /// new evidence adds nothing and a repaint alone never duplicates a point.
    last_temperature_sample: Option<(u64, SystemTime)>,
    notifier: AvailabilityNotifier,
    tray_wake_installed: bool,
    /// The panel's real viewport window (raw Win32 `HWND` value), located once at startup and
    /// registered with the tray backend on attach, so the native worker can restore/show/
    /// foreground the window itself while it is hidden and `update()` never runs.
    panel_hwnd: Option<isize>,
    single_instance: Option<SingleInstance>,
    /// Panel-owned settings side effects; `None` when the config paths or the autostart control
    /// are unavailable (or in a demo session), in which case pending writes fail with a stable
    /// code instead of being claimed as saved.
    settings_backend: Option<Arc<dyn SettingsBackend>>,
    /// Settings revision already covered by a `config.toml` write attempt. Initialised from the
    /// first snapshot so a freshly loaded configuration is never written straight back (which
    /// could also clobber a config that failed to parse).
    persisted_revision: u64,
    /// Settings revision of the last autostart registration attempt; `None` before the first.
    autostart_applied_revision: Option<u64>,
    /// Sequence of the last command-rejection notice shown as a toast, so each rejection is
    /// surfaced exactly once across the snapshots that carry it.
    last_feedback_seq: u64,
    /// True when this process carries no Authenticode signature (portable development build).
    /// Elevated repairs then run the unsigned sibling helper with an explicit warning in the
    /// confirmation dialog.
    dev_mode: bool,
    /// Native dialog backend; `None` in headless/demo sessions, where no boxes are presented.
    dialog_backend: Option<DialogBackend>,
    /// Whether a native dialog worker is currently showing a box; guards against stacking.
    dialog_busy: Arc<AtomicBool>,
    /// Operation id whose result box has already been presented (dedupe across snapshots).
    dialoged_result: Option<u64>,
    /// Optional-feature probe record shared with the production AT port; the overview correlates
    /// it against the current snapshot before showing any status text (demo/headless: `None`).
    feature_probe: Option<Arc<Mutex<crate::feature_probe::FeatureProbeState>>>,
}

impl PanelApp {
    /// Call only for the production launch after loading its persisted configuration.
    /// Demo/headless constructors deliberately leave onboarding hidden.
    pub fn configure_onboarding(&mut self, config: &ConfigV1) {
        self.loaded_config = config.clone();
        self.onboarding.completed = config.onboarding_completed;
        if !config.onboarding_completed {
            self.open_onboarding();
        }
    }

    pub fn configure_archive(&mut self, path: PathBuf, enabled: bool) {
        let mut archive = crate::sms_archive::ArchiveService::new(path);
        archive.set_enabled(enabled);
        self.archive = Some(archive);
    }

    fn finish_archive_for_exit(&mut self) -> bool {
        let final_snapshot = self
            .exit_archive_snapshot
            .get_or_insert_with(|| Arc::clone(&self.snapshot));
        let Some(archive) = &mut self.archive else {
            return false;
        };
        archive.poll();
        if !archive.busy() && !self.exit_archive_observed {
            archive.observe(final_snapshot);
            self.exit_archive_observed = true;
        }
        archive.busy()
    }

    #[cfg(debug_assertions)]
    pub fn set_review_archive(&mut self, loaded: bool) {
        self.page = Page::Sms;
        self.archive_view = true;
        self.archive = Some(crate::sms_archive::ArchiveService::review_fixture(loaded));
    }

    #[cfg(debug_assertions)]
    pub fn set_review_sms_storage_confirmation(&mut self, storage: dji4g_domain::SmsStorageId) {
        self.sms_compose.storage_confirmation = Some((
            self.snapshot.app.device.as_ref().map(|d| d.epoch),
            self.snapshot.sim_epoch,
            storage,
        ));
    }

    fn save_archive_preference(&mut self, enabled: bool) -> bool {
        let mut config = ConfigV1::from_settings(&self.snapshot.settings)
            .unwrap_or_else(|| self.loaded_config.clone());
        config.onboarding_completed = self.onboarding.completed;
        config.sms_archive_enabled = enabled;
        let result = self
            .settings_backend
            .as_ref()
            .ok_or_else(|| ConfigError::new("config:path_unavailable"))
            .and_then(|backend| backend.save_config(&config));
        match result {
            Ok(()) => {
                self.loaded_config = config;
                self.archive_ui.error = None;
                true
            }
            Err(_) => {
                self.archive_ui.error =
                    Some("保存历史开关失败，本次更改未保存。请检查用户目录权限后重试。".into());
                false
            }
        }
    }

    fn render_sms_page(&mut self, ui: &mut egui::Ui, snapshot: &ControllerSnapshot) {
        crate::ui::components::page_tabs(
            ui,
            egui::Id::new("sms-source-tabs"),
            &mut self.archive_view,
            &[
                crate::ui::components::TabItem::new(false, "模块短信"),
                crate::ui::components::TabItem::new(true, "本地历史"),
            ],
        );
        ui.add_space(6.0);
        if !self.archive_view {
            sms::render(
                ui,
                snapshot,
                &snapshot.sms_messages,
                self.language,
                self.commands.as_ref(),
                &mut self.sms_compose,
            );
            return;
        }
        use crate::ui::sms_archive::ArchiveAction;
        let action =
            crate::ui::sms_archive::render(ui, self.archive.as_ref(), &mut self.archive_ui);
        match action {
            Some(ArchiveAction::SetEnabled(enabled)) => {
                if self.save_archive_preference(enabled) {
                    if let Some(archive) = &mut self.archive {
                        archive.set_enabled(enabled);
                    }
                }
            }
            Some(ArchiveAction::Clear) => {
                // Persist disabled first, so a subsequent launch cannot immediately refill a cleared file.
                if self.save_archive_preference(false) {
                    if let Some(archive) = &mut self.archive {
                        archive.clear();
                    }
                    self.archive_ui.export_path = None;
                }
            }
            Some(ArchiveAction::Export) => {
                if let (Some(archive), Some(directory)) = (&mut self.archive, &self.exports_dir) {
                    let stamp = SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();
                    let path = directory.join(format!("短信历史-{stamp}.txt"));
                    archive.export_text(path.clone());
                    self.archive_ui.export_path = Some(path);
                } else {
                    self.archive_ui.error = Some("未导出：用户导出目录不可用。".into());
                }
            }
            None => {}
        }
    }

    pub fn open_onboarding(&mut self) {
        self.onboarding.open = true;
        if !self.snapshot.serial_work_busy {
            self.send(UiCommand::Refresh);
        }
    }

    pub fn configure_driver_setup_result(
        &mut self,
        outcome: dji4g_windows_platform::driver_setup::DriverSetupOutcome,
    ) {
        self.driver_setup_outcome = Some(outcome);
        self.onboarding.open = true;
        if outcome.allows_recheck() && !self.snapshot.serial_work_busy {
            self.send(UiCommand::Refresh);
        }
    }

    /// Explicit visual fixture: it uses only the supplied snapshot and starts no worker.
    pub fn review_onboarding(&mut self) {
        self.onboarding.open = true;
    }

    #[cfg(debug_assertions)]
    pub fn review_onboarding_with_driver(&mut self, bundled: bool) {
        self.review_onboarding();
        self.onboarding.driver_fixture = Some(bundled);
    }

    fn finish_onboarding(&mut self) {
        self.onboarding.open = false;
        self.onboarding.completed = true;
        let mut config = ConfigV1::from_settings(&self.snapshot.settings)
            .unwrap_or_else(|| self.loaded_config.clone());
        // Keep current non-registry settings even when autostart cannot be observed yet.
        config.language = self.snapshot.settings.language;
        config.start_minimized = self.snapshot.settings.start_minimized;
        config.active_probe = self.snapshot.settings.active_probe;
        config.log_level = self.snapshot.settings.log_level;
        config.onboarding_completed = true;
        config.sms_archive_enabled = self.loaded_config.sms_archive_enabled;
        self.loaded_config = config.clone();
        let result = self
            .settings_backend
            .as_ref()
            .ok_or_else(|| ConfigError::new("config:path_unavailable"))
            .and_then(|backend| backend.save_config(&config));
        if let Err(error) = result {
            self.toast = Some(ToastState {
                text: LocalizedText {
                    key: TextKey::ErrorInternal,
                    text: format!(
                        "已进入面板，但引导完成状态保存失败（{}）；下次启动可能再次显示。",
                        error.stable_code()
                    ),
                },
                expires_at: SystemTime::now() + Duration::from_secs(12),
            });
        }
    }

    #[must_use]
    pub fn new(inputs: PanelInputs, cc: &eframe::CreationContext<'_>) -> Self {
        let font_warning = match crate::font::install_chinese_font(&cc.egui_ctx) {
            Ok(_path) => None,
            Err(error) => {
                let warning = LocalizedText::new(
                    Language::ZhCn,
                    crate::localization::stable_code_text(error.stable_code())
                        .unwrap_or(TextKey::ErrorInternal),
                );
                #[cfg(debug_assertions)]
                eprintln!("{warning}");
                Some(warning)
            }
        };
        crate::ui::style_root(&cc.egui_ctx);
        let snapshot = inputs.snapshot_rx.borrow();
        let language = language_from_code(snapshot.settings.language);
        let persisted_revision = snapshot.settings.revision;
        Self {
            onboarding: Default::default(),
            loaded_config: ConfigV1::default(),
            driver_setup_outcome: None,
            archive: None,
            archive_ui: Default::default(),
            archive_view: false,
            exit_archive_snapshot: None,
            exit_archive_observed: false,
            snapshot_rx: inputs.snapshot_rx,
            commands: inputs.commands,
            snapshot,
            language,
            page: Page::Overview,
            sms_compose: sms::SmsComposeState::default(),
            device_tools: device_tools::DeviceToolsState::default(),
            wireless_view: false,
            wireless_history: crate::ui::wireless::WirelessHistory::default(),
            toast: None,
            font_warning,
            exports_dir: inputs.exports_dir,
            support_report: crate::support_report::ReportState::default(),
            window: WindowState {
                visible: true,
                ..WindowState::default()
            },
            tray: None,
            tray_tooltip: None,
            tray_tooltip_base: None,
            tray_tooltip_last: None,
            rate_history: crate::ui::RateHistory::default(),
            last_rate_sample_at: None,
            temperature_history: crate::ui::TemperatureHistory::default(),
            last_temperature_sample: None,
            notifier: AvailabilityNotifier::default(),
            tray_wake_installed: false,
            panel_hwnd: panel_window_handle(),
            single_instance: None,
            settings_backend: inputs.settings_backend,
            persisted_revision,
            autostart_applied_revision: None,
            last_feedback_seq: 0,
            dev_mode: dji4g_windows_platform::is_dev_build(),
            dialog_backend: inputs.dialog_backend,
            dialog_busy: Arc::new(AtomicBool::new(false)),
            dialoged_result: None,
            feature_probe: inputs.feature_probe,
        }
    }

    /// Headless constructor used by tests: no native window, and the live snapshot receiver from
    /// `inputs` is kept so follow-up snapshots can be published into it.
    #[must_use]
    pub fn headless(mut inputs: PanelInputs) -> Self {
        let snapshot = Arc::clone(&inputs.snapshot_rx.borrow_and_update());
        let language = language_from_code(snapshot.settings.language);
        let persisted_revision = snapshot.settings.revision;
        Self {
            onboarding: Default::default(),
            loaded_config: ConfigV1::default(),
            driver_setup_outcome: None,
            archive: None,
            archive_ui: Default::default(),
            archive_view: false,
            exit_archive_snapshot: None,
            exit_archive_observed: false,
            snapshot_rx: inputs.snapshot_rx,
            commands: inputs.commands,
            snapshot,
            language,
            page: Page::Overview,
            sms_compose: sms::SmsComposeState::default(),
            device_tools: device_tools::DeviceToolsState::default(),
            wireless_view: false,
            wireless_history: crate::ui::wireless::WirelessHistory::default(),
            toast: None,
            font_warning: None,
            exports_dir: inputs.exports_dir,
            support_report: crate::support_report::ReportState::default(),
            window: WindowState {
                visible: true,
                ..WindowState::default()
            },
            tray: None,
            tray_tooltip: None,
            tray_tooltip_base: None,
            tray_tooltip_last: None,
            rate_history: crate::ui::RateHistory::default(),
            last_rate_sample_at: None,
            temperature_history: crate::ui::TemperatureHistory::default(),
            last_temperature_sample: None,
            notifier: AvailabilityNotifier::default(),
            tray_wake_installed: false,
            panel_hwnd: None,
            single_instance: None,
            settings_backend: inputs.settings_backend,
            persisted_revision,
            autostart_applied_revision: None,
            last_feedback_seq: 0,
            dev_mode: dji4g_windows_platform::is_dev_build(),
            dialog_backend: inputs.dialog_backend,
            dialog_busy: Arc::new(AtomicBool::new(false)),
            dialoged_result: None,
            feature_probe: inputs.feature_probe,
        }
    }

    /// Constructor for headless/state tests. It never creates a native window.
    #[must_use]
    pub fn from_snapshot(
        snapshot: Arc<ControllerSnapshot>,
        commands: Arc<dyn UiCommandSink>,
    ) -> Self {
        let (sender, receiver) = dji4g_application::sync::watch::channel(Arc::clone(&snapshot));
        drop(sender);
        Self::headless(PanelInputs {
            snapshot_rx: receiver,
            commands,
            exports_dir: None,
            settings_backend: None,
            dialog_backend: None,
            feature_probe: None,
        })
    }

    /// Wiring seam for tests and for callers that resolve paths after construction.
    pub fn set_exports_dir(&mut self, exports_dir: Option<PathBuf>) {
        self.exports_dir = exports_dir;
    }

    /// Wiring seam for tests and for callers that resolve the native window handle after
    /// construction. Must be set before `attach_tray` for the registration to reach the backend.
    pub fn set_panel_window(&mut self, hwnd: Option<isize>) {
        self.panel_hwnd = hwnd;
    }

    /// Wiring seam for tests: attach a native dialog backend (recording fakes in tests). The
    /// production path attaches `NativeMessageBoxBackend` through [`PanelInputs::with_dialog_backend`].
    pub fn set_dialog_backend(&mut self, backend: DialogBackend) {
        self.dialog_backend = Some(backend);
    }

    /// Current toast text, exposed so headless tests can assert the closed zh-CN catalog
    /// rendered for an export outcome.
    #[must_use]
    pub fn toast_text(&self) -> Option<&str> {
        self.toast.as_ref().map(|toast| toast.text.text.as_str())
    }

    /// Sequence of the last command-rejection notice surfaced as a toast. An observation point
    /// for headless tests: the seq only advances when a *newer* rejection arrives.
    #[must_use]
    pub fn last_feedback_seq(&self) -> u64 {
        self.last_feedback_seq
    }

    /// Fulfil an export request from the snapshot the UI currently holds. The controller is
    /// never involved; failures surface the stable export codes through the closed catalog.
    pub fn export_diagnostics(&mut self, now: SystemTime) {
        let outcome = match self.exports_dir.as_ref() {
            Some(directory) => {
                crate::diagnostics_export::write_export(directory, &self.snapshot, now)
            }
            None => Err(ExportError::PATH_UNAVAILABLE),
        };
        match outcome {
            Ok(()) => self.show_toast(TextKey::DiagnosticsExportSuccess),
            Err(error) => self.show_toast(
                crate::localization::stable_code_text(error.stable_code())
                    .unwrap_or(TextKey::DiagnosticsExportFailed),
            ),
        }
    }

    pub fn attach_tray<B>(&mut self, mut tray: TrayController<B>)
    where
        B: TrayBackend + Send + 'static,
    {
        self.tray_tooltip_base = Some(tray.labels().tooltip.clone());
        // Hand the native worker the real viewport window: 「打开面板」 must restore, show, and
        // foreground the panel even while it is hidden, and a hidden window runs no frames — the
        // UI thread can never re-show itself, so the worker that received the click does it.
        if let Some(hwnd) = self.panel_hwnd {
            tray.register_panel_window(hwnd);
        }
        // The worker-side action hook fulfils the two commands that must work without a UI
        // frame: 「立即刷新」 goes straight to the controller runner, 「热点状态」 is answered by
        // a native message box on its own thread (never on the tray worker, which must keep
        // pumping messages and evaluating the exit backstop).
        tray.set_action_hook(self.tray_action_hook());
        let shared = Arc::new(Mutex::new(tray));
        self.tray = Some(Box::new(SharedTray(Arc::clone(&shared))));
        self.tray_tooltip = Some(Box::new(SharedTray(shared)));
    }

    /// Build the worker-side action hook: the off-UI fulfilment of the tray commands that would
    /// otherwise die with the hidden window.
    ///
    /// The hook runs on the tray worker thread for every queued command, so it stays bounded and
    /// non-blocking: the refresh is the same `try_send` the UI would do (the runner thread picks
    /// the signal up on its own poll, no frame needed), and the message box moves to a dedicated
    /// short-lived thread — a modal box on the worker would suspend the message pump and the
    /// hard-exit watchdog with it.
    fn tray_action_hook(&self) -> Arc<dyn Fn(TrayCommand) + Send + Sync> {
        let commands = Arc::clone(&self.commands);
        let snapshot_rx = self.snapshot_rx.clone();
        let title_base = self.tray_tooltip_base.clone();
        let gate = Arc::new(HotspotBoxGate::default());
        Arc::new(move |command| {
            if !off_ui_command(command) {
                // 「打开面板」 is fulfilled natively by the worker (ShowWindow +
                // SetForegroundWindow) and 「退出」 by the backstop plus the graceful UI path;
                // the hook must not duplicate or preempt either.
                return;
            }
            match command {
                TrayCommand::RefreshNow => {
                    let _ = commands.try_send(UiCommand::Refresh);
                }
                TrayCommand::HotspotStatus => {
                    let Some(guard) = gate.claim() else {
                        // A status box is already open; a second click must not stack modals.
                        return;
                    };
                    let snapshot_rx = snapshot_rx.clone();
                    let title_base = title_base.clone();
                    // The guard (and thus the gate) is released when the box is dismissed — or
                    // when the spawn fails and the closure is dropped without ever running.
                    let _ = std::thread::Builder::new()
                        .name("dji4g-tray-hotspot".to_owned())
                        .spawn(move || {
                            let _guard = guard;
                            // Fresh state at click time: the latest published snapshot, the same
                            // source the overview page renders, so the box can never contradict
                            // the window.
                            let snapshot = snapshot_rx.borrow();
                            let (title, text) = hotspot_status_message(
                                title_base.as_deref(),
                                &snapshot,
                                SystemTime::now(),
                            );
                            dji4g_windows_platform::show_message_box(&title, &text);
                        });
                }
                TrayCommand::Open | TrayCommand::Exit => {}
            }
        })
    }

    pub fn attach_single_instance(&mut self, instance: SingleInstance) {
        self.single_instance = Some(instance);
    }

    /// The page the panel currently shows.  An honest observation point for tests and the
    /// automated page-flow assertions; rendering stays the only other reader.
    #[must_use]
    pub fn current_page(&self) -> Page {
        self.page
    }

    /// Select a page for hardware-free native screenshot review.
    #[cfg(debug_assertions)]
    pub fn set_review_page(&mut self, page: Page) {
        self.page = page;
    }
    #[cfg(debug_assertions)]
    pub fn set_review_wireless(&mut self) {
        self.wireless_view = true;
        self.wireless_history.review_fixture();
    }
    /// Aim the SMS page at one inbox/outgoing tab and one selection. `selected` is the module
    /// index for an incoming row and the local transaction id for an outgoing record.
    #[cfg(debug_assertions)]
    pub fn set_review_sms_view(&mut self, outgoing: bool, selected: Option<u32>) {
        self.sms_compose.outgoing = outgoing;
        self.sms_compose.selected = selected.and_then(|index| {
            self.snapshot
                .sms_messages
                .iter()
                .find(|message| {
                    message.index == index
                        && (message.direction == dji4g_domain::SmsDirection::Outgoing) == outgoing
                })
                .map(|message| message.stable_id())
        });
    }
    /// Type a search string so the "no results" state can be captured.
    #[cfg(debug_assertions)]
    pub fn set_review_sms_search(&mut self, query: &str) {
        self.sms_compose.search = query.to_owned();
    }
    #[cfg(debug_assertions)]
    pub fn set_review_sms_editor(&mut self) {
        self.sms_compose.review_editor();
    }
    #[cfg(debug_assertions)]
    pub fn set_review_sms_confirmation(&mut self) {
        self.sms_compose.review_confirmation();
    }
    #[cfg(debug_assertions)]
    pub fn set_review_reply_replace(&mut self) {
        self.sms_compose.review_reply_replace();
    }
    /// Aim the device-tools page at one tab (0 预设 / 1 查询 / 2 专家) for hardware-free review.
    #[cfg(debug_assertions)]
    pub fn set_review_device_tools(&mut self, tab: usize) {
        self.page = Page::DeviceTools;
        self.device_tools.set_review_tab(tab);
    }

    /// Number of rate samples currently retained for the overview chart.
    #[must_use]
    pub fn rate_history_len(&self) -> usize {
        self.rate_history.len()
    }

    /// The tray tooltip text this panel last pushed (deduplicated before the sink).  An honest
    /// observation point for the tooltip-tracking tests.
    #[must_use]
    pub fn last_tray_tooltip(&self) -> Option<&str> {
        self.tray_tooltip_last.as_deref()
    }

    pub fn set_tray_error(&mut self, error: TrayError) {
        self.set_notice(
            crate::localization::stable_code_text(error.stable_code())
                .unwrap_or(TextKey::TrayUnavailableFallback),
        );
    }

    pub fn set_notice(&mut self, key: TextKey) {
        self.show_toast(key);
    }

    /// Handle one tray command drained by the UI thread.
    ///
    /// The hidden-window fulfilment deliberately lives elsewhere, because a window hidden to the
    /// tray runs no frames and this method may never be reached: 「打开面板」 is re-shown natively
    /// by the tray worker, 「立即刷新」 is dispatched to the controller runner by the worker-side
    /// action hook, and 「热点状态」 is answered by that hook as a native message box. What
    /// remains here is the visible-state behaviour and the state sync after a native show.
    pub fn handle_tray_command(&mut self, command: TrayCommand, ctx: &egui::Context) {
        match command {
            TrayCommand::Open => {
                self.page = Page::Overview;
                self.show_window(ctx);
            }
            TrayCommand::HotspotStatus => {
                // Answered natively by the action hook (message box on its own thread) in both
                // window states; the UI side deliberately leaves the window and page untouched,
                // so a status question never rips the user back to the overview.
            }
            TrayCommand::RefreshNow => {
                // The scan itself is dispatched by the action hook straight to the controller
                // runner — exactly once per click, with or without a UI frame. All the UI side
                // adds is bringing the window forward for a click that arrived while visible.
                self.show_window(ctx);
            }
            TrayCommand::Exit => {
                self.window.explicit_exit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                // Tell the tray worker that the graceful path is now running. This extends the
                // worker's hard-exit deadline once; it never cancels it, so if eframe then fails
                // to actually close the window the process is still terminated by the backstop.
                if let Some(tray) = self.tray.as_mut() {
                    tray.acknowledge_exit();
                }
            }
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.window.visible = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn show_toast(&mut self, key: TextKey) {
        self.toast = Some(ToastState {
            text: LocalizedText::new(self.language, key),
            expires_at: SystemTime::now() + Duration::from_secs(5),
        });
    }

    fn poll_shell_events(&mut self, ctx: &egui::Context) {
        if !self.tray_wake_installed {
            if let Some(tray) = self.tray.as_mut() {
                let repaint = ctx.clone();
                tray.set_wake_hook(Arc::new(move || repaint.request_repaint()));
                // Only mark this installed once a tray actually exists and has been handed the
                // hook. Setting the flag unconditionally would permanently skip installation if
                // an update ran before `attach_tray`, silently losing the wake path.
                self.tray_wake_installed = true;
            }
        }

        let mut activations = Vec::new();
        if let Some(instance) = self.single_instance.as_mut() {
            while let Some(request) = instance.try_recv() {
                activations.push(request);
            }
        }
        for request in activations {
            self.show_window(ctx);
            if matches!(request, ActivationRequest::OpenAndRefresh) {
                self.send(UiCommand::Refresh);
            }
        }

        let mut tray_commands = Vec::new();
        let mut tray_error = None;
        if let Some(tray) = self.tray.as_mut() {
            for _ in 0..8 {
                let Some(command) = tray.try_recv() else {
                    break;
                };
                tray_commands.push(command);
            }
            tray_error = tray.take_error();
        }
        for command in tray_commands {
            self.handle_tray_command(command, ctx);
        }
        if let Some(error) = tray_error {
            self.set_tray_error(error);
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        if self.window.explicit_exit {
            if self.finish_archive_for_exit() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.show_window(ctx);
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            return;
        }
        if self.tray.is_none() {
            self.show_toast(TextKey::TrayUnavailableFallback);
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.window.visible = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        if !self.window.close_hint_shown {
            self.window.close_hint_shown = true;
            self.show_toast(TextKey::CloseToTrayHint);
        }
    }

    pub fn receive_latest_nonblocking(&mut self, ctx: &egui::Context) {
        if self.snapshot_rx.has_changed() {
            let previous_send = self.snapshot.sms_send.clone();
            let previous_snapshot = Arc::clone(&self.snapshot);
            self.snapshot = self.snapshot_rx.borrow_and_update();
            self.support_report
                .observe(&previous_snapshot, &self.snapshot);
            self.wireless_history.observe(&self.snapshot);
            if self.snapshot.sms_send != previous_send {
                if let Some(send) = &self.snapshot.sms_send {
                    crate::logging::record_sms_state(send);
                }
            }
            self.language = language_from_code(self.snapshot.settings.language);
            // A rejected command surfaces as one toast per rejection: the controller bumps the
            // feedback sequence, and the panel compares it against the last one it showed.
            if let Some(feedback) = self.snapshot.feedback.as_ref() {
                if feedback.seq > self.last_feedback_seq {
                    self.last_feedback_seq = feedback.seq;
                    let key = crate::localization::failure_text(&feedback.code, self.language).key;
                    self.show_toast(key);
                }
            }
            self.service_pending_settings();
            self.update_tray_tooltip();
            self.consider_availability_balloon();
            ctx.request_repaint();
        }
    }

    /// Drive the panel-owned settings side effects for the snapshot just applied.
    ///
    /// - `config.toml`: at most one write attempt per settings revision. The revision is recorded
    ///   before the write, so a failure is reported once (and stays visible as `Failed`) instead
    ///   of being retried every frame; the next user change re-triggers. The very first snapshot
    ///   is never written back: its revision seeded `persisted_revision` at construction.
    /// - Autostart: at most one registration attempt per revision that carries a `Saving` state.
    ///   Every toggle bumps the settings revision, so observing the same `Saving` state again is
    ///   a no-op while a new request with the same desired value is still attempted.
    ///
    /// If the controller queue rejects the outcome report (`PanelApp::send` surfaces that as a
    /// toast), the attempt is still treated as spent: repeating the real write could duplicate
    /// side effects, and the next user change re-triggers everything anyway.
    /// Mirror the availability verdict into the tray tooltip so the always-visible surface
    /// answers 「现在怎么样了」 without reopening the window.  Pushed only when the short verdict
    /// changes; a failed tooltip is swallowed by the sink.
    fn update_tray_tooltip(&mut self) {
        let Some(sink) = self.tray_tooltip.as_mut() else {
            return;
        };
        let Some(base) = self.tray_tooltip_base.clone() else {
            return;
        };
        let status = LocalizedText::new(
            self.language,
            availability_title(self.snapshot.app.availability),
        )
        .text;
        let tooltip = format!("{base}：{status}");
        if self.tray_tooltip_last.as_deref() == Some(tooltip.as_str()) {
            return;
        }
        sink.set_tooltip(&tooltip);
        self.tray_tooltip_last = Some(tooltip);
    }

    /// Feed the overview chart on a fixed one-point-per-second cadence, wall-clock driven: the
    /// ring holds an honest sample every second even while the link is idle and the snapshot's
    /// rates never change (the previous change-driven sampling went silent in exactly that case).
    /// The sample reads the latest applied snapshot, so the ring and the hero numbers can never
    /// disagree. The first call samples immediately, then one full period apart.
    pub fn sample_rates_on_cadence(&mut self, now: SystemTime) {
        let due = self.last_rate_sample_at.is_none_or(|last| {
            now.duration_since(last)
                .is_ok_and(|elapsed| elapsed >= crate::ui::RATE_SAMPLE_PERIOD)
        });
        if !due {
            return;
        }
        self.last_rate_sample_at = Some(now);
        let network = self.snapshot.app.network.as_ref();
        self.rate_history.push((
            now,
            network.and_then(|network| network.down_bytes_per_sec),
            network.and_then(|network| network.up_bytes_per_sec),
        ));
    }

    /// Record one module-temperature point per published evidence cycle, stamped with that cycle's
    /// `observed_at` rather than the frame time.
    ///
    /// The value and its timestamp both come from the applied snapshot, so the trend and the
    /// 温度 row can never disagree, and a repaint alone adds nothing: a cycle that produced no new
    /// evidence (or was skipped because the port was busy) leaves an honest gap in the line.
    /// Changing device clears the ring, because readings from two modules are not one trend.
    pub fn sample_temperature_on_observation(&mut self) {
        let Some(device_epoch) = self
            .snapshot
            .app
            .device
            .as_ref()
            .map(|device| device.epoch.0)
        else {
            return;
        };
        let observed_at = self.snapshot.app.observed_at;
        if self.last_temperature_sample == Some((device_epoch, observed_at)) {
            return;
        }
        if self
            .last_temperature_sample
            .is_some_and(|(last_epoch, _)| last_epoch != device_epoch)
        {
            self.temperature_history = crate::ui::TemperatureHistory::default();
        }
        self.last_temperature_sample = Some((device_epoch, observed_at));
        self.temperature_history.push((
            observed_at,
            self.snapshot
                .app
                .cellular
                .as_ref()
                .and_then(|cellular| cellular.temperature_celsius),
        ));
    }

    /// Announce a hidden-window availability transition through the tray.  Title carries the
    /// verdict, body the precise reason sentence — both from the closed localization catalog, so
    /// no identifier or raw code ever reaches the shell.
    fn consider_availability_balloon(&mut self) {
        let Some(availability) = self.notifier.consider(
            self.snapshot.app.availability,
            self.window.visible,
            SystemTime::now(),
        ) else {
            return;
        };
        let verdict = LocalizedText::new(self.language, availability_title(availability)).text;
        let reason = availability_reason_with_diagnostics(
            &self.snapshot.app,
            &self.snapshot.diagnostics,
            SystemTime::now(),
            self.language,
        )
        .text;
        let title = match self.tray_tooltip_base.as_deref() {
            Some(base) => format!("{base}：{verdict}"),
            None => verdict,
        };
        if let Some(sink) = self.tray_tooltip.as_mut() {
            sink.show_balloon(&title, &reason);
        }
    }

    fn service_pending_settings(&mut self) {
        let revision = self.snapshot.settings.revision;
        let save_due = revision != self.persisted_revision;
        let config = if save_due {
            ConfigV1::from_settings(&self.snapshot.settings)
        } else {
            None
        };
        let desired_enabled = match &self.snapshot.settings.autostart {
            AutostartStatus::Saving {
                desired_enabled, ..
            } if self.autostart_applied_revision != Some(revision) => Some(*desired_enabled),
            _ => None,
        };

        if let Some(mut config) = config {
            config.onboarding_completed = self.onboarding.completed;
            config.sms_archive_enabled = self.loaded_config.sms_archive_enabled;
            self.loaded_config = config.clone();
            self.persisted_revision = revision;
            let result = match self.settings_backend.as_ref() {
                Some(backend) => backend.save_config(&config).map_err(config_failure_code),
                None => Err(config_failure_code(ConfigError::new(
                    "config:path_unavailable",
                ))),
            };
            self.send(UiCommand::SettingsPersisted(SettingsSaveOutcome {
                revision,
                result,
            }));
        }

        if let Some(desired_enabled) = desired_enabled {
            self.autostart_applied_revision = Some(revision);
            let observed = match self.settings_backend.as_ref() {
                Some(backend) => backend
                    .set_autostart(desired_enabled)
                    .map(map_autostart_state)
                    .map_err(platform_failure_code),
                None => Err(platform_failure_code(PlatformError {
                    code: "autostart:unsupported_platform",
                    os_code: None,
                })),
            };
            self.send(UiCommand::AutostartApplied(AutostartApplyOutcome {
                desired_enabled,
                observed,
            }));
        }
    }

    pub fn send(&mut self, command: UiCommand) {
        // ExportDiagnostics is UI-owned: it is fulfilled from the current snapshot before any
        // dispatch, no matter which path tried to send it.
        if matches!(command, UiCommand::ExportDiagnostics) {
            self.export_diagnostics(SystemTime::now());
            return;
        }
        if let Err(error) = self.commands.try_send(command) {
            let key = match error {
                ApplicationUiSendError::QueueFull => TextKey::StatusQueueFull,
                ApplicationUiSendError::Closed => TextKey::StatusBackendUnavailable,
            };
            self.toast = Some(ToastState {
                text: LocalizedText::new(self.language, key),
                expires_at: SystemTime::now() + Duration::from_secs(4),
            });
        }
    }

    /// Correlate the optional-probe record (written by the production AT port every observation
    /// cycle) against the snapshot about to be rendered.  The correlated view is the only way
    /// probe status text reaches the overview rows; without it the identity/wireless rows show
    /// their neutral values.  Reading is best-effort: a poisoned slot degrades to no annotation.
    fn current_probe_view(
        &self,
        snapshot: &ControllerSnapshot,
    ) -> Option<crate::feature_probe::FeatureProbeView> {
        let slot = self.feature_probe.as_ref()?;
        let state = slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let epoch = snapshot.app.device.as_ref().map(|device| device.epoch);
        Some(crate::feature_probe::probe_view(
            &state,
            snapshot.app.cellular.as_ref(),
            epoch,
        ))
    }

    pub fn render(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.render_ui(ctx);
    }

    /// Render the same UI without requiring a native window; callers supply their own input.
    pub fn render_ui(&mut self, ctx: &egui::Context) {
        self.support_report.poll();
        if let Some(archive) = &mut self.archive {
            archive.poll();
            if !self.window.explicit_exit {
                archive.observe(&self.snapshot);
            }
            if archive.busy() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        if self.window.explicit_exit {
            if self.finish_archive_for_exit() {
                if let Some(tray) = &mut self.tray {
                    tray.defer_exit_for_local_io();
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.heading("正在完成本地短信历史操作");
                    ui.spinner();
                    ui.label("保存、清空或导出结束后将自动退出，请稍候。");
                });
                ctx.request_repaint_after(Duration::from_millis(50));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        if self.onboarding.open {
            match crate::ui::onboarding::render(
                ctx,
                &self.snapshot,
                SystemTime::now(),
                self.onboarding.driver_fixture,
                self.driver_setup_outcome,
                self,
            ) {
                crate::ui::onboarding::OnboardingAction::OpenRepairs => {
                    self.finish_onboarding();
                    self.page = Page::Repairs;
                }
                crate::ui::onboarding::OnboardingAction::Enter => self.finish_onboarding(),
                crate::ui::onboarding::OnboardingAction::Refresh => self.send(UiCommand::Refresh),
                crate::ui::onboarding::OnboardingAction::InspectHostNetwork => {
                    self.send(UiCommand::InspectHostNetwork)
                }
                crate::ui::onboarding::OnboardingAction::InstallBundledDriver => {
                    self.start_driver_install(ctx)
                }
                crate::ui::onboarding::OnboardingAction::OpenWindowsUpdate => {
                    if dji4g_windows_platform::driver_setup::open_windows_update().is_err() {
                        dji4g_windows_platform::show_message_box(
                            "无法打开 Windows 更新",
                            "请从 Windows 设置打开“Windows 更新”，检查可选驱动更新。当前尚未安装任何驱动。",
                        );
                    }
                }
                crate::ui::onboarding::OnboardingAction::None => {}
            }
            ctx.request_repaint_after(Duration::from_millis(250));
            return;
        }
        let now = SystemTime::now();
        let snapshot = Arc::clone(&self.snapshot);
        self.wireless_history.observe(&snapshot);
        let probe_view = self.current_probe_view(&snapshot);
        let availability =
            availability_vm(&snapshot.app, &snapshot.diagnostics, now, self.language);
        // Set by the diagnostics page's UI-side command sink when the export button is used;
        // the export itself runs after layout so it can mutate the toast.
        let export_requested = Arc::new(AtomicBool::new(false));
        let mut driver_install_requested = false;

        egui::TopBottomPanel::top("panel-top")
            .show_separator_line(false)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(240, 244, 249))
                    .inner_margin(egui::Margin::symmetric(20.0, 12.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    crate::ui::shell::brand(ui);
                    ui.add_space(16.0);
                    ui.label(
                        RichText::new(&availability.title.text)
                            .size(14.0)
                            .color(availability.tone.color()),
                    );
                    if availability.is_loading {
                        ui.spinner();
                    }
                    if crate::ui::components::action_button(
                        ui,
                        "刷新",
                        crate::ui::components::ButtonKind::Outlined,
                        true,
                        None,
                    )
                    .clicked()
                    {
                        self.send(UiCommand::Refresh);
                    }
                    ui.menu_button("更多", |ui| {
                        if ui
                            .add_enabled(
                                !self.support_report.busy(),
                                egui::Button::new("导出详细日志"),
                            )
                            .on_hover_text("收集 USB、驱动、串口、网络与检测阶段，不包含短信正文。")
                            .clicked()
                        {
                            self.support_report
                                .request(self.exports_dir.clone(), Arc::clone(&snapshot));
                            ui.close_menu();
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("panel-footer")
            .show_separator_line(false)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(240, 244, 249))
                    .inner_margin(egui::Margin::symmetric(20.0, 8.0)),
            )
            .show(ctx, |ui| {
                if !self.support_report.status.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        if self.support_report.busy() {
                            ui.spinner();
                        }
                        ui.label(&self.support_report.status);
                        if let Some(path) = &self.support_report.path {
                            if ui.button("打开所在文件夹").clicked() {
                                if let Err(error) = crate::support_report::open_report_folder(path)
                                {
                                    self.support_report.status = format!(
                                        "打开目录失败：{error}；日志已保存，可复制路径打开"
                                    );
                                }
                            }
                            if ui.button("复制日志路径").clicked() {
                                ui.output_mut(|output| {
                                    output.copied_text = path.display().to_string()
                                });
                            }
                        }
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(crate::ui::meta_text(availability.freshness.text.clone()));
                    if snapshot
                        .sms_send
                        .as_ref()
                        .is_some_and(|send| send.phase != dji4g_application::SmsSendPhase::Finished)
                    {
                        ui.label(crate::ui::meta_text("短信发送进行中"));
                    }
                    if let Some(operation) = &snapshot.operation {
                        let text = match &operation.state {
                            OperationState::Running { phase } => {
                                crate::ui::operation_phase_text(*phase, None, self.language)
                            }
                            OperationState::Finished { outcome, .. } => {
                                crate::ui::operation_outcome_text(outcome, self.language)
                            }
                        };
                        ui.label(crate::ui::meta_text(text.text));
                    }
                });
            });
        let narrow = ctx.screen_rect().width() < 700.0;
        let menu_id = egui::Id::new("navigation-drawer-open");
        if narrow {
            egui::TopBottomPanel::top("compact-navigation").show(ctx, |ui| {
                if crate::ui::components::action_button(
                    ui,
                    "菜单",
                    crate::ui::components::ButtonKind::Tonal,
                    true,
                    None,
                )
                .clicked()
                {
                    ctx.data_mut(|d| {
                        let open = d.get_temp::<bool>(menu_id).unwrap_or(false);
                        d.insert_temp(menu_id, !open);
                    });
                }
            });
            let open = ctx.data(|d| d.get_temp::<bool>(menu_id).unwrap_or(false));
            if open {
                let previous = self.page;
                egui::Window::new("导航")
                    .id(menu_id.with("window"))
                    .collapsible(false)
                    .resizable(false)
                    .fixed_pos(egui::pos2(12.0, 72.0))
                    .default_width(220.0)
                    .show(ctx, |ui| {
                        crate::ui::shell::navigation(ui, &mut self.page, self.language);
                        if ui.button("收起菜单").clicked() {
                            ctx.data_mut(|d| d.insert_temp(menu_id, false));
                        }
                    });
                if self.page != previous {
                    ctx.data_mut(|d| d.insert_temp(menu_id, false));
                }
            }
        } else {
            egui::SidePanel::left("panel-navigation")
                .show_separator_line(false)
                .resizable(false)
                .exact_width(crate::ui::shell::sidebar_width(ctx.screen_rect().width()))
                .frame(
                    egui::Frame::none()
                        .fill(Color32::from_rgb(240, 244, 249))
                        .inner_margin(12.0),
                )
                .show(ctx, |ui| {
                    crate::ui::shell::navigation(ui, &mut self.page, self.language)
                });
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::WHITE)
                    .rounding(24.0)
                    .outer_margin(egui::Margin::symmetric(12.0, 0.0))
                    .inner_margin(if ctx.screen_rect().width() < 1000.0 {
                        20.0
                    } else {
                        28.0
                    }),
            )
            .show(ctx, |ui| {
                ui.set_max_width(ui.available_width().min(1200.0));
                if let Some(warning) = &self.font_warning {
                    wrapped_label(
                        ui,
                        RichText::new(&warning.text).color(StatusTone::Negative.color()),
                    );
                }
                // The SMS page owns a bounded workspace: its list and its reader scroll
                // independently inside the height the page really has, so it must not sit inside
                // the page-wide scroll area as well (two nested scroll areas would fight over the
                // wheel and the inner one used to be capped). Only a window too short to hold a
                // usable workspace keeps the old whole-page scroll behaviour.
                let sms_bounded = self.page == Page::Sms
                    && ui.available_height() >= crate::ui::sms_layout::MIN_BOUNDED_HEIGHT;
                if sms_bounded {
                    self.render_sms_page(ui, &snapshot);
                } else {
                    egui::ScrollArea::vertical()
                        .drag_to_scroll(false)
                        .id_salt(("panel-page-scroll", self.page as u8))
                        .auto_shrink([false, false])
                        .show(ui, |ui| match self.page {
                            Page::Overview => {
                                if crate::ui::module_network_check::render_compact(
                                    ui,
                                    &snapshot,
                                    now,
                                    self.language,
                                    self,
                                ) {
                                    self.page = Page::Repairs;
                                }
                                if let Some(destination) = overview::render_summary(
                                    ui,
                                    &snapshot,
                                    self.language,
                                    probe_view.as_ref(),
                                ) {
                                    self.page = destination;
                                }
                                if crate::ui::network_assistance::render_brief(
                                    ui,
                                    &snapshot,
                                    now,
                                    self.language,
                                ) {
                                    self.page = Page::Diagnostics;
                                }
                                crate::ui::components::page_tabs(
                                    ui,
                                    egui::Id::new("overview-view-tabs"),
                                    &mut self.wireless_view,
                                    &[
                                        crate::ui::components::TabItem::new(false, "连接概况"),
                                        crate::ui::components::TabItem::new(true, "无线观测"),
                                    ],
                                );
                                ui.add_space(10.0);
                                if self.wireless_view {
                                    crate::ui::wireless::render(
                                        ui,
                                        &snapshot,
                                        &self.wireless_history,
                                    );
                                } else {
                                    if let Some(destination) = overview::render(
                                        ui,
                                        &snapshot,
                                        self.language,
                                        self,
                                        &self.rate_history,
                                        &self.temperature_history,
                                        probe_view.as_ref(),
                                    ) {
                                        self.page = destination;
                                    }
                                }
                            }
                            Page::Diagnostics => {
                                crate::ui::components::page_heading(
                                    ui,
                                    "网络诊断",
                                    "分别检查模块通路与电脑网络，按证据定位问题",
                                );
                                if crate::ui::module_network_check::render(
                                    ui,
                                    &snapshot,
                                    now,
                                    self.language,
                                    self,
                                ) {
                                    self.page = Page::Repairs;
                                }
                                let ui_side = UiSideCommands {
                                    inner: Arc::clone(&self.commands),
                                    export_requested: Arc::clone(&export_requested),
                                };
                                diagnostics::render(ui, &snapshot, self.language, &ui_side);
                            }
                            Page::Repairs => {
                                driver_install_requested =
                                    repairs::render(ui, &snapshot, now, self.language, self);
                            }
                            // The stored messages ride along in the snapshot as a read-only copy
                            // (SmsMessage redacts sender/body in Debug/Serialize, so nothing leaks
                            // into logs or exports).
                            Page::Sms => self.render_sms_page(ui, &snapshot),
                            Page::DeviceTools => {
                                // The page needs the panel's confirmation entry point and its own
                                // mutable state at once; moving the state out keeps the two
                                // borrows disjoint.
                                let mut tools_state = std::mem::take(&mut self.device_tools);
                                device_tools::render(
                                    ui,
                                    &snapshot,
                                    self.language,
                                    self,
                                    &mut tools_state,
                                );
                                self.device_tools = tools_state;
                            }
                            Page::Settings => {
                                let _ = settings::render(
                                    ui,
                                    &snapshot,
                                    self.language,
                                    self.commands.as_ref(),
                                );
                                ui.separator();
                                ui.horizontal_wrapped(|ui| {
                                    if ui.button("重新查看首次使用引导").clicked() {
                                        self.open_onboarding();
                                    }
                                    if ui.button("使用说明").clicked() {
                                        show_help_dialog();
                                    }
                                    if ui.button("关于本应用").clicked() {
                                        show_about_dialog();
                                    }
                                    if ui.button("重启面板").clicked() {
                                        self.start_panel_restart(ctx);
                                    }
                                    if ui.button("退出应用").clicked() {
                                        self.window.explicit_exit = true;
                                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                        if let Some(tray) = self.tray.as_mut() {
                                            tray.acknowledge_exit();
                                        }
                                    }
                                });
                            }
                        });
                }
            });

        if export_requested.load(Ordering::Acquire) {
            self.export_diagnostics(now);
        }
        if driver_install_requested {
            self.start_driver_install(ctx);
        }

        if let Some(toast) = &self.toast {
            if now >= toast.expires_at {
                self.toast = None;
            } else {
                egui::Area::new("panel-toast".into())
                    .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -16.0])
                    .show(ctx, |ui| {
                        ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                        egui::Frame::dark_canvas(ui.style())
                            .inner_margin(egui::Margin::symmetric(
                                scale::SECTION_MARGIN[0],
                                scale::SECTION_MARGIN[1],
                            ))
                            .show(ui, |ui| {
                                wrapped_label(
                                    ui,
                                    RichText::new(toast.text.text.clone()).size(scale::BODY),
                                );
                            });
                    });
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
        // A steady cadence keeps every UI event snappy: a click opens its native confirmation
        // box immediately and dispatches the prepare command in the same frame; the controller
        // runner handles that command within one poll interval (~20 ms), so the plan is usually
        // published while the user is still reading the box.
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    /// Whether a serial task or an unconfirmed plan is in flight right now.
    ///
    /// Restarting or exiting in the middle of one would abandon a worker that still owns the
    /// port, so the button refuses instead of interrupting it.
    fn serial_work_busy(&self) -> bool {
        self.snapshot.serial_work_busy
            || self
                .snapshot
                .prepared_action
                .as_ref()
                .is_some_and(|prepared| {
                    matches!(
                        prepared.state,
                        dji4g_application::PreparedActionState::AwaitingConfirmation
                    )
                })
    }

    /// Restart the panel.
    ///
    /// The replacement is a sibling process of this same executable that waits for this one to
    /// release the single-instance mutex (see `--restart-after`), so the restart cannot degrade
    /// into a second copy fighting the first. This process then leaves through the ordinary
    /// graceful path: the tray worker is acknowledged and the controller runner is dropped, which
    /// cancels any tool transaction and lets the serial worker give its port lease back.
    fn start_panel_restart(&mut self, ctx: &egui::Context) {
        if self.serial_work_busy() {
            dji4g_windows_platform::show_message_box(
                "请等待当前任务完成",
                "正在发送短信、执行修复或运行设备工具。完成或取消后再重启面板，避免中断当前任务。",
            );
            return;
        }
        if !dji4g_windows_platform::confirm_message_box(
            None,
            "重启面板",
            "面板将关闭并立即重新启动。\n设备会在重启后重新识别；正在填写但未发送的短信草稿会丢失。\n\n现在重启？",
        ) {
            return;
        }
        let spawned = std::env::current_exe().and_then(|exe| {
            let directory = exe.parent().map(std::path::Path::to_path_buf);
            let mut command = std::process::Command::new(&exe);
            command.arg(format!("--restart-after={}", std::process::id()));
            if let Some(directory) = directory {
                command.current_dir(directory);
            }
            command.spawn()
        });
        match spawned {
            Ok(_) => {
                self.window.explicit_exit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                if let Some(tray) = self.tray.as_mut() {
                    tray.acknowledge_exit();
                }
            }
            Err(error) => dji4g_windows_platform::show_message_box(
                "无法重启面板",
                &format!("面板保持运行，未重启。\n{error}\n可先退出，再手动打开程序。"),
            ),
        }
    }

    fn start_driver_install(&mut self, ctx: &egui::Context) {
        if self.support_report.busy() || crate::ui::driver_setup::installation_busy(&self.snapshot)
        {
            dji4g_windows_platform::show_message_box(
                "请等待当前任务完成",
                "正在导出日志、发送短信或执行/确认修复。完成后再安装驱动，避免中断当前任务。",
            );
            return;
        }
        if !dji4g_windows_platform::confirm_message_box(
            None,
            "安装模块驱动",
            "面板将自动退出，随后显示 Windows 管理员授权，请选择“是”。\n仅安装硬件匹配的缺失驱动，正常接口不会强制重装。\n\n完成或取消后会返回普通权限的面板，显示结果和下一步；如提示重启，请先重启电脑。\n\n现在继续？",
        ) {
            return;
        }
        let result = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe.with_file_name("dji4g-driver-setup.exe"))
                .arg(format!("--wait-for-panel={}", std::process::id()))
                .spawn()
        });
        match result {
            Ok(_) => {
                self.window.explicit_exit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                if let Some(tray) = self.tray.as_mut() {
                    tray.acknowledge_exit();
                }
            }
            Err(error) => dji4g_windows_platform::show_message_box(
                "无法启动安装器",
                &format!("面板保持运行，尚未安装驱动。\n{error}\n请导出详细日志。"),
            ),
        }
    }

    /// Present the native result box for the current snapshot, exactly once per finished
    /// operation.
    ///
    /// The box is shown by a dedicated worker thread (the Win32 call blocks until the user
    /// dismisses it), and dismissal travels back through the ordinary command sink as
    /// `DismissOperation`. Confirmation boxes no longer live here: they are presented
    /// immediately by the button click itself (see [`PanelCommandSink`]), so a snapshot that
    /// merely *publishes* a prepared action never opens a box on its own. No backend (`None`)
    /// means a headless/demo session: the state machine simply does nothing. Public so headless
    /// tests can drive the same state machine `update` drives every frame.
    pub fn drive_native_dialogs(&mut self) {
        let Some(backend) = self.dialog_backend.clone() else {
            return;
        };
        if self.dialog_busy.load(Ordering::Acquire) {
            return;
        }
        if self.snapshot.operation.is_none() {
            self.dialoged_result = None;
        }
        let request = if let Some(operation) = self.snapshot.operation.as_ref() {
            if matches!(operation.state, OperationState::Finished { .. })
                && self.dialoged_result != Some(operation.operation_id)
            {
                result_request(&self.snapshot, operation.operation_id, self.language)
            } else {
                None
            }
        } else {
            None
        };
        let Some(request) = request else {
            return;
        };
        let DialogRequest::Result {
            operation_id,
            title,
            message,
        } = request;
        self.dialoged_result = Some(operation_id);
        self.dialog_busy.store(true, Ordering::Release);
        let owner = self.panel_hwnd;
        let commands = Arc::clone(&self.commands);
        let busy = Arc::clone(&self.dialog_busy);
        let spawn = std::thread::Builder::new()
            .name("dji4g-native-dialog".to_owned())
            .spawn(move || {
                backend.inform(owner, &title, &message);
                // The dismissal is sent before the busy flag clears, so the next dialog can only
                // be presented after the state this box dismissed has been published.
                let _ = commands.try_send(UiCommand::DismissOperation { operation_id });
                busy.store(false, Ordering::Release);
            });
        if spawn.is_err() {
            // A box that cannot be shown must not wedge the dialog state machine.
            self.dialog_busy.store(false, Ordering::Release);
            self.dialoged_result = None;
        }
    }

    /// Click-to-confirm path for a repair/hotspot button: claim the native-dialog slot, compose
    /// the confirmation box from the click's own action metadata, dispatch the prepare command,
    /// and hand the box to a worker thread.  The box therefore appears *immediately* on click —
    /// it never waits for the controller round-trip — while the prepare is processed in
    /// parallel.  A click while another native box owns the slot is ignored (the owner-modal box
    /// already disables the window; this gate is the second line of defence).
    fn confirm_and_prepare(&self, action: ActionKind, command: UiCommand) {
        let Some(backend) = self.dialog_backend.clone() else {
            // Headless/demo session without a native box backend: plain prepare dispatch.
            let _ = self.commands.try_send(command);
            return;
        };
        if self
            .dialog_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let Some((tag, disruption, risk)) = action_metadata(&action) else {
            // Not a confirmable action (Refresh never reaches this path): dispatch without a box.
            self.dialog_busy.store(false, Ordering::Release);
            let _ = self.commands.try_send(command);
            return;
        };
        let Some((title, mut message)) = confirm_message_for_action(
            &action,
            dji4g_application::action_requires_elevation(&action),
            Some(disruption),
            Some(risk),
            self.dev_mode,
            self.language,
        ) else {
            self.dialog_busy.store(false, Ordering::Release);
            let _ = self.commands.try_send(command);
            return;
        };
        if matches!(command, UiCommand::PrepareNetworkRepair { .. }) {
            message.push_str("\n\n完成后将只读复检一次，向固定端点发送少量公网与 DNS 请求；长期探测设置不变。失败或结果未知不会自动再次修复。");
        }
        // Send the prepare first: the controller handles it within one poll interval (~20 ms),
        // typically while the user is still reading the box that opens below.
        if self.commands.try_send(command).is_err() {
            // The prepare never reached the controller, so no plan can follow; show no box.
            self.dialog_busy.store(false, Ordering::Release);
            return;
        }
        let owner = self.panel_hwnd;
        let commands = Arc::clone(&self.commands);
        let busy = Arc::clone(&self.dialog_busy);
        let mut snapshot_rx = self.snapshot_rx.clone();
        let spawned = std::thread::Builder::new()
            .name("dji4g-native-dialog".to_owned())
            .spawn(move || {
                // The box itself blocks until the user answers; the prepare command above is
                // already on its way and normally completes while the user reads the box.
                let confirmed = backend.confirm(owner, &title, &message);
                // The answer turns into a command only once the plan this click prepared has
                // been published (bounded wait): a prepare the controller rejected never gets a
                // Confirm/Cancel, and the rejection surfaces through the ordinary feedback toast.
                if let Some(id) = wait_for_prepared_plan(&mut snapshot_rx, tag) {
                    let command = if confirmed {
                        UiCommand::ConfirmAction { id }
                    } else {
                        UiCommand::CancelAction { id }
                    };
                    let _ = commands.try_send(command);
                }
                busy.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            // A box that cannot be shown must not wedge the dialog slot.
            self.dialog_busy.store(false, Ordering::Release);
        }
    }
}

impl PanelCommandSink for PanelApp {
    fn try_send(&self, command: UiCommand) -> Result<(), ApplicationUiSendError> {
        self.commands.try_send(command)
    }

    fn prepare_repair_now(&self, request: ControlledRepairRequest) {
        let action = request.clone().into_action();
        self.confirm_and_prepare(action, UiCommand::PrepareRepair { request });
    }

    fn prepare_network_repair_now(
        &self,
        request_id: u64,
        repair: dji4g_application::NetworkRepairKind,
    ) {
        self.confirm_and_prepare(
            repair.request().into_action(),
            UiCommand::PrepareNetworkRepair { request_id, repair },
        );
    }
    fn prepare_action_now(&self, request: ActionRequest) {
        let action = request.clone();
        self.confirm_and_prepare(action, UiCommand::PrepareAction { request });
    }
}

impl eframe::App for PanelApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Rounded central panels do not paint their outer margins or corner cut-outs.
        // Use an opaque app surface rather than eframe's translucent dark default.
        Color32::from_rgb(240, 244, 249).to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll_shell_events(ctx);
        self.receive_latest_nonblocking(ctx);
        self.handle_close_request(ctx);
        // Drive native dialogs every frame (not only on snapshot change): a worker may have
        // just finished a box and cleared the busy flag without a state change yet.
        self.drive_native_dialogs();
        self.sample_rates_on_cadence(SystemTime::now());
        self.sample_temperature_on_observation();
        self.render(ctx, frame);
    }
}

/// Upper bound for the confirmation worker's wait for the plan its prepare command created.  A
/// prepare is normally handled within one `IDLE_POLL_INTERVAL` (~20 ms); the bound only covers
/// the slow path (e.g. a prepare the controller rejects never publishes a plan), so a worker can
/// never wait forever.
const PREPARED_PLAN_WAIT: Duration = Duration::from_secs(3);

/// Confirmation metadata for an action, as the native box needs it: the action tag (used to
/// match the plan this click created) plus the disruption and risk levels for the message.
/// `None` for actions that are not confirmable (`Refresh`).
#[must_use]
fn action_metadata(action: &ActionKind) -> Option<(ActionKindTag, DisruptionLevel, RiskLevel)> {
    let tag = ActionKindTag::from_action(action)?;
    let disruption = dji4g_application::action_disruption(action)?;
    let risk = dji4g_application::action_risk(action)?;
    Some((tag, disruption, risk))
}

/// Watch the published snapshot stream until a plan for `tag` appears (still awaiting
/// confirmation) or the bounded wait expires.  Polling is cheap: the watch receiver only reports
/// changes, and the worker sleeps 5 ms between checks, so the wait never spins or burns CPU.
#[must_use]
fn wait_for_prepared_plan(
    snapshot_rx: &mut dji4g_application::sync::watch::Receiver<Arc<ControllerSnapshot>>,
    tag: ActionKindTag,
) -> Option<ActionPlanId> {
    let deadline = std::time::Instant::now() + PREPARED_PLAN_WAIT;
    loop {
        if snapshot_rx.has_changed() {
            let snapshot = snapshot_rx.borrow_and_update();
            if let Some(prepared) = snapshot.prepared_action.as_ref() {
                if prepared.action == tag
                    && matches!(prepared.state, PreparedActionState::AwaitingConfirmation)
                {
                    return Some(prepared.id);
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Deduplicated, cooled-down availability-transition decisions for tray balloons.
///
/// A balloon is worth pushing only when the window is hidden AND the verdict changed AND the
/// same verdict has not been announced within the cooldown.  The first sighting never notifies
/// (startup is not an event), and the caller localizes whatever this returns.
#[derive(Default)]
pub struct AvailabilityNotifier {
    seen: Option<Availability>,
    last_notified: Option<(Availability, SystemTime)>,
}

/// Minimum spacing between two availability balloons, so a flapping link cannot spam toasts.
pub const AVAILABILITY_BALLOON_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(300);

impl AvailabilityNotifier {
    #[must_use]
    pub fn consider(
        &mut self,
        availability: Availability,
        window_visible: bool,
        now: SystemTime,
    ) -> Option<Availability> {
        let first = self.seen.is_none();
        let changed = self.seen != Some(availability);
        self.seen = Some(availability);
        if first || !changed || window_visible {
            return None;
        }
        if let Some((_, at)) = self.last_notified {
            if now.duration_since(at).unwrap_or_default() < AVAILABILITY_BALLOON_COOLDOWN {
                return None;
            }
        }
        self.last_notified = Some((availability, now));
        Some(availability)
    }
}

/// One-at-a-time gate for the native 「热点状态」 message box.
///
/// The box stays open until the user dismisses it; further tray clicks while it is open must not
/// stack more modal boxes. Pure claim/release semantics so the tests can pin them.
#[derive(Debug, Default)]
pub struct HotspotBoxGate {
    open: AtomicBool,
}

impl HotspotBoxGate {
    /// Claim the right to present one box; `None` while a box is already open.
    #[must_use]
    pub fn claim(self: &Arc<Self>) -> Option<HotspotBoxGuard> {
        self.open
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| HotspotBoxGuard(Arc::clone(self)))
    }
}

/// Releases the gate when dropped — including when the presenting thread panics — so a dismissed
/// or crashed box can never wedge the gate shut.
#[derive(Debug)]
pub struct HotspotBoxGuard(Arc<HotspotBoxGate>);

impl Drop for HotspotBoxGuard {
    fn drop(&mut self) {
        self.0.open.store(false, Ordering::Release);
    }
}

/// The native 「热点状态」 message content, built at click time from the latest published
/// snapshot — the same source the overview page renders, so the box can never contradict the
/// window. Title and body come only from the closed localization catalog; the freshness line
/// keeps the answer honest about how old the observation is.
#[must_use]
pub fn hotspot_status_message(
    base: Option<&str>,
    snapshot: &ControllerSnapshot,
    now: SystemTime,
) -> (String, String) {
    let language = language_from_code(snapshot.settings.language);
    let label = LocalizedText::new(language, TextKey::TrayHotspotStatus).text;
    let title = match base {
        Some(base) => format!("{base}：{label}"),
        None => label,
    };
    let vm = crate::ui::hotspot_vm(snapshot.app.hotspot, language);
    let field = LocalizedText::new(language, TextKey::FieldHotspot).text;
    let mut body = format!("{field}：{}", vm.status.text);
    if vm.reason.key != TextKey::ValueNotApplicable {
        body.push('\n');
        body.push_str(&vm.reason.text);
    }
    body.push('\n');
    body.push_str(&crate::ui::freshness_text(&snapshot.app, now, language).text);
    (title, body)
}

fn show_about_dialog() {
    dji4g_windows_platform::show_message_box(
        "关于 DJI 一代 4G 面板",
        &format!(
            "DJI 一代 4G 面板 v{}

非官方开源工具，用于诊断大疆一代 4G 模块的可用性；与大疆公司无关联。
许可：MIT OR Apache-2.0。
无遥测、不上传任何数据；日志与导出默认脱敏。",
            env!("CARGO_PKG_VERSION")
        ),
    );
}

fn show_help_dialog() {
    dji4g_windows_platform::show_message_box(
        "使用说明",
        "概览：模块当前能否上网的结论与原因。
诊断：设备、蜂窝、网卡的分层证据与探测结果。
修复：需要逐项确认的受控写操作。
设置：开机自启、托盘等持久化行为。

关闭窗口仅隐藏到托盘；悬停托盘图标可查看当前状态。",
    );
}

fn language_from_code(value: LanguageCode) -> Language {
    match value {
        LanguageCode::ZhCn => Language::ZhCn,
        LanguageCode::EnUs => Language::ZhCn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::TrayLabels;
    use dji4g_application::ReducerState;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The worker-side action hook and the slot through which tests observe its installation.
    type ActionHook = Arc<dyn Fn(TrayCommand) + Send + Sync>;
    type HookSlot = Arc<Mutex<Option<ActionHook>>>;

    /// A tray double that records how many times the UI thread acknowledged an Exit (which is
    /// what lets the native worker extend its hard-exit deadline), plus what `attach_tray` wired
    /// into the backend: the raw panel window handle and the worker-side action hook through
    /// which the frame-independent commands are fulfilled.
    struct RecordingTrayBackend {
        acknowledgements: Arc<AtomicUsize>,
        panel_window: Arc<Mutex<Option<isize>>>,
        action_hook: HookSlot,
    }

    impl RecordingTrayBackend {
        fn new(
            acknowledgements: Arc<AtomicUsize>,
            panel_window: Arc<Mutex<Option<isize>>>,
            action_hook: HookSlot,
        ) -> Self {
            Self {
                acknowledgements,
                panel_window,
                action_hook,
            }
        }
    }

    impl TrayBackend for RecordingTrayBackend {
        fn create(&mut self, _labels: &TrayLabels) -> Result<(), TrayError> {
            Ok(())
        }

        fn poll(&mut self) -> Option<TrayCommand> {
            None
        }

        fn recreate(&mut self) -> Result<(), TrayError> {
            Ok(())
        }

        fn set_tooltip(&mut self, _tooltip: &str) -> Result<(), TrayError> {
            Ok(())
        }

        fn register_panel_window(&mut self, hwnd: isize) {
            *self.panel_window.lock().expect("healthy lock") = Some(hwnd);
        }

        fn set_action_hook(&mut self, hook: Arc<dyn Fn(TrayCommand) + Send + Sync>) {
            *self.action_hook.lock().expect("healthy lock") = Some(hook);
        }

        fn acknowledge_exit(&mut self) {
            self.acknowledgements.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Debug)]
    struct NoopSink;

    impl UiCommandSink for NoopSink {
        fn try_send(&self, _command: UiCommand) -> Result<(), ApplicationUiSendError> {
            Ok(())
        }
    }

    /// Records every command dispatched, so the tests can prove the worker-side hook reaches the
    /// controller exactly once per click — with and without a UI frame.
    #[derive(Debug, Default)]
    struct RecordingSink {
        sent: Arc<Mutex<Vec<UiCommand>>>,
    }

    impl UiCommandSink for RecordingSink {
        fn try_send(&self, command: UiCommand) -> Result<(), ApplicationUiSendError> {
            self.sent.lock().expect("healthy lock").push(command);
            Ok(())
        }
    }

    #[test]
    fn native_canvas_is_opaque_material_surface() {
        let app = panel();
        for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
            assert_eq!(
                eframe::App::clear_color(&app, &visuals),
                Color32::from_rgb(240, 244, 249).to_normalized_gamma_f32()
            );
        }
    }

    fn panel() -> PanelApp {
        let snapshot = Arc::new(ReducerState::new(SystemTime::UNIX_EPOCH).snapshot());
        PanelApp::from_snapshot(snapshot, Arc::new(NoopSink))
    }

    #[test]
    fn first_run_skip_persists_and_settings_can_reopen_without_resetting_preferences() {
        struct Backend(Mutex<Vec<ConfigV1>>);
        impl SettingsBackend for Backend {
            fn save_config(&self, config: &ConfigV1) -> Result<(), ConfigError> {
                self.0.lock().unwrap().push(config.clone());
                Ok(())
            }
            fn set_autostart(&self, _: bool) -> Result<AutostartObservedState, PlatformError> {
                panic!("onboarding never changes registry")
            }
        }
        let backend = Arc::new(Backend(Mutex::new(Vec::new())));
        let mut app = panel();
        assert!(
            !app.onboarding.open,
            "demo and headless callers skip by default"
        );
        app.settings_backend = Some(backend.clone());
        let config = ConfigV1 {
            autostart: true,
            sms_archive_enabled: true,
            ..ConfigV1::default()
        };
        app.configure_onboarding(&config);
        assert!(app.onboarding.open);
        app.finish_onboarding();
        assert!(!app.onboarding.open);
        let saved = backend.0.lock().unwrap()[0].clone();
        assert!(saved.onboarding_completed);
        assert!(
            saved.sms_archive_enabled,
            "finishing onboarding preserves archive opt-in"
        );
        assert!(
            saved.autostart,
            "unknown registry state must preserve loaded intent"
        );
        let mut next = panel();
        next.configure_onboarding(&saved);
        assert!(!next.onboarding.open);
        next.open_onboarding();
        assert!(next.onboarding.open);
        assert!(next.onboarding.completed);
    }

    #[test]
    fn onboarding_save_failure_still_enters_and_reports_unsaved_state() {
        let mut app = panel();
        app.configure_onboarding(&ConfigV1::default());
        app.finish_onboarding();
        assert!(!app.onboarding.open);
        assert!(app.toast.as_ref().unwrap().text.text.contains("保存失败"));
        assert!(
            app.toast
                .as_ref()
                .unwrap()
                .text
                .text
                .contains("下次启动可能再次显示")
        );
    }

    #[test]
    fn first_run_checks_once_and_visual_fixture_never_starts_backend_work() {
        let sink = Arc::new(RecordingSink::default());
        let snapshot = Arc::new(ReducerState::new(SystemTime::UNIX_EPOCH).snapshot());
        let mut app = PanelApp::from_snapshot(snapshot, sink.clone());
        app.review_onboarding();
        assert!(sink.sent.lock().unwrap().is_empty());
        app.configure_onboarding(&ConfigV1::default());
        assert!(matches!(
            sink.sent.lock().unwrap().as_slice(),
            [UiCommand::Refresh]
        ));
        app.open_onboarding();
        assert_eq!(
            sink.sent.lock().unwrap().len(),
            2,
            "explicit reopening requests new evidence"
        );
    }

    fn panel_with_recording_tray(counter: Arc<AtomicUsize>) -> PanelApp {
        let mut app = panel();
        let tray = TrayController::initialize(
            RecordingTrayBackend::new(
                counter,
                Arc::new(Mutex::new(None)),
                Arc::new(Mutex::new(None)),
            ),
            TrayLabels::zh_cn(),
        )
        .expect("the recording backend always creates");
        app.attach_tray(tray);
        app
    }

    /// A panel whose tray records the `attach_tray` wiring: returns the app plus the slots
    /// holding the installed action hook and the registered window handle.
    fn panel_with_recording_hook(
        sink: Arc<RecordingSink>,
    ) -> (PanelApp, HookSlot, Arc<Mutex<Option<isize>>>) {
        let snapshot = Arc::new(ReducerState::new(SystemTime::UNIX_EPOCH).snapshot());
        let mut app =
            PanelApp::from_snapshot(snapshot, Arc::clone(&sink) as Arc<dyn UiCommandSink>);
        app.set_panel_window(Some(0x0000_0000_0001_2345));
        let hook_slot: HookSlot = Arc::new(Mutex::new(None));
        let hwnd_slot = Arc::new(Mutex::new(None));
        let tray = TrayController::initialize(
            RecordingTrayBackend::new(
                Arc::new(AtomicUsize::new(0)),
                Arc::clone(&hwnd_slot),
                Arc::clone(&hook_slot),
            ),
            TrayLabels::zh_cn(),
        )
        .expect("the recording backend always creates");
        app.attach_tray(tray);
        (app, hook_slot, hwnd_slot)
    }

    /// A snapshot carrying the given hotspot state, freshness, and observation time; everything
    /// else stays at the reducer's honest initial values.
    fn hotspot_snapshot(
        hotspot: dji4g_domain::HotspotStatus,
        freshness: dji4g_domain::Freshness,
        observed_at: SystemTime,
    ) -> ControllerSnapshot {
        let mut snapshot = ReducerState::new(SystemTime::UNIX_EPOCH).snapshot();
        snapshot.app = Arc::new(dji4g_domain::AppSnapshot {
            hotspot,
            freshness,
            observed_at,
            ..(*snapshot.app).clone()
        });
        snapshot
    }

    #[test]
    fn exit_acknowledges_the_watchdog_and_requests_a_real_close() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut app = panel_with_recording_tray(Arc::clone(&counter));
        app.handle_tray_command(TrayCommand::Exit, &egui::Context::default());
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        // `explicit_exit` is what makes the following close request actually exit instead of being
        // turned into another hide-to-tray by `handle_close_request`.
        assert!(app.window.explicit_exit);
    }

    #[test]
    fn non_exit_tray_commands_never_touch_the_shutdown_watchdog() {
        // Only Exit may participate in the shutdown backstop: acknowledging anything else would
        // let an ordinary tray click interfere with the worker's hard-exit deadline.
        for command in [
            TrayCommand::Open,
            TrayCommand::RefreshNow,
            TrayCommand::HotspotStatus,
        ] {
            let counter = Arc::new(AtomicUsize::new(0));
            let mut app = panel_with_recording_tray(Arc::clone(&counter));
            app.handle_tray_command(command, &egui::Context::default());
            assert_eq!(
                counter.load(Ordering::SeqCst),
                0,
                "{command:?} must not acknowledge an exit"
            );
            assert!(!app.window.explicit_exit);
        }
    }

    #[test]
    fn exit_without_an_attached_tray_still_requests_a_close() {
        // A failed native tray must never make the panel un-exitable from its own update loop.
        let mut app = panel();
        app.handle_tray_command(TrayCommand::Exit, &egui::Context::default());
        assert!(app.window.explicit_exit);
    }

    #[test]
    fn rate_sampling_records_exactly_one_point_per_second() {
        // The chart ring is wall-clock driven: the first call samples immediately, then exactly
        // one point per period — even while the snapshot's rates never change (an idle link must
        // still fill the chart instead of going silent).
        let mut app = panel();
        let start = SystemTime::now();
        app.sample_rates_on_cadence(start);
        assert_eq!(app.rate_history_len(), 1);
        app.sample_rates_on_cadence(start + Duration::from_millis(900));
        assert_eq!(app.rate_history_len(), 1, "inside the period adds nothing");
        app.sample_rates_on_cadence(start + Duration::from_secs(1));
        assert_eq!(app.rate_history_len(), 2);
        app.sample_rates_on_cadence(start + Duration::from_secs(2));
        assert_eq!(app.rate_history_len(), 3);
    }

    /// A snapshot with a bound device and one temperature reading, for the trend-sampling tests.
    fn temperature_snapshot(
        epoch: u64,
        observed_at: SystemTime,
        celsius: Option<i16>,
    ) -> Arc<ControllerSnapshot> {
        use dji4g_domain::{
            AppSnapshot, AttachState, CellularSnapshot, DeviceEpoch, DeviceSnapshot, FeatureStatus,
            Freshness, HotspotStatus, RegistrationState, SimState, StableDeviceIdentity,
        };
        let mut snapshot = ReducerState::new(SystemTime::UNIX_EPOCH).snapshot();
        snapshot.app = Arc::new(AppSnapshot {
            revision: 1,
            observed_at,
            freshness: Freshness::Fresh,
            availability: Availability::Available,
            hotspot: HotspotStatus::Off,
            device: Some(DeviceSnapshot {
                epoch: DeviceEpoch(epoch),
                identity: StableDeviceIdentity {
                    container_id: "container".to_owned(),
                    device_instance_id: "instance".to_owned(),
                    vid: 0x2ca3,
                    pid: 0x4006,
                },
                problem_code: None,
                at_port: Some("COM7".to_owned()),
                adapter_id: None,
            }),
            cellular: Some(CellularSnapshot {
                sim: SimState::Ready,
                registration: RegistrationState::RegisteredHome,
                attached: AttachState::Attached,
                carrier: None,
                radio_access_technology: None,
                signal_rssi_dbm: None,
                apn: None,
                pdp_address: None,
                pdp_state: None,
                firmware: None,
                serving_cell: None,
                sim_identity: None,
                numbers: None,
                temperature_celsius: celsius,
                temperature_status: FeatureStatus::Supported,
            }),
            network: None,
            active_operation: None,
            issues: Vec::new(),
        });
        Arc::new(snapshot)
    }

    fn panel_with_temperature(
        epoch: u64,
        observed_at: SystemTime,
        celsius: Option<i16>,
    ) -> PanelApp {
        PanelApp::from_snapshot(
            temperature_snapshot(epoch, observed_at, celsius),
            Arc::new(NoopSink),
        )
    }

    #[test]
    fn temperature_sampling_records_one_point_per_observation_cycle() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let mut app = panel_with_temperature(1, start, Some(57));
        app.sample_temperature_on_observation();
        assert_eq!(app.temperature_history.len(), 1);
        assert_eq!(
            app.temperature_history.last(),
            Some(&(start, Some(57))),
            "the sample is stamped with the observation time, not the frame time"
        );
        // A repaint, or any snapshot republish without new evidence, adds nothing.
        app.sample_temperature_on_observation();
        assert_eq!(app.temperature_history.len(), 1);
        // The next evidence cycle carries its own observation time.
        let mut next = (*app.snapshot).clone();
        let mut app_snapshot = (*next.app).clone();
        app_snapshot.observed_at = start + Duration::from_secs(10);
        next.app = Arc::new(app_snapshot);
        app.snapshot = Arc::new(next);
        app.sample_temperature_on_observation();
        assert_eq!(app.temperature_history.len(), 2);
        assert_eq!(app.temperature_history.previous_reading(), Some(57));
    }

    #[test]
    fn a_cycle_without_a_reading_stays_an_honest_gap() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let mut app = panel_with_temperature(1, start, Some(57));
        app.sample_temperature_on_observation();
        // The probe could not read the module this cycle: the ring records the gap, never a zero.
        app.snapshot = temperature_snapshot(1, start + Duration::from_secs(10), None);
        app.sample_temperature_on_observation();
        assert_eq!(
            app.temperature_history.last(),
            Some(&(start + Duration::from_secs(10), None))
        );
    }

    #[test]
    fn another_module_never_continues_the_previous_trend() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let mut app = panel_with_temperature(1, start, Some(57));
        app.sample_temperature_on_observation();
        assert_eq!(app.temperature_history.len(), 1);
        // A device change clears the ring: two modules are not one trend line.
        app.snapshot = temperature_snapshot(2, start + Duration::from_secs(10), Some(40));
        app.sample_temperature_on_observation();
        assert_eq!(app.temperature_history.len(), 1);
        assert_eq!(
            app.temperature_history.last(),
            Some(&(start + Duration::from_secs(10), Some(40)))
        );
        assert!(app.temperature_history.previous_reading().is_none());
    }

    #[test]
    fn a_panel_without_a_device_records_no_temperature_points() {
        let mut app = panel();
        app.sample_temperature_on_observation();
        assert!(app.temperature_history.is_empty());
    }

    #[test]
    fn attach_tray_registers_the_panel_window_and_installs_the_action_hook() {
        // The native worker can only re-show a hidden window it knows about, and can only
        // fulfil the frame-independent commands through a hook the panel installed.
        let (_, hook_slot, hwnd_slot) =
            panel_with_recording_hook(Arc::new(RecordingSink::default()));
        assert_eq!(
            *hwnd_slot.lock().expect("healthy lock"),
            Some(0x0000_0000_0001_2345)
        );
        assert!(
            hook_slot.lock().expect("healthy lock").is_some(),
            "attach_tray must install the worker-side action hook"
        );
    }

    #[test]
    fn refresh_now_reaches_the_runner_without_a_ui_frame_and_never_dispatches_twice() {
        let sink = Arc::new(RecordingSink::default());
        let sent = Arc::clone(&sink.sent);
        let (mut app, hook_slot, _) = panel_with_recording_hook(sink);
        let hook = hook_slot
            .lock()
            .expect("healthy lock")
            .clone()
            .expect("attach_tray installs the action hook");

        // The hidden-window click: only the worker-side hook runs (there is no UI frame), and
        // the scan must still reach the controller.
        hook(TrayCommand::RefreshNow);
        assert!(
            matches!(
                sent.lock().expect("healthy lock").as_slice(),
                [UiCommand::Refresh]
            ),
            "the hook alone must dispatch the scan"
        );

        // The visible-window click additionally drains the queued event on the next frame; that
        // path must not dispatch a second scan for the same click.
        app.handle_tray_command(TrayCommand::RefreshNow, &egui::Context::default());
        assert!(
            matches!(
                sent.lock().expect("healthy lock").as_slice(),
                [UiCommand::Refresh]
            ),
            "exactly one scan per click"
        );
        assert!(
            app.window.visible,
            "a visible click still brings the window forward"
        );

        // The hook must never preempt Open (fulfilled natively by the worker) or Exit (owned by
        // the backstop and the graceful UI path).
        hook(TrayCommand::Open);
        hook(TrayCommand::Exit);
        assert!(matches!(
            sent.lock().expect("healthy lock").as_slice(),
            [UiCommand::Refresh]
        ));
        assert!(!app.window.explicit_exit);
    }

    #[test]
    fn hotspot_status_leaves_the_page_and_window_alone() {
        // The answer is the native message box (action hook, own thread) in both window states;
        // the UI side must not navigate or re-show anything for a status question.
        let (mut app, _, _) = panel_with_recording_hook(Arc::new(RecordingSink::default()));
        app.page = Page::Diagnostics;
        app.handle_tray_command(TrayCommand::HotspotStatus, &egui::Context::default());
        assert_eq!(app.page, Page::Diagnostics);
    }

    #[test]
    fn the_hotspot_box_gate_admits_one_box_at_a_time() {
        let gate = Arc::new(HotspotBoxGate::default());
        let first = gate.claim().expect("the first claim succeeds");
        assert!(
            gate.claim().is_none(),
            "an open box must not stack a second one"
        );
        drop(first);
        let second = gate.claim().expect("the gate reopens once the box closed");
        drop(second);
        let third = gate.claim().expect("and stays reusable");
        drop(third);
    }

    #[test]
    fn hotspot_status_message_reports_current_state_from_the_closed_catalog() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let base = Some(PANEL_WINDOW_TITLE);

        let snapshot = hotspot_snapshot(
            dji4g_domain::HotspotStatus::On { clients: Some(2) },
            dji4g_domain::Freshness::Fresh,
            now - Duration::from_secs(5),
        );
        let (title, body) = hotspot_status_message(base, &snapshot, now);
        assert_eq!(title, "DJI 一代 4G 面板：热点状态");
        assert_eq!(
            body,
            "移动热点：已开启（2 台设备已连接）\n状态为最新 · 更新于 5 秒前"
        );

        // Off has no reason sentence: the 「不适用」 placeholder must never reach the box.
        let snapshot = hotspot_snapshot(
            dji4g_domain::HotspotStatus::Off,
            dji4g_domain::Freshness::Unknown,
            SystemTime::UNIX_EPOCH,
        );
        let (_, body) = hotspot_status_message(base, &snapshot, now);
        assert_eq!(body, "移动热点：已关闭\n尚无有效的更新时间");

        // Unsupported and Failed carry the precise reason line from the closed catalog.
        let snapshot = hotspot_snapshot(
            dji4g_domain::HotspotStatus::Unsupported(
                dji4g_domain::HotspotUnsupportedReason::MissingPackageIdentity,
            ),
            dji4g_domain::Freshness::Stale,
            now - Duration::from_secs(120),
        );
        let (_, body) = hotspot_status_message(base, &snapshot, now);
        assert_eq!(
            body,
            "移动热点：热点不可用\n当前运行方式没有热点功能所需的应用包身份。\n状态已过期，正在重新检测 · 更新于 2 分钟前"
        );
        let snapshot = hotspot_snapshot(
            dji4g_domain::HotspotStatus::Failed {
                code: dji4g_domain::ErrorCode::CapabilityUnavailable,
            },
            dji4g_domain::Freshness::Fresh,
            now,
        );
        let (_, body) = hotspot_status_message(base, &snapshot, now);
        assert_eq!(
            body,
            "移动热点：热点操作失败。\n所需的系统能力当前不可用。\n状态为最新 · 更新于 0 秒前"
        );

        // Without the tooltip base the closed menu label alone titles the box.
        let snapshot = hotspot_snapshot(
            dji4g_domain::HotspotStatus::Off,
            dji4g_domain::Freshness::Unknown,
            SystemTime::UNIX_EPOCH,
        );
        let (title, _) = hotspot_status_message(None, &snapshot, now);
        assert_eq!(title, "热点状态");
    }

    #[test]
    fn the_window_title_lookup_and_the_tray_tooltip_base_agree() {
        // The startup lookup finds the real window by the exact title main.rs passes to
        // `eframe::run_native`; the zh-CN tray labels carry the same string. Pinning the two
        // together makes a rename trip this test instead of silently killing 「打开面板」.
        assert_eq!(PANEL_WINDOW_TITLE, TrayLabels::zh_cn().tooltip);
    }

    #[test]
    fn navigation_offers_the_sms_page_exactly_once() {
        assert!(
            NAV_ITEMS.contains(&(Page::Sms, TextKey::NavSms)),
            "the 短信 tab must be part of the navigation strip"
        );
        let mut pages: Vec<Page> = NAV_ITEMS.iter().map(|(page, _)| *page).collect();
        pages.sort_by_key(|page| *page as u8);
        let count = pages.len();
        pages.dedup();
        assert_eq!(pages.len(), count, "each page appears exactly once");
    }
}
