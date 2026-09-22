//! SMS store deduplication, observed timeline derivation, adapter metrics flow, and the new SMS
//! UI commands (research document §5.4 timeline, §6.2 dedup, §7.1 adapter metrics).

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use dji4g_application::{
    AdapterBinding, AdapterMetrics, AdapterObservationDto, AdapterPort, AdapterStateDto,
    AtObservation, AtPort, BackendEvent, CheckResult, Clock, CommandReceipt, Controller,
    ControllerRunner, DeviceEpoch, EpochInvalidationReason, FakeActionExecutor, FakeClock,
    InventoryObservation, InventoryPort, MAX_STORED, MonitorPorts, NetworkProbePort, PortError,
    PortFuture, ProbeObservationDto, ProbeStageDto, RATE_TICK_INTERVAL, ReducerState,
    RefreshCycleId, SmsListing, SmsPort, SmsRequest, SmsSendReceipt, SmsSendResult, SmsStore,
    TargetContext, UiCommand, mask_recipient, reduce_state,
};
use dji4g_domain::{
    AtControlAvailability, AttachState, CellularSnapshot, DJI_GEN1, DevicePresence, FeatureStatus,
    ProtocolCoverage, RegistrationState, ServingCell, SimIdentity, SimState, SmsDirection,
    SmsEncoding, SmsMessage, SmsStatus, SmsStorageId, StableDeviceIdentity, TIMELINE_CAPACITY,
    TimelineEventKind,
};

const NOW: SystemTime = SystemTime::UNIX_EPOCH;

fn storage() -> SmsStorageId {
    SmsStorageId("SM".to_owned())
}

fn message(index: u32, body: &str) -> SmsMessage {
    SmsMessage::new(
        index,
        storage(),
        1,
        0,
        "+8613800138000",
        body,
        SmsEncoding::Gsm7,
        SmsStatus::Received,
    )
}

fn fingerprint(value: u8) -> [u8; 8] {
    [value; 8]
}

fn sim_identity(value: u8) -> SimIdentity {
    SimIdentity {
        iccid_masked: "8986****0123".to_owned(),
        fingerprint: fingerprint(value),
    }
}

fn serving_cell(cell_id: u32) -> ServingCell {
    ServingCell {
        state: Some("CONNECT".to_owned()),
        duplex: None,
        rat: Some("LTE".to_owned()),
        mcc: Some("460".to_owned()),
        mnc: Some("00".to_owned()),
        cell_id: Some(cell_id),
        pci: Some(1),
        earfcn: Some(100),
        band: Some(3),
        ul_mhz: None,
        dl_mhz: None,
        tac: None,
        rsrp_dbm: None,
        rsrq_db: None,
        rssi_dbm: None,
        sinr_raw: None,
        srxlev_raw: None,
    }
}

fn cellular(
    registration: RegistrationState,
    cell_id: Option<u32>,
    rsrp_dbm: i16,
    sim_identity: Option<SimIdentity>,
) -> CellularSnapshot {
    CellularSnapshot {
        sim: SimState::Ready,
        registration,
        attached: AttachState::Attached,
        carrier: Some("test".to_owned()),
        radio_access_technology: Some("LTE".to_owned()),
        signal_rssi_dbm: Some(rsrp_dbm),
        apn: None,
        pdp_address: None,
        pdp_state: None,
        firmware: Some("EC200A".to_owned()),
        serving_cell: cell_id.map(serving_cell),
        sim_identity,
        numbers: None,
        temperature_celsius: None,
        temperature_status: FeatureStatus::NotProbed,
    }
}

fn at_finished(cycle: u64, epoch: DeviceEpoch, cellular: Option<CellularSnapshot>) -> BackendEvent {
    BackendEvent::AtFinished {
        cycle: RefreshCycleId(cycle),
        epoch,
        result: CheckResult::Passed {
            value: AtObservation {
                availability: AtControlAvailability::Available,
                cellular,
            },
            observed_at: NOW,
        },
    }
}

fn identity() -> StableDeviceIdentity {
    StableDeviceIdentity {
        container_id: "{container}".to_owned(),
        device_instance_id: "USB\\VID_2CA3&PID_4006\\INSTANCE".to_owned(),
        vid: 0x2CA3,
        pid: 0x4006,
    }
}

fn inventory_finished(cycle: u64, epoch: DeviceEpoch, presence: DevicePresence) -> BackendEvent {
    let supported = matches!(&presence, DevicePresence::Supported(_));
    BackendEvent::InventoryFinished {
        cycle: RefreshCycleId(cycle),
        epoch,
        result: CheckResult::Passed {
            value: InventoryObservation {
                epoch,
                presence,
                identity: supported.then(identity),
                problem_code: None,
                at_port: Some("COM9".to_owned()),
                adapter_id: Some("{adapter}".to_owned()),
            },
            observed_at: NOW,
        },
    }
}

fn adapter_finished(cycle: u64, epoch: DeviceEpoch, state: AdapterStateDto) -> BackendEvent {
    BackendEvent::AdapterFinished {
        cycle: RefreshCycleId(cycle),
        epoch,
        result: CheckResult::Passed {
            value: AdapterObservationDto {
                epoch,
                binding: AdapterBinding {
                    target: identity(),
                    adapter_id: "{adapter}".to_owned(),
                },
                state,
                addresses: vec!["192.168.225.30".to_owned()],
                gateways: vec!["192.168.225.1".to_owned()],
                dns_servers: vec!["192.168.225.1".to_owned()],
                ipv4: true,
                ipv6: false,
                rx_bytes: Some(1_000),
                tx_bytes: Some(2_000),
            },
            observed_at: NOW,
        },
    }
}

fn probe_finished(cycle: u64, epoch: DeviceEpoch, dns: ProbeStageDto) -> BackendEvent {
    BackendEvent::ProbeFinished {
        cycle: RefreshCycleId(cycle),
        epoch,
        result: CheckResult::Passed {
            value: ProbeObservationDto {
                epoch,
                adapter_id: "{adapter}".to_owned(),
                gateway: ProbeStageDto::Passed,
                public: ProbeStageDto::Passed,
                dns,
                protocol_coverage: Some(ProtocolCoverage::AllRequiredFamilies),
                system_route: None,
            },
            observed_at: NOW,
        },
    }
}

/// Feed a device into the reducer and return the state anchored at `epoch`.
fn with_device(mut state: ReducerState, epoch: DeviceEpoch) -> ReducerState {
    state = reduce_state(
        &state,
        inventory_finished(1, epoch, DevicePresence::Supported(DJI_GEN1)),
        NOW,
    );
    state
}

// --- SmsStore ---------------------------------------------------------------------------------

#[test]
fn ingest_stores_new_messages_once_per_content_digest() {
    let mut store = SmsStore::new();
    assert!(store.ingest(message(1, "你好")));
    assert!(!store.ingest(message(1, "你好")));
    assert_eq!(store.messages().len(), 1);
}

#[test]
fn a_duplicate_never_regresses_a_stored_read_state() {
    let mut store = SmsStore::new();
    let mut first = message(1, "验证码 123456");
    first.read = Some(true);
    assert!(store.ingest(first));

    let duplicate = message(1, "验证码 123456");
    assert!(!store.ingest(duplicate));
    assert_eq!(store.messages()[0].read, Some(true));
}

#[test]
fn mark_read_requires_the_exact_storage_and_index() {
    let mut store = SmsStore::new();
    store.ingest(message(1, "a"));
    store.ingest(message(2, "b"));
    assert!(store.mark_read(2, &storage()));
    assert!(!store.mark_read(2, &storage()), "already read is a no-op");
    assert_eq!(store.messages()[1].read, Some(true));
    assert_eq!(store.messages()[0].read, None);
    assert!(!store.mark_read(9, &storage()));
    assert!(!store.mark_read(1, &SmsStorageId("ME".to_owned())));
}

#[test]
fn remove_drops_the_message_and_frees_its_digest() {
    let mut store = SmsStore::new();
    store.ingest(message(3, "c"));
    assert!(store.remove(3, &storage()));
    assert!(!store.remove(3, &storage()));
    assert!(store.messages().is_empty());
    assert!(store.ingest(message(3, "c")), "re-ingest after removal");
}

#[test]
fn summary_counts_read_state_capacity_and_incomplete_fragments() {
    let mut store = SmsStore::new();
    store.ingest(message(1, "unread"));
    let mut read = message(2, "read");
    read.read = Some(true);
    store.ingest(read);
    let mut explicitly_unread = message(3, "explicitly unread");
    explicitly_unread.read = Some(false);
    store.ingest(explicitly_unread);
    let mut incomplete = message(4, "long");
    incomplete.status = SmsStatus::Incomplete;
    store.ingest(incomplete);

    let summary = store.summary(FeatureStatus::Supported, Some((4, 30)));
    assert_eq!(summary.message_count, 4);
    assert_eq!(summary.unread_count, 3);
    assert_eq!(summary.capacity, Some((4, 30)));
    assert!(summary.has_incomplete);
    assert_eq!(summary.status, FeatureStatus::Supported);
}

#[test]
fn clear_drops_every_message_and_digest() {
    let mut store = SmsStore::new();
    store.ingest(message(1, "x"));
    store.clear();
    assert!(store.messages().is_empty());
    assert_eq!(
        store.summary(FeatureStatus::NotProbed, None).message_count,
        0
    );
    assert!(store.ingest(message(1, "x")));
}

#[test]
fn the_store_is_capped_and_evicts_the_oldest_first() {
    let mut store = SmsStore::new();
    for index in 0..(MAX_STORED as u32 + 5) {
        assert!(store.ingest(message(index, &format!("body {index}"))));
    }
    assert_eq!(store.messages().len(), MAX_STORED);
    assert_eq!(store.messages()[0].index, 5);
    assert_eq!(
        store.messages().last().map(|entry| entry.index),
        Some(MAX_STORED as u32 + 4)
    );
}

// --- Timeline derivation ----------------------------------------------------------------------

#[test]
fn timeline_records_a_registration_change_with_closed_chinese_wording() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        at_finished(
            1,
            epoch,
            Some(cellular(RegistrationState::RegisteredHome, None, -70, None)),
        ),
        NOW,
    );
    assert!(state.snapshot().timeline.is_empty());

    state = reduce_state(
        &state,
        at_finished(
            2,
            epoch,
            Some(cellular(RegistrationState::Searching, None, -70, None)),
        ),
        NOW,
    );
    let timeline = state.snapshot().timeline;
    assert_eq!(timeline.events().len(), 1);
    let event = &timeline.events()[0];
    assert_eq!(event.kind, TimelineEventKind::RegistrationChanged);
    assert_eq!(event.detail, "注册状态：已注册到本地网络 → 正在搜索");

    state = reduce_state(
        &state,
        at_finished(
            3,
            epoch,
            Some(cellular(RegistrationState::Searching, None, -60, None)),
        ),
        NOW,
    );
    assert_eq!(state.snapshot().timeline.events().len(), 1);
}

#[test]
fn timeline_ignores_signal_only_variation_and_records_cell_identity_changes() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        at_finished(
            1,
            epoch,
            Some(cellular(
                RegistrationState::RegisteredHome,
                Some(100),
                -70,
                None,
            )),
        ),
        NOW,
    );
    state = reduce_state(
        &state,
        at_finished(
            2,
            epoch,
            Some(cellular(
                RegistrationState::RegisteredHome,
                Some(100),
                -90,
                None,
            )),
        ),
        NOW,
    );
    assert!(state.snapshot().timeline.is_empty());

    state = reduce_state(
        &state,
        at_finished(
            3,
            epoch,
            Some(cellular(
                RegistrationState::RegisteredHome,
                Some(200),
                -90,
                None,
            )),
        ),
        NOW,
    );
    let timeline = state.snapshot().timeline;
    assert_eq!(timeline.events().len(), 1);
    assert_eq!(timeline.events()[0].kind, TimelineEventKind::CellChanged);
    assert_eq!(timeline.events()[0].detail, "服务小区已变化");
}

#[test]
fn timeline_records_a_sim_change_through_the_existing_epoch_logic() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        at_finished(
            1,
            epoch,
            Some(cellular(
                RegistrationState::RegisteredHome,
                None,
                -70,
                Some(sim_identity(1)),
            )),
        ),
        NOW,
    );
    assert!(state.snapshot().timeline.is_empty());

    state = reduce_state(
        &state,
        at_finished(
            2,
            epoch,
            Some(cellular(
                RegistrationState::RegisteredHome,
                None,
                -70,
                Some(sim_identity(2)),
            )),
        ),
        NOW,
    );
    let timeline = state.snapshot().timeline;
    assert_eq!(timeline.events().len(), 1);
    assert_eq!(timeline.events()[0].kind, TimelineEventKind::SimChanged);
    assert_eq!(timeline.events()[0].detail, "SIM 已更换");
}

#[test]
fn timeline_records_device_removal_and_reappearance() {
    let epoch = DeviceEpoch(1);
    let mut state = with_device(ReducerState::new(NOW), epoch);
    assert!(
        state.snapshot().timeline.is_empty(),
        "first contact is not a timeline event"
    );

    state = reduce_state(
        &state,
        BackendEvent::EpochInvalidated {
            next_epoch: DeviceEpoch(2),
            reason: EpochInvalidationReason::PhysicalRemoval,
        },
        NOW,
    );
    let timeline = state.snapshot().timeline;
    assert_eq!(timeline.events().len(), 1);
    assert_eq!(timeline.events()[0].kind, TimelineEventKind::DeviceRemoved);
    assert_eq!(timeline.events()[0].detail, "设备已断开");

    state = with_device(state, DeviceEpoch(2));
    let timeline = state.snapshot().timeline;
    assert_eq!(timeline.events().len(), 2);
    assert_eq!(timeline.events()[1].kind, TimelineEventKind::DeviceArrived);
    assert_eq!(timeline.events()[1].detail, "设备已重新枚举");
}

#[test]
fn timeline_records_adapter_link_and_dns_changes() {
    let epoch = DeviceEpoch(1);
    let mut state = with_device(ReducerState::new(NOW), epoch);
    state = reduce_state(
        &state,
        adapter_finished(2, epoch, AdapterStateDto::UsableAddressAndRoute),
        NOW,
    );
    state = reduce_state(&state, probe_finished(3, epoch, ProbeStageDto::Passed), NOW);
    assert!(
        state.snapshot().timeline.is_empty(),
        "first observations do not claim a change"
    );

    state = reduce_state(
        &state,
        adapter_finished(4, epoch, AdapterStateDto::NoUsableAddressOrRoute),
        NOW,
    );
    state = reduce_state(
        &state,
        probe_finished(
            5,
            epoch,
            ProbeStageDto::Failed {
                code: dji4g_application::PortError::new(
                    dji4g_domain::ErrorCode::ProbeFailed,
                    "test",
                )
                .code,
            },
        ),
        NOW,
    );

    let timeline = state.snapshot().timeline;
    assert_eq!(timeline.events().len(), 2);
    assert_eq!(
        timeline.events()[0].kind,
        TimelineEventKind::AdapterLinkChanged
    );
    assert_eq!(timeline.events()[0].detail, "网卡链路状态变化");
    assert_eq!(timeline.events()[1].kind, TimelineEventKind::DnsChanged);
    assert_eq!(timeline.events()[1].detail, "DNS 探测：通过 → 失败");
}

#[test]
fn timeline_stays_bounded_across_many_observed_changes() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    for cycle in 1..=(TIMELINE_CAPACITY as u64 + 20) {
        let registration = if cycle % 2 == 0 {
            RegistrationState::Searching
        } else {
            RegistrationState::RegisteredHome
        };
        state = reduce_state(
            &state,
            at_finished(cycle, epoch, Some(cellular(registration, None, -70, None))),
            NOW,
        );
    }
    let timeline = state.snapshot().timeline;
    assert!(!timeline.is_empty());
    assert!(timeline.events().len() <= TIMELINE_CAPACITY);
}

// --- Adapter metrics --------------------------------------------------------------------------

fn metrics() -> AdapterMetrics {
    AdapterMetrics {
        rx_bytes: 1,
        tx_bytes: 2,
        in_errors: 3,
        out_errors: 4,
        in_discards: 5,
        out_discards: 6,
        link_rx_bits_per_second: 1_000_000_000,
        link_tx_bits_per_second: 1_000_000_000,
    }
}

#[test]
fn adapter_metrics_sampled_moves_the_snapshot_and_a_none_sample_clears_it() {
    let epoch = DeviceEpoch(1);
    let mut state = with_device(ReducerState::new(NOW), epoch);
    state = reduce_state(
        &state,
        adapter_finished(2, epoch, AdapterStateDto::UsableAddressAndRoute),
        NOW,
    );
    assert_eq!(state.snapshot().adapter_metrics, None);

    state = reduce_state(
        &state,
        BackendEvent::AdapterMetricsSampled {
            adapter_id: "{adapter}".to_owned(),
            epoch,
            metrics: Some(metrics()),
            sampled_at: NOW,
        },
        NOW,
    );
    assert_eq!(state.snapshot().adapter_metrics, Some(metrics()));

    state = reduce_state(
        &state,
        BackendEvent::AdapterMetricsSampled {
            adapter_id: "other-adapter".to_owned(),
            epoch,
            metrics: None,
            sampled_at: NOW,
        },
        NOW,
    );
    assert_eq!(
        state.snapshot().adapter_metrics,
        Some(metrics()),
        "a mismatched sample cannot clear the bound adapter's metrics"
    );

    state = reduce_state(
        &state,
        BackendEvent::AdapterMetricsSampled {
            adapter_id: "{adapter}".to_owned(),
            epoch,
            metrics: None,
            sampled_at: NOW,
        },
        NOW,
    );
    assert_eq!(
        state.snapshot().adapter_metrics,
        None,
        "None means unavailable this sample"
    );

    state = reduce_state(
        &state,
        BackendEvent::AdapterMetricsSampled {
            adapter_id: "{adapter}".to_owned(),
            epoch,
            metrics: Some(metrics()),
            sampled_at: NOW,
        },
        NOW,
    );
    state = reduce_state(
        &state,
        BackendEvent::EpochInvalidated {
            next_epoch: DeviceEpoch(2),
            reason: EpochInvalidationReason::PhysicalRemoval,
        },
        NOW,
    );
    assert_eq!(state.snapshot().adapter_metrics, None);
}

// --- SMS UI commands --------------------------------------------------------------------------

#[test]
fn sms_commands_queue_module_requests_and_delete_waits_for_confirmation() {
    let sms = Arc::new(FakeSmsPort::with_replies([
        SmsReply::Messages(vec![message(3, "hi")]),
        SmsReply::Message(message(3, "hi")),
        SmsReply::Delete,
    ]));
    let (_, mut runner) = sms_runner(sms.clone(), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    for command in [UiCommand::SmsRefresh, UiCommand::SmsRead { index: 3 }] {
        runner.handle().try_send(command).unwrap();
        assert!(runner.poll_commands());
        assert!(runner.poll_sms_requests());
    }
    let fragments = runner.controller().snapshot().sms_messages[0]
        .fragments
        .clone();
    assert_eq!(
        runner
            .controller_mut()
            .handle_command(UiCommand::SmsDelete {
                fragments: fragments.clone()
            }),
        Ok(CommandReceipt::Accepted)
    );
    assert_eq!(runner.controller().snapshot().sms_inbox.message_count, 1);
    assert!(!runner.controller().snapshot().sms_delete.unwrap().finished);
    finish_delete(&mut runner);
    assert_eq!(runner.controller().snapshot().sms_inbox.message_count, 0);
    assert_eq!(
        sms.calls(),
        vec!["query", "enable", "list", "read", "delete_checked"]
    );
    assert_eq!(*sms.deleted_keys.lock().unwrap(), fragments);
    assert_eq!(sms.unchecked_delete_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn mark_sms_read_and_confirm_sms_delete_apply_only_after_the_module_acts() {
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Delete]));
    let (_, mut runner) = sms_runner(sms.clone(), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    let mut stored = message(4, "验证码 000000");
    stored.read = Some(false);
    assert!(runner.controller_mut().ingest_sms(stored.clone()));
    runner
        .controller_mut()
        .record_sms_probe(FeatureStatus::Supported, Some((2, 30)));
    assert_eq!(runner.controller().snapshot().sms_inbox.unread_count, 1);
    assert!(runner.controller_mut().mark_sms_read(&stored));
    assert_eq!(runner.controller().snapshot().sms_inbox.unread_count, 0);
    assert!(!runner.controller_mut().mark_sms_read(&stored));
    let fragments = runner.controller().snapshot().sms_messages[0]
        .fragments
        .clone();
    let mut wrong = fragments.clone();
    wrong[0].index = 9;
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::SmsDelete { fragments: wrong })
            .is_err()
    );
    assert_eq!(sms.delete_calls.load(Ordering::SeqCst), 0);
    assert_eq!(runner.controller().snapshot().sms_inbox.message_count, 1);
    runner
        .controller_mut()
        .handle_command(UiCommand::SmsDelete { fragments })
        .unwrap();
    assert_eq!(runner.controller().snapshot().sms_inbox.message_count, 1);
    finish_delete(&mut runner);
    assert_eq!(runner.controller().snapshot().sms_inbox.message_count, 0);
    assert_eq!(sms.unchecked_delete_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_device_epoch_change_clears_the_sms_store() {
    let mut controller = Controller::for_test(NOW);
    controller.ingest_sms(message(1, "x"));
    controller.apply_backend_event(BackendEvent::EpochInvalidated {
        next_epoch: DeviceEpoch(2),
        reason: EpochInvalidationReason::PhysicalRemoval,
    });
    assert_eq!(controller.snapshot().sms_inbox.message_count, 0);
}

#[test]
fn a_sim_change_clears_the_sms_store() {
    let mut controller = Controller::for_test(NOW);
    let epoch = DeviceEpoch(1);
    controller.apply_backend_event(at_finished(
        1,
        epoch,
        Some(cellular(
            RegistrationState::RegisteredHome,
            None,
            -70,
            Some(sim_identity(1)),
        )),
    ));
    controller.ingest_sms(message(1, "x"));
    assert_eq!(controller.snapshot().sms_inbox.message_count, 1);

    controller.apply_backend_event(at_finished(
        2,
        epoch,
        Some(cellular(
            RegistrationState::RegisteredHome,
            None,
            -70,
            Some(sim_identity(2)),
        )),
    ));
    assert_eq!(controller.snapshot().sms_inbox.message_count, 0);
}

#[test]
fn sms_port_is_object_safe_at_the_application_boundary() {
    struct FakeSms;

    impl SmsPort for FakeSms {
        fn query_pdu_mode(
            &self,
            _target: &TargetContext,
        ) -> PortFuture<'_, Result<Option<bool>, PortError>> {
            Box::pin(async { Ok(Some(true)) })
        }

        fn enable_pdu_mode(
            &self,
            _target: &TargetContext,
        ) -> PortFuture<'_, Result<(), PortError>> {
            Box::pin(async { Ok(()) })
        }

        fn list(&self, _target: &TargetContext) -> PortFuture<'_, Result<SmsListing, PortError>> {
            Box::pin(async { Ok(SmsListing::default()) })
        }

        fn read(
            &self,
            _target: &TargetContext,
            _index: u32,
        ) -> PortFuture<'_, Result<SmsMessage, PortError>> {
            Box::pin(async {
                Err(PortError::new(
                    dji4g_domain::ErrorCode::Unsupported,
                    "test:no_message",
                ))
            })
        }

        fn delete(
            &self,
            _target: &TargetContext,
            _index: u32,
        ) -> PortFuture<'_, Result<(), PortError>> {
            Box::pin(async { Ok(()) })
        }
    }

    let port: Arc<dyn SmsPort> = Arc::new(FakeSms);
    assert_eq!(Arc::strong_count(&port), 1);

    // The listing type carries decoded messages plus the CPMS capacity, and an absent capacity is
    // an honest `None` rather than a fabricated pair.
    let listing = SmsListing {
        messages: vec![message(1, "x")],
        capacity: Some((1, 30)),
    };
    assert_eq!(listing.messages.len(), 1);
    assert_eq!(listing.capacity, Some((1, 30)));
    assert_eq!(SmsListing::default().capacity, None);
}

// --- Runner-driven SMS orchestration ----------------------------------------------------------

#[test]
fn repeated_send_is_rejected_while_first_is_queued() {
    let mut controller = Controller::for_test(NOW);
    let send = || UiCommand::SmsSend {
        recipient: "+8613800138000".into(),
        body: "test".into(),
    };
    assert!(controller.handle_command(send()).is_ok());
    assert!(
        controller.handle_command(send()).is_err(),
        "one active send only"
    );
    assert_eq!(controller.take_sms_requests().len(), 1);
}

/// One canned transaction result for [`FakeSmsPort`].
enum SmsReply {
    Messages(Vec<SmsMessage>),
    Message(SmsMessage),
    Delete,
    Sent(SmsSendResult),

    Err(&'static str, dji4g_domain::ErrorCode),
}

/// A fake module-side SMS port that records every transaction it receives. Each `list` / `read` /
/// `delete` / `send` pops the next canned reply; an exhausted queue fails like a transport error
/// so a test can never silently execute more transactions than it staged.
#[derive(Default)]
struct FakeSmsPort {
    replies: Mutex<VecDeque<SmsReply>>,
    pdu_mode: Mutex<Option<bool>>,
    capacity: Mutex<Option<(u32, u32)>>,
    last_send: Mutex<Option<(String, String)>>,
    calls: Mutex<Vec<&'static str>>,
    enable_error: Mutex<Option<PortError>>,
    enable_effect: Mutex<Option<Option<bool>>>,
    query_calls: AtomicUsize,
    enable_calls: AtomicUsize,
    list_calls: AtomicUsize,
    read_calls: AtomicUsize,
    delete_calls: AtomicUsize,
    unchecked_delete_calls: AtomicUsize,
    deleted_keys: Mutex<Vec<dji4g_domain::SmsFragmentKey>>,
    send_calls: AtomicUsize,
    send_delay: Duration,
    send_gate: Option<Arc<AtomicBool>>,
    cancel_seen: AtomicBool,
    cleanup_completed: AtomicBool,
}

impl FakeSmsPort {
    fn with_replies(replies: impl IntoIterator<Item = SmsReply>) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter().collect()),
            ..Self::default()
        }
    }

    fn set_pdu_mode(&self, mode: Option<bool>) {
        *self.pdu_mode.lock().expect("pdu mode lock") = mode;
    }

    fn set_capacity(&self, capacity: Option<(u32, u32)>) {
        *self.capacity.lock().expect("capacity lock") = capacity;
    }

    /// Record one transaction so a test can assert the exact call sequence.
    fn record(&self, call: &'static str) {
        self.calls.lock().expect("calls lock").push(call);
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().expect("calls lock").clone()
    }

    fn next_reply(&self) -> Result<SmsReply, PortError> {
        self.replies
            .lock()
            .expect("replies lock")
            .pop_front()
            .ok_or_else(|| PortError::new(dji4g_domain::ErrorCode::Internal, "test:no_reply"))
    }
}

impl SmsPort for FakeSmsPort {
    fn send_controlled(
        &self,
        target: &TargetContext,
        recipient: &str,
        body: &str,
        control: dji4g_domain::SmsTransactionControl,
    ) -> PortFuture<'_, Result<SmsSendReceipt, PortError>> {
        if self.send_delay.is_zero() && self.send_gate.is_none() {
            return self.send(target, recipient, body);
        }
        self.send_calls.fetch_add(1, Ordering::SeqCst);
        self.record("send");
        let receipt = SmsSendReceipt::new(SmsSendResult::Submitted, recipient);
        Box::pin(async move {
            control.mark_submission_possible();
            control.set_phase(dji4g_domain::SmsSendPhase::WaitingForResult);
            let started = Instant::now();
            while self
                .send_gate
                .as_ref()
                .is_some_and(|gate| !gate.load(Ordering::Acquire))
                || started.elapsed() < self.send_delay
            {
                if control.is_cancelled() {
                    self.cancel_seen.store(true, Ordering::Release);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            self.cleanup_completed.store(true, Ordering::Release);
            if control.is_cancelled() {
                Err(PortError::new(
                    dji4g_domain::ErrorCode::Timeout,
                    "sms:timeout",
                ))
            } else {
                Ok(receipt)
            }
        })
    }
    fn query_pdu_mode(
        &self,
        _target: &TargetContext,
    ) -> PortFuture<'_, Result<Option<bool>, PortError>> {
        self.query_calls.fetch_add(1, Ordering::SeqCst);
        self.record("query");
        let mode = *self.pdu_mode.lock().expect("pdu mode lock");
        Box::pin(async move { Ok(mode) })
    }

    fn enable_pdu_mode(&self, _target: &TargetContext) -> PortFuture<'_, Result<(), PortError>> {
        self.enable_calls.fetch_add(1, Ordering::SeqCst);
        self.record("enable");
        let error = self.enable_error.lock().expect("enable error lock").clone();
        match error {
            Some(error) => Box::pin(async move { Err(error) }),
            None => {
                let effect = self
                    .enable_effect
                    .lock()
                    .expect("enable effect lock")
                    .unwrap_or(Some(true));
                *self.pdu_mode.lock().expect("pdu mode lock") = effect;
                Box::pin(async { Ok(()) })
            }
        }
    }

    fn list(&self, _target: &TargetContext) -> PortFuture<'_, Result<SmsListing, PortError>> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        self.record("list");
        match self.next_reply() {
            Ok(SmsReply::Messages(messages)) => {
                let capacity = *self.capacity.lock().expect("capacity lock");
                Box::pin(async move { Ok(SmsListing { messages, capacity }) })
            }
            Ok(SmsReply::Err(code, category)) => {
                Box::pin(async move { Err(PortError::new(category, code)) })
            }
            Ok(_) => Box::pin(async {
                Err(PortError::new(
                    dji4g_domain::ErrorCode::Internal,
                    "test:wrong_reply",
                ))
            }),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }

    fn read(
        &self,
        _target: &TargetContext,
        _index: u32,
    ) -> PortFuture<'_, Result<SmsMessage, PortError>> {
        self.read_calls.fetch_add(1, Ordering::SeqCst);
        self.record("read");
        match self.next_reply() {
            Ok(SmsReply::Message(message)) => Box::pin(async move { Ok(message) }),
            Ok(SmsReply::Err(code, category)) => {
                Box::pin(async move { Err(PortError::new(category, code)) })
            }
            Ok(_) => Box::pin(async {
                Err(PortError::new(
                    dji4g_domain::ErrorCode::Internal,
                    "test:wrong_reply",
                ))
            }),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }

    fn delete(
        &self,
        _target: &TargetContext,
        _index: u32,
    ) -> PortFuture<'_, Result<(), PortError>> {
        self.unchecked_delete_calls.fetch_add(1, Ordering::SeqCst);
        panic!("unchecked delete must never be dispatched");
    }

    fn delete_checked(
        &self,
        _target: &TargetContext,
        fragment: &dji4g_domain::SmsFragmentKey,
        control: dji4g_domain::SmsDeleteControl,
    ) -> PortFuture<'_, dji4g_domain::SmsDeleteReceipt> {
        self.delete_calls.fetch_add(1, Ordering::SeqCst);
        self.record("delete_checked");
        self.deleted_keys.lock().unwrap().push(fragment.clone());
        let receipt = match self.next_reply() {
            Ok(SmsReply::Delete) => {
                control.mark_delete_attempted();
                dji4g_domain::SmsDeleteReceipt {
                    result: dji4g_domain::SmsDeleteItemResult::Deleted,
                    code: None,
                }
            }
            Ok(SmsReply::Err(code, _)) => dji4g_domain::SmsDeleteReceipt {
                result: dji4g_domain::SmsDeleteItemResult::Failed,
                code: Some(code.into()),
            },
            _ => panic!("wrong reply for checked deletion"),
        };
        Box::pin(async move { receipt })
    }

    fn send(
        &self,
        _target: &TargetContext,
        recipient: &str,
        body: &str,
    ) -> PortFuture<'_, Result<SmsSendReceipt, PortError>> {
        self.send_calls.fetch_add(1, Ordering::SeqCst);
        self.record("send");
        *self.last_send.lock().expect("last send lock") =
            Some((recipient.to_owned(), body.to_owned()));
        match self.next_reply() {
            Ok(SmsReply::Sent(result)) => {
                let receipt = SmsSendReceipt::new(result, recipient);
                Box::pin(async move { Ok(receipt) })
            }

            Ok(SmsReply::Err(code, category)) => {
                Box::pin(async move { Err(PortError::new(category, code)) })
            }
            Ok(_) => Box::pin(async {
                Err(PortError::new(
                    dji4g_domain::ErrorCode::Internal,
                    "test:wrong_reply",
                ))
            }),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
}

struct StaticInventory {
    result: Result<InventoryObservation, PortError>,
}

impl InventoryPort for StaticInventory {
    fn scan(&self) -> PortFuture<'_, Result<InventoryObservation, PortError>> {
        let result = self.result.clone();
        Box::pin(async move { result })
    }
}

struct StaticAt {
    result: Result<AtObservation, PortError>,
}

impl AtPort for StaticAt {
    fn observe(&self, _target: &TargetContext) -> PortFuture<'_, Result<AtObservation, PortError>> {
        let result = self.result.clone();
        Box::pin(async move { result })
    }

    fn invalidate(&self, _epoch: DeviceEpoch) {}
}

/// An adapter port whose binding comes from a full refresh and whose read-only metrics sample is
/// configurable (supported, unsupported, or failing), so the rates-only tick can be driven
/// deterministically.
struct MetricsAdapter {
    metrics: Option<AdapterMetrics>,
}

impl AdapterPort for MetricsAdapter {
    fn resolve(
        &self,
        _target: &TargetContext,
    ) -> PortFuture<'_, Result<AdapterObservationDto, PortError>> {
        let adapter = AdapterObservationDto {
            epoch: DeviceEpoch(1),
            binding: AdapterBinding {
                target: identity(),
                adapter_id: "{adapter}".to_owned(),
            },
            state: AdapterStateDto::UsableAddressAndRoute,
            addresses: vec!["192.168.225.30".to_owned()],
            gateways: vec!["192.168.225.1".to_owned()],
            dns_servers: vec!["192.168.225.1".to_owned()],
            ipv4: true,
            ipv6: false,
            rx_bytes: Some(1_000),
            tx_bytes: Some(500),
        };
        Box::pin(async move { Ok(adapter) })
    }

    fn read_byte_counters(
        &self,
        _adapter_id: &str,
    ) -> PortFuture<'_, Result<(u64, u64), PortError>> {
        Box::pin(async { Ok((1_000, 500)) })
    }

    fn read_metrics(&self, _adapter_id: &str) -> PortFuture<'_, Result<AdapterMetrics, PortError>> {
        match self.metrics {
            Some(metrics) => Box::pin(async move { Ok(metrics) }),
            None => Box::pin(async {
                Err(PortError::new(
                    dji4g_domain::ErrorCode::Unsupported,
                    "test:metrics_unavailable",
                ))
            }),
        }
    }
}

struct QuietProbe;

impl NetworkProbePort for QuietProbe {
    fn observe(
        &self,
        _adapter: &dji4g_application::AdapterContext,
        _active: bool,
    ) -> PortFuture<'_, Result<ProbeObservationDto, PortError>> {
        Box::pin(async {
            Ok(ProbeObservationDto {
                epoch: DeviceEpoch(1),
                adapter_id: "{adapter}".to_owned(),
                gateway: ProbeStageDto::Passed,
                public: ProbeStageDto::Passed,
                dns: ProbeStageDto::Passed,
                protocol_coverage: None,
                system_route: None,
            })
        })
    }
}

fn sms_runner(
    sms: Arc<FakeSmsPort>,
    adapter: Arc<dyn AdapterPort>,
) -> (Arc<FakeClock>, ControllerRunner) {
    let clock = Arc::new(FakeClock::new(NOW));
    let controller = Controller::new(
        ReducerState::new(NOW),
        Arc::new(FakeActionExecutor::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    let (_handle, runner) = ControllerRunner::new(controller);
    let runner = runner.with_ports(MonitorPorts {
        inventory: Arc::new(StaticInventory {
            result: Ok(InventoryObservation {
                epoch: DeviceEpoch(1),
                presence: DevicePresence::Supported(DJI_GEN1),
                identity: Some(identity()),
                problem_code: None,
                at_port: Some("COM9".to_owned()),
                adapter_id: Some("{adapter}".to_owned()),
            }),
        }),
        at: Arc::new(StaticAt {
            result: Ok(AtObservation {
                availability: AtControlAvailability::Available,
                cellular: None,
            }),
        }),
        adapter,
        probe: Arc::new(QuietProbe) as Arc<dyn NetworkProbePort>,
        hotspot: None,
        sms: Some(Arc::clone(&sms) as Arc<dyn SmsPort>),
        device_tools: None,
    });
    (clock, runner)
}

#[test]
fn runner_refresh_lists_messages_and_records_the_supported_probe() {
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Messages(vec![
        message(1, "第一条"),
        message(2, "第二条"),
    ])]));
    sms.set_pdu_mode(Some(true));
    sms.set_capacity(Some((2, 30)));
    let (_clock, mut runner) =
        sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();

    runner
        .handle()
        .try_send(UiCommand::SmsRefresh)
        .expect("queue refresh");
    assert!(runner.poll_commands(), "the queued command must be handled");
    assert!(
        runner.poll_sms_requests(),
        "a queued request must be drained when a port is wired"
    );
    assert!(
        !runner.poll_sms_requests(),
        "the queue is empty after the drain"
    );

    assert_eq!(sms.query_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        sms.enable_calls.load(Ordering::SeqCst),
        0,
        "an already-PDU module must be left untouched"
    );
    assert_eq!(sms.list_calls.load(Ordering::SeqCst), 1);
    let snapshot = runner.controller().snapshot();
    assert_eq!(snapshot.sms_inbox.message_count, 2);
    assert_eq!(snapshot.sms_inbox.capacity, Some((2, 30)));
    assert_eq!(snapshot.sms_inbox.status, FeatureStatus::Supported);
}

#[test]
fn runner_refresh_enables_pdu_mode_once_when_the_module_is_not_confirmed_pdu() {
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Messages(Vec::new())]));
    sms.set_pdu_mode(Some(false));
    let (_clock, mut runner) =
        sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();

    runner
        .handle()
        .try_send(UiCommand::SmsRefresh)
        .expect("queue refresh");
    assert!(runner.poll_commands(), "the queued command must be handled");
    assert!(runner.poll_sms_requests());

    assert_eq!(
        sms.enable_calls.load(Ordering::SeqCst),
        1,
        "text mode must be switched to PDU exactly once"
    );
    assert_eq!(sms.list_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        runner.controller().snapshot().sms_inbox.status,
        FeatureStatus::Supported
    );
}

#[test]
fn runner_read_marks_the_stored_message_read() {
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Message(message(
        7,
        "验证码 123456",
    ))]));
    let (_clock, mut runner) =
        sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    assert!(
        runner
            .controller_mut()
            .ingest_sms(message(7, "验证码 123456"))
    );
    assert_eq!(runner.controller().snapshot().sms_inbox.unread_count, 1);

    runner
        .handle()
        .try_send(UiCommand::SmsRead { index: 7 })
        .expect("queue read");
    assert!(runner.poll_commands(), "the queued command must be handled");
    assert!(runner.poll_sms_requests());

    assert_eq!(sms.read_calls.load(Ordering::SeqCst), 1);
    let snapshot = runner.controller().snapshot();
    assert_eq!(snapshot.sms_inbox.message_count, 1);
    assert_eq!(snapshot.sms_inbox.unread_count, 0);
    assert_eq!(snapshot.sms_inbox.status, FeatureStatus::Supported);
}

#[test]
fn runner_delete_removes_the_message_only_after_the_module_confirms() {
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Delete]));
    let (_clock, mut runner) =
        sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    assert!(runner.controller_mut().ingest_sms(message(5, "待删除")));

    runner
        .handle()
        .try_send(UiCommand::SmsDelete {
            fragments: runner.controller().snapshot().sms_messages[0]
                .fragments
                .clone(),
        })
        .expect("queue delete");
    assert!(runner.poll_commands(), "the queued command must be handled");
    assert_eq!(
        runner.controller().snapshot().sms_inbox.message_count,
        1,
        "the queued delete must not remove the local copy"
    );

    finish_delete(&mut runner);
    assert_eq!(sms.delete_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sms.unchecked_delete_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        runner.controller().snapshot().sms_inbox.message_count,
        0,
        "the confirmed delete removes the local copy"
    );
    assert_eq!(
        runner.controller().snapshot().sms_delete.unwrap().items[0].result,
        dji4g_domain::SmsDeleteItemResult::Deleted
    );
}

#[test]
fn runner_delete_failure_keeps_the_local_copy() {
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Err(
        "sms:timeout",
        dji4g_domain::ErrorCode::Timeout,
    )]));
    let (_clock, mut runner) =
        sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    assert!(runner.controller_mut().ingest_sms(message(5, "仍保留")));

    runner
        .handle()
        .try_send(UiCommand::SmsDelete {
            fragments: runner.controller().snapshot().sms_messages[0]
                .fragments
                .clone(),
        })
        .expect("queue delete");
    assert!(runner.poll_commands(), "the queued command must be handled");
    finish_delete(&mut runner);

    let snapshot = runner.controller().snapshot();
    assert_eq!(snapshot.sms_inbox.message_count, 1);
    let batch = snapshot.sms_delete.unwrap();
    assert_eq!(
        batch.items[0].result,
        dji4g_domain::SmsDeleteItemResult::Failed
    );
    assert_eq!(batch.items[0].code.as_deref(), Some("sms:timeout"));
    assert_eq!(sms.unchecked_delete_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn runner_refresh_transport_failure_maps_to_transport_failure_without_panicking() {
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Err(
        "sms:timeout",
        dji4g_domain::ErrorCode::Timeout,
    )]));
    sms.set_pdu_mode(Some(true));
    let (_clock, mut runner) =
        sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();

    runner
        .handle()
        .try_send(UiCommand::SmsRefresh)
        .expect("queue refresh");
    assert!(runner.poll_commands(), "the queued command must be handled");
    assert!(runner.poll_sms_requests());

    let snapshot = runner.controller().snapshot();
    assert_eq!(snapshot.sms_inbox.status, FeatureStatus::TransportFailure);
    assert_eq!(snapshot.sms_inbox.message_count, 0);
}

#[test]
fn runner_without_a_wired_sms_port_keeps_requests_queued() {
    let sms = Arc::new(FakeSmsPort::default());
    let clock = Arc::new(FakeClock::new(NOW));
    let controller = Controller::new(
        ReducerState::new(NOW),
        Arc::new(FakeActionExecutor::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    let (_handle, runner) = ControllerRunner::new(controller);
    let mut runner = runner.with_ports(MonitorPorts {
        inventory: Arc::new(StaticInventory {
            result: Ok(InventoryObservation {
                epoch: DeviceEpoch(1),
                presence: DevicePresence::Supported(DJI_GEN1),
                identity: Some(identity()),
                problem_code: None,
                at_port: Some("COM9".to_owned()),
                adapter_id: Some("{adapter}".to_owned()),
            }),
        }),
        at: Arc::new(StaticAt {
            result: Ok(AtObservation {
                availability: AtControlAvailability::Available,
                cellular: None,
            }),
        }),
        adapter: Arc::new(MetricsAdapter { metrics: None }),
        probe: Arc::new(QuietProbe) as Arc<dyn NetworkProbePort>,
        hotspot: None,
        sms: None,
        device_tools: None,
    });

    runner
        .handle()
        .try_send(UiCommand::SmsRefresh)
        .expect("queue refresh");
    assert!(runner.poll_commands(), "the queued command must be handled");
    assert!(
        !runner.poll_sms_requests(),
        "no port means nothing may be drained or discarded"
    );
    assert_eq!(sms.list_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        runner.controller_mut().take_sms_requests(),
        vec![SmsRequest::Refresh],
        "the request survives until a port is wired"
    );
}

#[test]
fn rate_tick_records_adapter_metrics_from_the_same_worker() {
    let sampled = metrics();
    let (clock, mut runner) = sms_runner(
        Arc::new(FakeSmsPort::default()),
        Arc::new(MetricsAdapter {
            metrics: Some(sampled),
        }),
    );
    runner.run_one_refresh();
    assert_eq!(runner.controller().snapshot().adapter_metrics, None);

    // A full refresh realigns the rates baseline; the tick is due one period later.
    clock.advance_wall(RATE_TICK_INTERVAL);
    assert!(runner.poll_rate_cadence());
    assert_eq!(
        runner.controller().snapshot().adapter_metrics,
        Some(sampled)
    );
}

#[test]
fn rate_tick_with_unreadable_metrics_reports_none() {
    let (clock, mut runner) = sms_runner(
        Arc::new(FakeSmsPort::default()),
        Arc::new(MetricsAdapter { metrics: None }),
    );
    runner.run_one_refresh();

    clock.advance_wall(RATE_TICK_INTERVAL);
    assert!(runner.poll_rate_cadence());
    assert_eq!(
        runner.controller().snapshot().adapter_metrics,
        None,
        "an unreadable metrics sample is an honest gap, never stale data"
    );
    assert!(
        RATE_TICK_INTERVAL <= Duration::from_secs(2),
        "the tick stays a fast read-only sample"
    );
}

// --- Outgoing sends (research document §6.3) --------------------------------------------------

/// Queue one user-confirmed send and drain it through the runner's serial SMS stage.
fn queue_send(runner: &mut ControllerRunner, recipient: &str, body: &str) {
    runner
        .handle()
        .try_send(UiCommand::SmsSend {
            recipient: recipient.to_owned(),
            body: body.to_owned(),
        })
        .expect("queue send");
    assert!(runner.poll_commands(), "the queued send must be handled");
    assert!(
        runner.poll_sms_requests(),
        "the queued send must be drained when a port is wired"
    );
    finish_send(runner);
}

fn finish_send(runner: &mut ControllerRunner) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while runner.sms_pending() || runner.controller().sms_active() {
        assert!(
            Instant::now() < deadline,
            "SMS worker did not finish within 2 seconds"
        );
        runner.poll_sms_requests();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn outgoing_messages_carry_the_direction_and_local_transaction_id() {
    assert_eq!(message(1, "hi").direction, SmsDirection::Incoming);

    let outgoing = SmsMessage::new_outgoing(
        7,
        1,
        0,
        "+8613800138000",
        "收到",
        SmsEncoding::Gsm7,
        SmsStatus::Submitted,
    );
    assert_eq!(outgoing.direction, SmsDirection::Outgoing);
    assert_eq!(
        outgoing.index, 7,
        "the local transaction id occupies the index"
    );
    assert_eq!(outgoing.sender(), "+8613800138000");
    assert_eq!(outgoing.body(), "收到");
    assert_eq!(outgoing.service_centre_timestamp, None);
    assert_eq!(outgoing.status, SmsStatus::Submitted);
    assert_eq!(
        outgoing.read,
        Some(true),
        "an outgoing record is never unread inbox mail"
    );
    assert_eq!(outgoing.sender_masked(), "****8000");
}

#[test]
fn recipient_masking_keeps_at_most_the_last_four_characters() {
    assert_eq!(mask_recipient(""), "****");
    assert_eq!(mask_recipient("123"), "****");
    assert_eq!(
        mask_recipient("1234"),
        "****",
        "four characters are all masked"
    );
    assert_eq!(mask_recipient("+8613800138000"), "****8000");
    assert_eq!(mask_recipient("验证码1234"), "****1234");

    let receipt = SmsSendReceipt::new(SmsSendResult::Submitted, "+8613800138000");
    assert_eq!(receipt.result, SmsSendResult::Submitted);
    assert_eq!(receipt.recipient_masked, "****8000");
}

#[test]
fn runner_send_submitted_records_an_outgoing_message() {
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Sent(
        SmsSendResult::Submitted,
    )]));
    sms.set_pdu_mode(Some(true));
    let (_clock, mut runner) =
        sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();

    queue_send(&mut runner, "+8613800138000", "你好");

    assert_eq!(sms.send_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        sms.enable_calls.load(Ordering::SeqCst),
        0,
        "an already-PDU module is left untouched by a send"
    );
    assert_eq!(
        sms.last_send
            .lock()
            .expect("last send lock")
            .as_ref()
            .map(|(recipient, body)| (recipient.as_str(), body.as_str())),
        Some(("+8613800138000", "你好"))
    );

    let state = runner.controller().state();
    let stored = state.sms_store().messages();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].direction, SmsDirection::Outgoing);
    assert_eq!(stored[0].status, SmsStatus::Submitted);
    assert_eq!(stored[0].sender(), "+8613800138000");
    assert_eq!(stored[0].body(), "你好");
    assert_eq!(stored[0].service_centre_timestamp, None);

    let snapshot = runner.controller().snapshot();
    assert_eq!(snapshot.sms_inbox.message_count, 1);
    assert_eq!(
        snapshot.sms_inbox.unread_count, 0,
        "submitted does not make inbox mail unread"
    );
    assert_eq!(
        snapshot.sms_send.unwrap().result,
        Some(SmsSendResult::Submitted)
    );
}

#[test]
fn runner_send_failed_and_outcome_unknown_store_their_honest_status() {
    for (result, expected) in [
        (SmsSendResult::Failed, SmsStatus::Failed),
        (SmsSendResult::OutcomeUnknown, SmsStatus::OutcomeUnknown),
    ] {
        let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Sent(result)]));
        sms.set_pdu_mode(Some(true));
        let (_clock, mut runner) =
            sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
        runner.run_one_refresh();

        queue_send(&mut runner, "+8613800138000", "状态");

        assert_eq!(sms.send_calls.load(Ordering::SeqCst), 1);
        let state = runner.controller().state();
        let stored = state.sms_store().messages();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].direction, SmsDirection::Outgoing);
        assert_eq!(stored[0].status, expected);
        assert_eq!(stored[0].body(), "状态");
        assert_eq!(
            runner.controller().snapshot().sms_send.unwrap().result,
            Some(result),
            "send outcome is independent of inbox query status"
        );
    }
}

#[test]
fn runner_send_timeout_before_submission_is_failed_with_original_error() {
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Err(
        "sms:timeout",
        dji4g_domain::ErrorCode::Timeout,
    )]));
    sms.set_pdu_mode(Some(true));
    let (_clock, mut runner) =
        sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();

    queue_send(&mut runner, "+8613800138000", "超时");

    assert_eq!(
        sms.send_calls.load(Ordering::SeqCst),
        1,
        "a timed-out send is never retried automatically"
    );
    let state = runner.controller().state();
    let stored = state.sms_store().messages();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].direction, SmsDirection::Outgoing);
    assert_eq!(stored[0].status, SmsStatus::Failed);
    assert_eq!(stored[0].body(), "超时");
    let send = runner.controller().snapshot().sms_send.unwrap();
    assert_eq!(send.result, Some(SmsSendResult::Failed));
    assert_eq!(send.failure.unwrap().code, "sms:timeout");
}

#[test]
fn runner_send_definite_rejection_is_failed_not_unknown() {
    // A definite rejection (invalid recipient/body, PDU precondition) never leaves the module;
    // recording it as OutcomeUnknown would overstate what is unknown (review finding).
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Err(
        "sms:invalid_message",
        dji4g_domain::ErrorCode::VerificationFailed,
    )]));
    sms.set_pdu_mode(Some(true));
    let (_clock, mut runner) =
        sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();

    queue_send(&mut runner, "+12025550123", "bad\u{1F600}");

    assert_eq!(sms.send_calls.load(Ordering::SeqCst), 1);
    let state = runner.controller().state();
    let stored = state.sms_store().messages();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].status, SmsStatus::Failed);
}

#[test]
fn local_cap_evictions_are_reported_not_silent() {
    let mut store = SmsStore::new();
    for index in 0..(MAX_STORED as u32 + 2) {
        assert!(store.ingest(message(index, "x")));
    }
    let summary = store.summary(FeatureStatus::Supported, None);
    assert_eq!(summary.evicted, 2, "evictions must be counted and surfaced");
    assert_eq!(summary.message_count, MAX_STORED);
}

#[test]
fn runner_delegates_entire_send_preflight_to_the_sms_port() {
    // Unknown and text mode are delegated to the production SMS port for preflight.
    for mode in [Some(false), None] {
        let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Sent(
            SmsSendResult::Submitted,
        )]));
        sms.set_pdu_mode(mode);
        let (_clock, mut runner) =
            sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
        runner.run_one_refresh();

        queue_send(&mut runner, "+8613800138000", "切换后发送");

        assert_eq!(
            sms.calls(),
            vec!["send"],
            "the SMS port owns preflight and the runner dispatches one send"
        );
        assert_eq!(sms.enable_calls.load(Ordering::SeqCst), 0);
        assert_eq!(sms.send_calls.load(Ordering::SeqCst), 1);
        let state = runner.controller().state();
        let stored = state.sms_store().messages();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].direction, SmsDirection::Outgoing);
        assert_eq!(stored[0].status, SmsStatus::Submitted);
        assert_eq!(
            runner.controller().snapshot().sms_send.unwrap().result,
            Some(SmsSendResult::Submitted)
        );
    }
}

#[test]
fn identical_sends_are_each_recorded_as_a_distinct_transaction() {
    let sms = Arc::new(FakeSmsPort::with_replies([
        SmsReply::Sent(SmsSendResult::Submitted),
        SmsReply::Sent(SmsSendResult::Submitted),
    ]));
    sms.set_pdu_mode(Some(true));
    let (_clock, mut runner) =
        sms_runner(Arc::clone(&sms), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();

    queue_send(&mut runner, "+8613800138000", "重复内容");
    queue_send(&mut runner, "+8613800138000", "重复内容");

    assert_eq!(sms.send_calls.load(Ordering::SeqCst), 2);
    let state = runner.controller().state();
    let stored = state.sms_store().messages();
    assert_eq!(
        stored.len(),
        2,
        "each user-confirmed send is its own record, never deduplicated"
    );
    assert_ne!(stored[0].index, stored[1].index);
}

#[test]
fn a_module_delete_never_removes_a_local_outgoing_record() {
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Delete]));
    let (_, mut runner) = sms_runner(sms.clone(), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    assert!(runner.controller_mut().ingest_sms(SmsMessage::new_outgoing(
        3,
        1,
        0,
        "+8613800138000",
        "本地发送",
        SmsEncoding::Gsm7,
        SmsStatus::Submitted
    )));
    assert!(runner.controller_mut().ingest_sms(message(3, "incoming")));
    assert_eq!(runner.controller().snapshot().sms_inbox.message_count, 2);
    let fragments = runner
        .controller()
        .snapshot()
        .sms_messages
        .iter()
        .find(|row| row.direction == SmsDirection::Incoming)
        .unwrap()
        .fragments
        .clone();
    runner
        .controller_mut()
        .handle_command(UiCommand::SmsDelete { fragments })
        .unwrap();
    finish_delete(&mut runner);
    let state = runner.controller().state();
    let stored = state.sms_store().messages();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].direction, SmsDirection::Outgoing);
    assert_eq!(sms.unchecked_delete_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn send_has_queued_snapshot_before_worker_starts() {
    let mut controller = Controller::for_test(NOW);
    controller
        .handle_command(UiCommand::SmsSend {
            recipient: "+8613800138000".into(),
            body: "queued".into(),
        })
        .unwrap();
    let snapshot = controller.snapshot().sms_send.unwrap();
    assert_eq!(snapshot.phase, dji4g_domain::SmsSendPhase::Queued);
    assert_eq!(snapshot.result, None);
    assert!(controller.sms_active());
}

fn start_controlled_send(runner: &mut ControllerRunner) {
    runner
        .handle()
        .try_send(UiCommand::SmsSend {
            recipient: "+8613800138000".into(),
            body: "controlled".into(),
        })
        .unwrap();
    assert!(runner.poll_commands());
    assert!(runner.poll_sms_requests());
}

fn wait_for_send_worker(sms: &FakeSmsPort) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while sms.send_calls.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn send_can_outlast_monitor_stage_timeout_within_its_own_deadline() {
    let sms = Arc::new(FakeSmsPort {
        send_delay: Duration::from_millis(80),
        ..FakeSmsPort::default()
    });
    let (_, runner) = sms_runner(sms.clone(), Arc::new(MetricsAdapter { metrics: None }));
    let mut runner = runner.with_sms_timeout(Duration::from_secs(1));
    runner.run_one_refresh();
    // Establish device readiness before applying the deliberately tiny monitoring budget.
    let mut runner = runner.with_stage_timeout(Duration::from_millis(10));
    queue_send(&mut runner, "+8613800138000", "slow network");
    assert_eq!(
        runner.controller().snapshot().sms_send.unwrap().result,
        Some(SmsSendResult::Submitted)
    );
    assert!(sms.cleanup_completed.load(Ordering::Acquire));
    assert_eq!(sms.send_calls.load(Ordering::Acquire), 1);
}

#[test]
fn active_send_exposes_progress_and_blocks_refresh_until_cleanup() {
    let gate = Arc::new(AtomicBool::new(false));
    let sms = Arc::new(FakeSmsPort {
        send_gate: Some(gate.clone()),
        ..FakeSmsPort::default()
    });
    let (_, mut runner) = sms_runner(sms.clone(), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    start_controlled_send(&mut runner);
    wait_for_send_worker(&sms);
    runner.poll_sms_requests();
    assert!(runner.sms_pending());
    assert!(runner.controller().sms_active());
    assert!(
        runner
            .controller()
            .snapshot()
            .sms_send
            .unwrap()
            .result
            .is_none()
    );
    let generation = runner.controller().snapshot().publication_revision;
    runner.run_one_refresh();
    assert_eq!(
        runner.controller().snapshot().publication_revision,
        generation,
        "refresh must not begin while send owns the port"
    );
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::SmsRefresh)
            .is_err()
    );
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::SmsSend {
                recipient: "+8613800138000".into(),
                body: "duplicate".into()
            })
            .is_err()
    );
    gate.store(true, Ordering::Release);
    finish_send(&mut runner);
    assert_eq!(sms.send_calls.load(Ordering::Acquire), 1);
}

#[test]
fn timeout_does_not_finish_or_allow_reopen_before_worker_cleanup() {
    let gate = Arc::new(AtomicBool::new(false));
    let sms = Arc::new(FakeSmsPort {
        send_gate: Some(gate.clone()),
        ..FakeSmsPort::default()
    });
    let (_, runner) = sms_runner(sms.clone(), Arc::new(MetricsAdapter { metrics: None }));
    let mut runner = runner.with_sms_timeout(Duration::from_millis(20));
    runner.run_one_refresh();
    start_controlled_send(&mut runner);
    wait_for_send_worker(&sms);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !sms.cancel_seen.load(Ordering::Acquire) {
        runner.poll_sms_requests();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(runner.sms_pending());
    assert!(runner.controller().sms_active());
    assert!(!sms.cleanup_completed.load(Ordering::Acquire));
    assert!(
        runner
            .controller()
            .snapshot()
            .sms_send
            .unwrap()
            .result
            .is_none()
    );
    gate.store(true, Ordering::Release);
    finish_send(&mut runner);
    assert!(sms.cleanup_completed.load(Ordering::Acquire));
    assert_eq!(
        runner.controller().snapshot().sms_send.unwrap().result,
        Some(SmsSendResult::OutcomeUnknown)
    );
}

#[test]
fn stale_device_epoch_send_result_never_enters_new_device_store() {
    let gate = Arc::new(AtomicBool::new(false));
    let sms = Arc::new(FakeSmsPort {
        send_gate: Some(gate.clone()),
        ..FakeSmsPort::default()
    });
    let (_, mut runner) = sms_runner(sms.clone(), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    start_controlled_send(&mut runner);
    wait_for_send_worker(&sms);
    runner
        .controller_mut()
        .invalidate_epoch(DeviceEpoch(2), EpochInvalidationReason::PhysicalRemoval);
    runner.poll_sms_requests();
    gate.store(true, Ordering::Release);
    finish_send(&mut runner);
    assert!(
        runner
            .controller()
            .state()
            .sms_store()
            .messages()
            .is_empty()
    );
    assert_eq!(runner.controller().state().epoch(), DeviceEpoch(2));
    assert_ne!(
        runner
            .controller()
            .snapshot()
            .sms_send
            .and_then(|send| send.result),
        Some(SmsSendResult::Submitted)
    );
}

#[test]
fn stale_sim_epoch_send_result_never_enters_new_sim_store() {
    let gate = Arc::new(AtomicBool::new(false));
    let sms = Arc::new(FakeSmsPort {
        send_gate: Some(gate.clone()),
        ..FakeSmsPort::default()
    });
    let (_, mut runner) = sms_runner(sms.clone(), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    runner.controller_mut().apply_backend_event(at_finished(
        2,
        DeviceEpoch(1),
        Some(cellular(
            RegistrationState::RegisteredHome,
            None,
            -70,
            Some(sim_identity(1)),
        )),
    ));
    let sim_epoch = runner.controller().snapshot().sim_epoch;
    start_controlled_send(&mut runner);
    wait_for_send_worker(&sms);
    runner.controller_mut().apply_backend_event(at_finished(
        3,
        DeviceEpoch(1),
        Some(cellular(
            RegistrationState::RegisteredHome,
            None,
            -70,
            Some(sim_identity(2)),
        )),
    ));
    assert!(runner.controller().snapshot().sim_epoch > sim_epoch);
    runner.poll_sms_requests();
    gate.store(true, Ordering::Release);
    finish_send(&mut runner);
    assert!(
        runner
            .controller()
            .state()
            .sms_store()
            .messages()
            .is_empty()
    );
    assert_ne!(
        runner
            .controller()
            .snapshot()
            .sms_send
            .and_then(|send| send.result),
        Some(SmsSendResult::Submitted)
    );
}

#[test]
fn dropping_runner_cancels_pending_send_worker() {
    let gate = Arc::new(AtomicBool::new(false));
    let sms = Arc::new(FakeSmsPort {
        send_gate: Some(gate.clone()),
        ..FakeSmsPort::default()
    });
    let (_, mut runner) = sms_runner(sms.clone(), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    start_controlled_send(&mut runner);
    wait_for_send_worker(&sms);
    drop(runner);
    let deadline = Instant::now() + Duration::from_millis(100);
    while !sms.cancel_seen.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let cancelled = sms.cancel_seen.load(Ordering::Acquire);
    gate.store(true, Ordering::Release);
    assert!(
        cancelled,
        "runner shutdown must cancel its active SMS before abandoning the receiver"
    );
}

#[test]
fn queued_send_cannot_retarget_a_replaced_device_before_worker_starts() {
    let sms = Arc::new(FakeSmsPort::with_replies([SmsReply::Sent(
        SmsSendResult::Submitted,
    )]));
    let (_, mut runner) = sms_runner(sms.clone(), Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    runner
        .handle()
        .try_send(UiCommand::SmsSend {
            recipient: "+8613800138000".into(),
            body: "old device".into(),
        })
        .unwrap();
    assert!(runner.poll_commands());
    runner
        .controller_mut()
        .invalidate_epoch(DeviceEpoch(2), EpochInvalidationReason::PhysicalRemoval);
    runner
        .controller_mut()
        .apply_backend_event(inventory_finished(
            3,
            DeviceEpoch(2),
            DevicePresence::Supported(DJI_GEN1),
        ));
    runner.poll_sms_requests();
    finish_send(&mut runner);
    assert_eq!(
        sms.send_calls.load(Ordering::Acquire),
        0,
        "queued send belongs to the device present when accepted"
    );
    assert!(
        runner
            .controller()
            .state()
            .sms_store()
            .messages()
            .is_empty()
    );
}
#[test]
fn inbox_failure_retains_code_and_success_clears_it() {
    let sms = Arc::new(FakeSmsPort::with_replies([
        SmsReply::Err(
            "sms:port_busy",
            dji4g_domain::ErrorCode::CapabilityUnavailable,
        ),
        SmsReply::Messages(Vec::new()),
    ]));
    let (_, mut runner) = sms_runner(sms, Arc::new(MetricsAdapter { metrics: None }));
    runner.run_one_refresh();
    for failed in [true, false] {
        runner.handle().try_send(UiCommand::SmsRefresh).unwrap();
        runner.poll_commands();
        runner.poll_sms_requests();
        let snapshot = runner.controller().snapshot();
        assert_eq!(
            snapshot
                .sms_inbox_failure
                .as_ref()
                .map(|e| e.code.stable().as_str()),
            failed.then_some("sms:port_busy")
        );
    }
}

fn finish_delete(runner: &mut ControllerRunner) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        runner.poll_sms_requests();
        if runner
            .controller()
            .snapshot()
            .sms_delete
            .as_ref()
            .is_some_and(|batch| batch.finished)
        {
            break;
        }
        assert!(Instant::now() < deadline, "delete worker did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}
