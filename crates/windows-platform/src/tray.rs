//! Small, isolated Win32 notification-area adapter.
//!
//! The panel never owns an HWND and never calls Win32 from its egui update loop.  This module
//! keeps the hidden message window, icon, popup menu, and Explorer-restart recovery on a short
//! worker thread.  The public surface is a bounded, non-blocking event queue plus bounded command
//! acknowledgements, so a failed shell integration cannot strand the main window.
//!
//! Because a window hidden to the tray produces no frames — `PanelApp::update` (the only place
//! tray events are drained) simply stops running — the worker also fulfils two duties that must
//! not depend on a UI frame: it natively restores/shows/foregrounds the registered panel window
//! when 「打开面板」 or 「退出」 is selected (Exit resumes frames to finish pending local I/O), and it invokes a panel-provided action hook with every queued
//! event so 「立即刷新」/「热点状态」 can be served off-UI. Neither duty may block the worker:
//! it owns the message pump and the hard-exit watchdog.

use std::{
    fmt,
    sync::{
        Arc, Mutex, OnceLock,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::PlatformError;

const OP_TIMEOUT: Duration = Duration::from_secs(1);

/// Fixed menu/event ids shared with the panel's closed [`TrayCommand`] mapping.
pub const MENU_OPEN: u16 = 1;
pub const MENU_REFRESH_NOW: u16 = 2;
pub const MENU_HOTSPOT_STATUS: u16 = 3;
pub const MENU_EXIT: u16 = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrayLabels {
    pub open: String,
    pub refresh_now: String,
    pub hotspot_status: String,
    pub exit: String,
    pub tooltip: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayEvent {
    Menu(u16),
    /// Explorer broadcasts this message after its notification area is recreated.  The native
    /// worker re-adds the icon before exposing the event to the panel.
    TaskbarCreated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrayError {
    pub code: &'static str,
    pub os_code: Option<u32>,
}

impl TrayError {
    #[must_use]
    pub const fn new(code: &'static str) -> Self {
        Self {
            code,
            os_code: None,
        }
    }

    #[must_use]
    pub const fn with_os_code(code: &'static str, os_code: u32) -> Self {
        Self {
            code,
            os_code: Some(os_code),
        }
    }
}

impl fmt::Display for TrayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for TrayError {}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{ffi::c_void, time::Instant};

    use windows_sys::Win32::{
        Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, POINT, WPARAM},
        Graphics::Gdi::HBRUSH,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO,
                NIIF_RESPECT_QUIET_TIME, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
                NOTIFYICONDATAW, Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CREATESTRUCTW, CreateIconFromResourceEx, CreatePopupMenu,
                CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
                GWLP_USERDATA, GetCursorPos, GetWindowLongPtrW, HICON, IDI_APPLICATION,
                LR_DEFAULTSIZE, LoadIconW, MF_STRING, MSG, PM_REMOVE, PeekMessageW, PostMessageW,
                PostQuitMessage, PostThreadMessageW, RegisterClassW, RegisterWindowMessageW,
                SW_RESTORE, SetForegroundWindow, SetWindowLongPtrW, ShowWindow, TPM_RETURNCMD,
                TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_COMMAND,
                WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONDBLCLK, WM_NCCREATE, WM_NULL, WM_QUIT,
                WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW,
            },
        },
    };

    const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
    const TRAY_WINDOW_CLASS: &[u16] = &[
        b'D' as u16,
        b'j' as u16,
        b'i' as u16,
        b'4' as u16,
        b'G' as u16,
        b'P' as u16,
        b'a' as u16,
        b'n' as u16,
        b'e' as u16,
        b'l' as u16,
        b'.' as u16,
        b'T' as u16,
        b'r' as u16,
        b'a' as u16,
        b'y' as u16,
        b'.' as u16,
        b'v' as u16,
        b'1' as u16,
        0,
    ];
    const TRAY_WINDOW_TITLE: &[u16] = &[b'D' as u16, b'4' as u16, b'G' as u16, b'P' as u16, 0];
    const TRAY_ICON_ID: u32 = 1;

    /// The 32 px brand icon embedded for the tray; decoded by the shell via
    /// `CreateIconFromResourceEx` from the crate-embedded asset.
    const TRAY_ICON_PNG: &[u8] = include_bytes!("../assets/tray-32.png");
    const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 0x4D;
    const NOTIFY_ICON_VERSION_4: u32 = 4;

    /// Grace period the worker gives the graceful UI shutdown path before it ends the process.
    ///
    /// This must stay comfortably above normal graceful latency (drain the event in
    /// `PanelApp::update`, set `explicit_exit`, send `ViewportCommand::Close`, let eframe tear the
    /// window down) while remaining short enough that a user who clicked 「退出」 is never left
    /// looking at a process that refuses to die.
    const EXIT_BACKSTOP_GRACE: Duration = Duration::from_millis(1500);

    /// Extra grace granted once the UI thread acknowledges it received the Exit command.
    ///
    /// The acknowledgement never cancels the backstop; it pushes the deadline out exactly once so
    /// a slow-but-progressing graceful shutdown is not cut off, while a UI thread that acknowledges
    /// and then stalls is still terminated.
    const EXIT_BACKSTOP_ACKNOWLEDGED_GRACE: Duration = Duration::from_millis(1500);

    /// Whether a queued tray event must arm the hard-exit watchdog.
    ///
    /// Only the Exit selection may end the process. Open, RefreshNow, HotspotStatus and the
    /// Explorer `TaskbarCreated` recovery must never arm it, otherwise an ordinary tray click
    /// could kill the panel. An unrecognized menu id must not arm it either.
    const fn arms_exit_backstop(event: TrayEvent) -> bool {
        matches!(event, TrayEvent::Menu(MENU_EXIT))
    }

    /// Whether a queued tray event must make the worker natively restore/show/foreground the
    /// registered panel window.
    ///
    /// Open and Exit restore the window. A hidden window produces no UI frames; Exit must resume
    /// them to drain pending local archive writes/clears before normal shutdown and show progress.
    /// The existing watchdog still terminates a stalled UI. Refresh and hotspot status remain
    /// background actions and never show the window.
    /// An unrecognized menu id must not show the window either.
    const fn shows_panel_window(event: TrayEvent) -> bool {
        matches!(event, TrayEvent::Menu(MENU_OPEN | MENU_EXIT))
    }

    /// Hard-exit watchdog state, owned by the tray worker.
    ///
    /// A window hidden to the tray with `ViewportCommand::Visible(false)` stops being repainted,
    /// and eframe cannot deliver a repaint to a window that is not visible, so `PanelApp::update`
    /// — the only place tray commands are drained — may never run again. Queuing the event and
    /// calling `Context::request_repaint` is therefore a best effort, not a guarantee. This state
    /// machine records when Exit was selected and the instant after which the worker itself
    /// terminates the process, so the guarantee lives on the worker thread rather than depending
    /// on the UI thread making progress.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    enum ExitBackstop {
        /// No Exit selection has been queued.
        #[default]
        Idle,
        /// Exit was queued for the UI thread; end the process at `deadline` if it is still alive.
        Armed { deadline: Instant },
        /// The UI thread acknowledged Exit; end the process at the extended `deadline`.
        Acknowledged { deadline: Instant },
    }

    impl ExitBackstop {
        /// Arm the watchdog. The first Exit selection wins: re-arming is a no-op, so repeated
        /// clicks can never postpone termination and there is no double-exit race.
        fn arm(&mut self, now: Instant) {
            if *self == Self::Idle {
                *self = Self::Armed {
                    deadline: now + EXIT_BACKSTOP_GRACE,
                };
            }
        }

        /// Record that the UI thread received Exit. Extends the deadline once and never disarms, so
        /// termination remains guaranteed even if the acknowledged shutdown then stalls. An
        /// acknowledgement without an armed backstop is ignored.
        fn acknowledge(&mut self, now: Instant) {
            if let Self::Armed { deadline } = *self {
                *self = Self::Acknowledged {
                    deadline: deadline.max(now + EXIT_BACKSTOP_ACKNOWLEDGED_GRACE),
                };
            }
        }

        /// Refresh only an existing watchdog while UI frames report pending local archive I/O.
        /// Sender timestamps keep delayed queued heartbeats from reviving a stalled UI.
        fn defer_for_local_io(&mut self, sent_at: Instant) {
            match self {
                Self::Armed { deadline } | Self::Acknowledged { deadline } => {
                    *deadline = (*deadline).max(sent_at + EXIT_BACKSTOP_GRACE);
                }
                Self::Idle => {}
            }
        }

        #[must_use]
        fn is_due(&self, now: Instant) -> bool {
            match *self {
                Self::Idle => false,
                Self::Armed { deadline } | Self::Acknowledged { deadline } => now >= deadline,
            }
        }
    }

    #[derive(Clone)]
    struct CallbackState {
        events: SyncSender<TrayEvent>,
        taskbar_message: u32,
        labels: TrayLabels,
        icon_added: bool,
        error: Arc<Mutex<Option<TrayError>>>,
        /// Invoked on the tray worker thread right after an event is queued, so the UI thread can
        /// force a repaint. A window hidden to the tray stops repainting on its own, which would
        /// otherwise leave queued tray commands (notably Exit) unprocessed.
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
        /// Invoked on the tray worker thread with every queued event, so the panel can fulfil the
        /// commands that would otherwise die with the hidden window (「立即刷新」 straight to the
        /// controller runner, 「热点状态」 on its own thread). The hook must stay bounded,
        /// non-blocking, and panic-free: this thread owns the message pump and the exit backstop.
        action: Option<Arc<dyn Fn(TrayEvent) + Send + Sync>>,
        /// The panel's real viewport window (raw `HWND` value), registered once by the UI thread
        /// at startup. Carried as an opaque integer — never dereferenced off this worker thread —
        /// and used to natively restore/show/foreground the panel on Open/Exit, the only show
        /// path that works while the window is hidden and `PanelApp::update` never runs.
        panel_window: Option<isize>,
        /// Hard-exit watchdog, armed only by an Exit menu selection.
        ///
        /// `run_loop` owns this box and checks the deadline; `window_proc` mutates it through the
        /// `GWLP_USERDATA` pointer. Both run on this worker thread and `window_proc` is only
        /// reached from `DispatchMessageW` inside `run_loop`, so the two never overlap — the same
        /// serialization the existing `icon_added` and `wake` fields already rely on.
        exit_backstop: ExitBackstop,
    }

    enum WorkerCommand {
        Recreate(SyncSender<Result<(), TrayError>>),
        SetTooltip(String, SyncSender<Result<(), TrayError>>),
        ShowBalloon(String, String, SyncSender<Result<(), TrayError>>),
        SetWakeHook(
            Arc<dyn Fn() + Send + Sync>,
            SyncSender<Result<(), TrayError>>,
        ),
        /// Store the panel's real viewport window (raw `HWND` value) so the worker can natively
        /// restore/show/foreground it on 「打开面板」 while the window is hidden.
        RegisterPanelWindow(isize, SyncSender<Result<(), TrayError>>),
        /// Install the per-event action hook that lets the panel fulfil 「立即刷新」 and
        /// 「热点状态」 on the worker thread, without a UI frame.
        SetActionHook(
            Arc<dyn Fn(TrayEvent) + Send + Sync>,
            SyncSender<Result<(), TrayError>>,
        ),
        /// The UI thread drained an Exit command and is shutting down gracefully.
        ///
        /// Deliberately carries no response channel: it must never block the UI thread, and a lost
        /// acknowledgement is safe because the already-armed deadline still ends the process.
        AcknowledgeExit,
        /// Only UI progress on pending local I/O may renew an armed watchdog.
        DeferExitForLocalIo(Instant),
        Shutdown,
    }

    pub struct NativeTray {
        commands: SyncSender<WorkerCommand>,
        events: Receiver<TrayEvent>,
        error: Arc<Mutex<Option<TrayError>>>,
        thread_id: u32,
        worker: Option<JoinHandle<()>>,
    }

    impl fmt::Debug for NativeTray {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("NativeTray")
                .field("thread_id", &self.thread_id)
                .finish_non_exhaustive()
        }
    }

    impl NativeTray {
        pub fn create(labels: TrayLabels) -> Result<Self, TrayError> {
            validate_labels(&labels)?;
            let (commands, command_rx) = mpsc::sync_channel(8);
            let (events, event_rx) = mpsc::sync_channel(16);
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let error = Arc::new(Mutex::new(None));
            let worker_error = Arc::clone(&error);
            let worker = thread::Builder::new()
                .name("dji4g-tray".to_owned())
                .spawn(move || {
                    let worker_ready = ready_tx.clone();
                    let result = run_worker(labels, command_rx, events, worker_error, worker_ready);
                    if let Err(error) = result {
                        let _ = ready_tx.send(Err(error));
                    }
                })
                .map_err(|_| TrayError::new("tray:worker_create_failed"))?;

            match ready_rx.recv_timeout(OP_TIMEOUT) {
                Ok(Ok(thread_id)) => Ok(Self {
                    commands,
                    events: event_rx,
                    error,
                    thread_id,
                    worker: Some(worker),
                }),
                Ok(Err(error)) => {
                    let _ = worker.join();
                    Err(error)
                }
                Err(RecvTimeoutError::Timeout) => {
                    // The worker has not reached its initialization acknowledgement.  Post a
                    // quit message when possible and deliberately detach: this keeps shell
                    // startup bounded even if a broken shell extension blocks native setup.
                    drop(worker);
                    Err(TrayError::new("tray:create_timeout"))
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = worker.join();
                    Err(TrayError::new("tray:create_failed"))
                }
            }
        }

        pub fn try_recv(&self) -> Option<TrayEvent> {
            self.events.try_recv().ok()
        }

        pub fn recreate(&self) -> Result<(), TrayError> {
            let (response_tx, response_rx) = mpsc::sync_channel(1);
            self.commands
                .send(WorkerCommand::Recreate(response_tx))
                .map_err(|_| TrayError::new("tray:worker_stopped"))?;
            response_rx
                .recv_timeout(OP_TIMEOUT)
                .map_err(|error| match error {
                    RecvTimeoutError::Timeout => TrayError::new("tray:recreate_timeout"),
                    RecvTimeoutError::Disconnected => TrayError::new("tray:worker_stopped"),
                })?
        }

        pub fn set_tooltip(&self, tooltip: &str) -> Result<(), TrayError> {
            let tooltip = tooltip.to_owned();
            validate_text(&tooltip, 127)?;
            let (response_tx, response_rx) = mpsc::sync_channel(1);
            self.commands
                .send(WorkerCommand::SetTooltip(tooltip, response_tx))
                .map_err(|_| TrayError::new("tray:worker_stopped"))?;
            response_rx
                .recv_timeout(OP_TIMEOUT)
                .map_err(|error| match error {
                    RecvTimeoutError::Timeout => TrayError::new("tray:tooltip_timeout"),
                    RecvTimeoutError::Disconnected => TrayError::new("tray:worker_stopped"),
                })?
        }

        /// Push a shell balloon notification (rendered as a toast on Windows 10/11).  The text
        /// stays inside the closed verdict/reason vocabulary of the panel, never device
        /// identifiers.  `NIIF_RESPECT_QUIET_TIME` keeps focus-assist hours honoured.
        pub fn show_balloon(&self, title: &str, text: &str) -> Result<(), TrayError> {
            let title = title.to_owned();
            let text = text.to_owned();
            validate_text(&title, 63)?;
            validate_text(&text, 127)?;
            let (response_tx, response_rx) = mpsc::sync_channel(1);
            self.commands
                .send(WorkerCommand::ShowBalloon(title, text, response_tx))
                .map_err(|_| TrayError::new("tray:worker_stopped"))?;
            response_rx
                .recv_timeout(OP_TIMEOUT)
                .map_err(|error| match error {
                    RecvTimeoutError::Timeout => TrayError::new("tray:balloon_timeout"),
                    RecvTimeoutError::Disconnected => TrayError::new("tray:worker_stopped"),
                })?
        }

        pub fn set_wake_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) -> Result<(), TrayError> {
            let (response_tx, response_rx) = mpsc::sync_channel(1);
            self.commands
                .send(WorkerCommand::SetWakeHook(hook, response_tx))
                .map_err(|_| TrayError::new("tray:worker_stopped"))?;
            response_rx
                .recv_timeout(OP_TIMEOUT)
                .map_err(|error| match error {
                    RecvTimeoutError::Timeout => TrayError::new("tray:wake_hook_timeout"),
                    RecvTimeoutError::Disconnected => TrayError::new("tray:worker_stopped"),
                })?
        }

        /// Register the panel's real viewport window (raw `HWND` value) with the worker.
        ///
        /// The worker natively restores, shows, and foregrounds this window when 「打开面板」 (or
        /// a tray-icon double-click) is selected. This is the only show path that works while the
        /// window is hidden: a hidden window produces no frames, so `PanelApp::update` never runs
        /// and the UI thread cannot re-show itself. The handle is stored as an opaque integer and
        /// only ever dereferenced on the worker thread, and it stays valid for the lifetime of
        /// this tray (single-window application: destroying the panel window ends the process).
        pub fn register_panel_window(&self, hwnd: isize) -> Result<(), TrayError> {
            let (response_tx, response_rx) = mpsc::sync_channel(1);
            self.commands
                .send(WorkerCommand::RegisterPanelWindow(hwnd, response_tx))
                .map_err(|_| TrayError::new("tray:worker_stopped"))?;
            response_rx
                .recv_timeout(OP_TIMEOUT)
                .map_err(|error| match error {
                    RecvTimeoutError::Timeout => TrayError::new("tray:register_window_timeout"),
                    RecvTimeoutError::Disconnected => TrayError::new("tray:worker_stopped"),
                })?
        }

        /// Install a hook invoked on the worker thread with every queued tray event.
        ///
        /// A window hidden to the tray stops running `PanelApp::update`, so the queued events for
        /// 「立即刷新」 and 「热点状态」 would otherwise never be read. The hook lets the panel
        /// fulfil those commands without a UI frame. It must stay bounded, non-blocking, and
        /// panic-free: it runs on the same thread as the message pump and the hard-exit watchdog.
        pub fn set_action_hook(
            &self,
            hook: Arc<dyn Fn(TrayEvent) + Send + Sync>,
        ) -> Result<(), TrayError> {
            let (response_tx, response_rx) = mpsc::sync_channel(1);
            self.commands
                .send(WorkerCommand::SetActionHook(hook, response_tx))
                .map_err(|_| TrayError::new("tray:worker_stopped"))?;
            response_rx
                .recv_timeout(OP_TIMEOUT)
                .map_err(|error| match error {
                    RecvTimeoutError::Timeout => TrayError::new("tray:action_hook_timeout"),
                    RecvTimeoutError::Disconnected => TrayError::new("tray:worker_stopped"),
                })?
        }

        /// Report that the UI thread received and acted on an Exit command.
        ///
        /// This is non-blocking and best effort: it never waits on the worker, so it cannot stall
        /// the egui update loop, and it never disarms the watchdog. It only extends the worker's
        /// deadline once, so a graceful shutdown that is still making progress is not cut off by a
        /// hard exit. If the acknowledgement is dropped the original deadline stands and the
        /// worker still terminates the process.
        pub fn acknowledge_exit(&self) {
            let _ = self.commands.try_send(WorkerCommand::AcknowledgeExit);
        }

        /// Non-blocking heartbeat, sent only while local archive I/O is pending at exit.
        /// Losing heartbeats leaves the original 1.5-second watchdog bound in force.
        pub fn defer_exit_for_local_io(&self) {
            let _ = self
                .commands
                .try_send(WorkerCommand::DeferExitForLocalIo(Instant::now()));
        }

        pub fn take_error(&self) -> Option<TrayError> {
            self.error.lock().ok().and_then(|mut value| value.take())
        }
    }

    impl Drop for NativeTray {
        fn drop(&mut self) {
            let _ = self.commands.try_send(WorkerCommand::Shutdown);
            if self.thread_id != 0 {
                // SAFETY: the thread id was returned by the worker after its message queue was
                // created; posting WM_QUIT does not borrow any caller-owned memory.
                unsafe {
                    let _ = PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
                }
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn run_worker(
        labels: TrayLabels,
        commands: Receiver<WorkerCommand>,
        events: SyncSender<TrayEvent>,
        error: Arc<Mutex<Option<TrayError>>>,
        ready: SyncSender<Result<u32, TrayError>>,
    ) -> Result<u32, TrayError> {
        let taskbar_name = wide_null("TaskbarCreated");
        // SAFETY: the string is a stable, NUL-terminated UTF-16 buffer for this call.
        let taskbar_message = unsafe { RegisterWindowMessageW(taskbar_name.as_ptr()) };
        if taskbar_message == 0 {
            return Err(last_error("tray:taskbar_message_failed"));
        }
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        if instance.is_null() {
            return Err(last_error("tray:module_handle_failed"));
        }
        let mut callback = Box::new(CallbackState {
            events,
            taskbar_message,
            labels,
            icon_added: false,
            error: Arc::clone(&error),
            wake: None,
            action: None,
            panel_window: None,
            exit_backstop: ExitBackstop::Idle,
        });
        // A hidden tool window does not need a background brush, cursor, or class menu.
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance as HINSTANCE,
            lpszClassName: TRAY_WINDOW_CLASS.as_ptr(),
            hbrBackground: std::ptr::null_mut() as HBRUSH,
            ..WNDCLASSW::default()
        };
        // SAFETY: all pointers in `class` refer to static buffers or the current module.
        let class_atom = unsafe { RegisterClassW(&class) };
        if class_atom == 0 {
            let code = unsafe { GetLastError() };
            if code != ERROR_CLASS_ALREADY_EXISTS {
                return Err(TrayError::with_os_code("tray:class_register_failed", code));
            }
        }
        // SAFETY: callback remains boxed for the entire window lifetime and the class/title are
        // static NUL-terminated buffers.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                TRAY_WINDOW_CLASS.as_ptr(),
                TRAY_WINDOW_TITLE.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                (&mut *callback as *mut CallbackState).cast::<c_void>(),
            )
        };
        if hwnd.is_null() {
            return Err(last_error("tray:window_create_failed"));
        }
        if let Err(error_value) = add_icon(hwnd, &callback.labels, TRAY_CALLBACK_MESSAGE) {
            // SAFETY: hwnd is an owned window created immediately above.
            unsafe { DestroyWindow(hwnd) };
            return Err(error_value);
        }
        callback.icon_added = true;
        // `PeekMessageW` creates/observes this worker's queue.  The returned id lets Drop wake it
        // even if the command channel is currently unable to accept a shutdown message.
        let thread_id = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
        // Acknowledge readiness before entering the long-lived message loop.  This keeps tray
        // creation bounded while retaining the icon and message window on the worker thread.
        let _ = ready.send(Ok(thread_id));
        run_loop(hwnd, callback, commands, error);
        Ok(thread_id)
    }

    fn run_loop(
        hwnd: HWND,
        mut callback: Box<CallbackState>,
        commands: Receiver<WorkerCommand>,
        error: Arc<Mutex<Option<TrayError>>>,
    ) {
        let mut shutdown = false;
        while !shutdown {
            let mut message = MSG::default();
            loop {
                // SAFETY: `message` is writable storage owned by this loop; NULL hwnd drains the
                // worker queue only.
                let has_message =
                    unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) };
                if has_message <= 0 {
                    break;
                }
                if message.message == WM_QUIT {
                    shutdown = true;
                    break;
                }
                // SAFETY: message was filled by PeekMessageW and remains live for both calls.
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            while let Ok(command) = commands.try_recv() {
                match command {
                    WorkerCommand::Recreate(response) => {
                        let result = recreate_icon(hwnd, &mut callback);
                        if let Err(ref value) = result {
                            set_error(&error, value.clone());
                        }
                        let _ = response.send(result);
                    }
                    WorkerCommand::SetTooltip(tooltip, response) => {
                        let result = modify_tooltip(hwnd, &callback.labels, &tooltip);
                        if let Err(ref value) = result {
                            set_error(&error, value.clone());
                        }
                        let _ = response.send(result);
                    }
                    WorkerCommand::ShowBalloon(title, text, response) => {
                        let result = show_balloon_icon(hwnd, &title, &text);
                        if let Err(ref value) = result {
                            set_error(&error, value.clone());
                        }
                        let _ = response.send(result);
                    }
                    WorkerCommand::SetWakeHook(hook, response) => {
                        callback.wake = Some(hook);
                        let _ = response.send(Ok(()));
                    }
                    WorkerCommand::RegisterPanelWindow(hwnd, response) => {
                        // A zero handle is not a window; store nothing rather than a null value
                        // that `show_panel_window` would have to filter on every click.
                        callback.panel_window = (hwnd != 0).then_some(hwnd);
                        let _ = response.send(Ok(()));
                    }
                    WorkerCommand::SetActionHook(hook, response) => {
                        callback.action = Some(hook);
                        let _ = response.send(Ok(()));
                    }
                    WorkerCommand::AcknowledgeExit => {
                        callback.exit_backstop.acknowledge(Instant::now());
                    }
                    WorkerCommand::DeferExitForLocalIo(sent_at) => {
                        callback.exit_backstop.defer_for_local_io(sent_at);
                    }
                    WorkerCommand::Shutdown => shutdown = true,
                }
            }
            if !shutdown {
                // Hard-exit backstop, evaluated on the tray worker rather than the UI thread on
                // purpose: a window hidden to the tray stops being repainted, so the UI thread may
                // never drain the queued Exit and could never run a watchdog of its own. Checking
                // here makes termination independent of egui repaints entirely.
                if callback.exit_backstop.is_due(Instant::now()) {
                    force_process_exit(hwnd, &mut callback);
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        if callback.icon_added {
            delete_icon(hwnd);
        }
        // SAFETY: hwnd is the worker's owned hidden window; after destruction no callback can
        // access the boxed state because the thread is about to finish.
        unsafe { DestroyWindow(hwnd) };
    }

    /// Last-resort termination after an Exit selection the UI thread never completed.
    ///
    /// Our own notification icon is removed first so the shell is not left with a ghost icon, and
    /// `icon_added` is cleared so the normal `run_loop` teardown cannot delete it a second time.
    /// `std::process::exit` deliberately skips Rust destructors, which is acceptable here because
    /// the log writer already flushes on every append and Windows releases this process's handles
    /// (serial ports, the single-instance mutex) during teardown. This only ever runs when the
    /// graceful path has demonstrably failed, so it cannot cut off a working shutdown.
    fn force_process_exit(hwnd: HWND, callback: &mut CallbackState) -> ! {
        if callback.icon_added {
            delete_icon(hwnd);
            callback.icon_added = false;
        }
        std::process::exit(0);
    }

    fn recreate_icon(hwnd: HWND, callback: &mut CallbackState) -> Result<(), TrayError> {
        if callback.icon_added {
            delete_icon(hwnd);
            callback.icon_added = false;
        }
        add_icon(hwnd, &callback.labels, TRAY_CALLBACK_MESSAGE)?;
        callback.icon_added = true;
        Ok(())
    }

    fn set_error(error: &Arc<Mutex<Option<TrayError>>>, value: TrayError) {
        if let Ok(mut slot) = error.lock() {
            *slot = Some(value);
        }
    }

    fn add_icon(hwnd: HWND, labels: &TrayLabels, callback_message: u32) -> Result<(), TrayError> {
        let mut data = notify_data(hwnd, labels, callback_message)?;
        // SAFETY: `data` is the fully initialized shell structure and all referenced strings are
        // inline fixed arrays.
        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
            return Err(last_error("tray:icon_add_failed"));
        }
        // Request the v4 callback contract after adding the icon.  If this optional upgrade is
        // refused, the icon and ordinary mouse callbacks remain usable.
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        // SAFETY: the union is initialized before the shell reads it.
        data.Anonymous.uVersion = NOTIFY_ICON_VERSION_4;
        // SAFETY: the same live icon data is used for the version negotiation.
        let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };
        Ok(())
    }

    fn modify_tooltip(hwnd: HWND, labels: &TrayLabels, tooltip: &str) -> Result<(), TrayError> {
        let mut updated = labels.clone();
        updated.tooltip = tooltip.to_owned();
        let data = notify_data(hwnd, &updated, TRAY_CALLBACK_MESSAGE)?;
        // SAFETY: the inline data is valid for a NIM_MODIFY operation.
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } == 0 {
            Err(last_error("tray:tooltip_failed"))
        } else {
            Ok(())
        }
    }

    fn show_balloon_icon(hwnd: HWND, title: &str, text: &str) -> Result<(), TrayError> {
        let data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_INFO,
            szInfoTitle: encode_text(title, 63)?.try_into().ok().unwrap_or([0; 64]),
            szInfo: encode_text(text, 255)?.try_into().ok().unwrap_or([0; 256]),
            dwInfoFlags: NIIF_INFO | NIIF_RESPECT_QUIET_TIME,
            ..NOTIFYICONDATAW::default()
        };
        // SAFETY: the inline data is valid for a NIM_MODIFY operation on the worker's own icon.
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } == 0 {
            Err(last_error("tray:balloon_failed"))
        } else {
            Ok(())
        }
    }

    fn delete_icon(hwnd: HWND) {
        let data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            ..NOTIFYICONDATAW::default()
        };
        // SAFETY: only the worker's own icon id/window is addressed.
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    }

    /// The embedded 32 px brand icon, decoded by the shell once per process and shared by every
    /// NIM operation afterwards.  The single HICON intentionally lives for the whole process (the
    /// tray worker never tears down its icon), which keeps every caller leak-free without
    /// refcount bookkeeping.
    fn brand_icon() -> HICON {
        static BRAND_ICON: OnceLock<usize> = OnceLock::new();
        let stored = *BRAND_ICON.get_or_init(|| {
            // SAFETY: the buffer is a bounded embedded PNG; the returned HICON is valid for the
            // process lifetime (never destroyed), so transporting it as `usize` across threads
            // only carries an opaque handle value.
            let icon = unsafe {
                CreateIconFromResourceEx(
                    TRAY_ICON_PNG.as_ptr().cast(),
                    TRAY_ICON_PNG.len() as u32,
                    1,
                    0x0003_0000,
                    0,
                    0,
                    LR_DEFAULTSIZE,
                )
            };
            icon as usize
        });
        stored as HICON
    }

    fn notify_data(
        hwnd: HWND,
        labels: &TrayLabels,
        callback_message: u32,
    ) -> Result<NOTIFYICONDATAW, TrayError> {
        // Prefer the brand icon; if the shell refuses the PNG, fall back to the system icon so the
        // tray keeps working (degraded, not broken).
        let icon = {
            let brand = brand_icon();
            if brand.is_null() {
                // SAFETY: the system application icon is a shared resource and needs no lifetime guard.
                unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) }
            } else {
                brand
            }
        };
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: callback_message,
            hIcon: icon,
            ..NOTIFYICONDATAW::default()
        };
        if data.hIcon.is_null() {
            return Err(last_error("tray:icon_load_failed"));
        }
        let tooltip = encode_text(&labels.tooltip, 127)?;
        data.szTip[..tooltip.len()].copy_from_slice(&tooltip);
        Ok(data)
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> isize {
        if message == WM_NCCREATE {
            // SAFETY: WM_NCCREATE's lParam is a valid CREATESTRUCTW pointer supplied by
            // CreateWindowExW in run_worker.
            let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
            // SAFETY: hwnd is the just-created window and the callback pointer remains boxed.
            unsafe {
                SetWindowLongPtrW(
                    hwnd,
                    GWLP_USERDATA,
                    create.lpCreateParams.cast::<CallbackState>() as isize,
                );
            }
        }
        // SAFETY: the user-data value is written during WM_NCCREATE and remains valid until
        // DestroyWindow returns on this same worker thread.
        let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CallbackState };
        if !state_ptr.is_null() {
            // SAFETY: this callback is serialized on the worker thread that owns the Box.
            let state = unsafe { &mut *state_ptr };
            if message == state.taskbar_message {
                // Explorer removed the old icon.  Re-add it synchronously on the shell worker,
                // then expose a bounded notification for diagnostics/explicit recreation tests.
                match add_icon(hwnd, &state.labels, TRAY_CALLBACK_MESSAGE) {
                    Ok(()) => state.icon_added = true,
                    Err(error) => set_error(&state.error, error),
                }
                push_event(state, TrayEvent::TaskbarCreated);
                return 0;
            }
            if message == TRAY_CALLBACK_MESSAGE {
                match tray_callback_event(lparam) {
                    WM_LBUTTONDBLCLK => {
                        push_event(state, TrayEvent::Menu(MENU_OPEN));
                    }
                    WM_RBUTTONUP | WM_CONTEXTMENU => {
                        show_menu(hwnd, state);
                    }
                    _ => {}
                }
                return 0;
            }
            if message == WM_COMMAND {
                let id = (wparam & 0xFFFF) as u16;
                if (MENU_OPEN..=MENU_EXIT).contains(&id) {
                    // Route through `push_event` rather than sending on the channel directly: this
                    // fallback path must arm the hard-exit watchdog and wake the UI thread exactly
                    // like the `TrackPopupMenu` path, or an Exit arriving here would be stranded.
                    push_event(state, TrayEvent::Menu(id));
                    return 0;
                }
            }
            if message == WM_DESTROY {
                // SAFETY: this callback owns the worker message queue.
                unsafe { PostQuitMessage(0) };
                return 0;
            }
        }
        // SAFETY: unhandled messages are delegated to the standard window procedure.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    /// Decode the tray callback event id from `lParam`.
    ///
    /// `add_icon` requests `NOTIFYICON_VERSION_4`, whose callback contract packs the event
    /// (e.g. `WM_CONTEXTMENU`, or a mouse message) into `LOWORD(lParam)` and the icon id into
    /// `HIWORD(lParam)`, leaving the anchor coordinates in `wParam`. The legacy (version 0)
    /// contract instead puts the bare mouse message in `lParam` with a zero `HIWORD`. Masking to
    /// `LOWORD` yields the event under both contracts; matching the full `lParam` would miss every
    /// v4 mouse event and the context menu would silently never appear.
    const fn tray_callback_event(lparam: LPARAM) -> u32 {
        (lparam as u32) & 0xFFFF
    }

    /// Queue a tray event, fulfil the worker-side duties that cannot wait for a UI frame, then
    /// wake the UI.
    ///
    /// The wake is best effort only: eframe cannot force a repaint of a window hidden with
    /// `ViewportCommand::Visible(false)`, so `PanelApp::update` may never run again and the queued
    /// Exit would be stranded forever. Arming the watchdog is what turns termination into a
    /// guarantee, and it happens *before* the send so a full event channel — where `try_send`
    /// silently drops the event and the UI can never see it — still leaves that guarantee intact.
    ///
    /// The same reasoning applies to the other two hidden-state duties, both fulfilled here
    /// rather than in the (possibly never-running) UI thread:
    /// - 「打开面板」 natively restores/shows/foregrounds the registered panel window *before*
    ///   queueing, so the show happens even when the queued event is dropped or never drained —
    ///   and once the window is visible again the wake below can actually produce a frame.
    /// - the action hook runs after queueing regardless of whether the queue accepted the event,
    ///   so 「立即刷新」/「热点状态」 keep working while the hidden window never drains.
    fn push_event(state: &mut CallbackState, event: TrayEvent) {
        if arms_exit_backstop(event) {
            state.exit_backstop.arm(Instant::now());
        }
        if shows_panel_window(event) {
            show_panel_window(state.panel_window);
        }
        let _ = state.events.try_send(event);
        if let Some(wake) = &state.wake {
            wake();
        }
        if let Some(action) = &state.action {
            action(event);
        }
    }

    /// Restore, show, and foreground the panel's real viewport window from the worker thread.
    ///
    /// This is the only path that can re-show a window hidden with
    /// `ViewportCommand::Visible(false)`: a hidden window produces no frames, so
    /// `PanelApp::update` never runs and the UI thread can never re-show itself. `ShowWindow` and
    /// `SetForegroundWindow` are the documented cross-thread window-management calls; `SW_RESTORE`
    /// covers both the hidden and the minimized window in one step. The registered handle stays
    /// valid for as long as this worker lives: the panel is a single-window application, and
    /// destroying that window ends the process (the exit backstop included).
    fn show_panel_window(panel: Option<isize>) {
        let Some(raw) = panel else {
            return;
        };
        let hwnd = raw as HWND;
        if hwnd.is_null() {
            return;
        }
        // SAFETY: `raw` is the panel viewport HWND registered by the UI thread at startup and
        // stays valid for the process lifetime; both calls only take the window handle.
        unsafe {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }

    fn show_menu(hwnd: HWND, state: &mut CallbackState) {
        // A pending hard exit outranks the menu. `TrackPopupMenu` below runs a modal message loop
        // that suspends `run_loop`, so the pump's deadline check cannot run while a menu is open.
        // If the deadline already passed — for example because the user re-opened the tray menu
        // after clicking 「退出」 — terminate now rather than presenting a menu that cannot be
        // acted on. `is_due` is only ever true after a real Exit selection, so this cannot fire
        // spuriously.
        if state.exit_backstop.is_due(Instant::now()) {
            force_process_exit(hwnd, state);
        }
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return;
        }
        let labels = [
            (MENU_OPEN, state.labels.open.as_str()),
            (MENU_REFRESH_NOW, state.labels.refresh_now.as_str()),
            (MENU_HOTSPOT_STATUS, state.labels.hotspot_status.as_str()),
            (MENU_EXIT, state.labels.exit.as_str()),
        ];
        let encoded = labels
            .iter()
            .map(|(_, value)| encode_text(value, 255))
            .collect::<Result<Vec<_>, _>>();
        let Ok(encoded) = encoded else {
            unsafe { DestroyMenu(menu) };
            return;
        };
        for ((id, _), text) in labels.iter().zip(encoded.iter()) {
            // SAFETY: menu and each temporary UTF-16 label remain valid for the call.
            if unsafe { AppendMenuW(menu, MF_STRING, usize::from(*id), text.as_ptr()) } == 0 {
                unsafe { DestroyMenu(menu) };
                return;
            }
        }
        let mut point = POINT::default();
        // SAFETY: point is writable storage owned by this callback.
        if unsafe { GetCursorPos(&mut point) } != 0 {
            // SAFETY: menu, point, and hwnd are valid for the duration of this call.
            unsafe { SetForegroundWindow(hwnd) };
            let selected = unsafe {
                TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    point.x,
                    point.y,
                    0,
                    hwnd,
                    std::ptr::null(),
                )
            };
            if selected != 0 {
                push_event(state, TrayEvent::Menu(selected as u16));
            }
            // SAFETY: release the foreground-menu ownership required by TrackPopupMenu.
            unsafe { PostMessageW(hwnd, WM_NULL, 0, 0) };
        }
        // SAFETY: menu is owned by this callback and no longer in use.
        unsafe { DestroyMenu(menu) };
    }

    fn validate_labels(labels: &TrayLabels) -> Result<(), TrayError> {
        validate_text(&labels.open, 255)?;
        validate_text(&labels.refresh_now, 255)?;
        validate_text(&labels.hotspot_status, 255)?;
        validate_text(&labels.exit, 255)?;
        validate_text(&labels.tooltip, 127)
    }

    fn validate_text(value: &str, max_units: usize) -> Result<(), TrayError> {
        let _ = encode_text(value, max_units)?;
        Ok(())
    }

    fn encode_text(value: &str, max_units: usize) -> Result<Vec<u16>, TrayError> {
        if value.is_empty() || value.contains('\0') {
            return Err(TrayError::new("tray:text_invalid"));
        }
        let mut units = value.encode_utf16().collect::<Vec<_>>();
        if units.len() > max_units {
            return Err(TrayError::new("tray:text_too_long"));
        }
        // Every consumer treats this as a Win32 wide C string: AppendMenuW scans for a NUL, so an
        // unterminated buffer makes it read past the allocation and render garbage. The length
        // check above is on the content, so content (<= max_units) plus this terminator still fits
        // the 128-wide szTip when max_units is 127.
        units.push(0);
        Ok(units)
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain([0]).collect()
    }

    fn last_error(code: &'static str) -> TrayError {
        // SAFETY: GetLastError has no preconditions and returns the current worker status.
        TrayError::with_os_code(code, unsafe { GetLastError() })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn callback_event_is_decoded_from_loword_for_v4_and_legacy() {
            // NOTIFYICON_VERSION_4 packs the icon id into HIWORD(lParam); the legacy contract
            // leaves HIWORD zero. Masking to LOWORD must yield the event in both layouts, or the
            // right-click context menu silently never appears (the regression this guards).
            let v4_hiword = (TRAY_ICON_ID as LPARAM) << 16;
            assert_eq!(
                tray_callback_event(v4_hiword | WM_CONTEXTMENU as LPARAM),
                WM_CONTEXTMENU
            );
            assert_eq!(
                tray_callback_event(v4_hiword | WM_RBUTTONUP as LPARAM),
                WM_RBUTTONUP
            );
            assert_eq!(
                tray_callback_event(v4_hiword | WM_LBUTTONDBLCLK as LPARAM),
                WM_LBUTTONDBLCLK
            );
            // Legacy (version 0): lParam already is the bare mouse message.
            assert_eq!(
                tray_callback_event(WM_CONTEXTMENU as LPARAM),
                WM_CONTEXTMENU
            );
        }

        #[test]
        fn only_the_exit_selection_arms_the_hard_exit_watchdog() {
            // The watchdog ends the process, so an ordinary tray click must never arm it.
            assert!(arms_exit_backstop(TrayEvent::Menu(MENU_EXIT)));
            for event in [
                TrayEvent::Menu(MENU_OPEN),
                TrayEvent::Menu(MENU_REFRESH_NOW),
                TrayEvent::Menu(MENU_HOTSPOT_STATUS),
                TrayEvent::TaskbarCreated,
                // An id outside the closed menu range must not arm it either.
                TrayEvent::Menu(MENU_EXIT + 1),
                TrayEvent::Menu(0),
            ] {
                assert!(
                    !arms_exit_backstop(event),
                    "{event:?} must not arm the watchdog"
                );
            }
        }

        #[test]
        fn open_and_exit_show_the_panel_window_for_graceful_local_io_drain() {
            // Exit must resume UI frames long enough to finish pending local archive I/O.
            assert!(shows_panel_window(TrayEvent::Menu(MENU_OPEN)));
            assert!(shows_panel_window(TrayEvent::Menu(MENU_EXIT)));
            for event in [
                TrayEvent::Menu(MENU_REFRESH_NOW),
                TrayEvent::Menu(MENU_HOTSPOT_STATUS),
                TrayEvent::TaskbarCreated,
                TrayEvent::Menu(MENU_EXIT + 1),
                TrayEvent::Menu(0),
            ] {
                assert!(
                    !shows_panel_window(event),
                    "{event:?} must not touch the panel window"
                );
            }
        }

        fn test_labels() -> TrayLabels {
            TrayLabels {
                open: "打开面板".to_owned(),
                refresh_now: "立即刷新".to_owned(),
                hotspot_status: "热点状态".to_owned(),
                exit: "退出".to_owned(),
                tooltip: "DJI 一代 4G 面板".to_owned(),
            }
        }

        /// A `CallbackState` wired to a capacity-1 event queue whose single slot is already
        /// occupied: the hidden-window scenario, where nobody drains and the next `try_send`
        /// is guaranteed to drop the event.
        fn full_queue_state(
            seen: Arc<Mutex<Vec<TrayEvent>>>,
        ) -> (CallbackState, Receiver<TrayEvent>) {
            let (events_tx, events_rx) = mpsc::sync_channel(1);
            events_tx
                .try_send(TrayEvent::TaskbarCreated)
                .expect("the capacity-1 queue accepts the filler");
            let state = CallbackState {
                events: events_tx,
                taskbar_message: TRAY_CALLBACK_MESSAGE,
                labels: test_labels(),
                icon_added: false,
                error: Arc::new(Mutex::new(None)),
                wake: None,
                action: Some(Arc::new(move |event| {
                    seen.lock()
                        .expect("the recorder lock is healthy")
                        .push(event);
                })),
                panel_window: None,
                exit_backstop: ExitBackstop::Idle,
            };
            (state, events_rx)
        }

        #[test]
        fn the_action_hook_runs_even_when_the_event_queue_is_full() {
            // A hidden window never drains the queue; the hook is what keeps 「立即刷新」 and
            // 「热点状态」 alive there, so it must not depend on the queue accepting the event.
            let seen = Arc::new(Mutex::new(Vec::new()));
            let (mut state, events_rx) = full_queue_state(Arc::clone(&seen));
            push_event(&mut state, TrayEvent::Menu(MENU_REFRESH_NOW));
            assert_eq!(
                *seen.lock().expect("the recorder lock is healthy"),
                [TrayEvent::Menu(MENU_REFRESH_NOW)]
            );
            // The queue itself only ever held the filler: the refresh event was dropped there.
            assert_eq!(
                events_rx.try_recv().expect("the filler is queued"),
                TrayEvent::TaskbarCreated
            );
            assert!(events_rx.try_recv().is_err());
            // And a non-Exit selection still leaves the watchdog alone.
            assert_eq!(state.exit_backstop, ExitBackstop::Idle);
        }

        #[test]
        fn exit_arms_the_backstop_even_when_the_event_queue_is_full() {
            // The hard-exit guarantee must survive a dropped event: the arming happens before
            // the send, and the action hook (which also runs) never disarms anything.
            let seen = Arc::new(Mutex::new(Vec::new()));
            let (mut state, _events_rx) = full_queue_state(Arc::clone(&seen));
            push_event(&mut state, TrayEvent::Menu(MENU_EXIT));
            assert!(matches!(state.exit_backstop, ExitBackstop::Armed { .. }));
            assert!(!state.exit_backstop.is_due(Instant::now()));
            assert_eq!(
                *seen.lock().expect("the recorder lock is healthy"),
                [TrayEvent::Menu(MENU_EXIT)]
            );
        }

        #[test]
        fn local_io_heartbeats_preserve_watchdog_but_only_while_ui_is_alive() {
            let start = Instant::now();
            for acknowledged in [false, true] {
                let mut backstop = ExitBackstop::Idle;
                backstop.arm(start);
                if acknowledged {
                    backstop.acknowledge(start);
                }
                // Continued 50ms UI heartbeats allow a 3-second save, beyond the original limit.
                for tick in 1..=60 {
                    let now = start + Duration::from_millis(tick * 50);
                    backstop.defer_for_local_io(now);
                    assert!(!backstop.is_due(now));
                }
                let last = start + Duration::from_secs(3);
                assert!(!backstop.is_due(last + EXIT_BACKSTOP_GRACE - Duration::from_millis(1)));
                assert!(backstop.is_due(last + EXIT_BACKSTOP_GRACE));
                assert!(backstop.is_due(last + Duration::from_secs(20)));
                // A delayed old heartbeat cannot restart the grace clock when dequeued later.
                backstop.defer_for_local_io(start);
                assert!(backstop.is_due(last + Duration::from_secs(20)));
            }
            let mut idle = ExitBackstop::Idle;
            idle.defer_for_local_io(start);
            assert_eq!(idle, ExitBackstop::Idle);
        }

        #[test]
        fn idle_backstop_is_never_due() {
            let backstop = ExitBackstop::Idle;
            assert_eq!(backstop, ExitBackstop::default());
            assert!(!backstop.is_due(Instant::now()));
            assert!(!backstop.is_due(Instant::now() + Duration::from_secs(3600)));
        }

        #[test]
        fn armed_backstop_is_due_only_after_the_grace_period() {
            let mut backstop = ExitBackstop::Idle;
            let armed_at = Instant::now();
            backstop.arm(armed_at);
            // Graceful shutdown still has the whole grace period to finish on its own.
            assert!(!backstop.is_due(armed_at));
            assert!(!backstop.is_due(armed_at + EXIT_BACKSTOP_GRACE - Duration::from_millis(1)));
            // ...but termination is guaranteed the moment the grace period expires.
            assert!(backstop.is_due(armed_at + EXIT_BACKSTOP_GRACE));
            assert!(backstop.is_due(armed_at + EXIT_BACKSTOP_GRACE + Duration::from_secs(60)));
        }

        #[test]
        fn acknowledgement_extends_the_deadline_once_and_never_disarms() {
            let mut backstop = ExitBackstop::Idle;
            let armed_at = Instant::now();
            backstop.arm(armed_at);
            let acked_at = armed_at + Duration::from_millis(50);
            backstop.acknowledge(acked_at);
            // The UI reported progress, so the original deadline no longer fires...
            assert!(!backstop.is_due(armed_at + EXIT_BACKSTOP_GRACE));
            // ...but the extension is bounded: an acknowledged shutdown that then stalls is still
            // terminated. This is the invariant that keeps the backstop a guarantee.
            assert!(backstop.is_due(acked_at + EXIT_BACKSTOP_ACKNOWLEDGED_GRACE));
            // A second acknowledgement must not push the deadline out again.
            backstop.acknowledge(acked_at + Duration::from_millis(10));
            assert!(
                !backstop
                    .is_due(acked_at + EXIT_BACKSTOP_ACKNOWLEDGED_GRACE - Duration::from_millis(1))
            );
            assert!(backstop.is_due(acked_at + EXIT_BACKSTOP_ACKNOWLEDGED_GRACE));
        }

        #[test]
        fn acknowledgement_without_an_armed_backstop_is_ignored() {
            // A stray or replayed acknowledgement must not create a deadline out of nothing.
            let mut backstop = ExitBackstop::Idle;
            backstop.acknowledge(Instant::now());
            assert_eq!(backstop, ExitBackstop::Idle);
            assert!(!backstop.is_due(Instant::now() + Duration::from_secs(3600)));
        }

        #[test]
        fn repeated_exit_selections_cannot_postpone_termination() {
            // A user clicking 退出 several times must not keep pushing the deadline forward.
            let mut backstop = ExitBackstop::Idle;
            let first = Instant::now();
            backstop.arm(first);
            backstop.arm(first + Duration::from_secs(1));
            backstop.arm(first + Duration::from_secs(2));
            assert!(backstop.is_due(first + EXIT_BACKSTOP_GRACE));
        }

        #[test]
        fn encode_text_nul_terminates_the_wide_c_string() {
            // AppendMenuW scans for a NUL; an unterminated buffer makes it read past the
            // allocation and render garbage (the 乱码 this guards). "退出" is two UTF-16 units.
            let encoded = encode_text("退出", 255).unwrap();
            assert_eq!(encoded, [0x9000, 0x51FA, 0]);
            assert_eq!(*encoded.last().unwrap(), 0);
        }

        #[test]
        fn encode_text_length_limit_is_checked_on_content_before_the_terminator() {
            // A 127-unit tooltip is allowed and becomes 128 units with the terminator, which still
            // fits the 128-wide szTip; 128 units of content is rejected.
            let max_ok = "a".repeat(127);
            assert_eq!(encode_text(&max_ok, 127).unwrap().len(), 128);
            let too_long = "a".repeat(128);
            assert_eq!(
                encode_text(&too_long, 127).unwrap_err().code,
                "tray:text_too_long"
            );
        }
    }
}

#[cfg(not(windows))]
mod native {
    use super::*;

    #[derive(Debug)]
    pub struct NativeTray;

    impl NativeTray {
        pub fn create(_labels: TrayLabels) -> Result<Self, TrayError> {
            Err(TrayError::new("tray:unsupported_platform"))
        }

        pub fn try_recv(&self) -> Option<TrayEvent> {
            None
        }

        pub fn recreate(&self) -> Result<(), TrayError> {
            Err(TrayError::new("tray:unsupported_platform"))
        }

        pub fn set_tooltip(&self, _tooltip: &str) -> Result<(), TrayError> {
            Err(TrayError::new("tray:unsupported_platform"))
        }
        pub fn show_balloon(&self, _title: &str, _text: &str) -> Result<(), TrayError> {
            Err(TrayError::new("tray:unsupported_platform"))
        }

        pub fn set_wake_hook(&self, _hook: Arc<dyn Fn() + Send + Sync>) -> Result<(), TrayError> {
            Err(TrayError::new("tray:unsupported_platform"))
        }

        pub fn register_panel_window(&self, _hwnd: isize) -> Result<(), TrayError> {
            Err(TrayError::new("tray:unsupported_platform"))
        }

        pub fn set_action_hook(
            &self,
            _hook: Arc<dyn Fn(TrayEvent) + Send + Sync>,
        ) -> Result<(), TrayError> {
            Err(TrayError::new("tray:unsupported_platform"))
        }

        /// No worker thread exists on this platform, so there is no watchdog to extend.
        pub fn acknowledge_exit(&self) {}

        pub fn defer_exit_for_local_io(&self) {}

        pub fn take_error(&self) -> Option<TrayError> {
            None
        }
    }
}

pub use native::NativeTray;

/// Locate a window of the **current process** by its exact title and return its raw `HWND` value.
///
/// The panel uses this once at startup to hand the tray worker its real viewport window, so the
/// worker can natively restore/show/foreground it on 「打开面板」. The lookup must not depend on
/// a UI callback having run: with `--autostart`/`start_minimized` the window is created hidden and
/// `PanelApp::update` may never run at all. `FindWindowW` matches across the whole desktop, so the
/// process-id check rejects identically titled windows owned by other processes. The returned
/// value is an opaque handle for transport across threads; dereferencing it is the tray worker's
/// job (`NativeTray::register_panel_window`).
#[cfg(windows)]
#[must_use]
pub fn find_process_window(title: &str) -> Option<isize> {
    use windows_sys::Win32::{
        Foundation::HWND,
        System::Threading::GetCurrentProcessId,
        UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId},
    };

    if title.is_empty() {
        return None;
    }
    let wide: Vec<u16> = title.encode_utf16().chain([0]).collect();
    // SAFETY: `wide` is a NUL-terminated wide string that outlives the call, and a null class
    // name means "any window class"; the returned handle belongs to the desktop-wide lookup and
    // is only compared, never dereferenced, before the process check below.
    let hwnd: HWND = unsafe { FindWindowW(std::ptr::null(), wide.as_ptr()) };
    if hwnd.is_null() {
        return None;
    }
    let mut process_id: u32 = 0;
    // SAFETY: `hwnd` was just verified non-null and `process_id` is writable storage owned by
    // this call.
    unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) };
    // SAFETY: GetCurrentProcessId has no preconditions.
    let own_process = unsafe { GetCurrentProcessId() };
    (process_id == own_process).then_some(hwnd as isize)
}

/// No Win32 desktop exists on this platform, so there is no window to locate.
#[cfg(not(windows))]
#[must_use]
pub fn find_process_window(_title: &str) -> Option<isize> {
    None
}

impl From<TrayError> for PlatformError {
    fn from(error: TrayError) -> Self {
        Self {
            code: error.code,
            os_code: error.os_code,
        }
    }
}

#[cfg(all(test, windows))]
mod window_lookup_tests {
    #[test]
    fn find_process_window_rejects_titles_no_window_carries() {
        assert_eq!(
            super::find_process_window("Dji4GPanel-测试-不存在的窗口标题-xyz"),
            None
        );
        // An empty title would match any untitled desktop window; the guard rejects it without
        // running the lookup at all.
        assert_eq!(super::find_process_window(""), None);
    }
}
