//! No-device regression coverage for checked, frozen, multi-fragment deletion.
use dji4g_application::*;
use dji4g_domain::{SmsConcatReference, SmsEncoding, SmsMultipartInfo, SmsStatus};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    gate: AtomicBool,
    started: AtomicBool,
    calls: AtomicUsize,
}
impl DeletePort {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            gate: AtomicBool::new(false),
            started: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        })
    }
    fn wait_until_released(&self) {
        self.started.store(true, Ordering::SeqCst);
        while !self.gate.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
impl SmsPort for DeletePort {
    fn query_pdu_mode(&self, _: &TargetContext) -> PortFuture<'_, Result<Option<bool>, PortError>> {
        Box::pin(async { Ok(Some(true)) })
    }
    fn enable_pdu_mode(&self, _: &TargetContext) -> PortFuture<'_, Result<(), PortError>> {
        unavailable()
    }
    fn list(&self, _: &TargetContext) -> PortFuture<'_, Result<SmsListing, PortError>> {
        Box::pin(async move {
            self.wait_until_released();
            Ok(SmsListing {
                messages: vec![late_message()],
                capacity: None,
            })
        })
    }
    fn read(&self, _: &TargetContext, _: u32) -> PortFuture<'_, Result<SmsMessage, PortError>> {
        Box::pin(async move {
            self.wait_until_released();
            Ok(late_message())
        })
    }
    fn delete(&self, _: &TargetContext, _: u32) -> PortFuture<'_, Result<(), PortError>> {
        panic!("unchecked deletion forbidden")
    }
    fn delete_checked(
        &self,
        _: &TargetContext,
        _: &SmsFragmentKey,
        _: SmsDeleteControl,
    ) -> PortFuture<'_, SmsDeleteReceipt> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        panic!("delete must not start while read worker is active")
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
fn late_message() -> SmsMessage {
    SmsMessage::new(
        99,
        SmsStorageId("SM".into()),
        1,
        0,
        "+8613800138000",
        "late read",
        SmsEncoding::Ucs2,
        SmsStatus::Received,
    )
}

#[test]
fn timed_out_read_or_refresh_keeps_busy_until_worker_actually_finishes() {
    for changed_context in [false, true] {
        for command in [UiCommand::SmsRead { index: 1 }, UiCommand::SmsRefresh] {
            let port = DeletePort::new();
            port.gate.store(false, Ordering::SeqCst);
            let (runner, parts) = runner(port.clone(), 1);
            let mut runner = runner.with_stage_timeout(Duration::from_millis(10));
            runner.controller_mut().handle_command(command).unwrap();
            runner.poll_sms_requests();
            let revision = runner.controller().snapshot().publication_revision;
            runner.run_one_refresh();
            let refresh_deferred = runner.controller().snapshot().publication_revision == revision;
            if changed_context {
                runner
                    .controller_mut()
                    .apply_backend_event(BackendEvent::EpochInvalidated {
                        next_epoch: DeviceEpoch(2),
                        reason: EpochInvalidationReason::PhysicalRemoval,
                    });
            }
            let busy = runner.controller().snapshot().serial_work_busy;
            let rejected = runner
                .controller_mut()
                .handle_command(UiCommand::SmsDelete { fragments: parts })
                .is_err();
            port.gate.store(true, Ordering::SeqCst);
            assert!(
                port.started.load(Ordering::SeqCst),
                "read worker must actually have started"
            );
            assert!(
                refresh_deferred,
                "automatic monitoring must defer while reader still owns the serial port"
            );
            assert!(
                busy,
                "a watchdog expiry is not proof that the serial worker has stopped"
            );
            assert!(
                rejected,
                "new delete must wait until timed-out reader actually relinquishes the port"
            );
            let deadline = Instant::now() + Duration::from_secs(2);
            while runner.sms_pending() {
                assert!(Instant::now() < deadline);
                runner.poll_sms_requests();
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(!runner.controller().snapshot().serial_work_busy);
            assert_eq!(
                runner.controller().snapshot().sms_messages.len(),
                if changed_context { 0 } else { 1 },
                "late timed-out data must not be applied"
            );
            assert_eq!(port.calls.load(Ordering::SeqCst), 0);
        }
    }
}
