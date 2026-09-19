use std::{
    collections::VecDeque,
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use super::{ActorError, AtSessionActor, SerialIo, SerialIoCancellation};
use crate::pnp::selected_test_port;
use dji4g_at_protocol::{AtCommand, AtFinalCode, ProtocolErrorKind};
use dji4g_domain::DeviceEpoch;

struct PassiveCancellation;

impl SerialIoCancellation for PassiveCancellation {
    fn cancel(&self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct FakeState {
    writes: Vec<Vec<u8>>,
    write_attempts: usize,
    fail_write_at: Option<usize>,
    reads: VecDeque<io::Result<Vec<u8>>>,
    quiet_when_empty: bool,
}

#[derive(Default)]
struct BlockingState {
    writes: Vec<Vec<u8>>,
    reads: VecDeque<Vec<u8>>,
    released: bool,
}

struct BlockingSerial(Arc<(Mutex<BlockingState>, Condvar)>);

struct BlockingCancellation(Arc<(Mutex<BlockingState>, Condvar)>);

impl SerialIoCancellation for BlockingCancellation {
    fn cancel(&self) -> io::Result<()> {
        let (state, changed) = &*self.0;
        state.lock().unwrap().released = true;
        changed.notify_all();
        Ok(())
    }
}

impl SerialIo for BlockingSerial {
    fn cancellation_handle(&self) -> io::Result<Arc<dyn SerialIoCancellation>> {
        Ok(Arc::new(BlockingCancellation(self.0.clone())))
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        let (state, changed) = &*self.0;
        state.lock().unwrap().writes.push(bytes.to_vec());
        changed.notify_all();
        Ok(())
    }

    fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
        let (state, changed) = &*self.0;
        let state = state.lock().unwrap();
        let mut state = changed.wait_while(state, |state| !state.released).unwrap();
        state
            .reads
            .pop_front()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "removed"))
    }
}

fn blocking_actor(reads: &[&[u8]]) -> (AtSessionActor, Arc<(Mutex<BlockingState>, Condvar)>) {
    let state = Arc::new((
        Mutex::new(BlockingState {
            reads: reads.iter().map(|bytes| bytes.to_vec()).collect(),
            ..BlockingState::default()
        }),
        Condvar::new(),
    ));
    let actor = AtSessionActor::spawn(DeviceEpoch(8), Box::new(BlockingSerial(state.clone())));
    (actor, state)
}

fn wait_for_writes(state: &Arc<(Mutex<BlockingState>, Condvar)>, count: usize) {
    let (lock, changed) = &**state;
    let state = lock.lock().unwrap();
    let (state, timeout) = changed
        .wait_timeout_while(state, Duration::from_secs(2), |state| {
            state.writes.len() < count
        })
        .unwrap();
    assert!(!timeout.timed_out(), "actor did not write in time");
    assert_eq!(state.writes.len(), count);
}

struct FakeSerial(Arc<Mutex<FakeState>>);

impl SerialIo for FakeSerial {
    fn cancellation_handle(&self) -> io::Result<Arc<dyn SerialIoCancellation>> {
        Ok(Arc::new(PassiveCancellation))
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut state = self.0.lock().unwrap();
        let attempt = state.write_attempts;
        state.write_attempts += 1;
        if state.fail_write_at == Some(attempt) {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "write failed"));
        }
        state.writes.push(bytes.to_vec());
        Ok(())
    }

    fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
        let next = self.0.lock().unwrap().reads.pop_front();
        if let Some(result) = next {
            return result;
        }
        let quiet = self.0.lock().unwrap().quiet_when_empty;
        assert!(quiet, "FakeSerial has no queued read");
        thread::sleep(Duration::from_millis(5));
        Err(io::Error::new(io::ErrorKind::TimedOut, "quiet"))
    }
}

fn actor_with_state(state: FakeState) -> (AtSessionActor, Arc<Mutex<FakeState>>) {
    let state = Arc::new(Mutex::new(state));
    let actor = AtSessionActor::spawn(DeviceEpoch(7), Box::new(FakeSerial(state.clone())));
    (actor, state)
}

fn actor_with_reads(
    reads: impl IntoIterator<Item = io::Result<Vec<u8>>>,
) -> (AtSessionActor, Arc<Mutex<FakeState>>) {
    actor_with_state(FakeState {
        reads: reads.into_iter().collect(),
        ..FakeState::default()
    })
}

#[test]
fn idempotent_read_retries_exactly_once() {
    let (actor, state) = actor_with_reads([
        Err(io::Error::new(io::ErrorKind::TimedOut, "quiet")),
        Ok(b"+CSQ: 20,99\r\nOK\r\n".to_vec()),
    ]);

    let response = actor.execute(AtCommand::SignalQuality).unwrap();
    assert_eq!(response.lines, vec!["+CSQ: 20,99"]);
    assert_eq!(state.lock().unwrap().writes.len(), 2);
}

#[test]
fn write_command_is_never_retried() {
    let (actor, state) = actor_with_reads([Err(io::Error::new(io::ErrorKind::TimedOut, "quiet"))]);

    let result = actor.execute(AtCommand::RestartModule);
    assert!(result.is_err());
    assert_eq!(state.lock().unwrap().writes.len(), 1);
}

#[test]
fn epoch_invalidation_finishes_active_parser_as_removed() {
    let (actor, _) =
        actor_with_reads([Err(io::Error::new(io::ErrorKind::NotConnected, "removed"))]);

    let error = actor.execute(AtCommand::SignalQuality).unwrap_err();
    assert_eq!(
        error.protocol_kind(),
        Some(ProtocolErrorKind::DeviceRemoved)
    );
}

#[test]
fn safe_handshake_sends_only_at_then_ati() {
    let (actor, state) = actor_with_reads([
        Ok(b"OK\r\n".to_vec()),
        Ok(b"Quectel EC200A\r\nOK\r\n".to_vec()),
    ]);

    let identity = actor.safe_handshake().unwrap();
    assert_eq!(identity, vec!["Quectel EC200A"]);
    assert_eq!(
        state.lock().unwrap().writes,
        vec![b"AT\r".to_vec(), b"ATI\r".to_vec()]
    );
}

#[test]
fn successful_handshake_records_epoch_topology_and_identity_binding() {
    let state = Arc::new(Mutex::new(FakeState {
        reads: [
            Ok(b"OK\r\n".to_vec()),
            Ok(b"Quectel EC200A\r\nOK\r\n".to_vec()),
        ]
        .into_iter()
        .collect(),
        ..FakeState::default()
    }));
    let selected = selected_test_port();
    let actor =
        AtSessionActor::spawn_for_selected(DeviceEpoch(42), &selected, Box::new(FakeSerial(state)));

    actor.safe_handshake().unwrap();
    let binding = actor.binding().expect("selected actor must retain binding");
    assert_eq!(binding.epoch, DeviceEpoch(42));
    assert_eq!(binding.interface_path, selected.interface_path());
    assert_eq!(binding.container_id.as_deref(), selected.container_id());
    assert_eq!(binding.identity, vec!["Quectel EC200A"]);
}

#[test]
fn duplicate_queued_reads_share_one_serial_transaction() {
    let response = b"+CSQ: 20,99\r\nOK\r\n";
    let (actor, state) = blocking_actor(&[response, response]);
    let first = actor.try_execute(AtCommand::SignalQuality).unwrap();
    wait_for_writes(&state, 1);
    let second = actor.try_execute(AtCommand::SignalQuality).unwrap();
    let third = actor.try_execute(AtCommand::SignalQuality).unwrap();
    {
        let (lock, changed) = &*state;
        lock.lock().unwrap().released = true;
        changed.notify_all();
    }

    assert!(first.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
    assert!(second.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
    assert!(third.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
    assert_eq!(state.0.lock().unwrap().writes.len(), 2);
}

#[test]
fn sixty_fifth_waiting_request_is_rejected() {
    let response = b"+CSQ: 20,99\r\nOK\r\n";
    let (actor, state) = blocking_actor(&[response]);
    let _active = actor.try_execute(AtCommand::SignalQuality).unwrap();
    wait_for_writes(&state, 1);
    let mut waiting = Vec::new();
    for _ in 0..63 {
        waiting.push(actor.try_execute(AtCommand::SignalQuality).unwrap());
    }
    let error = actor.try_execute(AtCommand::SignalQuality).unwrap_err();
    assert_eq!(error.code(), "serial_actor:queue_full");

    actor.invalidate_epoch();
    let (lock, changed) = &*state;
    lock.lock().unwrap().released = true;
    changed.notify_all();
}

#[test]
fn handshake_and_coalesced_reads_share_the_same_sixty_four_operation_limit() {
    let response = b"+CSQ: 20,99\r\nOK\r\n";
    let (actor, state) = blocking_actor(&[response]);
    let _active = actor.try_execute(AtCommand::SignalQuality).unwrap();
    wait_for_writes(&state, 1);
    let mut waiting = Vec::new();
    for _ in 0..62 {
        waiting.push(actor.try_execute(AtCommand::SignalQuality).unwrap());
    }
    let _handshake = actor.try_safe_handshake().unwrap();

    assert_eq!(
        actor
            .try_execute(AtCommand::SignalQuality)
            .unwrap_err()
            .code(),
        "serial_actor:queue_full"
    );

    actor.invalidate_epoch();
    let (lock, changed) = &*state;
    lock.lock().unwrap().released = true;
    changed.notify_all();
}

#[derive(Default)]
struct InvalidationState {
    writes: Vec<Vec<u8>>,
    read_started: bool,
    cancellation_requested: bool,
    cancellation_registered_on: Option<thread::ThreadId>,
}

struct ValidAfterInvalidationSerial {
    state: Arc<(Mutex<InvalidationState>, Condvar)>,
    dropped: Arc<AtomicBool>,
}

struct InvalidationCancellation(Arc<(Mutex<InvalidationState>, Condvar)>);

impl SerialIoCancellation for InvalidationCancellation {
    fn cancel(&self) -> io::Result<()> {
        let (state, changed) = &*self.0;
        state.lock().unwrap().cancellation_requested = true;
        changed.notify_all();
        Ok(())
    }
}

impl Drop for ValidAfterInvalidationSerial {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Release);
    }
}

impl SerialIo for ValidAfterInvalidationSerial {
    fn cancellation_handle(&self) -> io::Result<Arc<dyn SerialIoCancellation>> {
        self.state.0.lock().unwrap().cancellation_registered_on = Some(thread::current().id());
        Ok(Arc::new(InvalidationCancellation(self.state.clone())))
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.state.0.lock().unwrap().writes.push(bytes.to_vec());
        Ok(())
    }

    fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
        let (lock, changed) = &*self.state;
        let mut state = lock.lock().unwrap();
        state.read_started = true;
        changed.notify_all();
        let state = changed
            .wait_while(state, |state| !state.cancellation_requested)
            .unwrap();
        drop(state);
        Ok(b"+CSQ: 20,99\r\nOK\r\n".to_vec())
    }
}

#[test]
fn full_queue_invalidation_clears_binding_rejects_success_and_closes_transport() {
    let caller_thread = thread::current().id();
    let state = Arc::new((Mutex::new(InvalidationState::default()), Condvar::new()));
    let dropped = Arc::new(AtomicBool::new(false));
    let selected = selected_test_port();
    let actor = AtSessionActor::spawn_for_selected(
        DeviceEpoch(99),
        &selected,
        Box::new(ValidAfterInvalidationSerial {
            state: state.clone(),
            dropped: dropped.clone(),
        }),
    );
    assert_ne!(
        state
            .0
            .lock()
            .unwrap()
            .cancellation_registered_on
            .expect("actor must register cancellation before spawn returns"),
        caller_thread,
        "synchronous I/O cancellation must be registered from the worker thread"
    );

    let mut replies = Vec::new();
    replies.push(actor.try_execute(AtCommand::SignalQuality).unwrap());
    {
        let (lock, changed) = &*state;
        let state = lock.lock().unwrap();
        let (state, timeout) = changed
            .wait_timeout_while(state, Duration::from_secs(2), |state| !state.read_started)
            .unwrap();
        assert!(!timeout.timed_out());
        drop(state);
    }
    for _ in 1..64 {
        replies.push(actor.try_execute(AtCommand::SignalQuality).unwrap());
    }
    assert_eq!(
        actor
            .try_execute(AtCommand::SignalQuality)
            .unwrap_err()
            .code(),
        "serial_actor:queue_full"
    );

    actor.invalidate_epoch();
    assert!(
        actor.binding().is_none(),
        "binding must clear synchronously"
    );
    assert!(
        state.0.lock().unwrap().cancellation_requested,
        "invalidate_epoch must synchronously invoke the paired canceller"
    );
    assert_eq!(
        actor
            .try_execute(AtCommand::SignalQuality)
            .unwrap_err()
            .protocol_kind(),
        Some(ProtocolErrorKind::DeviceRemoved)
    );
    for reply in replies {
        let result = reply.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            result.unwrap_err().protocol_kind(),
            Some(ProtocolErrorKind::DeviceRemoved)
        );
    }
    for _ in 0..100 {
        if dropped.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        dropped.load(Ordering::Acquire),
        "serial transport not dropped"
    );
}

#[test]
fn removal_io_error_is_terminal_and_rejects_later_requests() {
    let (actor, _) = actor_with_reads([Err(io::Error::new(io::ErrorKind::BrokenPipe, "removed"))]);

    let first = actor.execute(AtCommand::SignalQuality).unwrap_err();
    assert_eq!(
        first.protocol_kind(),
        Some(ProtocolErrorKind::DeviceRemoved)
    );
    let later = actor.try_execute(AtCommand::SignalQuality).unwrap_err();
    assert_eq!(
        later.protocol_kind(),
        Some(ProtocolErrorKind::DeviceRemoved)
    );
}

#[test]
fn orderly_drop_keeps_closed_when_cancelled_read_reports_removal() {
    let (actor, state) = blocking_actor(&[]);
    let reply = actor.try_execute(AtCommand::SignalQuality).unwrap();
    wait_for_writes(&state, 1);

    drop(actor);

    let error = reply
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code(), "serial_actor:closed");
    assert_eq!(error.protocol_kind(), None);
}

#[test]
fn prompt_transaction_writes_the_body_once_and_returns_the_final_response() {
    let body = b"0011000B916800000000FF".to_vec();
    let (actor, state) = actor_with_reads([
        Ok(b"\r\n>".to_vec()),
        Ok(b"\r\n+CMGS: 42\r\nOK\r\n".to_vec()),
    ]);

    let response = actor
        .execute_prompt(AtCommand::Identity, body.clone())
        .unwrap();

    assert_eq!(response.lines, vec!["+CMGS: 42"]);
    let mut submitted = body;
    submitted.push(0x1A);
    assert_eq!(
        state.lock().unwrap().writes,
        vec![b"ATI\r".to_vec(), submitted]
    );
}

#[test]
fn urc_before_the_prompt_does_not_corrupt_the_transaction() {
    let (actor, state) = actor_with_reads([
        Ok(b"\r\n+QIURC: \"recv\",0\r\n".to_vec()),
        Ok(b"\r\n>".to_vec()),
        Ok(b"\r\n+CMGS: 7\r\nOK\r\n".to_vec()),
    ]);

    let response = actor
        .execute_prompt(AtCommand::Identity, b"41".to_vec())
        .unwrap();

    assert_eq!(response.lines, vec!["+CMGS: 7"]);
    assert_eq!(state.lock().unwrap().write_attempts, 2);
}

#[test]
fn final_code_before_the_prompt_is_returned_without_writing_the_body() {
    let (actor, state) = actor_with_reads([Ok(b"\r\nERROR\r\n".to_vec())]);

    let error = actor
        .execute_prompt(AtCommand::Identity, b"41".to_vec())
        .unwrap_err();

    assert_eq!(error, ActorError::FinalCode(AtFinalCode::Error));
    let state = state.lock().unwrap();
    assert_eq!(state.write_attempts, 1);
    assert!(state.writes == vec![b"ATI\r".to_vec()]);
}

#[test]
fn prompt_timeout_returns_timeout_without_sending_the_body() {
    let (actor, state) = actor_with_state(FakeState {
        quiet_when_empty: true,
        ..FakeState::default()
    });

    let error = actor
        .execute_prompt(AtCommand::Identity, b"41".to_vec())
        .unwrap_err();

    assert_eq!(error.protocol_kind(), Some(ProtocolErrorKind::Timeout));
    let state = state.lock().unwrap();
    assert_eq!(state.write_attempts, 1);
    assert!(state.writes == vec![b"ATI\r".to_vec()]);
}

#[test]
fn failed_body_write_is_reported_and_never_retried() {
    let (actor, state) = actor_with_state(FakeState {
        fail_write_at: Some(1),
        reads: [Ok(b"\r\n>".to_vec())].into_iter().collect(),
        ..FakeState::default()
    });

    let error = actor
        .execute_prompt(AtCommand::Identity, b"41".to_vec())
        .unwrap_err();

    assert_eq!(
        error.protocol_kind(),
        Some(ProtocolErrorKind::DeviceRemoved)
    );
    let state = state.lock().unwrap();
    assert_eq!(state.write_attempts, 2);
    assert!(state.writes == vec![b"ATI\r".to_vec()]);
}

#[test]
fn missing_final_code_after_the_body_times_out_without_resending_it() {
    let (actor, state) = actor_with_state(FakeState {
        quiet_when_empty: true,
        reads: [Ok(b"\r\n>".to_vec())].into_iter().collect(),
        ..FakeState::default()
    });

    let error = actor
        .execute_prompt(AtCommand::Identity, b"41".to_vec())
        .unwrap_err();

    assert_eq!(error.protocol_kind(), Some(ProtocolErrorKind::Timeout));
    assert_eq!(state.lock().unwrap().write_attempts, 2);
}

struct SlowDropSerial(Arc<AtomicBool>);
impl SerialIo for SlowDropSerial {
    fn cancellation_handle(&self) -> io::Result<Arc<dyn SerialIoCancellation>> {
        Ok(Arc::new(PassiveCancellation))
    }
    fn write_all(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
    fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
        Ok(b"OK\r\n".to_vec())
    }
}
impl Drop for SlowDropSerial {
    fn drop(&mut self) {
        thread::sleep(Duration::from_millis(80));
        self.0.store(true, Ordering::Release);
    }
}

#[test]
fn dropping_actor_waits_for_actual_serial_drop() {
    let dropped = Arc::new(AtomicBool::new(false));
    let actor = AtSessionActor::spawn(DeviceEpoch(70), Box::new(SlowDropSerial(dropped.clone())));
    drop(actor);
    assert!(
        dropped.load(Ordering::Acquire),
        "actor returned before port was actually closed"
    );
}

#[test]
fn close_timeout_keeps_lease_until_actual_port_drop() {
    let dropped = Arc::new(AtomicBool::new(false));
    let path = "test-lifecycle-slow-drop";
    let lease = super::SerialLease::acquire(path, Duration::ZERO).unwrap();
    let mut actor = AtSessionActor::spawn_with_lease(
        DeviceEpoch(71),
        Box::new(SlowDropSerial(dropped.clone())),
        None,
        Some(lease),
    )
    .unwrap();
    assert_eq!(
        actor.close_and_wait(Duration::from_millis(1)),
        Err(ActorError::CloseTimeout)
    );
    assert!(matches!(
        super::SerialLease::acquire(path, Duration::from_millis(1)),
        Err(ActorError::LeaseBusy)
    ));
    actor.close_and_wait(Duration::from_secs(1)).unwrap();
    assert!(dropped.load(Ordering::Acquire));
    let _reopened = super::SerialLease::acquire(path, Duration::ZERO).unwrap();
    actor.close_and_wait(Duration::ZERO).unwrap();
}

#[test]
fn serial_lease_is_case_insensitive_and_waits_for_release() {
    let lease = super::SerialLease::acquire("Test-Serial-Lease", Duration::ZERO).unwrap();
    assert!(matches!(
        super::SerialLease::acquire("TEST-SERIAL-LEASE", Duration::ZERO),
        Err(ActorError::LeaseBusy)
    ));
    let worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        drop(lease);
    });
    let _next = super::SerialLease::acquire("test-serial-lease", Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
}

#[test]
fn controlled_prompt_cancelled_before_start_never_writes() {
    let (actor, state) = actor_with_reads([]);
    let control = dji4g_domain::SmsTransactionControl::new(Duration::from_secs(1));
    control.cancel();
    let result = actor.execute_prompt_controlled(AtCommand::Identity, vec![1], control.clone());
    assert_eq!(result, Err(ActorError::Io(io::ErrorKind::Interrupted)));
    assert!(state.lock().unwrap().writes.is_empty());
    assert!(!control.submission_possible());
}

#[test]
fn controlled_prompt_marks_submission_before_failed_body_write() {
    let (actor, _) = actor_with_state(FakeState {
        reads: [Ok(b">".to_vec())].into(),
        fail_write_at: Some(1),
        ..FakeState::default()
    });
    let control = dji4g_domain::SmsTransactionControl::new(Duration::from_secs(1));
    assert!(
        actor
            .execute_prompt_controlled(AtCommand::Identity, vec![1], control.clone())
            .is_err()
    );
    assert!(control.submission_possible());
}

#[test]
fn controlled_prompt_cancelled_during_read_never_submits_body() {
    let (actor, state) = blocking_actor(&[b">"]);
    let control = dji4g_domain::SmsTransactionControl::new(Duration::from_secs(1));
    let worker_control = control.clone();
    let worker = thread::spawn(move || {
        actor.execute_prompt_controlled(AtCommand::Identity, vec![1], worker_control)
    });
    wait_for_writes(&state, 1);
    control.cancel();
    state.0.lock().unwrap().released = true;
    state.1.notify_all();
    assert_eq!(
        worker.join().unwrap(),
        Err(ActorError::Io(io::ErrorKind::Interrupted))
    );
    assert!(!control.submission_possible());
    assert_eq!(state.0.lock().unwrap().writes.len(), 1);
}

#[test]
fn controlled_prompt_total_deadline_expires_without_body() {
    let (actor, state) = actor_with_state(FakeState {
        quiet_when_empty: true,
        ..FakeState::default()
    });
    let control = dji4g_domain::SmsTransactionControl::new(Duration::from_millis(10));
    let error = actor
        .execute_prompt_controlled(AtCommand::Identity, vec![1], control.clone())
        .unwrap_err();
    assert_eq!(error.protocol_kind(), Some(ProtocolErrorKind::Timeout));
    assert!(!control.submission_possible());
    assert_eq!(state.lock().unwrap().writes.len(), 1);
}

#[test]
fn os_open_errors_preserve_raw_code_separately_from_lease_busy() {
    let error = super::os_io(io::Error::from_raw_os_error(5));
    assert!(matches!(
        error,
        ActorError::OsIo {
            raw_os_error: Some(5),
            ..
        }
    ));
    assert_ne!(error.code(), ActorError::LeaseBusy.code());
}

struct PromptThenBlockingSerial {
    inner: BlockingSerial,
    prompt: bool,
}
impl SerialIo for PromptThenBlockingSerial {
    fn cancellation_handle(&self) -> io::Result<Arc<dyn SerialIoCancellation>> {
        self.inner.cancellation_handle()
    }
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_all(bytes)
    }
    fn read_chunk(&mut self) -> io::Result<Vec<u8>> {
        if self.prompt {
            self.prompt = false;
            return Ok(b">".to_vec());
        }
        self.inner.read_chunk()
    }
}

#[test]
fn controlled_prompt_deadline_actively_cancels_blocked_native_read() {
    for allow_body in [false, true] {
        let state = Arc::new((Mutex::new(BlockingState::default()), Condvar::new()));
        let actor = AtSessionActor::spawn(
            DeviceEpoch(90),
            Box::new(PromptThenBlockingSerial {
                inner: BlockingSerial(state.clone()),
                prompt: allow_body,
            }),
        );
        let control = dji4g_domain::SmsTransactionControl::new(Duration::from_millis(30));
        let worker_control = control.clone();
        let (done, completed) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let result =
                actor.execute_prompt_controlled(AtCommand::Identity, vec![1], worker_control);
            done.send(result).unwrap();
        });
        let result = completed.recv_timeout(Duration::from_millis(500));
        let actively_cancelled = state.0.lock().unwrap().released;
        // Release even on regression so the red test never leaves a blocked worker behind.
        state.0.lock().unwrap().released = true;
        state.1.notify_all();
        worker.join().unwrap();
        let error = result
            .expect("controlled waiter must cancel blocked native read at its deadline")
            .unwrap_err();
        assert_eq!(error.protocol_kind(), Some(ProtocolErrorKind::Timeout));
        assert!(actively_cancelled);
        assert_eq!(control.submission_possible(), allow_body);
    }
}

// ---------------------------------------------------------------------------------------------
// Device-tool transactions
// ---------------------------------------------------------------------------------------------

mod tool_transactions {
    use super::*;
    use dji4g_at_protocol::{
        ToolParseError, ToolReadId, ToolResponse, ToolWireRequest, ValidatedToolLine,
    };
    use dji4g_domain::ToolTransactionControl;

    fn expert(text: &str) -> ToolWireRequest {
        ToolWireRequest::from_expert(ValidatedToolLine::parse(text).expect("valid line"))
    }

    fn receive(
        actor: &AtSessionActor,
        request: ToolWireRequest,
        control: ToolTransactionControl,
    ) -> Result<ToolResponse, ActorError> {
        actor
            .try_execute_tool(request, control)
            .expect("queued")
            .recv_timeout(Duration::from_secs(2))
            .expect("reply within the test budget")
    }

    #[test]
    fn a_whitelisted_read_writes_once_and_returns_the_parsed_response() {
        let (actor, state) = actor_with_reads([Ok(b"+CSQ: 21,0\r\n\r\nOK\r\n".to_vec())]);
        let response = receive(
            &actor,
            ToolWireRequest::from_read(ToolReadId::SignalQuality),
            ToolTransactionControl::new(Duration::from_secs(5)),
        )
        .expect("the module answered");
        assert_eq!(response.final_code, AtFinalCode::Ok);
        assert_eq!(response.lines, vec!["+CSQ: 21,0"]);
        // Exactly one write: a query is not retried, not even a read-only one.
        assert_eq!(state.lock().unwrap().writes.len(), 1);
        assert_eq!(
            state.lock().unwrap().writes[0],
            b"AT+CSQ\r".to_vec(),
            "the wire form is the validated line plus one CR"
        );
    }

    #[test]
    fn a_module_refusal_comes_back_as_a_response_not_an_actor_error() {
        let (actor, _) = actor_with_reads([Ok(b"AT+VENDOR?\r\nERROR\r\n".to_vec())]);
        let response = receive(
            &actor,
            expert("AT+VENDOR?"),
            ToolTransactionControl::new(Duration::from_secs(5)),
        )
        .expect("a refusal is still an answer");
        assert_eq!(response.final_code, AtFinalCode::Error);
        assert!(response.lines.is_empty());
    }

    #[test]
    fn a_prompt_is_refused_and_retires_the_actor() {
        let (actor, state) = actor_with_reads([Ok(b"AT+VENDOR=1\r\n> ".to_vec())]);
        let error = receive(
            &actor,
            expert("AT+VENDOR=1"),
            ToolTransactionControl::new(Duration::from_secs(5)),
        )
        .expect_err("a prompt ends the transaction");
        assert_eq!(
            error,
            ActorError::Tool(ToolParseError::UnsupportedInteraction)
        );
        // Nothing else was written: no body, no Ctrl-Z, no recovery sequence.
        assert_eq!(state.lock().unwrap().writes.len(), 1);
        // The actor is retired, so a late `OK` cannot be read as the next command's answer.
        let second = actor.try_execute_tool(
            expert("AT+VENDOR?"),
            ToolTransactionControl::new(Duration::from_secs(5)),
        );
        assert!(second.is_err(), "a retired actor must refuse new work");
    }

    #[test]
    fn a_timeout_retires_the_actor_so_a_late_ok_cannot_satisfy_the_next_request() {
        // The module never answers the first command; the second command would have been answered
        // with the first command's late OK if the actor were reused.
        let (actor, state) = actor_with_state(FakeState {
            quiet_when_empty: true,
            ..FakeState::default()
        });
        let control = ToolTransactionControl::new(Duration::from_millis(40));
        let error = receive(
            &actor,
            ToolWireRequest::from_read(ToolReadId::SignalQuality),
            control,
        )
        .expect_err("no answer arrives");
        assert!(
            matches!(error, ActorError::Io(io::ErrorKind::TimedOut)),
            "{error:?}"
        );
        assert_eq!(state.lock().unwrap().writes.len(), 1);
        assert!(
            actor
                .try_execute_tool(
                    expert("AT+VENDOR?"),
                    ToolTransactionControl::new(Duration::from_secs(5)),
                )
                .is_err(),
            "the next request must not run on the retired actor"
        );
    }

    #[test]
    fn a_write_failure_marks_the_attempt_and_never_resends() {
        let (actor, state) = actor_with_state(FakeState {
            fail_write_at: Some(0),
            ..FakeState::default()
        });
        let control = ToolTransactionControl::new(Duration::from_secs(5));
        let error =
            receive(&actor, expert("AT+VENDOR=1"), control.clone()).expect_err("the write failed");
        assert_eq!(error, ActorError::Io(io::ErrorKind::BrokenPipe));
        // The attempt is recorded even though nothing reached the module: a partial write already
        // changed the module's input, so the caller must not classify it as "nothing happened".
        assert!(control.write_attempted());
        assert_eq!(state.lock().unwrap().write_attempts, 1);
        assert!(state.lock().unwrap().writes.is_empty());
    }

    #[test]
    fn a_cancelled_transaction_that_never_wrote_is_not_marked_as_a_write_attempt() {
        let (actor, state) = actor_with_state(FakeState {
            quiet_when_empty: true,
            ..FakeState::default()
        });
        let control = ToolTransactionControl::new(Duration::from_secs(5));
        control.cancel();
        let error = receive(
            &actor,
            ToolWireRequest::from_read(ToolReadId::SignalQuality),
            control.clone(),
        )
        .expect_err("cancelled before the write");
        assert_eq!(error, ActorError::Io(io::ErrorKind::Interrupted));
        assert!(!control.write_attempted());
        assert_eq!(state.lock().unwrap().write_attempts, 0);
    }

    #[test]
    fn cancelling_after_the_write_stops_the_wait_without_a_second_write() {
        let (actor, state) = actor_with_state(FakeState {
            quiet_when_empty: true,
            ..FakeState::default()
        });
        let control = ToolTransactionControl::new(Duration::from_secs(5));
        let receiver = actor
            .try_execute_tool(expert("AT+VENDOR=1"), control.clone())
            .expect("queued");
        // Wait until the command is on the wire, then cancel: the module has already seen it, so
        // the caller has to treat the effect as unknown, but nothing may be sent a second time.
        for _ in 0..400 {
            if state.lock().unwrap().write_attempts >= 1 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state.lock().unwrap().write_attempts, 1);
        control.cancel();
        let error = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("reply")
            .expect_err("cancelled");
        assert_eq!(error, ActorError::Io(io::ErrorKind::Interrupted));
        assert!(control.write_attempted());
        assert_eq!(state.lock().unwrap().write_attempts, 1);
    }

    #[test]
    fn a_failed_tool_transaction_does_not_disturb_the_monitoring_path() {
        // A monitoring-style `Execute` after a failed tool run reports the retirement rather than
        // hanging, and never returns a stale success.
        let (actor, _) = actor_with_reads([Ok(b"AT+VENDOR?\r\nERROR\r\n".to_vec())]);
        let _ = receive(
            &actor,
            expert("AT+VENDOR?"),
            ToolTransactionControl::new(Duration::from_secs(5)),
        )
        .expect("refusal");
        // A refusal is a valid answer, so the actor is still usable for a normal command.
        let follow_up = actor.try_execute(AtCommand::SignalQuality);
        match follow_up {
            Ok(receiver) => {
                let result = receiver.recv_timeout(Duration::from_secs(2));
                assert!(result.is_ok(), "the actor still answers");
            }
            Err(error) => assert!(matches!(
                error,
                ActorError::Closed | ActorError::LeaseBusy | ActorError::QueueFull
            )),
        }
    }
}
