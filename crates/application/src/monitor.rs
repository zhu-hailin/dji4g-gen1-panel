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
    AdapterPort, AtPort, BackendEvent, CheckResult, Controller, ControllerHandle, DeviceToolsPort,
    FeatureStatus, HotspotControl, InventoryPort, NetworkProbePort, SmsListing, SmsPort,
    SmsRequest, SmsSendResult, TargetContext, UiCommand,
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
    /// Device-tool transactions. `None` means this build cannot run tools at all: a request is
    /// refused as unsupported immediately instead of sitting queued forever.
    pub device_tools: Option<Arc<dyn DeviceToolsPort>>,
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
    host_port: Option<Arc<dyn crate::HostNetworkPort>>,
    pending_host:
        Option<mpsc::Receiver<Result<crate::host_network::HostNetworkOutcome, crate::PortError>>>,
    host_auto_requested: bool,
    last_host_trigger: Option<Instant>,
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
    /// A timed-out ordinary read/list may still be inside synchronous serial I/O.
    /// Retain its completion channel so timeout cannot release serial ownership early.
    pending_sms_read: Option<PendingSmsRead>,
    sms_read_timeout: Duration,
    pending_sms_delete: Option<PendingSmsDelete>,
    sms_delete_timeout: Duration,
    sms_timeout: Duration,
    refresh_deferred: bool,
    pending_tool: Option<PendingTool>,
    /// Ids for the individual transactions inside a batch. Distinct from the task id the UI
    /// tracks, so each read's receipt and history entry can be correlated on its own.
    next_tool_item_id: u64,
    /// Until this instant new tool work is refused after a worker could not be reclaimed.
    tool_busy_until: Option<Instant>,
    /// A tool task deferred the automatic refresh; resume exactly one sweep afterwards.
    tool_refresh_deferred: bool,
}

struct PendingSms {
    request: SmsRequest,
    receiver: mpsc::Receiver<Result<crate::SmsSendReceipt, crate::PortError>>,
    control: SmsTransactionControl,
    epoch: dji4g_domain::DeviceEpoch,
    sim_epoch: u64,
}

struct PendingSmsRead {
    request: SmsRequest,
    receiver: mpsc::Receiver<Result<(SmsStageOutcome, Instant), crate::PortError>>,
    control: dji4g_domain::SmsReadControl,
    epoch: dji4g_domain::DeviceEpoch,
    sim_epoch: u64,
    outcome: Option<(SmsStageOutcome, Instant)>,
    abandoned: Option<SmsReadAbandonment>,
    last_progress: (dji4g_domain::SmsReadPhase, usize),
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SmsReadAbandonment {
    Timeout,
    Cancelled,
    ContextChanged,
}

struct PendingSmsDelete {
    request_id: u64,
    next: usize,
    in_flight: Option<InFlightSmsDelete>,
}

struct InFlightSmsDelete {
    fragment: crate::SmsFragmentKey,
    receiver: mpsc::Receiver<crate::SmsDeleteReceipt>,
    control: crate::SmsDeleteControl,
    receipt: Option<crate::SmsDeleteReceipt>,
}

/// One tool task in flight on its own worker.
struct PendingTool {
    /// The id the UI sees. A capability sweep keeps one id for the whole batch.
    parent_id: u64,
    context: crate::ToolContext,
    /// Verified target the port is allowed to touch, resolved once for the whole batch.
    target: TargetContext,
    /// Sub-requests still to run, including the one in flight.
    queue: std::collections::VecDeque<crate::ToolOperation>,
    in_flight: Option<InFlightTool>,
    /// Absolute deadline for a capability sweep; `None` for a single command.
    batch_deadline: Option<Instant>,
    completed: usize,
    total: usize,
    /// The outcome of the most recent item. A single-command task reports exactly this, so a
    /// command whose effect could not be confirmed is never rounded up to a completed batch.
    last_outcome: Option<crate::ToolOutcome>,
}

struct InFlightTool {
    request: crate::ToolRequest,
    receiver: mpsc::Receiver<Result<crate::ToolReceipt, crate::PortError>>,
    control: crate::ToolControl,
    started_at: Instant,
    /// Set while the runner is waiting for a cancelled worker to release the port.
    cancelling_since: Option<Instant>,
}

/// How long the runner waits for a cancelled or timed-out tool worker before it reports the task
/// finished-but-unreclaimed. The port lease stays held by the worker either way, so a new task
/// still cannot overlap it; this only decides what the UI is told.
const TOOL_RECLAIM_GRACE: Duration = Duration::from_secs(5);

/// How long new tool work stays refused after a worker failed to release the port in time.
const TOOL_BUSY_WINDOW: Duration = Duration::from_secs(5);

impl Drop for ControllerRunner {
    fn drop(&mut self) {
        if let Some(read) = &self.pending_sms_read {
            read.control.cancel();
        }
        if let Some(item) = self
            .pending_sms_delete
            .as_ref()
            .and_then(|p| p.in_flight.as_ref())
        {
            item.control.cancel();
        }
        if let Some(pending) = &self.pending_sms {
            pending.control.cancel();
        }
        // A cancelled tool task must not outlive the runner either.
        if let Some(in_flight) = self
            .pending_tool
            .as_ref()
            .and_then(|pending| pending.in_flight.as_ref())
        {
            in_flight.control.cancel();
        }
    }
}

impl ControllerRunner {
    #[must_use]
    pub fn new(controller: Controller) -> (ControllerHandle, Self) {
        let initial = Arc::new(controller.snapshot());
        let (handle, commands, refresh) = ControllerHandle::channels(initial);
        let runner = Self {
            host_port: None,
            pending_host: None,
            host_auto_requested: false,
            last_host_trigger: None,
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
            pending_sms_read: None,
            sms_read_timeout: Duration::from_secs(60),
            pending_sms_delete: None,
            sms_delete_timeout: dji4g_domain::SMS_DELETE_TIMEOUT,
            sms_timeout: SMS_SEND_TIMEOUT,
            refresh_deferred: false,
            pending_tool: None,
            next_tool_item_id: 1,
            tool_busy_until: None,
            tool_refresh_deferred: false,
        };
        (handle, runner)
    }

    #[must_use]
    pub fn with_ports(mut self, ports: MonitorPorts) -> Self {
        self.ports = Some(ports);
        self
    }

    #[must_use]
    pub fn with_host_network_port(mut self, port: Arc<dyn crate::HostNetworkPort>) -> Self {
        self.host_port = Some(port);
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
        self.sms_read_timeout = stage_timeout;
        self
    }

    pub fn with_sms_timeout(mut self, timeout: Duration) -> Self {
        self.sms_timeout = timeout;
        self
    }
    /// Test seam; production retains the shared 30-second per-fragment deadline.
    pub fn with_sms_delete_timeout(mut self, timeout: Duration) -> Self {
        self.sms_delete_timeout = timeout;
        self
    }
    pub fn sms_pending(&self) -> bool {
        self.pending_sms.is_some()
            || self.pending_sms_read.is_some()
            || self.pending_sms_delete.is_some()
            || self.controller.sms_delete_active()
    }

    /// Whether a device-tool task is currently in flight.
    #[must_use]
    pub fn tool_pending(&self) -> bool {
        self.pending_tool.is_some() || self.controller.tool_active()
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
            if !self.host_auto_requested && self.host_port.is_some() {
                self.host_auto_requested = true;
                let _ = self
                    .controller
                    .handle_command(UiCommand::InspectHostNetwork);
                self.publish();
            }
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
            // Device-tool tasks run on their own worker and are advanced one step per iteration;
            // the runner thread never waits inside a tool command, so the UI keeps repainting and
            // a cancel is acted on within one poll interval.
            if self.poll_tool_requests() {
                self.publish();
            }
            if self.poll_host_requests() {
                self.publish();
            }
            // An explicit user command and the automatic monitoring cadence share one scan path.
            // Relying on the signal alone left a release build permanently unscanned, because its
            // only startup `Refresh` was compiled out and no other producer sets the signal.
            let signaled = self.refresh.take();
            if signaled
                && self.host_port.is_some()
                && self
                    .last_host_trigger
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(2))
            {
                self.last_host_trigger = Some(Instant::now());
                let _ = self
                    .controller
                    .handle_command(UiCommand::InspectHostNetwork);
                self.publish();
            }
            self.refresh_deferred |= signaled;
            if !self.sms_pending()
                && !self.tool_pending()
                && (self.refresh_deferred
                    || self.controller.network_check_queued()
                    || self.monitoring_cadence_due())
            {
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

    /// Advance one host task without taking the serial port or blocking the UI/controller loop.
    pub fn poll_host_requests(&mut self) -> bool {
        if let Some(receiver) = &self.pending_host {
            match receiver.try_recv() {
                Ok(Ok(outcome)) => {
                    self.pending_host = None;
                    self.controller.complete_host_request(outcome);
                    return true;
                }
                Ok(Err(error)) => {
                    self.pending_host = None;
                    self.controller.fail_host_request(&error);
                    return true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending_host = None;
                    self.controller.fail_host_request(&crate::PortError::new(
                        dji4g_domain::ErrorCode::Internal,
                        "host:worker_lost",
                    ));
                    return true;
                }
                Err(mpsc::TryRecvError::Empty) => return false,
            }
        }
        let Some(request) = self.controller.take_host_request() else {
            return false;
        };
        let Some(port) = self.host_port.as_ref().cloned() else {
            self.controller.fail_host_request(&crate::PortError::new(
                dji4g_domain::ErrorCode::CapabilityUnavailable,
                "host:port_unavailable",
            ));
            return true;
        };
        self.pending_host = Some(spawn_stage(move || {
            use crate::host_network::{HostNetworkOutcome as O, HostNetworkRequest as R};
            let timeout =
                || crate::PortError::new(dji4g_domain::ErrorCode::Timeout, "host:task_timeout");
            match request {
                R::Inspect => {
                    poll_ready(port.inspect(), Duration::from_secs(30), || Err(timeout()))
                        .map(O::Inspected)
                }
                R::Prepare(id) => {
                    poll_ready(port.prepare_repair(id), Duration::from_secs(30), || {
                        Err(timeout())
                    })
                    .map(O::Prepared)
                }
                R::Apply(id) => poll_ready(port.apply_repair(id), Duration::from_secs(30), || {
                    Err(timeout())
                })
                .map(O::Applied),
                R::Restore(id) => {
                    poll_ready(port.restore_repair(id), Duration::from_secs(30), || {
                        Err(timeout())
                    })
                    .map(|()| O::Restored)
                }
            }
        }));
        true
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
        if self.pending_sms_read.is_some() {
            return self.poll_sms_read();
        }
        if self.pending_sms_delete.is_some() {
            return self.poll_sms_delete();
        }
        if self.pending_sms.is_some() {
            return self.poll_sms_completion();
        }
        let Some(sms_port) = self
            .ports
            .as_ref()
            .and_then(|ports| ports.sms.as_ref())
            .map(Arc::clone)
        else {
            if self.controller.sms_delete_active() {
                if let Some(SmsRequest::Delete { request_id }) =
                    self.controller.take_next_sms_request()
                {
                    self.controller.record_sms_delete_item(
                        request_id,
                        0,
                        crate::SmsDeleteReceipt {
                            result: crate::SmsDeleteItemResult::NotAttempted,
                            code: Some("sms:checked_delete_unavailable".into()),
                        },
                    );
                    self.controller.finish_sms_delete(request_id);
                    self.publish();
                    return true;
                }
            }
            return false;
        };
        let Some(request) = self.controller.take_next_sms_request() else {
            return false;
        };
        if matches!(request, SmsRequest::Send { .. }) {
            self.start_sms_send(request, sms_port);
        } else if let SmsRequest::Delete { request_id } = request {
            self.pending_sms_delete = Some(PendingSmsDelete {
                request_id,
                next: 0,
                in_flight: None,
            });
            self.poll_sms_delete();
        } else {
            self.controller.set_sms_read_in_flight(true);
            self.run_sms_request(request, &sms_port);
            if self.pending_sms_read.is_none() {
                self.controller.set_sms_read_in_flight(false);
            }
        }
        true
    }

    fn poll_sms_delete(&mut self) -> bool {
        use crate::SmsDeleteItemResult as Result;
        let Some(mut pending) = self.pending_sms_delete.take() else {
            return false;
        };
        if let Some(mut item) = pending.in_flight.take() {
            let changed = (
                self.controller.state().epoch().0,
                self.controller.state().sim_epoch(),
            ) != (item.fragment.device_epoch, item.fragment.sim_epoch);
            if changed || item.control.is_expired() {
                item.control.cancel();
            }
            if item.receipt.is_none() {
                match item.receiver.try_recv() {
                    Ok(receipt) => item.receipt = Some(receipt),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        item.receipt = Some(crate::SmsDeleteReceipt {
                            result: if item.control.delete_attempted() {
                                Result::OutcomeUnknown
                            } else {
                                Result::Failed
                            },
                            code: Some("sms:delete_worker_closed".into()),
                        })
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            // A timeout requests cancellation; it is never permission to reuse a live worker's port.
            if item.receipt.is_none() || item.control.cleanup_pending() {
                pending.in_flight = Some(item);
                self.pending_sms_delete = Some(pending);
                return false;
            }
            let receipt = item.receipt.take().expect("checked receipt");
            let stop = changed || receipt.result != Result::Deleted;
            self.controller
                .record_sms_delete_item(pending.request_id, pending.next, receipt);
            pending.next += 1;
            if stop {
                self.controller.finish_sms_delete(pending.request_id);
                self.publish();
                return true;
            }
        }
        let fragment = self
            .controller
            .sms_delete_snapshot()
            .filter(|batch| batch.request_id == pending.request_id)
            .and_then(|batch| batch.items.get(pending.next))
            .map(|item| item.fragment.clone());
        let Some(fragment) = fragment else {
            self.controller.finish_sms_delete(pending.request_id);
            self.publish();
            return true;
        };
        let valid = (
            self.controller.state().epoch().0,
            self.controller.state().sim_epoch(),
        ) == (fragment.device_epoch, fragment.sim_epoch)
            && self
                .controller
                .state()
                .sms_store()
                .contains_fragment(&fragment);
        let target = self.controller.state().target_context();
        let port = self.ports.as_ref().and_then(|p| p.sms.as_ref()).cloned();
        if !valid || target.is_none() || port.is_none() {
            self.controller.record_sms_delete_item(
                pending.request_id,
                pending.next,
                crate::SmsDeleteReceipt {
                    result: Result::NotAttempted,
                    code: Some("sms:delete_target_changed".into()),
                },
            );
            self.controller.finish_sms_delete(pending.request_id);
            self.publish();
            return true;
        }
        let target = target.expect("checked target");
        let port = port.expect("checked port");
        let control = crate::SmsDeleteControl::new(self.sms_delete_timeout);
        let worker_control = control.clone();
        let worker_fragment = fragment.clone();
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("dji4g-sms-delete".into())
            .spawn(move || {
                let receipt = poll_ready(
                    port.delete_checked(&target, &worker_fragment, worker_control.clone()),
                    worker_control.remaining(),
                    || {
                        worker_control.cancel();
                        crate::SmsDeleteReceipt {
                            result: if worker_control.delete_attempted() {
                                Result::OutcomeUnknown
                            } else {
                                Result::Failed
                            },
                            code: Some("sms:delete_timeout".into()),
                        }
                    },
                );
                let _ = sender.send(receipt);
            });
        if spawned.is_err() {
            self.controller.record_sms_delete_item(
                pending.request_id,
                pending.next,
                crate::SmsDeleteReceipt {
                    result: Result::Failed,
                    code: Some("sms:delete_worker_failed".into()),
                },
            );
            self.controller.finish_sms_delete(pending.request_id);
        } else {
            pending.in_flight = Some(InFlightSmsDelete {
                fragment,
                receiver,
                control,
                receipt: None,
            });
            self.pending_sms_delete = Some(pending);
        }
        self.publish();
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
        let timeout = self.sms_read_timeout;
        let control = dji4g_domain::SmsReadControl::new(timeout);
        self.controller.set_sms_read_control(Some(control.clone()));
        let port = Arc::clone(sms_port);
        // The request is owned (a send carries its recipient and body) and the runner still needs
        // it to apply the outcome, so the worker gets a clone.
        let worker_request = request.clone();
        let worker_control = control.clone();
        let receiver = spawn_stage(move || {
            let outcome = poll_ready(
                run_sms_port_call(&port, &target, worker_request, worker_control),
                timeout,
                || SmsStageOutcome::timed_out(false),
            );
            // Stamp the actual worker completion, not the later runner/UI poll. A ready result
            // can wait in the channel while adapter work delays the controller's next iteration.
            Ok((outcome, Instant::now()))
        });
        self.pending_sms_read = Some(PendingSmsRead {
            request,
            receiver,
            epoch: self.controller.state().epoch(),
            sim_epoch: self.controller.state().sim_epoch(),
            last_progress: (control.phase(), control.progress()),
            control,
            outcome: None,
            abandoned: None,
        });
    }

    fn poll_sms_read(&mut self) -> bool {
        let Some(mut pending) = self.pending_sms_read.take() else {
            return false;
        };
        if pending.outcome.is_none() {
            match pending.receiver.try_recv() {
                Ok(Ok(outcome)) => pending.outcome = Some(outcome),
                Ok(Err(error)) => {
                    pending.outcome = Some((SmsStageOutcome::Failed(error), Instant::now()))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    pending.outcome = Some((SmsStageOutcome::timed_out(false), Instant::now()))
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let completed_in_time = pending
            .outcome
            .as_ref()
            .is_some_and(|(_, completed_at)| *completed_at <= pending.control.deadline());
        let context_changed = pending.epoch != self.controller.state().epoch()
            || pending.sim_epoch != self.controller.state().sim_epoch();
        let previous_abandonment = pending.abandoned;
        if context_changed {
            pending.abandoned = Some(SmsReadAbandonment::ContextChanged);
        } else if pending.abandoned.is_none() {
            if pending.control.is_cancelled() {
                pending.abandoned = Some(SmsReadAbandonment::Cancelled);
            } else if !completed_in_time && pending.control.is_expired() {
                pending.abandoned = Some(SmsReadAbandonment::Timeout);
            }
        }
        let mut changed = false;
        if pending.abandoned != previous_abandonment {
            pending.control.cancel();
            self.controller
                .set_sms_inbox_failure(Some(crate::PortError::new(
                    dji4g_domain::ErrorCode::Timeout,
                    match pending.abandoned.expect("new abandonment") {
                        SmsReadAbandonment::ContextChanged => "sms:context_changed",
                        SmsReadAbandonment::Timeout => "sms:timeout",
                        SmsReadAbandonment::Cancelled => "sms:read_cancelled",
                    },
                )));
            changed = true;
        }
        let progress = (pending.control.phase(), pending.control.progress());
        changed |= progress != pending.last_progress;
        pending.last_progress = progress;
        if pending.outcome.is_some() && !pending.control.cleanup_pending() {
            if pending.control.restoration() == dji4g_domain::SmsStorageRestoration::Unknown {
                self.controller
                    .set_sms_inbox_failure(Some(crate::PortError::new(
                        dji4g_domain::ErrorCode::Internal,
                        "sms:storage_restore_unknown",
                    )));
            } else if pending.abandoned.is_none()
                || (pending.abandoned == Some(SmsReadAbandonment::Timeout) && completed_in_time)
            {
                // The worker may have been descheduled between stamping completion and sending
                // the receipt. Only a watchdog abandonment can be corrected by timely proof;
                // explicit cancellation and context changes still discard all returned data.
                self.apply_sms_outcome(
                    pending.request,
                    pending.outcome.take().expect("completed read").0,
                );
            }
            self.controller.set_sms_refresh_pending(false);
            self.controller.set_sms_read_control(None);
            self.controller.set_sms_read_in_flight(false);
            self.publish();
            true
        } else {
            self.pending_sms_read = Some(pending);
            if changed {
                self.publish();
            }
            changed
        }
    }

    /// Apply one worker outcome to the controller. Runs on the runner thread only; every mutation
    /// here is a deliberate consequence of a completed module transaction.
    fn apply_sms_outcome(&mut self, request: SmsRequest, outcome: SmsStageOutcome) {
        if matches!(
            request,
            SmsRequest::Refresh | SmsRequest::ReadStorage { .. }
        ) {
            self.controller.set_sms_refresh_pending(false);
        }
        self.controller.set_sms_inbox_failure(None);
        match outcome {
            SmsStageOutcome::Refreshed {
                messages,
                capacity,
                status,
                report,
            } => {
                self.controller.set_sms_read_report(report);
                self.controller.reconcile_sms_listed_slots(&messages);
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
            && !self.tool_busy()
            && periodic_refresh_due(self.last_refresh_at, now, REFRESH_INTERVAL)
    }

    /// Whether a tool task is holding the serial port, or a worker that could not be reclaimed
    /// still may be.
    #[must_use]
    fn tool_busy(&self) -> bool {
        self.pending_tool.is_some()
            || self
                .tool_busy_until
                .is_some_and(|until| Instant::now() < until)
            || self.controller.tool_active()
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

    // -------------------------------------------------------------------------------------
    // Device tools
    // -------------------------------------------------------------------------------------

    /// Advance the device-tool pipeline by one step.
    ///
    /// Nothing here blocks: a tool transaction runs on its own worker and is polled once per loop
    /// iteration, so the UI stays responsive and a cancel is noticed within one poll interval. A
    /// capability sweep is expanded into its individual reads here — the port never receives a
    /// bulk request — and the parent task stays busy across the whole batch, so no SMS or repair
    /// transaction can slip between two sub-requests.
    pub fn poll_tool_requests(&mut self) -> bool {
        if self.pending_tool.is_some() {
            return self.poll_tool_completion();
        }
        let port = self
            .ports
            .as_ref()
            .and_then(|ports| ports.device_tools.as_ref())
            .map(Arc::clone);
        let Some(request) = self.controller.take_next_tool_request() else {
            return false;
        };
        let Some(port) = port else {
            // No tool port is wired in this build: say so instead of leaving the task queued
            // forever.
            self.controller
                .refuse_tool_task(request.id, crate::ToolOutcome::Unsupported);
            return true;
        };
        self.start_tool_task(request, port);
        true
    }

    fn start_tool_task(&mut self, request: crate::ToolRequest, port: Arc<dyn DeviceToolsPort>) {
        let current = (
            self.controller.state().epoch(),
            self.controller.snapshot().sim_epoch,
        );
        if (request.context.device_epoch, request.context.sim_epoch) != current {
            self.controller
                .refuse_tool_task(request.id, crate::ToolOutcome::ContextChanged);
            return;
        }
        // The device may have been swapped, or an SMS/repair may have started, while the request
        // waited in the queue.
        if self.controller.tool_start_conflict() {
            self.controller
                .refuse_tool_task(request.id, crate::ToolOutcome::Rejected);
            return;
        }
        let Some(target) = self.controller.state().target_context() else {
            self.controller
                .refuse_tool_task(request.id, crate::ToolOutcome::TransportFailure);
            return;
        };
        let (queue, batch_deadline) = match &request.operation {
            crate::ToolOperation::ProbeAll => (
                dji4g_at_protocol::ToolReadId::ALL
                    .iter()
                    .copied()
                    .map(crate::ToolOperation::Read)
                    .collect::<std::collections::VecDeque<_>>(),
                Some(Instant::now() + crate::PROBE_BATCH_BUDGET),
            ),
            operation => (std::collections::VecDeque::from([operation.clone()]), None),
        };
        let total = queue.len();
        self.controller.record_tool_progress(request.id, 0, total);
        self.controller
            .set_tool_phase(request.id, crate::ToolPhase::Running);
        self.pending_tool = Some(PendingTool {
            parent_id: request.id,
            context: request.context,
            target,
            queue,
            in_flight: None,
            batch_deadline,
            completed: 0,
            total,
            last_outcome: None,
        });
        self.start_tool_item(&port);
    }

    /// Start the next sub-request, if the batch still has budget.
    fn start_tool_item(&mut self, port: &Arc<dyn DeviceToolsPort>) {
        let Some(pending) = self.pending_tool.as_mut() else {
            return;
        };
        if pending.in_flight.is_some() {
            return;
        }
        let batch_remaining = pending
            .batch_deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()));
        if batch_remaining.is_some_and(|remaining| remaining.is_zero()) {
            // Out of budget. The remaining items are not attempted this round; the caller reports
            // them as not executed rather than inventing a result for them.
            return;
        }
        let Some(operation) = pending.queue.pop_front() else {
            return;
        };
        let timeout =
            crate::item_deadline(batch_remaining.unwrap_or(crate::TOOL_TRANSACTION_TIMEOUT));
        let item_id = self.next_tool_item_id;
        self.next_tool_item_id = self.next_tool_item_id.saturating_add(1);
        let request = crate::ToolRequest {
            id: item_id,
            context: pending.context.clone(),
            operation,
        };
        let control = crate::ToolControl::new(timeout);
        let worker_control = control.clone();
        let worker_request = request.clone();
        let target = pending.target.clone();
        let worker_port = Arc::clone(port);
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("dji4g-device-tool".into())
            .spawn(move || {
                let outcome = poll_ready(
                    worker_port.execute(&target, worker_request, worker_control.clone()),
                    timeout,
                    || {
                        // The worker's own budget expired. Cancelling the shared handle makes the
                        // platform side tear its session down instead of leaving it waiting.
                        worker_control.cancel();
                        Err(crate::PortError::new(
                            dji4g_domain::ErrorCode::Timeout,
                            "device_tools:timeout",
                        ))
                    },
                );
                let _ = sender.send(outcome);
            });
        let Some(pending) = self.pending_tool.as_mut() else {
            return;
        };
        match spawned {
            Ok(_) => {
                pending.in_flight = Some(InFlightTool {
                    request,
                    receiver,
                    control,
                    started_at: Instant::now(),
                    cancelling_since: None,
                });
            }
            Err(_) => {
                let parent = pending.parent_id;
                self.controller
                    .refuse_tool_task(parent, crate::ToolOutcome::TransportFailure);
            }
        }
    }

    fn poll_tool_completion(&mut self) -> bool {
        let current = (
            self.controller.state().epoch(),
            self.controller.snapshot().sim_epoch,
        );
        let Some(mut pending) = self.pending_tool.take() else {
            return false;
        };
        let context_changed = (pending.context.device_epoch, pending.context.sim_epoch) != current;
        let cancelled = self
            .controller
            .device_tools()
            .task
            .as_ref()
            .is_some_and(|task| {
                task.id == pending.parent_id && task.phase == crate::ToolPhase::Cancelling
            });
        if cancelled || context_changed {
            pending.queue.clear();
        }
        let Some(in_flight) = pending.in_flight.as_mut() else {
            if cancelled
                || context_changed
                || pending.queue.is_empty()
                || pending
                    .batch_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
            {
                self.finish_tool_batch(pending, context_changed);
                return true;
            }
            // Between items: nothing to collect, so start the next one.
            self.pending_tool = Some(pending);
            let port = self
                .ports
                .as_ref()
                .and_then(|ports| ports.device_tools.as_ref())
                .map(Arc::clone);
            if let Some(port) = port {
                self.start_tool_item(&port);
            }
            return self
                .pending_tool
                .as_ref()
                .is_some_and(|pending| pending.total > 0);
        };
        if (cancelled || context_changed) && !in_flight.control.is_cancelled() {
            // The result would belong to another device: stop, and let the outcome say why.
            in_flight.control.cancel();
        }
        let now = Instant::now();
        let mut outcome = None;
        match in_flight.receiver.try_recv() {
            Ok(result) => outcome = Some(result),
            Err(mpsc::TryRecvError::Empty) => {
                if in_flight.control.is_expired() && !in_flight.control.is_cancelled() {
                    in_flight.control.cancel();
                }
                if in_flight.control.is_cancelled() {
                    let since = *in_flight.cancelling_since.get_or_insert(now);
                    if now.saturating_duration_since(since) > TOOL_RECLAIM_GRACE {
                        // The worker did not give the port back in time. The port lease is still
                        // held by it, so the next task cannot overlap it; refuse new tool work for
                        // a bounded window as well instead of hammering a wedged port.
                        self.tool_busy_until = Some(now + TOOL_BUSY_WINDOW);
                        outcome = Some(Err(crate::PortError::new(
                            dji4g_domain::ErrorCode::Timeout,
                            "device_tools:reclaim_timeout",
                        )));
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                outcome = Some(Err(crate::PortError::new(
                    dji4g_domain::ErrorCode::Internal,
                    "device_tools:worker_failed",
                )));
            }
        }
        let Some(outcome) = outcome else {
            self.pending_tool = Some(pending);
            return false;
        };
        let in_flight = pending.in_flight.take().expect("in flight");
        let written = in_flight.control.write_attempted();
        let elapsed = in_flight.started_at.elapsed();
        let receipt = tool_receipt(
            &in_flight.request,
            outcome,
            elapsed,
            written,
            context_changed,
        );
        pending.last_outcome = Some(receipt.outcome);
        let next = self.finish_tool_item(&pending, receipt);
        pending.completed += 1;
        self.controller
            .record_tool_progress(pending.parent_id, pending.completed, pending.total);
        if cancelled {
            self.finish_tool_batch(pending, context_changed);
            return true;
        }
        let port = self
            .ports
            .as_ref()
            .and_then(|ports| ports.device_tools.as_ref())
            .map(Arc::clone);
        match (next, context_changed, port) {
            (NextToolItem::Stop, _, _) => {
                self.finish_tool_batch(pending, context_changed);
            }
            (NextToolItem::Continue, false, Some(port)) => {
                self.pending_tool = Some(pending);
                self.start_tool_item(&port);
            }
            (NextToolItem::Continue, _, _) => {
                self.finish_tool_batch(pending, context_changed);
            }
        }
        true
    }

    /// Publish one sub-request's evidence. Returns whether the batch should continue.
    fn finish_tool_item(
        &mut self,
        pending: &PendingTool,
        receipt: crate::ToolReceipt,
    ) -> NextToolItem {
        let operation = receipt.operation;
        if let crate::ToolOperationKind::Read(id) = operation {
            let now = self.controller.now();
            let mut row = if receipt.outcome == crate::ToolOutcome::Ok
                && receipt.payload_lines == 0
                && id != dji4g_at_protocol::ToolReadId::Attention
            {
                crate::ToolCapabilityRow::empty(id, pending.context.clone(), now)
            } else {
                crate::ToolCapabilityRow::new(id, receipt.outcome, pending.context.clone(), now)
            };
            if let crate::ToolOutcome::Ok = row.reason {
                self.learn_from_read(id, &receipt, &pending.context);
            }
            // A single item's failure stays on that item: the other rows keep their evidence.
            row.observed_at = now;
            self.controller.record_tool_capability(row);
        }
        self.controller.finish_tool_task(receipt);
        if pending.queue.is_empty() {
            NextToolItem::Stop
        } else {
            NextToolItem::Continue
        }
    }

    /// Extract parsed values from one read's response.
    ///
    /// Parsing uses the transcript's response lines only; a module-originated URC arrives prefixed
    /// and can therefore never be mistaken for this command's answer.
    fn learn_from_read(
        &mut self,
        id: dji4g_at_protocol::ToolReadId,
        receipt: &crate::ToolReceipt,
        context: &crate::ToolContext,
    ) {
        use dji4g_at_protocol::ToolReadId as Id;
        let lines = receipt.transcript.lines();
        match id {
            Id::Manufacturer => {
                if let Some(value) = crate::extract_identity(lines, "+CGMI:") {
                    self.controller
                        .update_tool_profile(context, |profile| profile.manufacturer = Some(value));
                }
            }
            Id::Model => {
                if let Some(value) = crate::extract_identity(lines, "+CGMM:") {
                    self.controller
                        .update_tool_profile(context, |profile| profile.model = Some(value));
                }
            }
            Id::Revision => {
                if let Some(value) = crate::extract_identity(lines, "+CGMR:") {
                    self.controller
                        .update_tool_profile(context, |profile| profile.revision = Some(value));
                }
            }
            Id::UsbNet => {
                if let Some(reading) = crate::parse_usb_net(lines) {
                    self.controller
                        .update_tool_profile(context, |profile| profile.usb_net = Some(reading));
                }
            }
            Id::PdpContexts => {
                let contexts = crate::as_at_response(
                    context.device_epoch,
                    dji4g_at_protocol::AtCommand::PdpContexts,
                    &tool_response_of(receipt),
                );
                if let Ok(parsed) = dji4g_at_protocol::parse_pdp_contexts(&contexts) {
                    self.controller
                        .update_tool_profile(context, |profile| profile.pdp_contexts = parsed);
                }
            }
            Id::Temperature => {
                let parsed = crate::parse_profile_temperature(&tool_response_of(receipt));
                if !parsed.is_empty() {
                    self.controller
                        .update_tool_profile(context, |profile| profile.temperature = parsed);
                }
            }
            _ => {}
        }
    }

    /// Close out a batch and publish its terminal state.
    fn finish_tool_batch(&mut self, pending: PendingTool, context_changed: bool) {
        let skipped = pending.total.saturating_sub(pending.completed);
        let outcome = if context_changed {
            crate::ToolOutcome::ContextChanged
        } else if skipped > 0 {
            // Out of budget: the items that did not run are unqueried, not failed.
            if pending.last_outcome == Some(crate::ToolOutcome::OutcomeUnknown) {
                crate::ToolOutcome::OutcomeUnknown
            } else {
                crate::ToolOutcome::CancelledBeforeWrite
            }
        } else if pending.total == 1 {
            // A single command's task row reports that command's own result.
            pending
                .last_outcome
                .unwrap_or(crate::ToolOutcome::OutcomeUnknown)
        } else {
            crate::ToolOutcome::Ok
        };
        self.controller
            .set_tool_phase(pending.parent_id, crate::ToolPhase::Finished);
        self.controller
            .finish_tool_batch_task(pending.parent_id, outcome);
        self.controller
            .record_tool_progress(pending.parent_id, pending.completed, pending.total);
        if context_changed {
            self.tool_busy_until = None;
        }
        self.pending_tool = None;
        // One deferred sweep, not one per skipped cycle.
        if self.tool_refresh_deferred {
            self.tool_refresh_deferred = false;
            self.refresh_deferred = true;
        }
    }

    fn run_refresh(&mut self) {
        if self.controller.interaction_in_flight(self.controller.now())
            || self.controller.host_work_busy()
        {
            self.refresh_deferred = true;
            return;
        }
        // A device-tool task holds the module's AT port. The whole scan is deferred rather than
        // half-run: an AT poll interleaved with a tool command would read the tool's own response.
        if self.controller.tool_active() {
            self.tool_refresh_deferred = true;
            self.refresh_deferred = true;
            return;
        }
        if self.controller.serial_work_busy() || self.pending_sms_read.is_some() {
            self.refresh_deferred = true;
            return;
        }
        // Owned clones of the port handles so stage workers can be `'static`. The runner keeps
        // `self.ports` untouched; an `Arc` clone per stage is all a worker ever sees.
        let Some(ports) = self.ports.clone() else {
            // No ports means a deterministic test/demo backend. Still expose a bounded refresh
            // cycle so a burst cannot create an unbounded queue.
            self.controller.network_check_unavailable();
            return;
        };
        let MonitorPorts {
            device_tools: _,
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
        let allow_probe_once = self.controller.begin_network_check(cycle);
        let epoch = self.controller.state().epoch();
        self.controller
            .apply_backend_event(BackendEvent::RefreshStarted {
                cycle,
                epoch,
                scheduled: crate::CheckMask::all(),
            });

        self.publish();
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
        self.publish();
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

        self.publish();
        // Stages 4 and 5 — probe and hotspot run concurrently. Both need the adapter context,
        // which is derived from the reducer state after the adapter event above has been
        // applied, exactly as the sequential version did. Each stage either runs on a worker or
        // is decided up front (disabled, or its prerequisite is missing); both workers are
        // started before either outcome is joined, and the events are applied in the fixed
        // order probe then hotspot.
        let adapter_context = self.controller.state().adapter_context();
        let probe_plan = if self.controller.state().active_probe() || allow_probe_once {
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
        report: dji4g_domain::SmsReadReport,
    },
    /// One successfully read message.
    Read { message: SmsMessage },
    /// One successfully deleted message (the module returned the final `OK`).
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
/// Whether a batch has more work after the item that just finished.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NextToolItem {
    Continue,
    Stop,
}

/// Turn a port outcome into the task receipt the controller records.
///
/// A port error is classified here, once: a command that was written and then lost is
/// `OutcomeUnknown` (never retried), a command that never reached the port is a plain transport
/// failure, and a result that arrived after the device changed belongs to the old context.
fn tool_receipt(
    request: &crate::ToolRequest,
    outcome: Result<crate::ToolReceipt, crate::PortError>,
    elapsed: Duration,
    written: bool,
    context_changed: bool,
) -> crate::ToolReceipt {
    let operation = request.operation.kind();
    match outcome {
        Ok(mut receipt) => {
            if context_changed {
                receipt.outcome = crate::ToolOutcome::ContextChanged;
            }
            receipt.operation = operation;
            receipt
        }
        Err(_) => crate::ToolReceipt {
            id: request.id,
            context: request.context.clone(),
            operation,
            outcome: if context_changed {
                crate::ToolOutcome::ContextChanged
            } else if written {
                crate::ToolOutcome::OutcomeUnknown
            } else {
                // A port-level failure never reached the final code, whatever its stable code.
                crate::ToolOutcome::TransportFailure
            },
            elapsed,
            transcript: Arc::new(crate::ToolTranscript::new()),
            saw_final_code: false,
            payload_lines: 0,
        },
    }
}

/// Rebuild a `ToolResponse` view from a receipt's collected response lines, for the shared
/// parsers. URC lines are dropped: they belong to the module, not to this command.
fn tool_response_of(receipt: &crate::ToolReceipt) -> dji4g_at_protocol::ToolResponse {
    dji4g_at_protocol::ToolResponse {
        lines: receipt
            .transcript
            .lines()
            .iter()
            .filter(|line| !line.starts_with("[模块主动上报]"))
            .cloned()
            .collect(),
        urc_lines: Vec::new(),
        unclassified_lines: 0,
        final_code: if receipt.saw_final_code && receipt.outcome == crate::ToolOutcome::Ok {
            dji4g_at_protocol::AtFinalCode::Ok
        } else {
            dji4g_at_protocol::AtFinalCode::Error
        },
    }
}

fn run_sms_port_call<'a>(
    port: &'a Arc<dyn SmsPort>,
    target: &'a TargetContext,
    request: SmsRequest,
    control: dji4g_domain::SmsReadControl,
) -> crate::PortFuture<'a, SmsStageOutcome> {
    Box::pin(async move {
        match request {
            SmsRequest::Refresh | SmsRequest::ReadStorage { .. } => {
                // The UI owns the user-consent flow and only dispatches a refresh after the user
                // has accepted the session-setting change; while no consent state travels in the
                // command, this path attempts `enable_pdu_mode` once whenever the observed mode is
                // not confirmed PDU. An already-PDU module is left untouched.
                let storage = match request {
                    SmsRequest::ReadStorage { storage } => Some(storage),
                    _ => None,
                };
                let result = port.list_controlled(target, storage, control).await;
                match result {
                    Ok(crate::SmsReadResult {
                        listing: SmsListing { messages, capacity },
                        report,
                    }) => SmsStageOutcome::Refreshed {
                        messages,
                        capacity,
                        status: FeatureStatus::Supported,
                        report,
                    },
                    Err(error) => SmsStageOutcome::Failed(error),
                }
            }
            SmsRequest::Read { index } => match port.read(target, index).await {
                Ok(message) => SmsStageOutcome::Read { message },
                Err(error) => SmsStageOutcome::Failed(error),
            },
            SmsRequest::Delete { .. } => {
                unreachable!("checked deletion owns its controlled worker")
            }
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
