//! Tray lifecycle and UI-thread command projection.
//!
//! Backends only report closed [`TrayCommand`] values.  They never call the controller, touch a
//! device, or manipulate egui from their event callback.  This keeps native message handling and
//! the UI event loop independently testable.

use std::{collections::VecDeque, fmt, sync::Arc};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayCommand {
    Open,
    RefreshNow,
    HotspotStatus,
    Exit,
}

impl TrayCommand {
    #[must_use]
    pub const fn from_menu_id(id: u16) -> Option<Self> {
        match id {
            1 => Some(Self::Open),
            2 => Some(Self::RefreshNow),
            3 => Some(Self::HotspotStatus),
            4 => Some(Self::Exit),
            _ => None,
        }
    }

    #[must_use]
    pub const fn menu_id(self) -> u16 {
        match self {
            Self::Open => 1,
            Self::RefreshNow => 2,
            Self::HotspotStatus => 3,
            Self::Exit => 4,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrayLabels {
    pub open: String,
    pub refresh_now: String,
    pub hotspot_status: String,
    pub exit: String,
    pub tooltip: String,
}

impl TrayLabels {
    #[must_use]
    pub fn zh_cn() -> Self {
        Self {
            open: "打开面板".to_owned(),
            refresh_now: "立即刷新".to_owned(),
            hotspot_status: "热点状态".to_owned(),
            exit: "退出".to_owned(),
            tooltip: "DJI 一代 4G 面板".to_owned(),
        }
    }

    #[must_use]
    pub fn menu_items(&self) -> [&str; 4] {
        [
            &self.open,
            &self.refresh_now,
            &self.hotspot_status,
            &self.exit,
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayState {
    Ready,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrayError {
    stable_code: &'static str,
    os_code: Option<u32>,
}

impl TrayError {
    #[must_use]
    pub const fn new(stable_code: &'static str) -> Self {
        Self {
            stable_code,
            os_code: None,
        }
    }

    #[must_use]
    pub const fn with_os_code(stable_code: &'static str, os_code: Option<u32>) -> Self {
        Self {
            stable_code,
            os_code,
        }
    }

    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        self.stable_code
    }

    #[must_use]
    pub const fn os_code(&self) -> Option<u32> {
        self.os_code
    }
}

impl fmt::Display for TrayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.stable_code)
    }
}

impl std::error::Error for TrayError {}

pub trait TrayBackend {
    fn create(&mut self, labels: &TrayLabels) -> Result<(), TrayError>;
    fn poll(&mut self) -> Option<TrayCommand>;
    fn recreate(&mut self) -> Result<(), TrayError>;
    fn set_tooltip(&mut self, tooltip: &str) -> Result<(), TrayError>;

    /// Push a shell balloon notification (a toast on Windows 10/11).  Title and text come only
    /// from the closed verdict/reason vocabulary — never device identifiers.  Best effort: a
    /// backend without balloon support simply declines.
    fn show_balloon(&mut self, _title: &str, _text: &str) -> Result<(), TrayError> {
        Err(TrayError::new("tray:balloon_unsupported"))
    }

    /// Install a hook invoked on the backend's event thread whenever a tray event is queued, so
    /// the UI can force a repaint. A window hidden to the tray stops repainting on its own and
    /// would otherwise never drain queued commands such as Exit.
    fn set_wake_hook(&mut self, _hook: Arc<dyn Fn() + Send + Sync>) {}

    /// Register the panel's real viewport window (raw Win32 `HWND` value) with the backend.
    ///
    /// A window hidden to the tray produces no frames, so `PanelApp::update` never runs and the
    /// UI thread cannot re-show itself; the native worker uses this handle to restore, show, and
    /// foreground the panel itself on Open, or Exit so pending local archive I/O can finish.
    /// Backends without a native window decline silently.
    fn register_panel_window(&mut self, _hwnd: isize) {}

    /// Install a hook the backend's event thread invokes with every tray command it queues.
    ///
    /// The commands that would otherwise die with the hidden window — 「立即刷新」 (dispatched
    /// straight to the controller runner) and 「热点状态」 (native message box on its own thread)
    /// — are fulfilled there without a UI frame. The hook runs on the backend's worker thread,
    /// so it must stay bounded, non-blocking, and panic-free.
    fn set_action_hook(&mut self, _hook: Arc<dyn Fn(TrayCommand) + Send + Sync>) {}

    /// Report that the UI thread drained an Exit command and is shutting down gracefully.
    ///
    /// This only extends the backend's hard-exit deadline; it never cancels it. The wake hook
    /// above is best effort — a window hidden with `ViewportCommand::Visible(false)` cannot be
    /// repainted, so this acknowledgement may never be sent at all, and termination must not
    /// depend on it.
    fn acknowledge_exit(&mut self) {}

    /// Best-effort non-blocking heartbeat, only while local archive I/O is pending at exit.
    fn defer_exit_for_local_io(&mut self) {}

    fn take_error(&mut self) -> Option<TrayError> {
        None
    }
}

pub struct TrayController<B> {
    backend: B,
    labels: TrayLabels,
    state: TrayState,
}

impl<B: TrayBackend> TrayController<B> {
    pub fn initialize(mut backend: B, labels: TrayLabels) -> Result<Self, TrayError> {
        backend.create(&labels)?;
        Ok(Self {
            backend,
            labels,
            state: TrayState::Ready,
        })
    }

    #[must_use]
    pub fn state(&self) -> TrayState {
        self.state
    }

    #[must_use]
    pub fn labels(&self) -> &TrayLabels {
        &self.labels
    }

    pub fn try_recv(&mut self) -> Option<TrayCommand> {
        self.backend.poll()
    }

    pub fn recreate(&mut self) -> Result<(), TrayError> {
        self.backend.recreate().inspect_err(|_error| {
            self.state = TrayState::Unavailable;
        })?;
        self.state = TrayState::Ready;
        Ok(())
    }

    pub fn set_tooltip(&mut self, tooltip: &str) -> Result<(), TrayError> {
        self.backend.set_tooltip(tooltip)
    }

    pub fn show_balloon(&mut self, title: &str, text: &str) -> Result<(), TrayError> {
        self.backend.show_balloon(title, text)
    }

    pub fn set_wake_hook(&mut self, hook: Arc<dyn Fn() + Send + Sync>) {
        self.backend.set_wake_hook(hook);
    }

    pub fn register_panel_window(&mut self, hwnd: isize) {
        self.backend.register_panel_window(hwnd);
    }

    pub fn set_action_hook(&mut self, hook: Arc<dyn Fn(TrayCommand) + Send + Sync>) {
        self.backend.set_action_hook(hook);
    }

    pub fn acknowledge_exit(&mut self) {
        self.backend.acknowledge_exit();
    }

    /// Keep graceful shutdown alive only while local archive I/O is still pending.
    pub fn defer_exit_for_local_io(&mut self) {
        self.backend.defer_exit_for_local_io();
    }

    pub fn take_error(&mut self) -> Option<TrayError> {
        self.backend.take_error()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseAction {
    HideToTray,
    Exit,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowState {
    pub visible: bool,
    pub explicit_exit: bool,
    pub close_hint_shown: bool,
}

pub fn close_window(window: &mut WindowState, explicit_exit: bool) -> CloseAction {
    if explicit_exit || window.explicit_exit {
        window.explicit_exit = true;
        CloseAction::Exit
    } else {
        window.visible = false;
        window.close_hint_shown = true;
        CloseAction::HideToTray
    }
}

#[must_use]
pub fn merge_activation(pending: Option<TrayCommand>, next: TrayCommand) -> Option<TrayCommand> {
    match (pending, next) {
        (_, TrayCommand::Exit) => Some(TrayCommand::Exit),
        (Some(TrayCommand::RefreshNow), _) => Some(TrayCommand::RefreshNow),
        (Some(TrayCommand::HotspotStatus), TrayCommand::Open) => Some(TrayCommand::HotspotStatus),
        (Some(TrayCommand::Open), TrayCommand::RefreshNow) => Some(TrayCommand::RefreshNow),
        (Some(TrayCommand::Open), TrayCommand::HotspotStatus) => Some(TrayCommand::HotspotStatus),
        (Some(existing), _) => Some(existing),
        (None, value) => Some(value),
    }
}

/// Which tray commands the worker-side action hook must fulfil without a UI frame.
///
/// A window hidden to the tray produces no frames, so `PanelApp::update` never runs and queued
/// events are never read. Each of the four commands has exactly one owner that works in that
/// state, and the hook must not duplicate any of them:
/// - 「打开面板」 — the native worker itself (`ShowWindow` + `SetForegroundWindow`);
/// - 「立即刷新」 and 「热点状态」 — the action hook (this predicate);
/// - 「退出」 — the worker's hard-exit backstop plus the graceful UI path when it can run.
#[must_use]
pub const fn off_ui_command(command: TrayCommand) -> bool {
    matches!(
        command,
        TrayCommand::RefreshNow | TrayCommand::HotspotStatus
    )
}

/// A deterministic backend used by UI/lifecycle tests.  The production backend is kept behind the
/// same trait so a native tray failure can be represented as `TrayState::Unavailable` without
/// making the panel window unreachable.
#[derive(Clone, Debug, Default)]
pub struct MemoryTrayBackend {
    pub events: VecDeque<TrayCommand>,
    pub fail_create: bool,
    pub fail_recreate: bool,
    pub fail_tooltip: bool,
    pub fail_balloon: bool,
    pub create_count: usize,
    pub recreate_count: usize,
    pub tooltip: Option<String>,
    pub balloons: Vec<(String, String)>,
    /// The raw window handle handed to the backend by `register_panel_window`, recorded so tests
    /// can prove the wiring reached the backend.
    pub panel_window: Option<isize>,
}

impl MemoryTrayBackend {
    pub fn push(&mut self, command: TrayCommand) {
        self.events.push_back(command);
    }
}

impl TrayBackend for MemoryTrayBackend {
    fn create(&mut self, _labels: &TrayLabels) -> Result<(), TrayError> {
        self.create_count += 1;
        if self.fail_create {
            Err(TrayError::new("tray:create_failed"))
        } else {
            Ok(())
        }
    }

    fn poll(&mut self) -> Option<TrayCommand> {
        self.events.pop_front()
    }

    fn recreate(&mut self) -> Result<(), TrayError> {
        self.recreate_count += 1;
        if self.fail_recreate {
            Err(TrayError::new("tray:recreate_failed"))
        } else {
            Ok(())
        }
    }

    fn set_tooltip(&mut self, tooltip: &str) -> Result<(), TrayError> {
        if self.fail_tooltip {
            Err(TrayError::new("tray:tooltip_failed"))
        } else {
            self.tooltip = Some(tooltip.to_owned());
            Ok(())
        }
    }

    fn show_balloon(&mut self, title: &str, text: &str) -> Result<(), TrayError> {
        if self.fail_balloon {
            Err(TrayError::new("tray:balloon_failed"))
        } else {
            self.balloons.push((title.to_owned(), text.to_owned()));
            Ok(())
        }
    }

    fn register_panel_window(&mut self, hwnd: isize) {
        self.panel_window = Some(hwnd);
    }
}

/// Native shell backend.  The platform crate owns the Win32 HWND/icon and exposes only bounded
/// events here, keeping unsafe/message-pump code out of the egui crate.
#[derive(Debug, Default)]
pub struct NativeTrayBackend {
    native: Option<dji4g_windows_platform::NativeTray>,
    last_error: Option<TrayError>,
}

impl TrayBackend for NativeTrayBackend {
    fn create(&mut self, labels: &TrayLabels) -> Result<(), TrayError> {
        let native_labels = dji4g_windows_platform::NativeTrayLabels {
            open: labels.open.clone(),
            refresh_now: labels.refresh_now.clone(),
            hotspot_status: labels.hotspot_status.clone(),
            exit: labels.exit.clone(),
            tooltip: labels.tooltip.clone(),
        };
        self.native = Some(
            dji4g_windows_platform::NativeTray::create(native_labels)
                .map_err(|error| TrayError::with_os_code(error.code, error.os_code))?,
        );
        Ok(())
    }

    fn poll(&mut self) -> Option<TrayCommand> {
        let native = self.native.as_ref()?;
        loop {
            match native.try_recv()? {
                dji4g_windows_platform::TrayEvent::Menu(id) => {
                    return TrayCommand::from_menu_id(id);
                }
                dji4g_windows_platform::TrayEvent::TaskbarCreated => continue,
            }
        }
    }

    fn recreate(&mut self) -> Result<(), TrayError> {
        self.native
            .as_ref()
            .ok_or_else(|| TrayError::new("tray:native_unavailable"))?
            .recreate()
            .map_err(|error| TrayError::with_os_code(error.code, error.os_code))
    }

    fn set_tooltip(&mut self, tooltip: &str) -> Result<(), TrayError> {
        self.native
            .as_ref()
            .ok_or_else(|| TrayError::new("tray:native_unavailable"))?
            .set_tooltip(tooltip)
            .map_err(|error| TrayError::with_os_code(error.code, error.os_code))
    }

    fn show_balloon(&mut self, title: &str, text: &str) -> Result<(), TrayError> {
        self.native
            .as_ref()
            .ok_or_else(|| TrayError::new("tray:native_unavailable"))?
            .show_balloon(title, text)
            .map_err(|error| TrayError::with_os_code(error.code, error.os_code))
    }

    fn set_wake_hook(&mut self, hook: Arc<dyn Fn() + Send + Sync>) {
        match self.native.as_ref() {
            Some(native) => {
                if let Err(error) = native.set_wake_hook(hook) {
                    self.last_error = Some(TrayError::with_os_code(error.code, error.os_code));
                }
            }
            None => {
                self.last_error = Some(TrayError::new("tray:native_unavailable"));
            }
        }
    }

    fn register_panel_window(&mut self, hwnd: isize) {
        match self.native.as_ref() {
            Some(native) => {
                if let Err(error) = native.register_panel_window(hwnd) {
                    self.last_error = Some(TrayError::with_os_code(error.code, error.os_code));
                }
            }
            None => {
                self.last_error = Some(TrayError::new("tray:native_unavailable"));
            }
        }
    }

    fn set_action_hook(&mut self, hook: Arc<dyn Fn(TrayCommand) + Send + Sync>) {
        let Some(native) = self.native.as_ref() else {
            self.last_error = Some(TrayError::new("tray:native_unavailable"));
            return;
        };
        // The native layer speaks in raw tray events; the closed menu-id mapping drops anything
        // outside the four known commands, and the Explorer-recovery event carries no command.
        let bridged: Arc<dyn Fn(dji4g_windows_platform::TrayEvent) + Send + Sync> =
            Arc::new(move |event| match event {
                dji4g_windows_platform::TrayEvent::Menu(id) => {
                    if let Some(command) = TrayCommand::from_menu_id(id) {
                        hook(command);
                    }
                }
                dji4g_windows_platform::TrayEvent::TaskbarCreated => {}
            });
        if let Err(error) = native.set_action_hook(bridged) {
            self.last_error = Some(TrayError::with_os_code(error.code, error.os_code));
        }
    }

    fn acknowledge_exit(&mut self) {
        // Non-blocking and error-free by design: this runs on the UI thread during shutdown, and a
        // missing or unresponsive native tray must neither stall egui nor raise a tray error. The
        // worker's already-armed deadline covers us if this never arrives.
        if let Some(native) = self.native.as_ref() {
            native.acknowledge_exit();
        }
    }

    fn defer_exit_for_local_io(&mut self) {
        if let Some(native) = self.native.as_ref() {
            native.defer_exit_for_local_io();
        }
    }

    fn take_error(&mut self) -> Option<TrayError> {
        self.last_error.take().or_else(|| {
            self.native.as_ref().and_then(|native| {
                native
                    .take_error()
                    .map(|error| TrayError::with_os_code(error.code, error.os_code))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_creation_is_explicit_and_does_not_create_an_unreachable_state() {
        let backend = MemoryTrayBackend {
            fail_create: true,
            ..MemoryTrayBackend::default()
        };
        assert!(TrayController::initialize(backend, TrayLabels::zh_cn()).is_err());
    }

    #[test]
    fn recreate_and_command_polling_are_bounded() {
        let mut backend = MemoryTrayBackend::default();
        backend.push(TrayCommand::Open);
        let mut tray = TrayController::initialize(backend, TrayLabels::zh_cn()).unwrap();
        assert_eq!(tray.try_recv(), Some(TrayCommand::Open));
        assert_eq!(tray.try_recv(), None);
        tray.recreate().unwrap();
        assert_eq!(tray.state(), TrayState::Ready);
    }

    #[test]
    fn local_io_heartbeat_forwards_separately_from_exit_acknowledgement() {
        #[derive(Default)]
        struct Recording {
            heartbeats: usize,
            acks: usize,
        }
        impl TrayBackend for Recording {
            fn create(&mut self, _: &TrayLabels) -> Result<(), TrayError> {
                Ok(())
            }
            fn poll(&mut self) -> Option<TrayCommand> {
                None
            }
            fn recreate(&mut self) -> Result<(), TrayError> {
                Ok(())
            }
            fn set_tooltip(&mut self, _: &str) -> Result<(), TrayError> {
                Ok(())
            }
            fn acknowledge_exit(&mut self) {
                self.acks += 1;
            }
            fn defer_exit_for_local_io(&mut self) {
                self.heartbeats += 1;
            }
        }
        let mut tray =
            TrayController::initialize(Recording::default(), TrayLabels::zh_cn()).unwrap();
        tray.acknowledge_exit();
        tray.defer_exit_for_local_io();
        tray.defer_exit_for_local_io();
        assert_eq!(tray.backend.heartbeats, 2);
        assert_eq!(tray.backend.acks, 1);
    }
}
