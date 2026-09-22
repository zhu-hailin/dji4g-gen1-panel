//! No-device regression coverage for checked, frozen, multi-fragment deletion.
use dji4g_application::*;
use dji4g_domain::{SmsConcatReference, SmsEncoding, SmsMultipartInfo, SmsStatus};
use std::{
    sync::{
        Arc, Mutex,
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
fn runner(port: Arc<dyn SmsPort>, count: u8) -> (ControllerRunner, Vec<SmsFragmentKey>) {
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
fn history_listing_does_not_block_command_polling() {
    let port = DeletePort::new();
    let (runner, _) = runner(port.clone(), 1);
    let mut runner = runner.with_stage_timeout(Duration::from_millis(300));
    runner
        .controller_mut()
        .handle_command(UiCommand::SmsRefresh)
        .unwrap();
    let start = Instant::now();
    runner.poll_sms_requests();
    let elapsed = start.elapsed();
    port.gate.store(true, Ordering::SeqCst);
    assert!(
        elapsed < Duration::from_millis(100),
        "blocked command polling for {elapsed:?}"
    );
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
            // Async polling returns immediately; advance the watchdog while the fake I/O remains blocked.
            let deadline = Instant::now() + Duration::from_millis(20);
            while Instant::now() < deadline {
                runner.poll_sms_requests();
                std::thread::sleep(Duration::from_millis(1));
            }
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

struct ControlledReadPort {
    control: Mutex<Option<dji4g_domain::SmsReadControl>>,
    release: AtomicBool,
    completed: AtomicBool,
    hold_cleanup: bool,
    restoration_unknown: bool,
}
impl ControlledReadPort {
    fn new(hold_cleanup: bool, restoration_unknown: bool) -> Arc<Self> {
        Arc::new(Self {
            control: Mutex::new(None),
            release: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            hold_cleanup,
            restoration_unknown,
        })
    }
    fn wait_started(&self) -> dji4g_domain::SmsReadControl {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(control) = self.control.lock().unwrap().clone() {
                return control;
            }
            assert!(Instant::now() < deadline, "controlled read did not start");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn finish(&self) {
        self.release.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.completed.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "controlled read did not complete"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
impl SmsPort for ControlledReadPort {
    fn list_controlled(
        &self,
        _: &TargetContext,
        _: Option<SmsStorageId>,
        control: dji4g_domain::SmsReadControl,
    ) -> PortFuture<'_, Result<SmsReadResult, PortError>> {
        Box::pin(async move {
            if self.hold_cleanup {
                control.mark_cleanup_pending();
            }
            *self.control.lock().unwrap() = Some(control.clone());
            while !self.release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            if self.restoration_unknown {
                control.set_restoration(dji4g_domain::SmsStorageRestoration::Unknown);
            }
            control.set_phase(dji4g_domain::SmsReadPhase::Complete);
            self.completed.store(true, Ordering::SeqCst);
            Ok(SmsReadResult {
                listing: SmsListing {
                    messages: vec![late_message()],
                    capacity: Some((1, 50)),
                },
                report: dji4g_domain::SmsReadReport {
                    storage: Some(SmsStorageId("SM".into())),
                    raw_records: 1,
                    decoded_records: 1,
                    ..Default::default()
                },
            })
        })
    }
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
        panic!("unchecked delete forbidden")
    }
}
fn finish_read_polling(runner: &mut ControllerRunner) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while runner.sms_pending() {
        assert!(
            Instant::now() < deadline,
            "read remained busy after cleanup"
        );
        runner.poll_sms_requests();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn timely_history_completion_survives_runner_poll_after_deadline() {
    let port = ControlledReadPort::new(false, false);
    let (runner, _) = runner(port.clone(), 1);
    let mut runner = runner.with_stage_timeout(Duration::from_millis(500));
    runner
        .controller_mut()
        .handle_command(UiCommand::SmsRefresh)
        .unwrap();
    runner.poll_sms_requests();
    let control = port.wait_started();
    port.finish();
    assert!(
        !control.is_expired(),
        "fixture must finish before its deadline"
    );
    std::thread::sleep(control.remaining() + Duration::from_millis(20));
    finish_read_polling(&mut runner);
    let snapshot = runner.controller().snapshot();
    assert!(
        snapshot.sms_inbox_failure.is_none(),
        "timely completion was incorrectly reported as timeout"
    );
    assert_eq!(snapshot.sms_messages.len(), 2);
    assert_eq!(snapshot.sms_read_report.unwrap().decoded_records, 1);
    assert!(!snapshot.serial_work_busy);
}

#[test]
fn cancelled_history_keeps_serial_busy_until_real_cleanup_and_discards_result() {
    let port = ControlledReadPort::new(true, false);
    let (mut runner, parts) = runner(port.clone(), 1);
    runner
        .controller_mut()
        .handle_command(UiCommand::SmsRefresh)
        .unwrap();
    runner.poll_sms_requests();
    let control = port.wait_started();
    runner
        .controller_mut()
        .handle_command(UiCommand::SmsCancelRead)
        .unwrap();
    runner.poll_sms_requests();
    assert!(control.is_cancelled());
    port.finish();
    runner.poll_sms_requests();
    assert!(runner.controller().snapshot().serial_work_busy);
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::SmsDelete { fragments: parts })
            .is_err()
    );
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::SmsReadStorage {
                storage: SmsStorageId("ME".into())
            })
            .is_err()
    );
    control.mark_cleanup_complete();
    finish_read_polling(&mut runner);
    let snapshot = runner.controller().snapshot();
    assert_eq!(
        snapshot.sms_messages.len(),
        1,
        "cancelled result must not be ingested"
    );
    assert!(snapshot.sms_read_report.is_none());
    assert_eq!(
        snapshot.sms_inbox_failure.unwrap().code.stable.as_str(),
        "sms:read_cancelled"
    );
    assert!(!snapshot.serial_work_busy);
}

fn observed_sim(fingerprint: u8) -> BackendEvent {
    use dji4g_domain::{
        AttachState, CellularSnapshot, FeatureStatus, RegistrationState, SimIdentity, SimState,
    };
    BackendEvent::AtFinished {
        cycle: RefreshCycleId(u64::from(fingerprint)),
        epoch: DeviceEpoch(1),
        result: CheckResult::Passed {
            observed_at: SystemTime::UNIX_EPOCH,
            value: AtObservation {
                availability: AtControlAvailability::Available,
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
                    numbers: None,
                    sim_identity: Some(SimIdentity {
                        iccid_masked: "test".into(),
                        fingerprint: [fingerprint; 8],
                    }),
                    temperature_celsius: None,
                    temperature_status: FeatureStatus::NotProbed,
                }),
            },
        },
    }
}

#[test]
fn changed_device_or_sim_discards_old_history_and_report_after_cleanup() {
    for swap_sim in [false, true] {
        let port = ControlledReadPort::new(true, false);
        let (mut runner, _) = runner(port.clone(), 1);
        runner.controller_mut().apply_backend_event(observed_sim(1));
        let previous_sim_epoch = runner.controller().snapshot().sim_epoch;
        runner
            .controller_mut()
            .handle_command(UiCommand::SmsRefresh)
            .unwrap();
        runner.poll_sms_requests();
        let control = port.wait_started();
        if swap_sim {
            runner.controller_mut().apply_backend_event(observed_sim(2));
            assert!(runner.controller().snapshot().sim_epoch > previous_sim_epoch);
        } else {
            runner
                .controller_mut()
                .apply_backend_event(BackendEvent::EpochInvalidated {
                    next_epoch: DeviceEpoch(2),
                    reason: EpochInvalidationReason::PhysicalRemoval,
                });
        }
        runner.poll_sms_requests();
        port.finish();
        runner.poll_sms_requests();
        assert!(control.is_cancelled());
        assert!(runner.controller().snapshot().serial_work_busy);
        control.mark_cleanup_complete();
        finish_read_polling(&mut runner);
        let snapshot = runner.controller().snapshot();
        assert!(snapshot.sms_messages.is_empty());
        assert!(snapshot.sms_read_report.is_none());
        assert_eq!(
            snapshot.sms_inbox_failure.unwrap().code.stable.as_str(),
            "sms:context_changed"
        );
        assert!(!snapshot.serial_work_busy);
    }
}

#[test]
fn uncertain_storage_restoration_discards_rows_and_reports_failure_after_cleanup() {
    let port = ControlledReadPort::new(true, true);
    let (mut runner, _) = runner(port.clone(), 1);
    runner
        .controller_mut()
        .handle_command(UiCommand::SmsReadStorage {
            storage: SmsStorageId("ME".into()),
        })
        .unwrap();
    runner.poll_sms_requests();
    let control = port.wait_started();
    port.finish();
    runner.poll_sms_requests();
    assert!(runner.controller().snapshot().serial_work_busy);
    control.mark_cleanup_complete();
    finish_read_polling(&mut runner);
    let snapshot = runner.controller().snapshot();
    assert_eq!(snapshot.sms_messages.len(), 1);
    assert!(snapshot.sms_read_report.is_none());
    assert_eq!(
        snapshot.sms_inbox_failure.unwrap().code.stable.as_str(),
        "sms:storage_restore_unknown"
    );
    assert!(!snapshot.serial_work_busy);
}

#[test]
fn genuinely_late_completion_is_discarded_even_when_first_polled_after_result() {
    let port = ControlledReadPort::new(false, false);
    let (runner, _) = runner(port.clone(), 1);
    let mut runner = runner.with_stage_timeout(Duration::from_millis(100));
    runner
        .controller_mut()
        .handle_command(UiCommand::SmsRefresh)
        .unwrap();
    runner.poll_sms_requests();
    let control = port.wait_started();
    std::thread::sleep(control.remaining() + Duration::from_millis(20));
    port.finish();
    finish_read_polling(&mut runner);
    let snapshot = runner.controller().snapshot();
    assert_eq!(snapshot.sms_messages.len(), 1);
    assert!(snapshot.sms_read_report.is_none());
    assert_eq!(
        snapshot.sms_inbox_failure.unwrap().code.stable.as_str(),
        "sms:timeout"
    );
}

#[test]
fn timely_receipt_waiting_for_cleanup_keeps_its_original_completion_time() {
    let port = ControlledReadPort::new(true, false);
    let (runner, _) = runner(port.clone(), 1);
    let mut runner = runner.with_stage_timeout(Duration::from_millis(300));
    runner
        .controller_mut()
        .handle_command(UiCommand::SmsRefresh)
        .unwrap();
    runner.poll_sms_requests();
    let control = port.wait_started();
    port.finish();
    while !control.is_expired() {
        runner.poll_sms_requests();
        std::thread::sleep(Duration::from_millis(1));
    }
    runner.poll_sms_requests();
    assert!(runner.controller().snapshot().serial_work_busy);
    assert!(
        !control.is_cancelled(),
        "cleanup waiting must not cancel a timely completed read"
    );
    control.mark_cleanup_complete();
    finish_read_polling(&mut runner);
    let snapshot = runner.controller().snapshot();
    assert_eq!(snapshot.sms_messages.len(), 2);
    assert!(snapshot.sms_inbox_failure.is_none());
}
