//! No-device regression coverage for checked, frozen, multi-fragment deletion.
use dji4g_application::*;
use dji4g_domain::{SmsConcatReference, SmsEncoding, SmsMultipartInfo, SmsStatus};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

struct Unused;
fn unavailable<T: Send + 'static>() -> PortFuture<'static, Result<T, PortError>> {
    Box::pin(async { Err(PortError::new(ErrorCode::Unsupported, "test:unused")) })
}
impl InventoryPort for Unused {
    fn scan(&self) -> PortFuture<'_, Result<InventoryObservation, PortError>> {
        unavailable()
    }
}
impl AtPort for Unused {
    fn invalidate(&self, _: DeviceEpoch) {}
    fn observe(&self, _target: &TargetContext) -> PortFuture<'_, Result<AtObservation, PortError>> {
        unavailable()
    }
}
impl AdapterPort for Unused {
    fn resolve(
        &self,
        _target: &TargetContext,
    ) -> PortFuture<'_, Result<AdapterObservationDto, PortError>> {
        unavailable()
    }
    fn read_byte_counters(
        &self,
        _adapter_id: &str,
    ) -> PortFuture<'_, Result<(u64, u64), PortError>> {
        unavailable()
    }
}
impl NetworkProbePort for Unused {
    fn observe(
        &self,
        _adapter: &AdapterContext,
        _active: bool,
    ) -> PortFuture<'_, Result<ProbeObservationDto, PortError>> {
        unavailable()
    }
}

struct DeletePort {
    outcomes: Mutex<VecDeque<SmsDeleteItemResult>>,
    calls: Mutex<Vec<SmsFragmentKey>>,
    gate: Arc<AtomicBool>,
    cleanup_gate: Arc<AtomicBool>,
    started: AtomicBool,
    cancelled: AtomicBool,
}
impl DeletePort {
    fn new(outcomes: &[SmsDeleteItemResult]) -> Arc<Self> {
        Arc::new(Self {
            outcomes: Mutex::new(outcomes.iter().copied().collect()),
            calls: Mutex::new(Vec::new()),
            gate: Arc::new(AtomicBool::new(true)),
            cleanup_gate: Arc::new(AtomicBool::new(true)),
            started: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        })
    }
}
impl SmsPort for DeletePort {
    fn query_pdu_mode(&self, _: &TargetContext) -> PortFuture<'_, Result<Option<bool>, PortError>> {
        unavailable()
    }
    fn enable_pdu_mode(&self, _: &TargetContext) -> PortFuture<'_, Result<(), PortError>> {
        unavailable()
    }
    fn list(&self, _: &TargetContext) -> PortFuture<'_, Result<SmsListing, PortError>> {
        unavailable()
    }
    fn read(&self, _: &TargetContext, _: u32) -> PortFuture<'_, Result<SmsMessage, PortError>> {
        unavailable()
    }
    fn delete(&self, _: &TargetContext, _: u32) -> PortFuture<'_, Result<(), PortError>> {
        panic!("unchecked delete must never be called")
    }
    fn delete_checked(
        &self,
        _: &TargetContext,
        part: &SmsFragmentKey,
        control: SmsDeleteControl,
    ) -> PortFuture<'_, SmsDeleteReceipt> {
        let part = part.clone();
        Box::pin(async move {
            self.calls.lock().unwrap().push(part);
            self.started.store(true, Ordering::SeqCst);
            while !self.gate.load(Ordering::SeqCst) && !control.is_cancelled() {
                std::thread::sleep(Duration::from_millis(1));
            }
            if control.is_cancelled() {
                self.cancelled.store(true, Ordering::SeqCst);
                return SmsDeleteReceipt {
                    result: SmsDeleteItemResult::Failed,
                    code: Some("sms:cancelled".into()),
                };
            }
            let result = self
                .outcomes
                .lock()
                .unwrap()
                .pop_front()
                .expect("no automatic retries");
            if !self.cleanup_gate.load(Ordering::SeqCst) {
                control.mark_cleanup_pending();
                let gate = Arc::clone(&self.cleanup_gate);
                let cleanup = control.clone();
                std::thread::spawn(move || {
                    while !gate.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    cleanup.mark_cleanup_complete();
                });
            }
            SmsDeleteReceipt { result, code: None }
        })
    }
}
fn runner(port: Arc<DeletePort>, count: u8) -> (ControllerRunner, Vec<SmsFragmentKey>) {
    let mut controller = Controller::for_test(SystemTime::UNIX_EPOCH);
    for n in 1..=count {
        let mut message = SmsMessage::new(
            u32::from(n),
            SmsStorageId("SM".into()),
            controller.state().epoch().0,
            controller.snapshot().sim_epoch,
            "+8613800138000",
            format!("fragment {n}"),
            SmsEncoding::Ucs2,
            SmsStatus::Received,
        );
        message.multipart = Some(SmsMultipartInfo {
            reference: SmsConcatReference::SixteenBit(0x1234),
            total: count,
            sequence: n,
        });
        controller.ingest_sms(message);
    }
    let parts = controller.snapshot().sms_messages[0].fragments.clone();
    let (_, runner) = ControllerRunner::new(controller);
    (
        runner.with_ports(MonitorPorts {
            inventory: Arc::new(Unused),
            at: Arc::new(Unused),
            adapter: Arc::new(Unused),
            probe: Arc::new(Unused),
            hotspot: None,
            sms: Some(port),
            device_tools: None,
        }),
        parts,
    )
}
fn queue(runner: &mut ControllerRunner, fragments: Vec<SmsFragmentKey>) {
    runner
        .controller_mut()
        .handle_command(UiCommand::SmsDelete { fragments })
        .unwrap();
    assert!(runner.controller().snapshot().serial_work_busy);
}
fn settle(runner: &mut ControllerRunner) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while runner
        .controller()
        .snapshot()
        .sms_delete
        .as_ref()
        .is_some_and(|b| !b.finished)
    {
        assert!(
            Instant::now() < deadline,
            "delete must finish without blocking the runner"
        );
        runner.poll_sms_requests();
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn whole_message_deletes_every_frozen_fragment_once() {
    let port = DeletePort::new(&[SmsDeleteItemResult::Deleted; 3]);
    let (mut runner, parts) = runner(Arc::clone(&port), 3);
    queue(&mut runner, parts.clone());
    settle(&mut runner);
    assert_eq!(*port.calls.lock().unwrap(), parts);
    assert!(runner.controller().snapshot().sms_messages.is_empty());
    assert!(!runner.controller().snapshot().serial_work_busy);
}
#[test]
fn partial_failure_or_unknown_stops_and_preserves_unconfirmed_fragments() {
    for result in [
        SmsDeleteItemResult::Failed,
        SmsDeleteItemResult::OutcomeUnknown,
    ] {
        let port = DeletePort::new(&[SmsDeleteItemResult::Deleted, result]);
        let (mut runner, parts) = runner(Arc::clone(&port), 3);
        queue(&mut runner, parts);
        settle(&mut runner);
        let snapshot = runner.controller().snapshot();
        let batch = snapshot.sms_delete.unwrap();
        assert_eq!(batch.items[0].result, SmsDeleteItemResult::Deleted);
        assert_eq!(batch.items[1].result, result);
        assert_eq!(batch.items[2].result, SmsDeleteItemResult::NotAttempted);
        assert_eq!(snapshot.sms_messages[0].fragments.len(), 2);
        assert_eq!(port.calls.lock().unwrap().len(), 2);
    }
}
#[test]
fn stale_duplicate_empty_and_partial_group_requests_are_refused() {
    let port = DeletePort::new(&[]);
    let (mut runner, parts) = runner(port, 2);
    let mut stale = parts.clone();
    stale[0].payload_fingerprint = [0; 32];
    for fragments in [
        Vec::new(),
        vec![parts[0].clone(); 2],
        vec![parts[0].clone()],
        stale,
    ] {
        assert!(
            runner
                .controller_mut()
                .handle_command(UiCommand::SmsDelete { fragments })
                .is_err()
        );
        assert!(!runner.controller().snapshot().serial_work_busy);
    }
}
#[test]
fn batch_excludes_send_tool_refresh_read_and_repair_and_releases_afterwards() {
    let port = DeletePort::new(&[SmsDeleteItemResult::Deleted]);
    let (mut runner, parts) = runner(port, 1);
    queue(&mut runner, parts);
    let commands = [
        UiCommand::SmsSend {
            recipient: "+8613800138000".into(),
            body: "x".into(),
        },
        UiCommand::SmsRefresh,
        UiCommand::SmsRead { index: 1 },
        UiCommand::RunToolRead {
            id: dji4g_at_protocol::ToolReadId::Attention,
        },
        UiCommand::PrepareRepair {
            request: ControlledRepairRequest::RestartModule,
        },
    ];
    for command in commands {
        assert!(runner.controller_mut().handle_command(command).is_err());
    }
    settle(&mut runner);
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::SmsRefresh)
            .is_ok()
    );
}
#[test]
fn changed_context_cancels_pending_fragment_and_never_touches_new_device() {
    let port = DeletePort::new(&[SmsDeleteItemResult::Deleted]);
    port.gate.store(false, Ordering::SeqCst);
    let (mut runner, parts) = runner(Arc::clone(&port), 2);
    queue(&mut runner, parts);
    runner.poll_sms_requests();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !port.started.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    runner
        .controller_mut()
        .apply_backend_event(BackendEvent::EpochInvalidated {
            next_epoch: DeviceEpoch(2),
            reason: EpochInvalidationReason::PhysicalRemoval,
        });
    settle(&mut runner);
    assert!(port.cancelled.load(Ordering::SeqCst));
    assert_eq!(port.calls.lock().unwrap().len(), 1);
}
#[test]
fn successful_receipt_does_not_release_busy_before_actor_cleanup() {
    let port = DeletePort::new(&[SmsDeleteItemResult::Deleted]);
    port.cleanup_gate.store(false, Ordering::SeqCst);
    let (mut runner, parts) = runner(Arc::clone(&port), 1);
    queue(&mut runner, parts);
    runner.poll_sms_requests();
    for _ in 0..20 {
        runner.poll_sms_requests();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(runner.controller().snapshot().serial_work_busy);
    assert!(!runner.controller().snapshot().sms_delete.unwrap().finished);
    port.cleanup_gate.store(true, Ordering::SeqCst);
    settle(&mut runner);
    assert!(!runner.controller().snapshot().serial_work_busy);
}

#[test]
fn deadline_cancels_the_actual_delete_and_does_not_retry() {
    let port = DeletePort::new(&[]);
    port.gate.store(false, Ordering::SeqCst);
    let (runner, parts) = runner(Arc::clone(&port), 2);
    let mut runner = runner.with_sms_delete_timeout(Duration::from_millis(20));
    queue(&mut runner, parts);
    settle(&mut runner);
    assert!(port.cancelled.load(Ordering::SeqCst));
    assert_eq!(port.calls.lock().unwrap().len(), 1);
    let snapshot = runner.controller().snapshot();
    assert_eq!(
        snapshot.sms_delete.unwrap().items[1].result,
        SmsDeleteItemResult::NotAttempted
    );
    assert_eq!(snapshot.sms_messages[0].fragments.len(), 2);
    assert!(!snapshot.serial_work_busy);
}

#[test]
fn unavailable_sms_port_finishes_without_removing_records() {
    let (mut runner, parts) = runner(DeletePort::new(&[]), 1);
    runner = runner.with_ports(MonitorPorts {
        inventory: Arc::new(Unused),
        at: Arc::new(Unused),
        adapter: Arc::new(Unused),
        probe: Arc::new(Unused),
        hotspot: None,
        sms: None,
        device_tools: None,
    });
    queue(&mut runner, parts);
    settle(&mut runner);
    let snapshot = runner.controller().snapshot();
    assert_eq!(snapshot.sms_messages.len(), 1);
    assert!(!snapshot.serial_work_busy);
    assert_eq!(
        snapshot.sms_delete.unwrap().items[0].result,
        SmsDeleteItemResult::NotAttempted
    );
}
