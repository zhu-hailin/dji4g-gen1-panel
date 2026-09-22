use std::{
    collections::{HashSet, VecDeque},
    fmt, io,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicU8, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

use dji4g_at_protocol::{
    AtCommand, AtEvent, AtFinalCode, AtResponse, ProtocolError, ProtocolErrorKind, RetryPolicy,
    StreamingParser, ToolParseError, ToolResponse, ToolResponseParser, ToolWireRequest,
};
use dji4g_domain::{
    DeviceEpoch, SmsDeleteControl, SmsDeleteReceipt, SmsFragmentKey, SmsReadControl, SmsReadPhase,
    SmsSendPhase, SmsStorageId, SmsTransactionControl,
};

/// Cancellation and write-attempt handle for one tool transaction. The platform crate must not
/// depend on the application crate, so the shared primitive comes from the domain crate.
pub use dji4g_domain::ToolTransactionControl as ToolIoControl;

use crate::SelectedPort;

const REQUEST_QUEUE_CAPACITY: usize = 64;

/// The CMGS `>` prompt wait (research §6.3: the prompt is not a CRLF-terminated line, and a final
/// code can arrive instead). Bounded by wall clock, not by the line parser.
#[cfg(not(test))]
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
pub const PROMPT_TIMEOUT: Duration = Duration::from_millis(50);

/// Final-code wait after the body and Ctrl-Z have been written. Slow networks may take a while to
/// acknowledge; a timeout here must be surfaced without ever resending the body.
#[cfg(not(test))]
pub const PROMPT_RESULT_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
pub const PROMPT_RESULT_TIMEOUT: Duration = Duration::from_millis(50);

/// Ctrl-Z ends the CMGS body (research §6.3).
const SUBMIT_TERMINATOR: u8 = 0x1A;

pub(crate) trait SerialIo: Send + 'static {
    fn cancellation_handle(&self) -> io::Result<Arc<dyn SerialIoCancellation>>;
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn read_chunk(&mut self) -> io::Result<Vec<u8>>;
}

pub(crate) trait SerialIoCancellation: Send + Sync + 'static {
    fn cancel(&self) -> io::Result<()>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActorError {
    LeaseBusy,
    CloseTimeout,
    OsIo {
        kind: io::ErrorKind,
        raw_os_error: Option<i32>,
    },
    QueueFull,
    Closed,
    Io(io::ErrorKind),
    Protocol(ProtocolError),
    FinalCode(AtFinalCode),
    /// A tool transaction that could not be parsed as one text command/response exchange. The
    /// variant carries no response text, only the reason.
    Tool(ToolParseError),
}

impl ActorError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::LeaseBusy => "serial_actor:lease_busy",
            Self::CloseTimeout => "serial_actor:close_timeout",
            Self::OsIo { .. } => "serial_actor:os_io",
            Self::QueueFull => "serial_actor:queue_full",
            Self::Closed => "serial_actor:closed",
            Self::Io(_) => "serial_actor:io",
            Self::Protocol(error) => error.kind.code(),
            Self::FinalCode(_) => "serial_actor:at_final_error",
            Self::Tool(error) => error.code(),
        }
    }

    #[must_use]
    pub const fn protocol_kind(&self) -> Option<ProtocolErrorKind> {
        match self {
            Self::Protocol(error) => Some(error.kind),
            _ => None,
        }
    }
}

impl fmt::Display for ActorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ActorError {}

enum Request {
    SmsHistory {
        expected_sim: Option<[u8; 8]>,
        storage: Option<SmsStorageId>,
        control: SmsReadControl,
        reply: OperationReply<Result<crate::SmsListing, crate::PlatformError>>,
    },
    CheckedDelete {
        expected: SmsFragmentKey,
        control: SmsDeleteControl,
        reply: OperationReply<SmsDeleteReceipt>,
    },
    Execute {
        command: AtCommand,
        reply: OperationReply<AtResponse>,
    },
    Prompt {
        command: AtCommand,
        body: Vec<u8>,
        control: SmsTransactionControl,
        reply: OperationReply<AtResponse>,
    },
    Handshake {
        reply: OperationReply<Vec<String>>,
    },
    Tool {
        request: ToolWireRequest,
        control: ToolIoControl,
        reply: OperationReply<ToolResponse>,
    },
}

/// Owns the only public path from an inventory-proven [`SelectedPort`] to AT I/O.
///
/// Transport injection is intentionally unavailable outside this crate:
///
/// ```compile_fail
/// use dji4g_windows_platform::{SerialIo, SerialIoCancellation};
/// ```
///
/// The raw spawn entry point is also crate-private:
///
/// ```compile_fail
/// use dji4g_windows_platform::AtSessionActor;
///
/// let _ = AtSessionActor::spawn(panic!(), panic!());
/// ```
///
/// The selected-port test injection path is not public either:
///
/// ```compile_fail
/// use dji4g_windows_platform::AtSessionActor;
///
/// let _ = AtSessionActor::spawn_for_selected(panic!(), panic!(), panic!());
/// ```
pub struct AtSessionActor {
    sender: SyncSender<Request>,
    state: Arc<ActorState>,
    cancellation: Arc<dyn SerialIoCancellation>,
    completion: Receiver<()>,
    worker: Option<thread::JoinHandle<()>>,
    drop_timeout: Duration,
}

static SERIAL_LEASES: OnceLock<(Mutex<HashSet<String>>, Condvar)> = OnceLock::new();

struct SerialLease(String);

impl SerialLease {
    fn acquire(path: &str, timeout: Duration) -> Result<Self, ActorError> {
        let key = path.to_ascii_lowercase();
        let (lock, changed) = SERIAL_LEASES.get_or_init(Default::default);
        let held = lock.lock().unwrap_or_else(|e| e.into_inner());
        let (mut held, _) = changed
            .wait_timeout_while(held, timeout, |held| held.contains(&key))
            .unwrap_or_else(|e| e.into_inner());
        if !held.insert(key.clone()) {
            return Err(ActorError::LeaseBusy);
        }
        Ok(Self(key))
    }
}

impl Drop for SerialLease {
    fn drop(&mut self) {
        let (lock, changed) = SERIAL_LEASES.get().expect("lease registry initialized");
        lock.lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
        changed.notify_all();
    }
}

fn os_io(error: io::Error) -> ActorError {
    ActorError::OsIo {
        kind: error.kind(),
        raw_os_error: error.raw_os_error(),
    }
}

pub type AtResponseReceiver = mpsc::Receiver<Result<AtResponse, ActorError>>;
pub type AtHandshakeReceiver = mpsc::Receiver<Result<Vec<String>, ActorError>>;
pub type ToolResponseReceiver = mpsc::Receiver<Result<ToolResponse, ActorError>>;

#[derive(Clone, Eq, PartialEq)]
pub struct AtPortBinding {
    pub epoch: DeviceEpoch,
    pub interface_path: String,
    pub container_id: Option<String>,
    pub identity: Vec<String>,
}

const STATE_RUNNING: u8 = 0;
const STATE_REMOVED: u8 = 1;
const STATE_CLOSED: u8 = 2;

struct ActorState {
    delete_cleanup: Mutex<Option<SmsDeleteControl>>,
    epoch: DeviceEpoch,
    terminal: AtomicU8,
    outstanding: AtomicUsize,
    submission_gate: Mutex<()>,
    binding: Mutex<Option<AtPortBinding>>,
}

struct WorkerCompletion {
    state: Arc<ActorState>,
    completed: mpsc::Sender<()>,
}

impl Drop for WorkerCompletion {
    fn drop(&mut self) {
        if let Some(control) = self
            .state
            .delete_cleanup
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            control.mark_cleanup_complete();
        }
        let _ = self.completed.send(());
    }
}

impl ActorState {
    fn is_running(&self) -> bool {
        self.terminal.load(Ordering::Acquire) == STATE_RUNNING
    }

    fn terminal_error(&self) -> ActorError {
        match self.terminal.load(Ordering::Acquire) {
            STATE_REMOVED => removed_error(self.epoch),
            _ => ActorError::Closed,
        }
    }

    fn transition_locked(&self, terminal: u8) {
        if self
            .terminal
            .compare_exchange(STATE_RUNNING, terminal, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.binding.lock().expect("binding lock poisoned").take();
        }
    }

    fn finish_removed_locked(&self) {
        self.transition_locked(STATE_REMOVED);
    }

    fn finish_closed_locked(&self) {
        self.transition_locked(STATE_CLOSED);
    }
}

struct OperationReply<T> {
    sender: Option<mpsc::Sender<Result<T, ActorError>>>,
    state: Arc<ActorState>,
}

impl<T> OperationReply<T> {
    fn send(mut self, result: Result<T, ActorError>) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(result);
            let previous = self.state.outstanding.fetch_sub(1, Ordering::AcqRel);
            debug_assert!(previous > 0, "outstanding request counter underflow");
        }
    }
}

impl<T> Drop for OperationReply<T> {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(Err(self.state.terminal_error()));
            let previous = self.state.outstanding.fetch_sub(1, Ordering::AcqRel);
            debug_assert!(previous > 0, "outstanding request counter underflow");
        }
    }
}

impl fmt::Debug for AtPortBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let identity: Vec<_> = self
            .identity
            .iter()
            .map(|line| dji4g_at_protocol::redact_at_text(line))
            .collect();
        formatter
            .debug_struct("AtPortBinding")
            .field("epoch", &self.epoch)
            .field("interface_path", &"[REDACTED_DEVICE_INTERFACE]")
            .field(
                "container_id",
                &self
                    .container_id
                    .as_ref()
                    .map(|_| "[REDACTED_CONTAINER_ID]"),
            )
            .field("identity", &identity)
            .finish()
    }
}

impl AtSessionActor {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn spawn(epoch: DeviceEpoch, serial: Box<dyn SerialIo>) -> Self {
        Self::spawn_inner(epoch, serial, None).expect("test AT actor must initialize")
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn spawn_for_selected(
        epoch: DeviceEpoch,
        selected: &SelectedPort,
        serial: Box<dyn SerialIo>,
    ) -> Self {
        Self::spawn_inner(
            epoch,
            serial,
            Some(AtPortBinding {
                epoch,
                interface_path: selected.interface_path().to_owned(),
                container_id: selected.container_id().map(str::to_owned),
                identity: Vec::new(),
            }),
        )
        .expect("test selected AT actor must initialize")
    }

    #[cfg(test)]
    fn spawn_inner(
        epoch: DeviceEpoch,
        serial: Box<dyn SerialIo>,
        binding: Option<AtPortBinding>,
    ) -> Result<Self, ActorError> {
        Self::spawn_with_lease(epoch, serial, binding, None)
    }

    fn spawn_with_lease(
        epoch: DeviceEpoch,
        serial: Box<dyn SerialIo>,
        binding: Option<AtPortBinding>,
        lease: Option<SerialLease>,
    ) -> Result<Self, ActorError> {
        let (sender, receiver) = mpsc::sync_channel(REQUEST_QUEUE_CAPACITY);
        let (cancellation_ready, cancellation_receiver) = mpsc::sync_channel(1);
        let state = Arc::new(ActorState {
            delete_cleanup: Mutex::new(None),
            epoch,
            terminal: AtomicU8::new(STATE_RUNNING),
            outstanding: AtomicUsize::new(0),
            submission_gate: Mutex::new(()),
            binding: Mutex::new(binding),
        });
        let worker_state = Arc::clone(&state);
        let (completed, completion) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("dji4g-at-session".to_owned())
            .spawn(move || {
                let _completion = WorkerCompletion {
                    state: Arc::clone(&worker_state),
                    completed,
                };
                // Local declaration order preserves serial-before-lease destruction on unwind.
                let lease = lease;
                let serial = serial;
                match serial.cancellation_handle() {
                    Ok(cancellation) => {
                        if cancellation_ready.send(Ok(cancellation)).is_ok() {
                            run_actor(epoch, serial, receiver, Arc::clone(&worker_state));
                        } else {
                            drop(serial);
                        }
                    }
                    Err(error) => {
                        let _ = cancellation_ready.send(Err(ActorError::Io(error.kind())));
                        drop(serial);
                    }
                }
                drop(lease);
            })
            .map_err(|error| ActorError::Io(error.kind()))?;
        // The production canceller must own a handle for this exact worker before callers can
        // submit I/O or invalidate the actor. Waiting here closes the registration race.
        let cancellation = cancellation_receiver
            .recv()
            .map_err(|_| ActorError::Closed)??;
        Ok(Self {
            sender,
            state,
            cancellation,
            completion,
            worker: Some(worker),
            drop_timeout: Duration::from_secs(2),
        })
    }

    /// The caller can cancel a native open: its worker handle is registered before opening.
    #[cfg(windows)]
    pub(crate) fn open_selected_delete(
        epoch: DeviceEpoch,
        selected: &SelectedPort,
        control: &SmsDeleteControl,
    ) -> Result<Self, ActorError> {
        let lease = loop {
            if control.is_cancelled() || control.is_expired() {
                return Err(ActorError::Io(io::ErrorKind::Interrupted));
            }
            match SerialLease::acquire(
                selected.interface_path(),
                control.remaining().min(Duration::from_millis(25)),
            ) {
                Ok(lease) => break lease,
                Err(ActorError::LeaseBusy) => continue,
                Err(error) => return Err(error),
            }
        };
        let binding = Some(AtPortBinding {
            epoch,
            interface_path: selected.interface_path().to_owned(),
            container_id: selected.container_id().map(str::to_owned),
            identity: Vec::new(),
        });
        let selected = selected.clone();
        Self::spawn_delete_opener(
            epoch,
            binding,
            Some(lease),
            control,
            || {
                OwnedWorkerThreadHandle::duplicate_current().map(|worker_thread| {
                    Arc::new(WindowsSynchronousIoCancellation { worker_thread })
                        as Arc<dyn SerialIoCancellation>
                })
            },
            move || {
                WindowsSerialPort::open(&selected)
                    .map(|serial| Box::new(serial) as Box<dyn SerialIo>)
            },
        )
    }

    fn spawn_delete_opener(
        epoch: DeviceEpoch,
        binding: Option<AtPortBinding>,
        lease: Option<SerialLease>,
        control: &SmsDeleteControl,
        register_cancellation: impl FnOnce() -> io::Result<Arc<dyn SerialIoCancellation>>
        + Send
        + 'static,
        open: impl FnOnce() -> io::Result<Box<dyn SerialIo>> + Send + 'static,
    ) -> Result<Self, ActorError> {
        let (sender, receiver) = mpsc::sync_channel(REQUEST_QUEUE_CAPACITY);
        let (ready, ready_receiver) = mpsc::sync_channel(1);
        let (completed, completion) = mpsc::channel();
        let state = Arc::new(ActorState {
            epoch,
            terminal: AtomicU8::new(STATE_RUNNING),
            outstanding: AtomicUsize::new(0),
            submission_gate: Mutex::new(()),
            delete_cleanup: Mutex::new(None),
            binding: Mutex::new(binding),
        });
        let worker_state = Arc::clone(&state);
        let worker_control = control.clone();
        let worker = thread::Builder::new()
            .name("dji4g-sms-delete".into())
            .spawn(move || {
                let _completion = WorkerCompletion {
                    state: Arc::clone(&worker_state),
                    completed,
                };
                let lease = lease;
                let cancellation = register_cancellation();
                let ready = match cancellation {
                    Ok(cancellation) => ready.send(Ok(cancellation)).is_ok(),
                    Err(error) => {
                        let _ = ready.send(Err(os_io(error)));
                        false
                    }
                };
                if ready && !worker_control.is_cancelled() && !worker_control.is_expired() {
                    if let Ok(serial) = open() {
                        run_actor(epoch, serial, receiver, Arc::clone(&worker_state));
                    }
                }
                drop(lease);
            })
            .map_err(|error| ActorError::Io(error.kind()))?;
        let cancellation = ready_receiver.recv().map_err(|_| ActorError::Closed)??;
        Ok(Self {
            sender,
            state,
            cancellation,
            completion,
            worker: Some(worker),
            drop_timeout: Duration::ZERO,
        })
    }

    pub(crate) fn execute_sms_history(
        &self,
        expected_sim: Option<[u8; 8]>,
        storage: Option<SmsStorageId>,
        control: SmsReadControl,
    ) -> Result<crate::SmsListing, crate::PlatformError> {
        let (reply, response) = mpsc::channel();
        self.try_send(|state| Request::SmsHistory {
            expected_sim,
            storage,
            control: control.clone(),
            reply: OperationReply {
                sender: Some(reply),
                state,
            },
        })
        .map_err(|e| crate::sms_history::actor_failure(e, &control))?;
        let mut cleanup_deadline = None;
        loop {
            match response.try_recv() {
                Ok(result) => {
                    return result
                        .unwrap_or_else(|e| Err(crate::sms_history::actor_failure(e, &control)));
                }
                Err(TryRecvError::Disconnected) => {
                    return Err(crate::sms_history::actor_failure(
                        ActorError::Closed,
                        &control,
                    ));
                }
                Err(TryRecvError::Empty) => {}
            }
            if (control.is_cancelled()
                || control.is_expired()
                || control.phase() == SmsReadPhase::RestoringStorage)
                && cleanup_deadline.is_none()
            {
                cleanup_deadline = Some(Instant::now() + Duration::from_secs(4));
                // Keep actor state live for restoration. Interrupt only the original blocked I/O.
                if control.phase() != SmsReadPhase::RestoringStorage {
                    let _ = self.cancellation.cancel();
                }
            }
            if cleanup_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                self.invalidate_epoch();
                return Err(
                    if control.restoration() == dji4g_domain::SmsStorageRestoration::Unknown {
                        crate::PlatformError {
                            code: "sms:storage_restore_unknown",
                            os_code: None,
                        }
                    } else {
                        crate::sms_history::actor_failure(ActorError::CloseTimeout, &control)
                    },
                );
            }
            match response.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => {
                    return result
                        .unwrap_or_else(|e| Err(crate::sms_history::actor_failure(e, &control)));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(crate::sms_history::actor_failure(
                        ActorError::Closed,
                        &control,
                    ));
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    pub(crate) fn execute_checked_delete(
        &self,
        expected: SmsFragmentKey,
        control: SmsDeleteControl,
    ) -> SmsDeleteReceipt {
        let (reply, response) = mpsc::channel();
        if let Err(error) = self.try_send(|state| Request::CheckedDelete {
            expected,
            control: control.clone(),
            reply: OperationReply {
                sender: Some(reply),
                state,
            },
        }) {
            return crate::sms_delete::actor_failure(error, &control);
        }
        loop {
            // Consume a deterministic final acknowledgement before checking a simultaneous cancel.
            match response.try_recv() {
                Ok(result) => {
                    return result
                        .unwrap_or_else(|error| crate::sms_delete::actor_failure(error, &control));
                }
                Err(TryRecvError::Disconnected) => {
                    return crate::sms_delete::actor_failure(ActorError::Closed, &control);
                }
                Err(TryRecvError::Empty) => {}
            }
            if control.is_cancelled() || control.is_expired() {
                self.invalidate_epoch();
                return crate::sms_delete::actor_failure(
                    ActorError::Io(io::ErrorKind::Interrupted),
                    &control,
                );
            }
            match response.recv_timeout(control.remaining().min(Duration::from_millis(25))) {
                Ok(result) => {
                    return result
                        .unwrap_or_else(|error| crate::sms_delete::actor_failure(error, &control));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return crate::sms_delete::actor_failure(ActorError::Closed, &control);
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    pub(crate) fn close_delete_and_wait(
        &mut self,
        control: &SmsDeleteControl,
        timeout: Duration,
    ) -> Result<(), ActorError> {
        control.mark_cleanup_pending();
        *self.state.delete_cleanup.lock().unwrap() = Some(control.clone());
        self.drop_timeout = Duration::ZERO;
        let result = self.close_and_wait(timeout);
        if self.worker.is_none() {
            control.mark_cleanup_complete();
        }
        result
    }

    #[cfg(windows)]
    pub fn open_selected(epoch: DeviceEpoch, selected: &SelectedPort) -> Result<Self, ActorError> {
        let lease = SerialLease::acquire(selected.interface_path(), Duration::from_secs(2))?;
        let serial = WindowsSerialPort::open(selected).map_err(os_io)?;
        Self::spawn_with_lease(
            epoch,
            Box::new(serial),
            Some(AtPortBinding {
                epoch,
                interface_path: selected.interface_path().to_owned(),
                container_id: selected.container_id().map(str::to_owned),
                identity: Vec::new(),
            }),
            Some(lease),
        )
    }

    pub fn execute(&self, command: AtCommand) -> Result<AtResponse, ActorError> {
        self.try_execute(command)?
            .recv()
            .map_err(|_| ActorError::Closed)?
    }

    pub fn try_execute(&self, command: AtCommand) -> Result<AtResponseReceiver, ActorError> {
        let (reply, response) = mpsc::channel();
        self.try_send(|state| Request::Execute {
            command,
            reply: OperationReply {
                sender: Some(reply),
                state,
            },
        })?;
        Ok(response)
    }

    /// Run one exclusive prompt transaction: write `command`, wait for the `>` prompt (bounded by
    /// [`PROMPT_TIMEOUT`]), write `body` followed by Ctrl-Z exactly once, then wait for the final
    /// code (bounded by [`PROMPT_RESULT_TIMEOUT`]).
    ///
    /// The request queues with every other transaction on the actor's single worker, so it never
    /// overlaps another command and never blocks another thread's read path. A failed or timed-out
    /// body write is surfaced to the caller and never retried: an unproven send may already have
    /// reached the network (research §6.3).
    pub fn execute_prompt(
        &self,
        command: AtCommand,
        body: Vec<u8>,
    ) -> Result<AtResponse, ActorError> {
        self.execute_prompt_controlled(
            command,
            body,
            SmsTransactionControl::new(PROMPT_TIMEOUT + PROMPT_RESULT_TIMEOUT),
        )
    }

    pub fn execute_prompt_controlled(
        &self,
        command: AtCommand,
        body: Vec<u8>,
        control: SmsTransactionControl,
    ) -> Result<AtResponse, ActorError> {
        let mut deadline_parser =
            StreamingParser::new_with_prompt(self.state.epoch, command.clone());
        check_prompt_control(&mut deadline_parser, &control)?;
        let (reply, response) = mpsc::channel();
        self.try_send(|state| Request::Prompt {
            command,
            body,
            control: control.clone(),
            reply: OperationReply {
                sender: Some(reply),
                state,
            },
        })?;
        loop {
            if let Err(error) = check_prompt_control(&mut deadline_parser, &control) {
                // The worker may be blocked inside native I/O and unable to inspect the token.
                // Cancellation targets that worker; actual port ownership remains with it.
                self.invalidate_epoch();
                return Err(error);
            }
            let remaining = control.deadline().saturating_duration_since(Instant::now());
            match response.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err(ActorError::Closed),
            }
        }
    }

    /// Queue one device-tool transaction.
    ///
    /// The request queues with every other transaction on this actor's single worker, so it can
    /// never overlap a monitoring command or an SMS transaction on the same port. The returned
    /// receiver yields the module's response — including a final error code, which is a valid
    /// answer, not a transport failure.
    ///
    /// Cancellation is cooperative through `control`: the caller cancels the handle, and the
    /// worker notices between reads. The worker is never force-detached, so the port lease stays
    /// held until it really exits.
    pub fn try_execute_tool(
        &self,
        request: ToolWireRequest,
        control: ToolIoControl,
    ) -> Result<ToolResponseReceiver, ActorError> {
        let (reply, response) = mpsc::channel();
        self.try_send(|state| Request::Tool {
            request,
            control,
            reply: OperationReply {
                sender: Some(reply),
                state,
            },
        })?;
        Ok(response)
    }

    pub fn safe_handshake(&self) -> Result<Vec<String>, ActorError> {
        let response = self.try_safe_handshake()?;
        let identity = response.recv().map_err(|_| ActorError::Closed)??;
        if let Some(binding) = self
            .state
            .binding
            .lock()
            .expect("binding lock poisoned")
            .as_mut()
        {
            binding.identity.clone_from(&identity);
        }
        Ok(identity)
    }

    pub fn try_safe_handshake(&self) -> Result<AtHandshakeReceiver, ActorError> {
        let (reply, response) = mpsc::channel();
        self.try_send(|state| Request::Handshake {
            reply: OperationReply {
                sender: Some(reply),
                state,
            },
        })?;
        Ok(response)
    }

    #[must_use]
    pub fn binding(&self) -> Option<AtPortBinding> {
        self.state
            .binding
            .lock()
            .expect("binding lock poisoned")
            .clone()
    }

    pub fn invalidate_epoch(&self) {
        let _gate = self
            .state
            .submission_gate
            .lock()
            .expect("submission gate poisoned");
        self.state.finish_removed_locked();
        drop(_gate);
        let _ = self.cancellation.cancel();
    }

    /// Wait for actual port destruction. A timeout retains worker ownership for a later retry.
    pub fn close_and_wait(&mut self, timeout: Duration) -> Result<(), ActorError> {
        if self.worker.is_none() {
            return Ok(());
        }
        {
            let _gate = self
                .state
                .submission_gate
                .lock()
                .expect("submission gate poisoned");
            self.state.finish_closed_locked();
        }
        let cancellation_error = self.cancellation.cancel().err().map(os_io);
        match self.completion.recv_timeout(timeout) {
            Ok(()) => {
                self.worker
                    .take()
                    .expect("worker exists")
                    .join()
                    .map_err(|_| ActorError::Closed)?;
                cancellation_error.map_or(Ok(()), Err)
            }
            Err(RecvTimeoutError::Timeout) => Err(ActorError::CloseTimeout),
            Err(RecvTimeoutError::Disconnected) => {
                self.worker
                    .take()
                    .expect("worker exists")
                    .join()
                    .map_err(|_| ActorError::Closed)?;
                Err(ActorError::Closed)
            }
        }
    }

    fn try_send(
        &self,
        make_request: impl FnOnce(Arc<ActorState>) -> Request,
    ) -> Result<(), ActorError> {
        let _gate = self
            .state
            .submission_gate
            .lock()
            .expect("submission gate poisoned");
        if !self.state.is_running() {
            return Err(self.state.terminal_error());
        }
        let admitted = self
            .state
            .outstanding
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < REQUEST_QUEUE_CAPACITY).then_some(current + 1)
            })
            .is_ok();
        if !admitted {
            return Err(ActorError::QueueFull);
        }
        let request = make_request(Arc::clone(&self.state));
        match self.sender.try_send(request) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(ActorError::QueueFull),
            Err(TrySendError::Disconnected(_)) => Err(ActorError::Closed),
        }
    }
}

/// Open one candidate port, run the read-only safe handshake, and drop the actor.
///
/// This is the bounded probe behind the handshake-verified AT-port tier: the only bytes ever
/// written are the whitelist `AT\r` / `ATI\r`, and every failure path (NMEA stream, binary
/// garbage, silent port) is bounded by the configured `CommTimeouts`, so a non-AT candidate is
/// rejected in well under two seconds and the module state is never mutated.
#[cfg(windows)]
pub fn probe_at_port(epoch: DeviceEpoch, port: &SelectedPort) -> Result<Vec<String>, ActorError> {
    let actor = AtSessionActor::open_selected(epoch, port)?;
    actor.safe_handshake()
}

impl Drop for AtSessionActor {
    fn drop(&mut self) {
        let _ = self.close_and_wait(self.drop_timeout);
    }
}

fn run_actor(
    epoch: DeviceEpoch,
    mut serial: Box<dyn SerialIo>,
    receiver: Receiver<Request>,
    state: Arc<ActorState>,
) {
    let mut pending = VecDeque::new();
    loop {
        if !state.is_running() {
            break;
        }
        let request = if let Some(request) = pending.pop_front() {
            Some(request)
        } else {
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(request) => Some(request),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };
        if request.is_none() {
            continue;
        }
        let Some(request) = request else { break };
        match request {
            Request::SmsHistory {
                expected_sim,
                storage,
                control,
                reply,
            } => {
                let mut cleanup_control = None;
                let result = crate::sms_history::list_in_session(
                    epoch,
                    expected_sim,
                    storage,
                    &control,
                    |command, cleanup| {
                        let transaction_control = if cleanup {
                            cleanup_control.get_or_insert_with(|| {
                                SmsDeleteControl::new(Duration::from_secs(3))
                            })
                        } else {
                            control.transport_control()
                        };
                        execute_history_command(
                            epoch,
                            serial.as_mut(),
                            &state,
                            command,
                            transaction_control,
                            &control,
                        )
                    },
                );
                reply.send(Ok(result));
                let _gate = state
                    .submission_gate
                    .lock()
                    .expect("submission gate poisoned");
                state.finish_closed_locked();
            }
            Request::CheckedDelete {
                expected,
                control,
                reply,
            } => {
                let receipt =
                    crate::sms_delete::delete_in_session(epoch, &expected, &control, |command| {
                        execute_delete_command(epoch, serial.as_mut(), &state, command, &control)
                    });
                reply.send(Ok(receipt));
                // A checked delete owns the whole session; late bytes never enter another request.
                let _gate = state
                    .submission_gate
                    .lock()
                    .expect("submission gate poisoned");
                state.finish_closed_locked();
            }
            Request::Execute { command, reply } => {
                let mut replies = vec![reply];
                drain_duplicate_reads(&receiver, &mut pending, &command, &mut replies);
                let result = execute_command(epoch, serial.as_mut(), &state, command);
                let removed = result.as_ref().is_err_and(|error| {
                    error.protocol_kind() == Some(ProtocolErrorKind::DeviceRemoved)
                });
                let _gate = state
                    .submission_gate
                    .lock()
                    .expect("submission gate poisoned");
                if removed {
                    state.finish_removed_locked();
                }
                let result = if state.is_running() {
                    result
                } else {
                    Err(state.terminal_error())
                };
                for reply in replies {
                    reply.send(result.clone());
                }
            }
            Request::Prompt {
                command,
                body,
                control,
                reply,
            } => {
                let result =
                    run_prompt_transaction(epoch, serial.as_mut(), &state, command, body, &control);
                let removed = result.as_ref().is_err_and(|error| {
                    error.protocol_kind() == Some(ProtocolErrorKind::DeviceRemoved)
                });
                let _gate = state
                    .submission_gate
                    .lock()
                    .expect("submission gate poisoned");
                if removed {
                    state.finish_removed_locked();
                }
                let result = if state.is_running() {
                    result
                } else {
                    Err(state.terminal_error())
                };
                reply.send(result);
            }
            Request::Tool {
                request,
                control,
                reply,
            } => {
                let result = run_tool_transaction(serial.as_mut(), &state, request, &control);
                let _gate = state
                    .submission_gate
                    .lock()
                    .expect("submission gate poisoned");
                // A concurrent invalidation must not let a stale success through; a failure keeps
                // its own specific error so the caller can tell a refused command from a broken
                // port.
                let result = match result {
                    Ok(response) if state.is_running() => Ok(response),
                    Ok(_) => Err(state.terminal_error()),
                    Err(error) => Err(error),
                };
                if result.is_err() {
                    // Retire this actor whatever went wrong. A failed tool transaction can leave
                    // the module waiting at a prompt, and a late `OK` from this command must never
                    // be read as the answer to the next one. Retiring also ends the worker loop,
                    // so the port lease is released as soon as the worker really exits.
                    state.finish_removed_locked();
                }
                reply.send(result);
            }
            Request::Handshake { reply } => {
                let result = safe_handshake(epoch, serial.as_mut(), &state);
                let removed = result.as_ref().is_err_and(|error| {
                    error.protocol_kind() == Some(ProtocolErrorKind::DeviceRemoved)
                });
                let _gate = state
                    .submission_gate
                    .lock()
                    .expect("submission gate poisoned");
                if removed {
                    state.finish_removed_locked();
                }
                let result = if state.is_running() {
                    result
                } else {
                    Err(state.terminal_error())
                };
                reply.send(result);
            }
        }
    }
}

fn drain_duplicate_reads(
    receiver: &Receiver<Request>,
    pending: &mut VecDeque<Request>,
    command: &AtCommand,
    replies: &mut Vec<OperationReply<AtResponse>>,
) {
    if command.is_write() {
        return;
    }
    loop {
        match receiver.try_recv() {
            Ok(Request::Execute {
                command: queued,
                reply,
            }) if queued == *command => replies.push(reply),
            Ok(other) => {
                pending.push_back(other);
                break;
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
}

fn safe_handshake(
    epoch: DeviceEpoch,
    serial: &mut dyn SerialIo,
    state: &ActorState,
) -> Result<Vec<String>, ActorError> {
    execute_command(epoch, serial, state, AtCommand::Attention)?;
    let identity = execute_command(epoch, serial, state, AtCommand::Identity)?;
    Ok(identity.lines)
}

fn execute_command(
    epoch: DeviceEpoch,
    serial: &mut dyn SerialIo,
    state: &ActorState,
    command: AtCommand,
) -> Result<AtResponse, ActorError> {
    let attempts = match command.retry_policy() {
        RetryPolicy::Never => 1,
        RetryPolicy::OnceAfterQuietPeriod => 2,
    };
    let mut last_error = ActorError::Closed;
    for _ in 0..attempts {
        match execute_once(epoch, serial, state, command.clone()) {
            Ok(response) => return Ok(response),
            Err(error) => {
                let retryable = error.protocol_kind() == Some(ProtocolErrorKind::Timeout);
                last_error = error;
                if !retryable {
                    break;
                }
            }
        }
    }
    Err(last_error)
}

fn execute_once(
    epoch: DeviceEpoch,
    serial: &mut dyn SerialIo,
    state: &ActorState,
    command: AtCommand,
) -> Result<AtResponse, ActorError> {
    let mut parser = StreamingParser::new(epoch, command.clone());
    if !state.is_running() {
        return Err(ActorError::Protocol(parser.finish_removed()));
    }
    serial
        .write_all(command.encode().as_bytes())
        .map_err(|error| io_error(&mut parser, error))?;

    loop {
        if !state.is_running() {
            return Err(ActorError::Protocol(parser.finish_removed()));
        }
        let bytes = serial
            .read_chunk()
            .map_err(|error| io_error(&mut parser, error))?;
        if !state.is_running() {
            return Err(ActorError::Protocol(parser.finish_removed()));
        }
        for event in parser.push(&bytes).map_err(ActorError::Protocol)? {
            if let AtEvent::Response(response) = event {
                if !state.is_running() {
                    return Err(ActorError::Protocol(parser.finish_removed()));
                }
                return if response.final_code == AtFinalCode::Ok {
                    Ok(response)
                } else {
                    Err(ActorError::FinalCode(response.final_code))
                };
            }
        }
    }
}

/// One typed command under the fragment's absolute deadline; no command is retried.
fn execute_delete_command(
    epoch: DeviceEpoch,
    serial: &mut dyn SerialIo,
    state: &ActorState,
    command: AtCommand,
    control: &SmsDeleteControl,
) -> Result<AtResponse, ActorError> {
    let mut parser = StreamingParser::new(epoch, command.clone());
    let check = || {
        if !state.is_running() {
            return Err(state.terminal_error());
        }
        if control.is_cancelled() {
            return Err(ActorError::Io(io::ErrorKind::Interrupted));
        }
        if control.is_expired() {
            return Err(ActorError::Io(io::ErrorKind::TimedOut));
        }
        Ok(())
    };
    {
        // Synchronize invalidation with the attempt flag before entering potentially blocked I/O.
        let _gate = state
            .submission_gate
            .lock()
            .expect("submission gate poisoned");
        check()?;
        if matches!(command, AtCommand::SmsDelete { .. }) {
            control.mark_delete_attempted();
        }
    }
    serial
        .write_all(command.encode().as_bytes())
        .map_err(|error| io_error(&mut parser, error))?;
    loop {
        check()?;
        let bytes = match serial.read_chunk() {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
            Err(error) => return Err(io_error(&mut parser, error)),
        };
        check()?;
        for event in parser.push(&bytes).map_err(ActorError::Protocol)? {
            if let AtEvent::Response(response) = event {
                return if response.final_code == AtFinalCode::Ok {
                    Ok(response)
                } else {
                    Err(ActorError::FinalCode(response.final_code))
                };
            }
        }
    }
}

fn execute_history_command(
    epoch: DeviceEpoch,
    serial: &mut dyn SerialIo,
    state: &ActorState,
    command: AtCommand,
    control: &SmsDeleteControl,
    read_control: &SmsReadControl,
) -> Result<AtResponse, ActorError> {
    let mut parser = StreamingParser::new(epoch, command.clone());
    let check = || {
        if !state.is_running() {
            return Err(state.terminal_error());
        }
        if control.is_cancelled() {
            return Err(ActorError::Io(io::ErrorKind::Interrupted));
        }
        if control.is_expired() {
            return Err(ActorError::Io(io::ErrorKind::TimedOut));
        }
        Ok(())
    };
    {
        // Synchronize invalidation with the attempt flag before entering potentially blocked I/O.
        let _gate = state
            .submission_gate
            .lock()
            .expect("submission gate poisoned");
        check()?;
    }
    serial
        .write_all(command.encode().as_bytes())
        .map_err(|error| io_error(&mut parser, error))?;
    loop {
        check()?;
        let bytes = match serial.read_chunk() {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => continue,
            Err(error) => return Err(io_error(&mut parser, error)),
        };
        check()?;
        let events = parser.push(&bytes).map_err(ActorError::Protocol)?;
        if matches!(command, AtCommand::SmsList) {
            read_control.set_progress(parser.sms_record_count());
        }
        for event in events {
            if let AtEvent::Response(response) = event {
                return if response.final_code == AtFinalCode::Ok {
                    Ok(response)
                } else {
                    Err(ActorError::FinalCode(response.final_code))
                };
            }
        }
    }
}

/// Run exactly one tool transaction: write the validated line once and read until the module's
/// final code.
///
/// The command is written **at most once**, whitelisted reads included. Nothing here resends,
/// because a second write could put the module into a state the caller never asked for; a caller
/// that wants a retry can issue a new task deliberately.
///
/// A final error code (`ERROR`, `+CME ERROR: …`) is returned as `Ok(ToolResponse)`: the module
/// answered, and the caller keeps its explanation. Only transport failures, cancellation, deadline
/// expiry and parser refusals are errors.
fn run_tool_transaction(
    serial: &mut dyn SerialIo,
    state: &ActorState,
    request: ToolWireRequest,
    control: &ToolIoControl,
) -> Result<ToolResponse, ActorError> {
    if !state.is_running() {
        return Err(state.terminal_error());
    }
    if control.is_cancelled() {
        return Err(ActorError::Io(io::ErrorKind::Interrupted));
    }
    if control.is_expired() {
        return Err(ActorError::Io(io::ErrorKind::TimedOut));
    }
    let mut parser = ToolResponseParser::new(&request);
    // Marked before the write call, not after it returns: a partially written line already
    // changed the module's input, so its effect must be treated as unknown from here on.
    control.mark_write_attempted();
    if let Err(error) = serial.write_all(&request.wire_bytes()) {
        return Err(io_error_raw(error));
    }
    read_tool_response(&mut parser, serial, state, control)
}

fn read_tool_response(
    parser: &mut ToolResponseParser,
    serial: &mut dyn SerialIo,
    state: &ActorState,
    control: &ToolIoControl,
) -> Result<ToolResponse, ActorError> {
    loop {
        if control.is_cancelled() {
            return Err(ActorError::Io(io::ErrorKind::Interrupted));
        }
        if control.is_expired() {
            return Err(ActorError::Io(io::ErrorKind::TimedOut));
        }
        if !state.is_running() {
            return Err(state.terminal_error());
        }
        match serial.read_chunk() {
            Ok(bytes) => match parser.push(&bytes) {
                Ok(Some(response)) => return Ok(response),
                Ok(None) => continue,
                Err(error) => return Err(ActorError::Tool(error)),
            },
            Err(error) if is_quiet(&error) => continue,
            Err(error) => return Err(io_error_raw(error)),
        }
    }
}

/// Map an operating-system I/O error to a stable actor error without a parser, for paths that are
/// not driving a `StreamingParser`.
fn io_error_raw(error: io::Error) -> ActorError {
    #[cfg(windows)]
    if error.raw_os_error() == Some(ERROR_OPERATION_ABORTED) {
        return ActorError::Io(io::ErrorKind::Interrupted);
    }
    match error.kind() {
        // A quiet read interval is handled by the caller; reaching here means the deadline passed
        // or the device stopped answering.
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
            ActorError::Io(io::ErrorKind::TimedOut)
        }
        io::ErrorKind::NotConnected | io::ErrorKind::UnexpectedEof => {
            ActorError::Io(io::ErrorKind::NotConnected)
        }
        // A broken pipe is a write failure, not a disconnect: the caller uses the difference to
        // tell "the module stopped answering" from "our bytes never went out".
        kind => ActorError::Io(kind),
    }
}

fn removed_error(epoch: DeviceEpoch) -> ActorError {
    ActorError::Protocol(StreamingParser::new(epoch, AtCommand::Attention).finish_removed())
}

fn io_error(parser: &mut StreamingParser, error: io::Error) -> ActorError {
    #[cfg(windows)]
    if error.raw_os_error() == Some(ERROR_OPERATION_ABORTED) {
        return ActorError::Io(io::ErrorKind::Interrupted);
    }
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
            ActorError::Protocol(parser.finish_timeout())
        }
        io::ErrorKind::NotConnected | io::ErrorKind::BrokenPipe | io::ErrorKind::UnexpectedEof => {
            ActorError::Protocol(parser.finish_removed())
        }
        kind => ActorError::Io(kind),
    }
}

enum PromptOutcome {
    Ready,
    Finished(AtResponse),
}

fn run_prompt_transaction(
    epoch: DeviceEpoch,
    serial: &mut dyn SerialIo,
    state: &ActorState,
    command: AtCommand,
    body: Vec<u8>,
    control: &SmsTransactionControl,
) -> Result<AtResponse, ActorError> {
    let mut parser = StreamingParser::new_with_prompt(epoch, command.clone());
    check_prompt_control(&mut parser, control)?;
    if !state.is_running() {
        return Err(ActorError::Protocol(parser.finish_removed()));
    }
    serial
        .write_all(command.encode().as_bytes())
        .map_err(|error| io_error(&mut parser, error))?;

    if let PromptOutcome::Finished(response) = wait_for_prompt(&mut parser, serial, state, control)?
    {
        return finish_prompt_response(response);
    }

    let mut payload = body;
    payload.push(SUBMIT_TERMINATOR);
    if !state.is_running() {
        return Err(ActorError::Protocol(parser.finish_removed()));
    }
    check_prompt_control(&mut parser, control)?;
    control.set_phase(SmsSendPhase::Submitting);
    control.mark_submission_possible();
    serial
        .write_all(&payload)
        .map_err(|error| io_error(&mut parser, error))?;
    control.set_phase(SmsSendPhase::WaitingForResult);
    let response = wait_for_result(&mut parser, serial, state, control)?;
    finish_prompt_response(response)
}

fn wait_for_prompt(
    parser: &mut StreamingParser,
    serial: &mut dyn SerialIo,
    state: &ActorState,
    control: &SmsTransactionControl,
) -> Result<PromptOutcome, ActorError> {
    let deadline = (Instant::now() + PROMPT_TIMEOUT).min(control.deadline());
    loop {
        let bytes = read_until_deadline(parser, serial, state, deadline, control)?;
        if !state.is_running() {
            return Err(ActorError::Protocol(parser.finish_removed()));
        }
        for event in parser.push(&bytes).map_err(ActorError::Protocol)? {
            match event {
                AtEvent::Prompt => return Ok(PromptOutcome::Ready),
                AtEvent::Response(response) => return Ok(PromptOutcome::Finished(response)),
                AtEvent::Urc(_) => {}
            }
        }
    }
}

fn wait_for_result(
    parser: &mut StreamingParser,
    serial: &mut dyn SerialIo,
    state: &ActorState,
    control: &SmsTransactionControl,
) -> Result<AtResponse, ActorError> {
    let deadline = (Instant::now() + PROMPT_RESULT_TIMEOUT).min(control.deadline());
    loop {
        let bytes = read_until_deadline(parser, serial, state, deadline, control)?;
        if !state.is_running() {
            return Err(ActorError::Protocol(parser.finish_removed()));
        }
        for event in parser.push(&bytes).map_err(ActorError::Protocol)? {
            if let AtEvent::Response(response) = event {
                return Ok(response);
            }
        }
    }
}

fn read_until_deadline(
    parser: &mut StreamingParser,
    serial: &mut dyn SerialIo,
    state: &ActorState,
    deadline: Instant,
    control: &SmsTransactionControl,
) -> Result<Vec<u8>, ActorError> {
    loop {
        check_prompt_control(parser, control)?;
        if !state.is_running() {
            return Err(ActorError::Protocol(parser.finish_removed()));
        }
        if Instant::now() >= deadline {
            return Err(ActorError::Protocol(parser.finish_timeout()));
        }
        let result = serial.read_chunk();
        check_prompt_control(parser, control)?;
        if Instant::now() >= deadline {
            return Err(ActorError::Protocol(parser.finish_timeout()));
        }
        match result {
            Ok(bytes) => return Ok(bytes),
            Err(error) if is_quiet(&error) => continue,
            Err(error) => return Err(io_error(parser, error)),
        }
    }
}

fn check_prompt_control(
    parser: &mut StreamingParser,
    control: &SmsTransactionControl,
) -> Result<(), ActorError> {
    if Instant::now() >= control.deadline() {
        return Err(ActorError::Protocol(parser.finish_timeout()));
    }
    if control.is_cancelled() {
        return Err(ActorError::Io(io::ErrorKind::Interrupted));
    }
    Ok(())
}

fn finish_prompt_response(response: AtResponse) -> Result<AtResponse, ActorError> {
    if response.final_code == AtFinalCode::Ok {
        Ok(response)
    } else {
        Err(ActorError::FinalCode(response.final_code))
    }
}

fn is_quiet(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

#[cfg(windows)]
struct WindowsSerialPort {
    file: std::fs::File,
}

#[cfg(windows)]
struct WindowsSynchronousIoCancellation {
    worker_thread: OwnedWorkerThreadHandle,
}

#[cfg(windows)]
struct OwnedWorkerThreadHandle {
    // Store the opaque HANDLE value as an integer so the RAII owner can be Send + Sync. Windows
    // thread handles may be used from another thread, and ownership remains unique in this type.
    handle: usize,
}

#[cfg(windows)]
impl OwnedWorkerThreadHandle {
    fn duplicate_current() -> io::Result<Self> {
        use std::{os::windows::io::RawHandle, ptr::null_mut};

        let mut duplicate: RawHandle = null_mut();
        // SAFETY: both pseudo handles are valid in the current process for the duration of this
        // call. `duplicate` points to writable storage. DUPLICATE_SAME_ACCESS produces a distinct,
        // independently owned handle to this exact actor worker thread; Drop closes it once.
        let succeeded = unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                GetCurrentThread(),
                GetCurrentProcess(),
                &mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            handle: duplicate as usize,
        })
    }

    fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
        self.handle as std::os::windows::io::RawHandle
    }
}

#[cfg(windows)]
impl Drop for OwnedWorkerThreadHandle {
    fn drop(&mut self) {
        // SAFETY: this value came from one successful DuplicateHandle call, remains owned by this
        // object, and is closed exactly once here after no further cancellation calls can borrow it.
        let _ = unsafe { CloseHandle(self.as_raw_handle()) };
    }
}

#[cfg(windows)]
impl WindowsSerialPort {
    fn open(selected: &SelectedPort) -> io::Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;

        let path = selected.interface_path();
        if path.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "selected port has no enumerated device-interface path",
            ));
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .access_mode(GENERIC_READ | GENERIC_WRITE)
            .open(path)?;
        configure_serial(&file)?;
        Ok(Self { file })
    }
}

#[cfg(windows)]
impl SerialIoCancellation for WindowsSynchronousIoCancellation {
    fn cancel(&self) -> io::Result<()> {
        cancel_synchronous_io_with(self.worker_thread.as_raw_handle(), CancelSynchronousIo)
    }
}

#[cfg(windows)]
impl SerialIo for WindowsSerialPort {
    fn cancellation_handle(&self) -> io::Result<Arc<dyn SerialIoCancellation>> {
        Ok(Arc::new(WindowsSynchronousIoCancellation {
            worker_thread: OwnedWorkerThreadHandle::duplicate_current()?,
        }))
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        io::Write::write_all(&mut &self.file, bytes)
    }

    fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0_u8; 4096];
        let count = io::Read::read(&mut &self.file, &mut buffer)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "serial handle closed",
            ));
        }
        buffer.truncate(count);
        Ok(buffer)
    }
}

#[cfg(windows)]
const GENERIC_READ: u32 = 0x8000_0000;
#[cfg(windows)]
const GENERIC_WRITE: u32 = 0x4000_0000;
#[cfg(windows)]
const ERROR_NOT_FOUND: i32 = 1168;
#[cfg(windows)]
const ERROR_OPERATION_ABORTED: i32 = 995;
#[cfg(windows)]
const DUPLICATE_SAME_ACCESS: u32 = 0x0000_0002;

#[cfg(windows)]
type CancelSynchronousIoFn = unsafe extern "system" fn(std::os::windows::io::RawHandle) -> i32;

#[cfg(windows)]
fn cancel_synchronous_io_with(
    worker_thread: std::os::windows::io::RawHandle,
    cancel: CancelSynchronousIoFn,
) -> io::Result<()> {
    // SAFETY: the caller supplies a live thread HANDLE for the duration of the call. Production
    // passes the RAII-owned duplicate of the actor worker; tests inject only an ABI-compatible seam.
    if unsafe { cancel(worker_thread) } != 0 {
        return Ok(());
    }
    cancel_synchronous_io_error(io::Error::last_os_error())
}

#[cfg(windows)]
fn cancel_synchronous_io_error(error: io::Error) -> io::Result<()> {
    // No operation being pending is an idempotent cancellation success. Any other OS failure must
    // remain visible to the caller rather than being confused with normal invalidation.
    if error.raw_os_error() == Some(ERROR_NOT_FOUND) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(windows)]
#[repr(C)]
struct Dcb {
    length: u32,
    baud_rate: u32,
    flags: u32,
    reserved: u16,
    xon_limit: u16,
    xoff_limit: u16,
    byte_size: u8,
    parity: u8,
    stop_bits: u8,
    xon_char: i8,
    xoff_char: i8,
    error_char: i8,
    eof_char: i8,
    event_char: i8,
    reserved1: u16,
}

#[cfg(windows)]
#[repr(C)]
struct CommTimeouts {
    read_interval_timeout: u32,
    read_total_timeout_multiplier: u32,
    read_total_timeout_constant: u32,
    write_total_timeout_multiplier: u32,
    write_total_timeout_constant: u32,
}

#[cfg(windows)]
fn configure_serial(file: &std::fs::File) -> io::Result<()> {
    use std::{mem::zeroed, os::windows::io::AsRawHandle};

    let handle = file.as_raw_handle();
    // SAFETY: DCB is a plain C data structure that Windows initializes after its size is set.
    let mut dcb: Dcb = unsafe { zeroed() };
    dcb.length = size_of::<Dcb>() as u32;
    // SAFETY: `handle` is a live serial file handle and `dcb` is writable for its declared size.
    if unsafe { GetCommState(handle, &mut dcb) } == 0 {
        return Err(io::Error::last_os_error());
    }
    dcb.baud_rate = 115_200;
    dcb.flags = 0x0000_0001 | 0x0000_0010 | 0x0000_1000;
    dcb.byte_size = 8;
    dcb.parity = 0;
    dcb.stop_bits = 0;
    // SAFETY: same live serial handle; all DCB fields above describe 115200 8N1 with binary mode.
    if unsafe { SetCommState(handle, &dcb) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let timeouts = CommTimeouts {
        read_interval_timeout: 50,
        read_total_timeout_multiplier: 0,
        read_total_timeout_constant: 250,
        write_total_timeout_multiplier: 0,
        write_total_timeout_constant: 2_000,
    };
    // SAFETY: `timeouts` is an initialized C structure and `handle` remains live for the call.
    if unsafe { SetCommTimeouts(handle, &timeouts) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
use std::mem::size_of;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> std::os::windows::io::RawHandle;
    fn GetCurrentThread() -> std::os::windows::io::RawHandle;
    fn DuplicateHandle(
        source_process: std::os::windows::io::RawHandle,
        source_handle: std::os::windows::io::RawHandle,
        target_process: std::os::windows::io::RawHandle,
        target_handle: *mut std::os::windows::io::RawHandle,
        desired_access: u32,
        inherit_handle: i32,
        options: u32,
    ) -> i32;
    fn CloseHandle(handle: std::os::windows::io::RawHandle) -> i32;
    fn CancelSynchronousIo(thread: std::os::windows::io::RawHandle) -> i32;
    fn GetCommState(handle: std::os::windows::io::RawHandle, dcb: *mut Dcb) -> i32;
    fn SetCommState(handle: std::os::windows::io::RawHandle, dcb: *const Dcb) -> i32;
    fn SetCommTimeouts(
        handle: std::os::windows::io::RawHandle,
        timeouts: *const CommTimeouts,
    ) -> i32;
}

#[cfg(all(test, windows))]
mod windows_cancellation_tests {
    use std::{io, os::windows::io::RawHandle, sync::atomic::AtomicUsize};

    use dji4g_at_protocol::{AtCommand, StreamingParser};
    use dji4g_domain::DeviceEpoch;

    use super::{
        ActorError, ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, cancel_synchronous_io_error,
        cancel_synchronous_io_with, io_error,
    };

    static CANCELLED_THREAD: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "system" fn record_cancel_synchronous_io(thread: RawHandle) -> i32 {
        CANCELLED_THREAD.store(thread as usize, std::sync::atomic::Ordering::Release);
        1
    }

    #[test]
    fn production_cancellation_seam_targets_synchronous_worker_io() {
        let worker = 0x5a17usize as RawHandle;

        cancel_synchronous_io_with(worker, record_cancel_synchronous_io).unwrap();

        assert_eq!(
            CANCELLED_THREAD.load(std::sync::atomic::Ordering::Acquire),
            worker as usize
        );
    }

    #[test]
    fn no_pending_synchronous_io_is_an_idempotent_cancel_success() {
        assert!(cancel_synchronous_io_error(io::Error::from_raw_os_error(ERROR_NOT_FOUND)).is_ok());
        assert!(
            cancel_synchronous_io_error(io::Error::from_raw_os_error(5)).is_err(),
            "real cancellation failures must propagate"
        );
    }

    #[test]
    fn aborted_synchronous_read_has_stable_interrupted_mapping() {
        let mut parser = StreamingParser::new(DeviceEpoch(3), AtCommand::Attention);
        let error = io_error(
            &mut parser,
            io::Error::from_raw_os_error(ERROR_OPERATION_ABORTED),
        );

        assert_eq!(error, ActorError::Io(io::ErrorKind::Interrupted));
    }
}

#[cfg(test)]
#[path = "serial_actor_tests.rs"]
mod actor_tests;
