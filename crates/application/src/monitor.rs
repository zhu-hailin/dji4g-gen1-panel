use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::{Duration, Instant, SystemTime},
};

use std::sync::mpsc;

use dji4g_domain::{
    SMS_SEND_TIMEOUT, SmsEncoding, SmsFailureDetail, SmsMessage, SmsSendPhase, SmsStatus,
    SmsTransactionControl,
};

use crate::controller::RefreshSignal;
use crate::{
    AdapterPort, AtPort, BackendEvent, CheckResult, Controller, ControllerHandle, FeatureStatus,
    HotspotControl, InventoryPort, NetworkProbePort, SmsListing, SmsPort, SmsRequest,
    SmsSendResult, TargetContext, UiCommand,
};

/// Upper bound on one monitoring stage (inventory, AT, adapter, probe, hotspot).
///
/// Every stage runs on its own detached worker and is joined with a bounded wait; if the stage
/// has not produced a result within this budget, the runner reports
/// `app:stage_timeout` for that check only and moves on to the next stage (and the next refresh
/// cycle) instead of wedging the monitor thread forever.
pub const STAGE_TIMEOUT: Duration = Duration::from_secs(15);

/// Platform ports used by one refresh DAG. The application owns this shape; platform crates only
/// provide adapters implementing these traits.
#[derive(Clone)]
pub struct MonitorPorts {
    pub inventory: Arc<dyn InventoryPort>,
    pub at: Arc<dyn AtPort>,
    pub adapter: Arc<dyn AdapterPort>,
    pub probe: Arc<dyn NetworkProbePort>,
    pub hotspot: Option<Arc<dyn HotspotControl>>,
    /// SMS module transactions (list/read/delete). `None` means this build has no SMS support and
    /// queued [`SmsRequest`]s stay queued until a port is wired.
    pub sms: Option<Arc<dyn SmsPort>>,
}

/// How long the dedicated controller thread sleeps before re-checking the refresh signal when no
/// command is pending.  A bounded wait replaces the previous `try_recv` + `yield_now` spin so a
/// panel resident in the tray no longer burns a CPU core.  The interval is deliberately small
/// (20 ms) so a prepare issued by a button click is handled almost immediately: the native
/// confirmation box opens at click time and the plan it prepares is usually published while the
/// user is still reading the box.  A small park timeout is not a spin — the thread stays parked
/// in `recv_timeout` between polls and does not burn CPU.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Cadence of the automatic, read-only monitoring scan.
///
/// The design requires "PnP and network notifications + periodic monitor".  Positive evidence
/// expires after `EVIDENCE_TTL` (30 s, see `crate::ReducerState`), so a panel that only scanned on
/// an explicit user command could never hold a fresh classification: it would either stay in its
/// initial `Detecting` state forever (nothing had scanned yet) or decay to `Stale` half a minute
/// after the one manual scan and never recover.  This interval sits comfortably inside the TTL so
/// the published state stays honest without any user interaction, including for a module that is
/// plugged in after the panel starts.
///
/// The cadence only ever drives the observation ports (inventory, AT, adapter, bound probe,
/// hotspot status).  It can never prepare, confirm, or execute an action, so the invariant that a
/// non-repeatable write is executed at most once and is never automatically retried is untouched.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(10);

/// Decide whether an automatic monitoring scan is due.
///
/// `None` means the runner has never scanned, so the first pass is always due; that is what makes
/// a freshly started panel collect evidence in every build configuration instead of waiting for a
/// user to press 刷新.  A wall-clock step backwards makes `duration_since` fail, which is treated
/// as due so a clock change can never stall the monitor.
#[must_use]
pub fn periodic_refresh_due(last: Option<SystemTime>, now: SystemTime, interval: Duration) -> bool {
    match last {
        None => true,
        Some(last) => match now.duration_since(last) {
            // A wall-clock step backwards must not stall the monitor.
            Err(_) => true,
            Ok(age) => age >= interval,
        },
    }
}

/// Cadence of the rates-only tick that feeds the header throughput chart.
///
/// The evidence refresh above is deliberately slow (`REFRESH_INTERVAL`, 10 s) because every scan
/// bumps `evidence_revision` and re-derives the whole classification.  The throughput chart, by
/// contrast, wants one honest sample per second.  This tick re-reads *only* the bound module
/// adapter's monotonic byte counters and recomputes `down/up_bytes_per_sec`; it never runs the
/// refresh DAG, never advances `observed_at`, and never changes freshness or availability, so the
/// safety-relevant evidence semantics stay pinned to the 10 s refresh.
pub const RATE_TICK_INTERVAL: Duration = Duration::from_secs(1);

/// Upper bound on the read-only byte-counter fetch driven by one rates-only tick.
///
/// The counter read is a single native `GetIfEntry2` call that completes immediately in practice.
/// This short budget is the watchdog that guarantees a pathological or wedged native call can never
/// stall the monitor loop the way the much longer [`STAGE_TIMEOUT`] would: if the read has not
/// completed in time, the tick reports honest `None` rates and the loop carries on.
pub const RATE_READ_TIMEOUT: Duration = Duration::from_secs(2);

/// Decide whether a rates-only tick is due.
///
/// Structurally identical to [`periodic_refresh_due`] so the two cadences share one well-understood
/// contract: `None` means the runner has never ticked, so the first tick is due immediately (it only
/// establishes a baseline), and a wall-clock step backwards makes `duration_since` fail, which is
/// treated as due so a clock change can never stall the throughput chart.
#[must_use]
pub fn rate_tick_due(last: Option<SystemTime>, now: SystemTime, period: Duration) -> bool {
    match last {
        None => true,
        Some(last) => match now.duration_since(last) {
            // A wall-clock step backwards must not stall the rates cadence.
            Err(_) => true,
            Ok(age) => age >= period,
        },
    }
}

pub struct ControllerRunner {
    controller: Controller,
    commands: mpsc::Receiver<UiCommand>,
    refresh: Arc<RefreshSignal>,
    handle: ControllerHandle,
    ports: Option<MonitorPorts>,
    /// Wall-clock time of the most recent scan, used only by the periodic monitoring cadence.
    last_refresh_at: Option<SystemTime>,
    /// Wall-clock time of the most recent rates-only tick (or full refresh, which also samples the
    /// counters), used only by the 1 s throughput cadence. Kept separate from `last_refresh_at` so
    /// the fast rates tick and the slow evidence refresh never disturb each other's schedule.
    last_rate_tick_at: Option<SystemTime>,
    /// Per-stage watchdog budget. Production runners keep [`STAGE_TIMEOUT`]; scenario tests
    /// shorten it so a timeout does not have to wait out a full 15 s window.
    stage_timeout: Duration,
    /// Locally assigned identifier for outgoing records. Used only to keep two otherwise identical
    /// send records distinct in the store's digest set; never a module storage index.
    next_sms_transaction_id: u32,
    pending_sms: Option<PendingSms>,
    sms_timeout: Duration,
    refresh_deferred: bool,
}

struct PendingSms {
    request: SmsRequest,
    receiver: mpsc::Receiver<Result<crate::SmsSendReceipt, crate::PortError>>,
    control: SmsTransactionControl,
    epoch: dji4g_domain::DeviceEpoch,
    sim_epoch: u64,
}

impl Drop for ControllerRunner {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending_sms {
            pending.control.cancel();
        }
    }
}

impl ControllerRunner {
    #[must_use]
    pub fn new(controller: Controller) -> (ControllerHandle, Self) {
        let initial = Arc::new(controller.snapshot());
        let (handle, commands, refresh) = ControllerHandle::channels(initial);
        let runner = Self {
            controller,
            commands,
            refresh,
            handle: handle.clone(),
            ports: None,
            last_refresh_at: None,
            last_rate_tick_at: None,
            stage_timeout: STAGE_TIMEOUT,
            next_sms_transaction_id: 1,
            pending_sms: None,
            sms_timeout: SMS_SEND_TIMEOUT,
            refresh_deferred: false,
        };
        (handle, runner)
    }

    #[must_use]
    pub fn with_ports(mut self, ports: MonitorPorts) -> Self {
        self.ports = Some(ports);
        self
    }

    /// Override the per-stage watchdog budget for every subsequent refresh cycle.
    ///
    /// This setter exists so scenario tests can inject a short budget (a `pub(crate)` knob is
    /// invisible to integration tests in `tests/`, and `#[cfg(test)]` constructors do not exist
    /// for them either, so a documented public builder is the honest option). Production runners
    /// never call it and keep [`STAGE_TIMEOUT`].
    #[must_use]
    pub fn with_stage_timeout(mut self, stage_timeout: Duration) -> Self {
        self.stage_timeout = stage_timeout;
        self
    }

    pub fn with_sms_timeout(mut self, timeout: Duration) -> Self {
        self.sms_timeout = timeout;
        self
    }
    pub fn sms_pending(&self) -> bool {
        self.pending_sms.is_some()
    }

    #[must_use]
    pub fn handle(&self) -> ControllerHandle {
        self.handle.clone()
    }

    #[must_use]
    pub fn controller(&self) -> &Controller {
        &self.controller
    }

    /// Mutable controller access for deterministic scenario tests that must pre-seed state (for
    /// example an SMS message the module is about to delete). Production code never needs this:
    /// every state change flows through a command, a port outcome, or a backend event.
    pub fn controller_mut(&mut self) -> &mut Controller {
        &mut self.controller
    }

    /// Drain and handle every command already queued on the channel, exactly as [`Self::run`]
    /// does each loop iteration. Returns whether at least one command was handled.
    ///
    /// This exists so deterministic scenario tests can drive `handle.try_send` + command handling
    /// without spawning the async loop, and it is behaviourally identical to the loop's own
    /// receive branch.
    pub fn poll_commands(&mut self) -> bool {
        let mut handled = false;
        loop {
            match self.commands.try_recv() {
                Ok(command) => {
                    let _ = self.controller.handle_command(command);
                    handled = true;
                }
                Err(mpsc::TryRecvError::Empty) => return handled,
                Err(mpsc::TryRecvError::Disconnected) => return handled,
            }
        }
    }

    pub async fn run(mut self) {
        loop {
            // Collect a background operation's terminal result before anything else, so the
            // Finished state is published promptly (bounded by IDLE_POLL_INTERVAL) and the
            // runner thread never blocks on the executor itself.
            if self.controller.poll_operation_completion().is_some() {
                self.publish();
            }
            match self.commands.recv_timeout(IDLE_POLL_INTERVAL) {
                Ok(command) => {
                    let _ = self.controller.handle_command(command);
                    self.publish();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            // Module-side SMS requests are transactional (a send/delete must not interleave with
            // a low-priority AT poll; research document §8.3), so each one is drained and run to
            // completion here — on a bounded stage worker exactly like a refresh stage — before
            // the next monitoring cadence is considered.
            if self.poll_sms_requests() {
                self.publish();
            }
            // An explicit user command and the automatic monitoring cadence share one scan path.
            // Relying on the signal alone left a release build permanently unscanned, because its
            // only startup `Refresh` was compiled out and no other producer sets the signal.
            let signaled = self.refresh.take();
            self.refresh_deferred |= signaled;
            if !self.sms_pending() && (self.refresh_deferred || self.monitoring_cadence_due()) {
                self.refresh_deferred = false;
                self.run_refresh();
                self.publish();
            } else if self.rate_cadence_due() {
                // Rates-only tick: re-read just the bound adapter's byte counters and republish the
                // throughput, without running the refresh DAG or advancing evidence freshness. The
                // refresh branch above takes precedence, and `run_refresh` realigns the rates
                // baseline, so the two cadences never double-sample in one iteration.
                self.run_rate_tick();
                self.publish();
            }
        }
    }

    /// Run one scan of the periodic monitoring cadence if, and only if, it is due.
    ///
    /// Returns whether a scan ran.  This is the same path `run` takes when no explicit user signal
    /// is pending, exposed so the cadence can be driven deterministically from a `FakeClock`.
    pub fn poll_monitoring_cadence(&mut self) -> bool {
        if !self.monitoring_cadence_due() {
            return false;
        }
        self.run_refresh();
        self.publish();
        true
    }

    /// Run one rates-only tick if, and only if, it is due.
    ///
    /// Returns whether a tick ran.  This is the same path `run` takes for the 1 s throughput
    /// cadence, exposed so scenario tests can drive it deterministically from a `FakeClock` without
    /// waiting out a real second.  It never runs the refresh DAG.
    pub fn poll_rate_cadence(&mut self) -> bool {
        if !self.rate_cadence_due() {
            return false;
        }
        self.run_rate_tick();
        self.publish();
        true
    }

    /// Drain and execute every queued module-side SMS request, oldest first.
    ///
    /// Returns whether at least one request was handled. Each request is driven to completion on
    /// its own detached stage worker with the same bounded watchdog as a refresh stage, so a
    /// wedged AT transaction can never stall the monitor loop; the worker only touches the
    /// `SmsPort` and returns an owned outcome, and the controller/reducer are updated here, on
    /// the runner thread, keeping the single-threaded state invariant.
    ///
    /// The SMS port is deliberately absent from the periodic refresh DAG: it is only ever driven
    /// by an explicit user request, never on a timer. With no port wired the requests stay queued
    /// (the controller records intent; a later composition may attach the port and drain them).
    pub fn poll_sms_requests(&mut self) -> bool {
        if self.pending_sms.is_some() {
            return self.poll_sms_completion();
        }
        let Some(sms_port) = self
            .ports
            .as_ref()
            .and_then(|ports| ports.sms.as_ref())
            .map(Arc::clone)
        else {
            return false;
        };
        let Some(request) = self.controller.take_next_sms_request() else {
            return false;
        };
        if matches!(request, SmsRequest::Send { .. }) {
            self.start_sms_send(request, sms_port);
        } else {
            self.run_sms_request(request, &sms_port);
        }
        true
    }

    fn start_sms_send(&mut self, request: SmsRequest, port: Arc<dyn SmsPort>) {
        let current_context = (
            self.controller.state().epoch(),
            self.controller.snapshot().sim_epoch,
        );
        if self.controller.sms_send_context() != Some(current_context) {
            self.controller.update_sms_send(
                SmsSendPhase::Finished,
                Some(SmsSendResult::Failed),
                Some(SmsFailureDetail::new(
                    SmsSendPhase::Queued,
                    "sms:device_changed",
                    false,
                )),
            );
            self.publish();
            return;
        }
        let Some(target) = self.controller.state().target_context() else {
            self.controller.update_sms_send(
                SmsSendPhase::Finished,
                Some(SmsSendResult::Failed),
                Some(SmsFailureDetail::new(
                    SmsSendPhase::Preparing,
                    "sms:no_device",
                    false,
                )),
            );
            self.publish();
            return;
        };
        let request_id = self
            .controller
            .snapshot()
            .sms_send
            .as_ref()
            .map_or(0, |send| send.request_id);
        let control = SmsTransactionControl::new(self.sms_timeout).with_request_id(request_id);
        let worker_control = control.clone();
        let epoch = target.epoch();
        let sim_epoch = self.controller.snapshot().sim_epoch;
        let worker_request = request.clone();
        let timeout = self.sms_timeout;
        self.controller
            .update_sms_send(SmsSendPhase::Preparing, None, None);
        self.publish();
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("dji4g-sms-send".into())
            .spawn(move || {
                let SmsRequest::Send { recipient, body } = worker_request else {
                    unreachable!()
                };
                let outcome = poll_ready(
                    port.send_controlled(&target, &recipient, &body, worker_control.clone()),
                    timeout,
                    || {
                        worker_control.cancel();
                        Err(crate::PortError::new(
                            dji4g_domain::ErrorCode::Timeout,
                            "sms:timeout",
                        ))
                    },
                );
                let _ = sender.send(outcome);
            });
        if spawned.is_err() {
            self.controller.update_sms_send(
                SmsSendPhase::Finished,
                Some(SmsSendResult::Failed),
                Some(SmsFailureDetail::new(
                    SmsSendPhase::Preparing,
                    "sms:worker_failed",
                    false,
                )),
            );
            return;
        }
        self.pending_sms = Some(PendingSms {
            request,
            receiver,
            control,
            epoch,
            sim_epoch,
        });
    }

    fn poll_sms_completion(&mut self) -> bool {
        let pending = self.pending_sms.as_ref().expect("pending SMS");
        let current = self.controller.snapshot();
        if pending.control.is_cancelled()
            || current.sim_epoch != pending.sim_epoch
            || self.controller.state().epoch() != pending.epoch
        {
            pending.control.cancel();
        }
        let outcome = match pending.receiver.try_recv() {
            Ok(value) => value,
            Err(mpsc::TryRecvError::Empty) => {
                let phase = pending.control.phase();
                let failure = pending.control.cleanup_pending().then(|| {
                    SmsFailureDetail::new(
                        phase,
                        "sms:cleanup_timeout",
                        pending.control.submission_possible(),
                    )
                });
                let changed = current
                    .sms_send
                    .as_ref()
                    .is_some_and(|send| send.phase != phase || send.failure != failure);
                self.controller.update_sms_send(phase, None, failure);
                return changed;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(crate::PortError::new(
                dji4g_domain::ErrorCode::Internal,
                "sms:worker_failed",
            )),
        };
        let pending = self.pending_sms.take().expect("pending SMS");
        let stale = current.sim_epoch != pending.sim_epoch
            || self.controller.state().epoch() != pending.epoch;
        let (result, failure) = match outcome {
            Ok(receipt) => (receipt.result, receipt.failure),
            Err(error) => (
                if pending.control.submission_possible() {
                    SmsSendResult::OutcomeUnknown
                } else {
                    SmsSendResult::Failed
                },
                Some(SmsFailureDetail::new(
                    pending.control.phase(),
                    error.code.stable().as_str(),
                    pending.control.submission_possible(),
                )),
            ),
        };
        if stale {
            self.controller.update_sms_send(
                SmsSendPhase::Finished,
                Some(SmsSendResult::OutcomeUnknown),
                Some(SmsFailureDetail::new(
                    pending.control.phase(),
                    "sms:device_changed",
                    pending.control.submission_possible(),
                )),
            );
            return true;
        }
        self.controller
            .update_sms_send(SmsSendPhase::Finished, Some(result), failure);
        if let SmsRequest::Send { recipient, body } = pending.request {
            let status = match result {
                SmsSendResult::Submitted => SmsStatus::Submitted,
                SmsSendResult::Failed => SmsStatus::Failed,
                SmsSendResult::OutcomeUnknown => SmsStatus::OutcomeUnknown,
            };
            self.controller.ingest_sms(SmsMessage::new_outgoing(
                self.next_sms_transaction_id,
                pending.epoch.0,
                pending.sim_epoch,
                recipient,
                body,
                SmsEncoding::Ucs2,
                status,
            ));
            self.next_sms_transaction_id = self.next_sms_transaction_id.wrapping_add(1);
        }
        true
    }

    /// Execute one SMS request on a bounded stage worker and apply its outcome to the controller.
    fn run_sms_request(&mut self, request: SmsRequest, sms_port: &Arc<dyn SmsPort>) {
        // A request needs a validated device/SIM context. Without one there is nothing honest to
        // execute against: record the gap as a temporary unavailability and move on.
        let Some(target) = self.controller.state().target_context() else {
            self.controller.set_sms_refresh_pending(false);
            self.controller
                .set_sms_inbox_failure(Some(crate::PortError::new(
                    dji4g_domain::ErrorCode::DeviceRemoved,
                    "sms:no_device",
                )));
            self.controller.record_sms_probe(
                FeatureStatus::TemporarilyUnavailable,
                self.current_sms_capacity(),
            );
            return;
        };
        let timeout = self.stage_timeout;
        let port = Arc::clone(sms_port);
        // The request is owned (a send carries its recipient and body) and the runner still needs
        // it to apply the outcome, so the worker gets a clone.
        let worker_request = request.clone();
        let is_send = matches!(request, SmsRequest::Send { .. });
        let receiver = spawn_stage(move || {
            Ok(poll_ready(
                run_sms_port_call(&port, &target, worker_request),
                timeout,
                || SmsStageOutcome::timed_out(is_send),
            ))
        });
        let outcome = match join_stage(receiver, Instant::now() + self.stage_timeout) {
            Some(Ok(outcome)) => outcome,
            // The watchdog expired or the worker died: the transaction outcome is unknown, so the
            // inbox status records a transport failure. A send additionally records the honest
            // `OutcomeUnknown` store state below; nothing else is mutated.
            _ => SmsStageOutcome::timed_out(is_send),
        };
        self.apply_sms_outcome(request, outcome);
    }

    /// Apply one worker outcome to the controller. Runs on the runner thread only; every mutation
    /// here is a deliberate consequence of a completed module transaction.
    fn apply_sms_outcome(&mut self, request: SmsRequest, outcome: SmsStageOutcome) {
        if matches!(request, SmsRequest::Refresh) {
            self.controller.set_sms_refresh_pending(false);
        }
        self.controller.set_sms_inbox_failure(None);
        match outcome {
            SmsStageOutcome::Refreshed {
                messages,
                capacity,
                status,
            } => {
                for message in messages {
                    self.controller.ingest_sms(message);
                }
                self.controller.record_sms_probe(status, capacity);
            }
            SmsStageOutcome::Read { message } => {
                self.controller.ingest_sms(message.clone());
                self.controller.mark_sms_read(&message);
                self.controller
                    .record_sms_probe(FeatureStatus::Supported, self.current_sms_capacity());
            }
            SmsStageOutcome::Deleted => {
                if let SmsRequest::Delete { index } = request {
                    self.controller.confirm_sms_delete(index);
                }
                self.controller
                    .record_sms_probe(FeatureStatus::Supported, self.current_sms_capacity());
            }
            SmsStageOutcome::SendOutcome {
                store_status,
                probe,
            } => {
                // Every completed attempt — including a refused PDU preflight — leaves exactly one
                // outgoing record. The recipient stays in the sender slot and is only displayed
                // masked (the UI owns masking); the local transaction id keeps repeated sends of
                // identical text distinct. `Submitted` is submission only and is never retried.
                if let SmsRequest::Send { recipient, body } = request {
                    let message = SmsMessage::new_outgoing(
                        self.next_sms_transaction_id,
                        self.controller.state().epoch().0,
                        0,
                        recipient,
                        body,
                        // The platform codec picks GSM-7 or UCS-2 internally and the receipt does
                        // not report which; `Other` records that honestly.
                        SmsEncoding::Other,
                        store_status,
                    );
                    self.next_sms_transaction_id = self.next_sms_transaction_id.wrapping_add(1);
                    self.controller.ingest_sms(message);
                }
                self.controller
                    .record_sms_probe(probe, self.current_sms_capacity());
            }
            SmsStageOutcome::Failed(error) => {
                let status = map_sms_error(&error);
                self.controller.set_sms_inbox_failure(Some(error));
                self.controller
                    .record_sms_probe(status, self.current_sms_capacity());
            }
        }
    }

    fn current_sms_capacity(&self) -> Option<(u32, u32)> {
        self.controller.snapshot().sms_inbox.capacity
    }

    /// The cadence only applies to a runner wired to real observation ports, and it yields while an
    /// explicit user interaction is in flight.  A portless runner is a deterministic test or demo
    /// backend and must not scan on a timer; and because every scan bumps `evidence_revision`, an
    /// unconditional timer would invalidate a repair plan awaiting confirmation every interval.
    /// The yielding window is bounded by the plan's own expiry, so monitoring can never stall.
    fn monitoring_cadence_due(&self) -> bool {
        let now = self.controller.now();
        self.ports.is_some()
            && !self.controller.interaction_in_flight(now)
            && periodic_refresh_due(self.last_refresh_at, now, REFRESH_INTERVAL)
    }

    /// Whether a rates-only tick is due right now.
    ///
    /// Same yielding rule as [`Self::monitoring_cadence_due`]: it applies only to a runner wired to
    /// real ports, and it yields while an explicit user interaction is in flight so a pending
    /// confirmation is never disturbed.  Unlike the refresh cadence this tick does not bump
    /// `evidence_revision`, so it could not invalidate a plan anyway; yielding anyway keeps the two
    /// cadences behaviourally identical and leaves a confirmation utterly undisturbed.
    fn rate_cadence_due(&self) -> bool {
        let now = self.controller.now();
        self.ports.is_some()
            && !self.controller.interaction_in_flight(now)
            && rate_tick_due(self.last_rate_tick_at, now, RATE_TICK_INTERVAL)
    }

    /// Run one rates-only tick: read *only* the bound module adapter's byte counters and full
    /// interface metrics, recompute the throughput, and record the metrics sample, leaving every
    /// evidence field — `observed_at`, freshness, the diagnostic checks, availability — exactly
    /// as the last full refresh left it.
    ///
    /// This path is strictly read-only.  It goes through the same [`AdapterPort`] seam as the
    /// refresh's adapter stage but calls only the already-bound-GUID readers, which never
    /// re-enumerate, never open the AT port, and never prepare, confirm, or execute an action.
    /// Both reads run in one bounded detached worker (one thread per tick, not two) exactly like a
    /// refresh stage, so a wedged native call is abandoned at [`RATE_READ_TIMEOUT`] and reported
    /// as honest `None` values instead of stalling the loop.
    fn run_rate_tick(&mut self) {
        let Some(ports) = self.ports.clone() else {
            return;
        };
        // Recorded before the read so a slow or failing counter fetch cannot make the cadence spin:
        // the next tick is one full period away either way. A full refresh also samples the counters
        // and realigns this baseline, so the two cadences never double-sample.
        let now = self.controller.now();
        self.last_rate_tick_at = Some(now);
        // Only the already-bound module adapter is ever read; with no bound adapter there is nothing
        // honest to sample, so the tick is a no-op until the next refresh binds one.
        let epoch = self.controller.state().epoch();
        let Some(adapter_id) = self.controller.state().adapter_id() else {
            return;
        };
        let adapter_port = ports.adapter;
        let receiver = spawn_stage({
            let adapter_id = adapter_id.clone();
            move || {
                let counters = poll_ready(
                    adapter_port.read_byte_counters(&adapter_id),
                    RATE_READ_TIMEOUT,
                    || Err(stage_timeout_error()),
                )
                .ok();
                let metrics = poll_ready(
                    adapter_port.read_metrics(&adapter_id),
                    RATE_READ_TIMEOUT,
                    || Err(stage_timeout_error()),
                )
                .ok();
                Ok((counters, metrics))
            }
        });
        let deadline = Instant::now() + RATE_READ_TIMEOUT;
        // An unreadable counter, a port error, or a watchdog timeout all collapse to `None` values,
        // which the reducer turns into honest `None` rates/metrics — never stale or fabricated
        // numbers.
        let (counters, metrics) = match join_stage(receiver, deadline) {
            Some(Ok(sampled)) => sampled,
            _ => (None, None),
        };
        let (rx, tx) = counters.map_or((None, None), |(rx, tx)| (Some(rx), Some(tx)));
        self.controller
            .apply_backend_event(BackendEvent::RatesSampled {
                adapter_id: adapter_id.clone(),
                epoch,
                rx,
                tx,
                sampled_at: now,
            });
        self.controller
            .apply_backend_event(BackendEvent::AdapterMetricsSampled {
                adapter_id,
                epoch,
                metrics,
                sampled_at: now,
            });
    }

    pub fn run_one_refresh(&mut self) {
        let _ = self.refresh.take();
        self.run_refresh();
        self.publish();
    }

    fn run_refresh(&mut self) {
        if self.controller.sms_active() {
            self.refresh_deferred = true;
            return;
        }
        // Owned clones of the port handles so stage workers can be `'static`. The runner keeps
        // `self.ports` untouched; an `Arc` clone per stage is all a worker ever sees.
        let Some(ports) = self.ports.clone() else {
            // No ports means a deterministic test/demo backend. Still expose a bounded refresh
            // cycle so a burst cannot create an unbounded queue.
            return;
        };
        let MonitorPorts {
            inventory: inventory_port,
            at: at_port,
            adapter: adapter_port,
            probe: probe_port,
            hotspot: hotspot_port,
            // SMS is driven only by explicit requests via `poll_sms_requests` (research §8.3:
            // transactions get exclusive execution; the timed DAG never touches the SMS path).
            sms: _,
        } = ports;
        let stage_timeout = self.stage_timeout;
        // Recorded before the scan so a slow or failing stage cannot make the cadence spin: the
        // next automatic scan is one full interval away either way.
        self.last_refresh_at = Some(self.controller.now());
        // A full refresh samples the same counters through its adapter stage, so realign the rates
        // baseline too: the next rates-only tick is then a full period away and the two cadences
        // never sample at the same instant (which the baseline logic would reject as non-advancing).
        self.last_rate_tick_at = Some(self.controller.now());
        let cycle = self.controller.next_cycle();
        let epoch = self.controller.state().epoch();
        self.controller
            .apply_backend_event(BackendEvent::RefreshStarted {
                cycle,
                epoch,
                scheduled: crate::CheckMask::all(),
            });

        // Stage 1 — inventory. Sequential by design: it produces the epoch and the target the
        // rest of the cycle depends on, and a missing device ends the cycle early.
        let inventory = stage_check(
            join_stage(
                spawn_stage(move || {
                    poll_ready(inventory_port.scan(), stage_timeout, || {
                        Err(stage_timeout_error())
                    })
                }),
                Instant::now() + self.stage_timeout,
            ),
            self.controller.now(),
        );
        let inventory_epoch = match &inventory {
            CheckResult::Passed { value, .. } => value.epoch,
            _ => epoch,
        };
        self.controller
            .apply_backend_event(BackendEvent::InventoryFinished {
                cycle,
                epoch: inventory_epoch,
                result: inventory,
            });
        let epoch = self.controller.state().epoch();
        let snapshot = self.controller.snapshot();
        let Some(device) = snapshot.app.device.as_ref() else {
            self.controller
                .apply_backend_event(BackendEvent::RefreshFinished { cycle, epoch });
            return;
        };
        let target = match self.controller.state().target_context().or_else(|| {
            TargetContext::new(
                epoch,
                device.identity.clone(),
                device.at_port.clone(),
                device.adapter_id.clone(),
            )
            .ok()
        }) {
            Some(target) => target,
            None => {
                self.controller
                    .apply_backend_event(BackendEvent::AtFinished {
                        cycle,
                        epoch,
                        result: CheckResult::Failed {
                            code: crate::PortError::new(
                                dji4g_domain::ErrorCode::Unsupported,
                                "pnp:unsupported_device",
                            )
                            .code,
                            observed_at: self.controller.now(),
                        },
                    });
                self.controller
                    .apply_backend_event(BackendEvent::RefreshFinished { cycle, epoch });
                return;
            }
        };

        // Stages 2 and 3 — AT and adapter run concurrently: both need only the target. Both
        // workers are started before either is joined; their events are then applied here, on
        // the runner thread, in the fixed deterministic order AT then adapter, so the reducer
        // still sees one ordered event stream.
        let at_receiver = spawn_stage({
            let target = target.clone();
            move || {
                poll_ready(at_port.observe(&target), stage_timeout, || {
                    Err(stage_timeout_error())
                })
            }
        });
        let adapter_receiver = spawn_stage({
            let target = target.clone();
            move || {
                poll_ready(adapter_port.resolve(&target), stage_timeout, || {
                    Err(stage_timeout_error())
                })
            }
        });
        let deadline = Instant::now() + self.stage_timeout;
        let at = stage_check(join_stage(at_receiver, deadline), self.controller.now());
        let adapter = stage_check(
            join_stage(adapter_receiver, deadline),
            self.controller.now(),
        );
        self.controller
            .apply_backend_event(BackendEvent::AtFinished {
                cycle,
                epoch,
                result: at,
            });
        self.controller
            .apply_backend_event(BackendEvent::AdapterFinished {
                cycle,
                epoch,
                result: adapter,
            });

        // Stages 4 and 5 — probe and hotspot run concurrently. Both need the adapter context,
        // which is derived from the reducer state after the adapter event above has been
        // applied, exactly as the sequential version did. Each stage either runs on a worker or
        // is decided up front (disabled, or its prerequisite is missing); both workers are
        // started before either outcome is joined, and the events are applied in the fixed
        // order probe then hotspot.
        let adapter_context = self.controller.state().adapter_context();
        let probe_plan = if self.controller.state().active_probe() {
            match adapter_context.clone() {
                Some(context) => StagePlan::Running(spawn_stage(move || {
                    poll_ready(probe_port.observe(&context, true), stage_timeout, || {
                        Err(stage_timeout_error())
                    })
                })),
                None => StagePlan::Decided(CheckResult::Unavailable {
                    code: crate::PortError::new(
                        dji4g_domain::ErrorCode::CapabilityUnavailable,
                        "app:adapter_not_ready",
                    )
                    .code,
                    observed_at: self.controller.now(),
                }),
            }
        } else {
            StagePlan::Decided(CheckResult::Unexecuted {
                reason: crate::UnexecutedReason::DisabledBySetting,
            })
        };
        let hotspot_plan = match (adapter_context, hotspot_port) {
            (Some(context), Some(hotspot)) => StagePlan::Running(spawn_stage(move || {
                poll_ready(hotspot.observe(Some(&context)), stage_timeout, || {
                    Err(stage_timeout_error())
                })
            })),
            _ => StagePlan::Decided(CheckResult::Unavailable {
                code: crate::PortError::new(
                    dji4g_domain::ErrorCode::CapabilityUnavailable,
                    "app:hotspot_unavailable",
                )
                .code,
                observed_at: self.controller.now(),
            }),
        };
        let deadline = Instant::now() + self.stage_timeout;
        let probe = match probe_plan {
            StagePlan::Decided(result) => result,
            StagePlan::Running(receiver) => {
                stage_check(join_stage(receiver, deadline), self.controller.now())
            }
        };
        let hotspot = match hotspot_plan {
            StagePlan::Decided(result) => result,
            StagePlan::Running(receiver) => {
                stage_check(join_stage(receiver, deadline), self.controller.now())
            }
        };
        self.controller
            .apply_backend_event(BackendEvent::ProbeFinished {
                cycle,
                epoch,
                result: probe,
            });
        self.controller
            .apply_backend_event(BackendEvent::HotspotFinished {
                cycle,
                epoch,
                result: hotspot,
            });
        self.controller
            .apply_backend_event(BackendEvent::RefreshFinished { cycle, epoch });
    }

    fn publish(&self) {
        let _ = self
            .handle
            .snapshot_tx
            .send(Arc::new(self.controller.snapshot()));
    }
}

/// One dependent stage's disposition: either already decided without running (the stage is
/// disabled, or its prerequisite such as the adapter context is missing) or in flight on a
/// worker whose bounded join yields the stage's raw outcome.
enum StagePlan<T> {
    Decided(CheckResult<T>),
    Running(mpsc::Receiver<Result<T, crate::PortError>>),
}

/// Owned terminal outcome of one `SmsPort` transaction, produced on the stage worker and applied
/// by the runner. Only owned domain values cross the thread boundary; the controller itself never
/// leaves the runner thread.
enum SmsStageOutcome {
    /// A completed list (or a list with an honest probe status when the store itself could not be
    /// read). Messages carry no plaintext metadata beyond what the port reported.
    Refreshed {
        messages: Vec<SmsMessage>,
        capacity: Option<(u32, u32)>,
        status: FeatureStatus,
    },
    /// One successfully read message.
    Read { message: SmsMessage },
    /// One successfully deleted message (the module returned the final `OK`).
    Deleted,
    /// One send attempt whose store status and feature verdict were already derived on the
    /// worker. The runner only records the outgoing entry and the probe.
    SendOutcome {
        store_status: SmsStatus,
        probe: FeatureStatus,
    },
    /// The transaction failed; the runner records this status and mutates nothing.
    Failed(crate::PortError),
}

impl SmsStageOutcome {
    /// The watchdog outcome for a transaction that produced no terminal result.
    ///
    /// A send in that state is `OutcomeUnknown` by definition — the PDU may already have left the
    /// module — and is never retried. Every other transaction is a plain transport failure and
    /// mutates nothing.
    fn timed_out(is_send: bool) -> Self {
        if is_send {
            Self::SendOutcome {
                store_status: SmsStatus::OutcomeUnknown,
                probe: FeatureStatus::TransportFailure,
            }
        } else {
            Self::Failed(stage_timeout_error())
        }
    }
}

/// Drive one inbox request. Sends use the separate controlled worker and the port owns their
/// entire preflight and submission, without reopening the serial handle between steps.
fn run_sms_port_call<'a>(
    port: &'a Arc<dyn SmsPort>,
    target: &'a TargetContext,
    request: SmsRequest,
) -> crate::PortFuture<'a, SmsStageOutcome> {
    Box::pin(async move {
        match request {
            SmsRequest::Refresh => {
                // The UI owns the user-consent flow and only dispatches a refresh after the user
                // has accepted the session-setting change; while no consent state travels in the
                // command, this path attempts `enable_pdu_mode` once whenever the observed mode is
                // not confirmed PDU. An already-PDU module is left untouched.
                let result = async {
                    let mode = port.query_pdu_mode(target).await?;
                    if matches!(mode, Some(false) | None) {
                        port.enable_pdu_mode(target).await?;
                    }
                    port.list(target).await
                }
                .await;
                match result {
                    Ok(SmsListing { messages, capacity }) => SmsStageOutcome::Refreshed {
                        messages,
                        capacity,
                        status: FeatureStatus::Supported,
                    },
                    Err(error) => SmsStageOutcome::Failed(error),
                }
            }
            SmsRequest::Read { index } => match port.read(target, index).await {
                Ok(message) => SmsStageOutcome::Read { message },
                Err(error) => SmsStageOutcome::Failed(error),
            },
            SmsRequest::Delete { index } => match port.delete(target, index).await {
                Ok(()) => SmsStageOutcome::Deleted,
                Err(error) => SmsStageOutcome::Failed(error),
            },
            SmsRequest::Send { .. } => unreachable!("send owns its dedicated controlled worker"),
        }
    })
}
/// Map one module-transaction failure onto the feature verdict vocabulary (research §8.1).
///
/// A transaction that never completed (`Timeout`, `DeviceRemoved`, an AT final error, an
/// unexpected response) is a transport failure. An unconfirmed capability (`Unsupported`,
/// `CapabilityUnavailable`, the platform's `sms:pdu_mode_required`) is intentionally recorded as
/// `TemporarilyUnavailable` — `UnsupportedConfirmed` requires interpretable evidence that the
/// module cannot do this at all, and a mode/session precondition does not prove that.
fn map_sms_error(error: &crate::PortError) -> FeatureStatus {
    match error.code.stable().as_str() {
        "sms:timeout" | "sms:device_removed" | "app:stage_timeout" => {
            FeatureStatus::TransportFailure
        }
        "sms:unsupported"
        | "sms:pdu_mode_required"
        | "sms:pdu_confirm_failed"
        | "sms:verification_failed"
        | "sms:internal" => FeatureStatus::TemporarilyUnavailable,
        _ => match error.code.category() {
            dji4g_domain::ErrorCode::Unsupported
            | dji4g_domain::ErrorCode::CapabilityUnavailable => {
                FeatureStatus::TemporarilyUnavailable
            }
            dji4g_domain::ErrorCode::DeviceRemoved | dji4g_domain::ErrorCode::Timeout => {
                FeatureStatus::TransportFailure
            }
            _ => FeatureStatus::TemporarilyUnavailable,
        },
    }
}

/// Spawn the detached worker for one monitoring stage and return the receiver of its outcome.
///
/// Workers are deliberately detached, stateless, and bounded: each holds only an owned `Arc`
/// port handle plus that stage's inputs, drives exactly one port future to completion with
/// [`poll_ready`], and reports once. They never touch the controller or the reducer, so the
/// controller stays single-threaded. If the runner has already given up on the stage after
/// [`STAGE_TIMEOUT`], the receiver is gone and the late `send` fails; dropping that late result
/// is the documented tradeoff, so the failed send is explicitly ignored. The process is
/// long-lived, so a worker still blocked inside a port call costs one parked thread until the
/// underlying call returns — acceptable for a handful of bounded, one-shot workers, and far
/// cheaper than letting one wedged stage stall the monitor forever.
fn spawn_stage<T, Job>(job: Job) -> mpsc::Receiver<Result<T, crate::PortError>>
where
    T: Send + 'static,
    Job: FnOnce() -> Result<T, crate::PortError> + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = job();
        // The receiver may already be gone after a stage timeout; the late result is dropped.
        let _ = sender.send(outcome);
    });
    receiver
}

/// Wait for one stage's outcome until `deadline`. `None` means the budget expired (or the
/// worker died without reporting), and the runner moves on without the stage's real result.
fn join_stage<T>(
    receiver: mpsc::Receiver<Result<T, crate::PortError>>,
    deadline: Instant,
) -> Option<Result<T, crate::PortError>> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    match receiver.recv_timeout(remaining) {
        Ok(outcome) => Some(outcome),
        // Timeout: the stage exceeded its watchdog budget. Disconnected: the worker panicked or
        // vanished without reporting. Both are reported as `app:stage_timeout` so the cycle
        // always completes and the next refresh can proceed normally.
        Err(mpsc::RecvTimeoutError::Timeout) | Err(mpsc::RecvTimeoutError::Disconnected) => None,
    }
}

/// Map a stage's raw outcome onto the reducer vocabulary, substituting the watchdog failure
/// `app:stage_timeout` when the stage produced nothing within its budget. The timestamp is taken
/// on the runner thread from the controller clock so `observed_at` stays on one deterministic
/// clock (workers have no controller access and must not stamp wall-clock time themselves).
fn stage_check<T>(
    outcome: Option<Result<T, crate::PortError>>,
    observed_at: SystemTime,
) -> CheckResult<T> {
    match outcome {
        Some(Ok(value)) => CheckResult::Passed { value, observed_at },
        Some(Err(error)) => CheckResult::Failed {
            code: error.code,
            observed_at,
        },
        None => CheckResult::Failed {
            code: crate::PortError::new(dji4g_domain::ErrorCode::ProbeFailed, "app:stage_timeout")
                .code,
            observed_at,
        },
    }
}

fn poll_ready<'a, T>(
    mut future: crate::PortFuture<'a, T>,
    budget: Duration,
    on_timeout: impl FnOnce() -> T,
) -> T {
    let mut context = Context::from_waker(Waker::noop());
    let started = Instant::now();
    loop {
        match Future::poll(future.as_mut(), &mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                // A stage future that never becomes ready (a wedged WinRT await, for example)
                // must not spin a detached worker thread forever: the worker self-terminates at
                // its budget and parks 1 ms between polls so the bounded wait does not burn a
                // core.  The runner's own watchdog stays as the second line of defence.
                if started.elapsed() >= budget {
                    return on_timeout();
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

/// The canonical watchdog outcome, shared by the runner-side deadline and the worker-side
/// bounded drive so both report the identical stable code.
fn stage_timeout_error() -> crate::PortError {
    crate::PortError::new(dji4g_domain::ErrorCode::Timeout, "app:stage_timeout")
}
