use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
    time::{Duration, SystemTime},
};

use dji4g_application::{
    AdapterContext, AdapterObservationDto, AdapterPort, AdapterStateDto, AtObservation, AtPort,
    BackendEvent, CheckResult, Clock, Controller, ControllerRunner, DefaultRouteDto, DeviceEpoch,
    DevicePresenceDto, DiagnosticCheckId, DiagnosticCheckState, EpochInvalidationReason,
    FailureCode, FakeActionExecutor, FakeClock, InventoryObservation, InventoryPort, LanguageCode,
    MonitorPorts, NetworkProbePort, PortError, PortFuture, ProbeObservationDto, ProbeStageDto,
    RATE_TICK_INTERVAL, REFRESH_INTERVAL, ReducerState, RefreshCycleId, STAGE_TIMEOUT, StableCode,
    SystemRouteDto, TargetContext, periodic_refresh_due, rate_tick_due, reduce_state,
};
use dji4g_domain::{
    AdapterBinding, AtControlAvailability, Availability, BoundDnsStatus, BoundPublicStatus,
    DeviceProfile, ErrorCode, Freshness, LimitedReason, ProtocolCoverage, StableDeviceIdentity,
};

const NOW: SystemTime = SystemTime::UNIX_EPOCH;

fn target() -> StableDeviceIdentity {
    StableDeviceIdentity {
        container_id: "{container}".into(),
        device_instance_id: "USB\\VID_2CA3&PID_4006\\INSTANCE".into(),
        vid: 0x2CA3,
        pid: 0x4006,
    }
}

fn binding() -> AdapterBinding {
    AdapterBinding {
        target: target(),
        adapter_id: "{adapter}".into(),
    }
}

fn passed<T>(value: T) -> CheckResult<T> {
    CheckResult::Passed {
        value,
        observed_at: NOW,
    }
}

fn full_inventory(epoch: DeviceEpoch) -> InventoryObservation {
    InventoryObservation {
        epoch,
        presence: DevicePresenceDto::Supported(DeviceProfile::DJI_GEN1),
        identity: Some(target()),
        problem_code: None,
        at_port: Some("COM9".into()),
        adapter_id: Some("{adapter}".into()),
    }
}

fn full_adapter(epoch: DeviceEpoch) -> AdapterObservationDto {
    counted_adapter(epoch, None, None)
}

fn counted_adapter(epoch: DeviceEpoch, rx: Option<u64>, tx: Option<u64>) -> AdapterObservationDto {
    AdapterObservationDto {
        epoch,
        binding: binding(),
        state: AdapterStateDto::UsableAddressAndRoute,
        addresses: vec!["192.168.225.30".into()],
        gateways: vec!["192.168.225.1".into()],
        dns_servers: vec!["192.168.225.1".into()],
        ipv4: true,
        ipv6: true,
        rx_bytes: rx,
        tx_bytes: tx,
    }
}

fn adapter_finished(
    epoch: DeviceEpoch,
    cycle: RefreshCycleId,
    rx: Option<u64>,
    tx: Option<u64>,
    observed_at: SystemTime,
) -> BackendEvent {
    BackendEvent::AdapterFinished {
        cycle,
        epoch,
        result: CheckResult::Passed {
            value: counted_adapter(epoch, rx, tx),
            observed_at,
        },
    }
}

fn full_probe(epoch: DeviceEpoch) -> ProbeObservationDto {
    ProbeObservationDto {
        epoch,
        adapter_id: "{adapter}".into(),
        gateway: ProbeStageDto::Passed,
        public: ProbeStageDto::Passed,
        dns: ProbeStageDto::Passed,
        protocol_coverage: Some(ProtocolCoverage::AllRequiredFamilies),
        system_route: Some(SystemRouteDto {
            owner: DefaultRouteDto::TargetAdapter,
            explanation_only: true,
        }),
    }
}

fn stable(code: &'static str) -> FailureCode {
    FailureCode::new(
        ErrorCode::ProbeFailed,
        StableCode::try_from_static(code).unwrap(),
    )
}

#[test]
fn startup_keeps_every_diagnostic_explicitly_unexecuted_or_running() {
    let state = ReducerState::new(NOW);
    let snapshot = state.snapshot();

    assert_eq!(
        snapshot.app.availability,
        dji4g_domain::Availability::Detecting
    );
    assert_eq!(snapshot.diagnostics.len(), DiagnosticCheckId::ORDERED.len());
    assert!(snapshot.diagnostics.iter().all(|check| matches!(
        check.state,
        DiagnosticCheckState::Unexecuted { .. } | DiagnosticCheckState::Running { .. }
    )));
    assert_eq!(snapshot.settings.language, LanguageCode::ZhCn);
}

#[test]
fn removal_invalidates_epoch_and_clears_old_positive_evidence_before_scan() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_inventory(epoch)),
        },
        NOW,
    );
    state = reduce_state(
        &state,
        BackendEvent::AdapterFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_adapter(epoch)),
        },
        NOW,
    );
    state = reduce_state(
        &state,
        BackendEvent::ProbeFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_probe(epoch)),
        },
        NOW,
    );
    let before = state.snapshot();
    assert!(matches!(
        before.app.availability,
        dji4g_domain::Availability::Limited(_) | dji4g_domain::Availability::Detecting
    ));

    let removed = reduce_state(
        &state,
        BackendEvent::EpochInvalidated {
            next_epoch: DeviceEpoch(2),
            reason: EpochInvalidationReason::PhysicalRemoval,
        },
        NOW,
    );
    let snapshot = removed.snapshot();
    assert_eq!(removed.epoch(), DeviceEpoch(2));
    assert_eq!(snapshot.app.freshness, dji4g_domain::Freshness::Unknown);
    assert_eq!(
        snapshot.app.availability,
        dji4g_domain::Availability::Detecting
    );
    assert!(snapshot.app.device.is_none());
    assert!(
        snapshot
            .diagnostics
            .iter()
            .all(|check| check.epoch == DeviceEpoch(2))
    );
}

#[test]
fn stale_result_after_removal_is_ignored_without_revision_or_detail_resurrection() {
    let mut state = ReducerState::new(NOW);
    state = reduce_state(
        &state,
        BackendEvent::EpochInvalidated {
            next_epoch: DeviceEpoch(3),
            reason: EpochInvalidationReason::PhysicalRemoval,
        },
        NOW,
    );
    let removed = state.snapshot();
    let evidence_revision = removed.app.revision;
    let publication_revision = removed.publication_revision;
    let after = reduce_state(
        &state,
        BackendEvent::ProbeFinished {
            cycle: RefreshCycleId(1),
            epoch: DeviceEpoch(2),
            result: passed(full_probe(DeviceEpoch(2))),
        },
        NOW,
    )
    .snapshot();

    assert_eq!(after.app.revision, evidence_revision);
    assert_eq!(after.publication_revision, publication_revision);
    assert!(after.app.network.is_none());
    assert_eq!(
        after.app.availability,
        dji4g_domain::Availability::Detecting
    );
}

#[test]
fn dns_failure_retains_bound_public_success_and_maps_to_limited() {
    let mut state = ReducerState::new(ReducerState::default_time());
    let epoch = DeviceEpoch(1);
    for event in [
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_inventory(epoch)),
        },
        BackendEvent::AdapterFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_adapter(epoch)),
        },
        BackendEvent::AtFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: CheckResult::Passed {
                value: AtObservation {
                    availability: AtControlAvailability::Available,
                    cellular: None,
                },
                observed_at: NOW,
            },
        },
        BackendEvent::ProbeFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: CheckResult::Passed {
                value: ProbeObservationDto {
                    dns: ProbeStageDto::Failed {
                        code: stable("probe:dns_timeout"),
                    },
                    ..full_probe(epoch)
                },
                observed_at: NOW,
            },
        },
    ] {
        state = reduce_state(&state, event, NOW);
    }
    let snapshot = state.snapshot();
    assert_eq!(
        snapshot.app.network.as_ref().unwrap().bound_public,
        BoundPublicStatus::Succeeded
    );
    assert_eq!(
        snapshot.app.network.as_ref().unwrap().bound_dns,
        BoundDnsStatus::Failed
    );
    assert_eq!(
        snapshot.app.availability,
        dji4g_domain::Availability::Limited(dji4g_domain::LimitedReason::DnsFailure)
    );
    assert_eq!(
        snapshot
            .diagnostics
            .get(DiagnosticCheckId::BoundPublic)
            .state,
        DiagnosticCheckState::Passed
    );
    assert!(matches!(
        snapshot.diagnostics.get(DiagnosticCheckId::BoundDns).state,
        DiagnosticCheckState::Failed { .. }
    ));
}

#[test]
fn tun_route_is_explanation_only_and_does_not_replace_bound_source() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    for event in [
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_inventory(epoch)),
        },
        BackendEvent::AdapterFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_adapter(epoch)),
        },
        BackendEvent::AtFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: CheckResult::Passed {
                value: AtObservation {
                    availability: AtControlAvailability::Available,
                    cellular: None,
                },
                observed_at: NOW,
            },
        },
        BackendEvent::ProbeFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: CheckResult::Passed {
                value: ProbeObservationDto {
                    system_route: Some(SystemRouteDto {
                        owner: DefaultRouteDto::VpnOrTun,
                        explanation_only: true,
                    }),
                    ..full_probe(epoch)
                },
                observed_at: NOW,
            },
        },
    ] {
        state = reduce_state(&state, event, NOW);
    }
    let snapshot = state.snapshot();
    assert_eq!(
        snapshot.app.network.as_ref().unwrap().bound_public,
        BoundPublicStatus::Succeeded
    );
    assert_eq!(
        snapshot.app.network.as_ref().unwrap().system_default_route,
        dji4g_domain::DefaultRouteOwner::VpnOrTun
    );
    assert_eq!(
        snapshot.app.availability,
        dji4g_domain::Availability::Available
    );
}

#[test]
fn active_probe_disabled_is_unexecuted_and_never_proves_available() {
    let mut state = ReducerState::new(NOW);
    state.set_active_probe(false);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_inventory(epoch)),
        },
        NOW,
    );
    state = reduce_state(
        &state,
        BackendEvent::AdapterFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_adapter(epoch)),
        },
        NOW,
    );
    let snapshot = state.snapshot();
    assert!(matches!(
        snapshot
            .diagnostics
            .get(DiagnosticCheckId::BoundPublic)
            .state,
        DiagnosticCheckState::Unexecuted { .. }
    ));
    assert!(!matches!(
        snapshot.app.availability,
        dji4g_domain::Availability::Available
    ));
}

#[test]
fn old_cycle_result_cannot_overwrite_new_cycle() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(2),
            epoch,
            result: passed(full_inventory(epoch)),
        },
        NOW,
    );
    let before = state.snapshot();
    let after = reduce_state(
        &state,
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: CheckResult::Failed {
                code: stable("pnp:old"),
                observed_at: NOW,
            },
        },
        NOW,
    )
    .snapshot();
    assert_eq!(after.publication_revision, before.publication_revision);
    assert_eq!(after.app.revision, before.app.revision);
    assert_eq!(after.app.device, before.app.device);
}

#[test]
fn fake_clock_separates_wall_time_from_monotonic_scheduler_time() {
    let start = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let clock = FakeClock::new(start);
    clock.advance_mono(Duration::from_secs(5));
    clock.set_wall_backwards(Duration::from_secs(60));
    assert_eq!(clock.monotonic_now().ticks(), 5_000);
    assert_eq!(clock.system_now(), start - Duration::from_secs(60));
}

#[test]
fn first_and_second_bound_public_failures_have_different_availability() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    for event in [
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_inventory(epoch)),
        },
        BackendEvent::AdapterFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_adapter(epoch)),
        },
        BackendEvent::AtFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: CheckResult::Passed {
                value: AtObservation {
                    availability: AtControlAvailability::Available,
                    cellular: None,
                },
                observed_at: NOW,
            },
        },
    ] {
        state = reduce_state(&state, event, NOW);
    }
    let failed_probe = || CheckResult::Passed {
        value: ProbeObservationDto {
            public: ProbeStageDto::Failed {
                code: stable("probe:public_failed"),
            },
            ..full_probe(epoch)
        },
        observed_at: NOW,
    };
    state = reduce_state(
        &state,
        BackendEvent::ProbeFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: failed_probe(),
        },
        NOW,
    );
    assert_eq!(
        state.snapshot().app.availability,
        dji4g_domain::Availability::Limited(dji4g_domain::LimitedReason::IncompleteEvidence)
    );
    assert_eq!(
        state.snapshot().app.network.as_ref().unwrap().bound_public,
        BoundPublicStatus::Failed {
            consecutive_cycles: 1
        }
    );
    state = reduce_state(
        &state,
        BackendEvent::ProbeFinished {
            cycle: RefreshCycleId(2),
            epoch,
            result: failed_probe(),
        },
        NOW,
    );
    assert_eq!(
        state.snapshot().app.availability,
        dji4g_domain::Availability::Unavailable(
            dji4g_domain::UnavailableReason::BoundPublicProbeFailed
        )
    );
    state = reduce_state(
        &state,
        BackendEvent::ProbeFinished {
            cycle: RefreshCycleId(3),
            epoch,
            result: passed(full_probe(epoch)),
        },
        NOW,
    );
    assert_eq!(
        state.snapshot().app.availability,
        dji4g_domain::Availability::Available
    );
}

#[test]
fn future_observation_is_ignored_without_revision_change() {
    let state = ReducerState::test_ready(NOW);
    let before = state.snapshot();
    let after = reduce_state(
        &state,
        BackendEvent::AtFinished {
            cycle: RefreshCycleId(1),
            epoch: DeviceEpoch(1),
            result: CheckResult::Failed {
                code: stable("at:future"),
                observed_at: NOW + Duration::from_secs(1),
            },
        },
        NOW,
    )
    .snapshot();
    assert_eq!(after.publication_revision, before.publication_revision);
    assert_eq!(after.app.revision, before.app.revision);
    assert_eq!(after.app.availability, before.app.availability);
}

#[test]
fn absent_inventory_marks_dependent_checks_unavailable() {
    let state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    let state = reduce_state(
        &state,
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(InventoryObservation {
                epoch,
                presence: DevicePresenceDto::NotDetected,
                identity: None,
                problem_code: None,
                at_port: None,
                adapter_id: None,
            }),
        },
        NOW,
    );
    let snapshot = state.snapshot();
    assert_eq!(
        snapshot.app.availability,
        dji4g_domain::Availability::NotDetected
    );
    for id in [
        DiagnosticCheckId::AtControl,
        DiagnosticCheckId::Cellular,
        DiagnosticCheckId::WindowsAdapter,
        DiagnosticCheckId::BoundGateway,
        DiagnosticCheckId::BoundPublic,
        DiagnosticCheckId::BoundDns,
        DiagnosticCheckId::SystemRoute,
        DiagnosticCheckId::Hotspot,
    ] {
        assert!(matches!(
            snapshot.diagnostics.get(id).state,
            DiagnosticCheckState::Unavailable { .. }
        ));
    }
}

#[test]
fn refresh_finished_closes_running_check_as_stage_missing() {
    let state = ReducerState::new(NOW);
    let state = reduce_state(
        &state,
        BackendEvent::RefreshStarted {
            cycle: RefreshCycleId(1),
            epoch: DeviceEpoch(0),
            scheduled: dji4g_application::CheckMask::only(DiagnosticCheckId::AtControl),
        },
        NOW,
    );
    let state = reduce_state(
        &state,
        BackendEvent::RefreshFinished {
            cycle: RefreshCycleId(1),
            epoch: DeviceEpoch(0),
        },
        NOW,
    );
    assert!(matches!(
        state.snapshot().diagnostics.get(DiagnosticCheckId::AtControl).state,
        DiagnosticCheckState::Failed { ref code } if code.stable().as_str() == "app:stage_missing"
    ));
}

#[test]
fn periodic_refresh_keeps_current_fresh_classification_while_checks_run() {
    let state = ReducerState::test_ready(NOW);
    let state = reduce_state(
        &state,
        BackendEvent::RefreshStarted {
            cycle: RefreshCycleId(1),
            epoch: DeviceEpoch(1),
            scheduled: dji4g_application::CheckMask::all(),
        },
        NOW,
    );
    let snapshot = state.snapshot();
    assert_eq!(
        snapshot.app.availability,
        dji4g_domain::Availability::Available
    );
    assert_eq!(snapshot.app.freshness, dji4g_domain::Freshness::Fresh);
    assert!(
        snapshot
            .diagnostics
            .iter()
            .all(|check| matches!(check.state, DiagnosticCheckState::Running { .. }))
    );
}

#[test]
fn refresh_signal_is_coalesced_and_non_refresh_queue_is_bounded() {
    let (handle, _runner) =
        dji4g_application::ControllerRunner::new(dji4g_application::Controller::for_test(NOW));
    assert_eq!(
        handle.try_send(dji4g_application::UiCommand::Refresh),
        Ok(dji4g_application::CommandReceipt::Accepted)
    );
    for _ in 0..99 {
        assert_eq!(
            handle.try_send(dji4g_application::UiCommand::Refresh),
            Ok(dji4g_application::CommandReceipt::Coalesced)
        );
    }
    for _ in 0..dji4g_application::COMMAND_QUEUE_CAPACITY {
        assert_eq!(
            handle.try_send(dji4g_application::UiCommand::SetStartMinimized(true)),
            Ok(dji4g_application::CommandReceipt::Accepted)
        );
    }
    assert_eq!(
        handle.try_send(dji4g_application::UiCommand::SetStartMinimized(false)),
        Err(dji4g_application::UiSendError::QueueFull)
    );
}

struct StaticInventory {
    result: Result<InventoryObservation, dji4g_application::PortError>,
}

impl InventoryPort for StaticInventory {
    fn scan(&self) -> PortFuture<'_, Result<InventoryObservation, dji4g_application::PortError>> {
        let result = self.result.clone();
        Box::pin(async move { result })
    }
}

struct StaticAt {
    result: Result<AtObservation, dji4g_application::PortError>,
}

impl AtPort for StaticAt {
    fn observe(
        &self,
        _target: &TargetContext,
    ) -> PortFuture<'_, Result<AtObservation, dji4g_application::PortError>> {
        let result = self.result.clone();
        Box::pin(async move { result })
    }

    fn invalidate(&self, _epoch: DeviceEpoch) {}
}

struct StaticAdapter {
    result: Result<AdapterObservationDto, dji4g_application::PortError>,
}

impl AdapterPort for StaticAdapter {
    fn resolve(
        &self,
        _target: &TargetContext,
    ) -> PortFuture<'_, Result<AdapterObservationDto, dji4g_application::PortError>> {
        let result = self.result.clone();
        Box::pin(async move { result })
    }

    fn read_byte_counters(
        &self,
        _adapter_id: &str,
    ) -> PortFuture<'_, Result<(u64, u64), dji4g_application::PortError>> {
        let result = self.result.clone();
        Box::pin(async move {
            match result {
                Ok(dto) => match (dto.rx_bytes, dto.tx_bytes) {
                    (Some(rx), Some(tx)) => Ok((rx, tx)),
                    _ => Err(PortError::new(
                        ErrorCode::CapabilityUnavailable,
                        "net:rate_counters_unavailable",
                    )),
                },
                Err(error) => Err(error),
            }
        })
    }
}

/// An adapter port whose read-only counters can be changed between rates-only ticks, so a scenario
/// test can drive the 1 s cadence deterministically and count how many read-only fetches happened.
/// `resolve` returns a fixed observation (used by the full refresh to bind the adapter); the rates
/// tick reads `counters` instead, exactly as production reads live `GetIfEntry2` octets.
struct RateTickAdapter {
    observation: AdapterObservationDto,
    counters: Mutex<Option<(u64, u64)>>,
    reads: AtomicUsize,
}

impl RateTickAdapter {
    fn set_counters(&self, counters: Option<(u64, u64)>) {
        *self.counters.lock().expect("counters lock") = counters;
    }
}

impl AdapterPort for RateTickAdapter {
    fn resolve(
        &self,
        _target: &TargetContext,
    ) -> PortFuture<'_, Result<AdapterObservationDto, dji4g_application::PortError>> {
        let observation = self.observation.clone();
        Box::pin(async move { Ok(observation) })
    }

    fn read_byte_counters(
        &self,
        _adapter_id: &str,
    ) -> PortFuture<'_, Result<(u64, u64), dji4g_application::PortError>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let counters = *self.counters.lock().expect("counters lock");
        Box::pin(async move {
            counters.ok_or_else(|| {
                PortError::new(
                    ErrorCode::CapabilityUnavailable,
                    "net:rate_counters_unavailable",
                )
            })
        })
    }
}

struct StaticProbe {
    result: Result<ProbeObservationDto, dji4g_application::PortError>,
    calls: AtomicUsize,
}

impl NetworkProbePort for StaticProbe {
    fn observe(
        &self,
        _adapter: &AdapterContext,
        _active: bool,
    ) -> PortFuture<'_, Result<ProbeObservationDto, dji4g_application::PortError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let result = self.result.clone();
        Box::pin(async move { result })
    }
}

/// A probe whose future stays `Pending` for a bounded number of polls before resolving.  Stage
/// workers must drive such futures to completion (the runner-side `poll_ready` loop), instead of
/// assuming every port future is ready on first poll.
struct PendingThenReadyProbe {
    result: Result<ProbeObservationDto, dji4g_application::PortError>,
    calls: AtomicUsize,
}

impl NetworkProbePort for PendingThenReadyProbe {
    fn observe(
        &self,
        _adapter: &AdapterContext,
        _active: bool,
    ) -> PortFuture<'_, Result<ProbeObservationDto, dji4g_application::PortError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let result = self.result.clone();
        Box::pin(async move {
            let mut pending_polls = 8;
            std::future::poll_fn(move |context| {
                if pending_polls == 0 {
                    Poll::Ready(())
                } else {
                    pending_polls -= 1;
                    context.waker().wake_by_ref();
                    Poll::Pending
                }
            })
            .await;
            result
        })
    }
}

/// An AT port whose first observation blocks longer than the injected stage timeout (inside the
/// future's first poll, like the production ports).  Later calls return immediately so the
/// subsequent refresh cycle can succeed and prove the monitor was not wedged.
struct SlowFirstAt {
    block: Duration,
    calls: AtomicUsize,
}

impl AtPort for SlowFirstAt {
    fn observe(
        &self,
        _target: &TargetContext,
    ) -> PortFuture<'_, Result<AtObservation, dji4g_application::PortError>> {
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let block = self.block;
        Box::pin(async move {
            if first {
                std::thread::sleep(block);
            }
            Ok(AtObservation {
                availability: AtControlAvailability::Available,
                cellular: None,
            })
        })
    }

    fn invalidate(&self, _epoch: DeviceEpoch) {}
}

#[test]
fn runner_executes_inventory_at_adapter_probe_dag_and_publishes_one_snapshot() {
    let epoch = DeviceEpoch(1);
    let probe = Arc::new(PendingThenReadyProbe {
        result: Ok(full_probe(epoch)),
        calls: AtomicUsize::new(0),
    });
    let controller = Controller::new(
        ReducerState::new(NOW),
        Arc::new(FakeActionExecutor::new()),
        Arc::new(FakeClock::new(NOW)),
    );
    let (handle, runner) = ControllerRunner::new(controller);
    let mut runner = runner.with_ports(MonitorPorts {
        inventory: Arc::new(StaticInventory {
            result: Ok(full_inventory(epoch)),
        }),
        at: Arc::new(StaticAt {
            result: Ok(AtObservation {
                availability: AtControlAvailability::Available,
                cellular: None,
            }),
        }),
        adapter: Arc::new(StaticAdapter {
            result: Ok(full_adapter(epoch)),
        }),
        probe: Arc::clone(&probe) as Arc<dyn NetworkProbePort>,
        hotspot: None,
        sms: None,
        device_tools: None,
    });
    runner.run_one_refresh();
    let snapshot = handle.subscribe().borrow();
    assert_eq!(
        snapshot.app.availability,
        dji4g_domain::Availability::Available
    );
    assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    // Every stage of the cycle reported exactly once: inventory, AT, adapter, the probe family,
    // and the (portless here) hotspot check, and `RefreshFinished` closed the cycle, so no check
    // is left `Running`.
    assert_eq!(
        snapshot.diagnostics.get(DiagnosticCheckId::UsbDevice).state,
        DiagnosticCheckState::Passed
    );
    assert_eq!(
        snapshot.diagnostics.get(DiagnosticCheckId::AtControl).state,
        DiagnosticCheckState::Passed
    );
    assert_eq!(
        snapshot
            .diagnostics
            .get(DiagnosticCheckId::WindowsAdapter)
            .state,
        DiagnosticCheckState::Passed
    );
    for id in [
        DiagnosticCheckId::BoundGateway,
        DiagnosticCheckId::BoundPublic,
        DiagnosticCheckId::BoundDns,
        DiagnosticCheckId::SystemRoute,
    ] {
        assert_eq!(
            snapshot.diagnostics.get(id).state,
            DiagnosticCheckState::Passed
        );
    }
    assert!(matches!(
        snapshot.diagnostics.get(DiagnosticCheckId::Hotspot).state,
        DiagnosticCheckState::Unavailable { ref code } if code.stable().as_str() == "app:hotspot_unavailable"
    ));
    assert!(
        snapshot
            .diagnostics
            .iter()
            .all(|check| !matches!(check.state, DiagnosticCheckState::Running { .. }))
    );
}

/// The stage watchdog: a stage whose port blocks longer than the injected (short) stage timeout
/// is failed with `app:stage_timeout` for that check only, the cycle still finalizes, and a
/// subsequent cycle succeeds normally — a wedged stage must never wedge the monitor.
#[test]
fn stage_watchdog_bounds_a_blocked_stage_and_the_next_cycle_recovers() {
    // The production budget is a 15 s bound per stage; the scenario injects a short one via the
    // documented test knob instead of waiting it out.
    assert_eq!(STAGE_TIMEOUT, Duration::from_secs(15));

    let epoch = DeviceEpoch(1);
    let controller = Controller::new(
        ReducerState::new(NOW),
        Arc::new(FakeActionExecutor::new()),
        Arc::new(FakeClock::new(NOW)),
    );
    let (handle, runner) = ControllerRunner::new(controller);
    let mut runner = runner
        .with_ports(MonitorPorts {
            inventory: Arc::new(StaticInventory {
                result: Ok(full_inventory(epoch)),
            }),
            at: Arc::new(SlowFirstAt {
                block: Duration::from_secs(2),
                calls: AtomicUsize::new(0),
            }) as Arc<dyn AtPort>,
            adapter: Arc::new(StaticAdapter {
                result: Ok(full_adapter(epoch)),
            }),
            probe: Arc::new(StaticProbe {
                result: Ok(full_probe(epoch)),
                calls: AtomicUsize::new(0),
            }) as Arc<dyn NetworkProbePort>,
            hotspot: None,
            sms: None,
            device_tools: None,
        })
        .with_stage_timeout(Duration::from_millis(100));

    runner.run_one_refresh();
    {
        let snapshot = handle.subscribe().borrow();
        // The blocked stage is reported with the watchdog failure code.
        assert!(matches!(
            snapshot
                .diagnostics
                .get(DiagnosticCheckId::AtControl)
                .state,
            DiagnosticCheckState::Failed { ref code } if code.stable().as_str() == "app:stage_timeout"
        ));
        // The other stages still reported for the same cycle.
        assert_eq!(
            snapshot.diagnostics.get(DiagnosticCheckId::UsbDevice).state,
            DiagnosticCheckState::Passed
        );
        assert_eq!(
            snapshot
                .diagnostics
                .get(DiagnosticCheckId::WindowsAdapter)
                .state,
            DiagnosticCheckState::Passed
        );
        assert_eq!(
            snapshot
                .diagnostics
                .get(DiagnosticCheckId::BoundPublic)
                .state,
            DiagnosticCheckState::Passed
        );
        // RefreshFinished still ran: no check is left Running.
        assert!(
            snapshot
                .diagnostics
                .iter()
                .all(|check| !matches!(check.state, DiagnosticCheckState::Running { .. }))
        );
    }

    // The next refresh cycle proceeds normally: the monitor was not wedged.
    runner.run_one_refresh();
    let snapshot = handle.subscribe().borrow();
    assert_eq!(
        snapshot.diagnostics.get(DiagnosticCheckId::AtControl).state,
        DiagnosticCheckState::Passed
    );
    assert_eq!(
        snapshot.app.availability,
        dji4g_domain::Availability::Available
    );
}

/// The reported field failure.  The module is present and its RNDIS adapter works, but its AT
/// serial interfaces are in an error state, so AT selection fails with `pnp:no_safe_at_port`.  One
/// automatic scan must recognise the device and name the AT fault, instead of leaving the panel in
/// its initial 「正在检测」 state with no recorded evidence at all.
#[test]
fn present_device_with_broken_at_port_is_recognised_and_explained() {
    let epoch = DeviceEpoch(1);
    let clock = Arc::new(FakeClock::new(NOW));
    let controller = Controller::new(
        ReducerState::new(NOW),
        Arc::new(FakeActionExecutor::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    let (handle, runner) = ControllerRunner::new(controller);
    let mut runner = runner.with_ports(MonitorPorts {
        inventory: Arc::new(StaticInventory {
            result: Ok(InventoryObservation {
                at_port: None,
                ..full_inventory(epoch)
            }),
        }),
        at: Arc::new(StaticAt {
            result: Err(PortError::new(
                ErrorCode::CapabilityUnavailable,
                "pnp:no_safe_at_port",
            )),
        }),
        adapter: Arc::new(StaticAdapter {
            result: Ok(full_adapter(epoch)),
        }),
        probe: Arc::new(StaticProbe {
            result: Ok(full_probe(epoch)),
            calls: AtomicUsize::new(0),
        }) as Arc<dyn NetworkProbePort>,
        hotspot: None,
        sms: None,
        device_tools: None,
    });

    assert!(
        runner.poll_monitoring_cadence(),
        "the first scan must happen without any user command"
    );

    let snapshot = handle.subscribe().borrow();
    assert_eq!(
        snapshot.app.availability,
        Availability::Limited(LimitedReason::AtControlUnavailable)
    );
    assert_eq!(snapshot.app.freshness, Freshness::Fresh);
    let device = snapshot
        .app
        .device
        .as_ref()
        .expect("a present device must be recognised so the overview can show its identity");
    assert_eq!(device.identity, target());
    assert_eq!(device.at_port, None);
    assert_eq!(device.adapter_id.as_deref(), Some("{adapter}"));
    // A broken AT port must never be promoted to a green state.
    assert_ne!(snapshot.app.availability, Availability::Available);
}

/// Evidence expires after `EVIDENCE_TTL` (30 s), so the runner must rescan on its own.  A panel
/// that only scanned on an explicit user command decayed to 「状态已过期」 and never recovered.
#[test]
fn monitoring_cadence_rescans_without_any_user_command() {
    let epoch = DeviceEpoch(1);
    let clock = Arc::new(FakeClock::new(NOW));
    let probe = Arc::new(StaticProbe {
        result: Ok(full_probe(epoch)),
        calls: AtomicUsize::new(0),
    });
    let controller = Controller::new(
        ReducerState::new(NOW),
        Arc::new(FakeActionExecutor::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    let (_handle, runner) = ControllerRunner::new(controller);
    let mut runner = runner.with_ports(MonitorPorts {
        inventory: Arc::new(StaticInventory {
            result: Ok(full_inventory(epoch)),
        }),
        at: Arc::new(StaticAt {
            result: Ok(AtObservation {
                availability: AtControlAvailability::Available,
                cellular: None,
            }),
        }),
        adapter: Arc::new(StaticAdapter {
            result: Ok(full_adapter(epoch)),
        }),
        probe: Arc::clone(&probe) as Arc<dyn NetworkProbePort>,
        hotspot: None,
        sms: None,
        device_tools: None,
    });

    assert!(
        runner.poll_monitoring_cadence(),
        "the first pass is always due"
    );
    assert!(
        !runner.poll_monitoring_cadence(),
        "a second pass must not be due immediately"
    );

    clock.advance_wall(REFRESH_INTERVAL - Duration::from_secs(1));
    assert!(
        !runner.poll_monitoring_cadence(),
        "still inside the cadence interval"
    );

    clock.advance_wall(Duration::from_secs(1));
    assert!(
        runner.poll_monitoring_cadence(),
        "due again once the interval elapsed"
    );

    assert_eq!(
        probe.calls.load(Ordering::SeqCst),
        2,
        "exactly the two due passes may scan"
    );
    assert!(
        REFRESH_INTERVAL < Duration::from_secs(30),
        "the cadence must stay inside EVIDENCE_TTL or the published state decays to Stale"
    );
}

/// A portless runner is a deterministic test or demo backend and must never scan on a timer.
#[test]
fn portless_runner_never_scans_on_a_timer() {
    let (_handle, mut runner) = ControllerRunner::new(Controller::for_test(NOW));
    assert!(!runner.poll_monitoring_cadence());
    assert!(!runner.poll_monitoring_cadence());
}

#[test]
fn periodic_refresh_due_is_immediate_first_then_interval_bounded() {
    let interval = REFRESH_INTERVAL;
    assert!(
        periodic_refresh_due(None, NOW, interval),
        "a runner that never scanned is due immediately"
    );
    assert!(!periodic_refresh_due(Some(NOW), NOW, interval));
    assert!(!periodic_refresh_due(
        Some(NOW),
        NOW + interval - Duration::from_millis(1),
        interval
    ));
    assert!(periodic_refresh_due(Some(NOW), NOW + interval, interval));
    // A wall-clock step backwards must not stall the monitor.
    assert!(periodic_refresh_due(Some(NOW + interval), NOW, interval));
}

fn not_detected_inventory(epoch: DeviceEpoch) -> InventoryObservation {
    InventoryObservation {
        epoch,
        presence: DevicePresenceDto::NotDetected,
        identity: None,
        problem_code: None,
        at_port: None,
        adapter_id: None,
    }
}

/// A transient empty enumeration (or an unplug/replug) makes the inventory port bump its
/// epoch. The reducer must adopt the strictly newer epoch; dropping that event strands the
/// panel forever on frozen evidence: UsbDevice force-fails with `app:stage_missing` every
/// cycle and the header stays 「状态已过期」.
#[test]
fn transient_empty_scan_advances_epoch_instead_of_stranding_the_panel() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_inventory(epoch)),
        },
        NOW,
    );
    state = reduce_state(
        &state,
        BackendEvent::AdapterFinished {
            cycle: RefreshCycleId(1),
            epoch,
            result: passed(full_adapter(epoch)),
        },
        NOW,
    );

    let bumped = DeviceEpoch(2);
    state = reduce_state(
        &state,
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(2),
            epoch: bumped,
            result: passed(not_detected_inventory(bumped)),
        },
        NOW,
    );
    assert_eq!(state.epoch(), DeviceEpoch(2));
    state = reduce_state(
        &state,
        BackendEvent::RefreshFinished {
            cycle: RefreshCycleId(2),
            epoch: bumped,
        },
        NOW,
    );
    let snapshot = state.snapshot();
    assert!(snapshot.app.device.is_none());
    assert_eq!(
        snapshot.app.availability,
        dji4g_domain::Availability::NotDetected
    );
    assert!(snapshot.diagnostics.iter().all(|check| !matches!(
        check.state,
        DiagnosticCheckState::Failed { ref code } if code.stable().as_str() == "app:stage_missing"
    )));

    let returned = DeviceEpoch(3);
    state = reduce_state(
        &state,
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(3),
            epoch: returned,
            result: passed(full_inventory(returned)),
        },
        NOW,
    );
    state = reduce_state(
        &state,
        BackendEvent::AdapterFinished {
            cycle: RefreshCycleId(3),
            epoch: returned,
            result: passed(full_adapter(returned)),
        },
        NOW,
    );
    state = reduce_state(
        &state,
        BackendEvent::ProbeFinished {
            cycle: RefreshCycleId(3),
            epoch: returned,
            result: passed(full_probe(returned)),
        },
        NOW,
    );
    state = reduce_state(
        &state,
        BackendEvent::RefreshFinished {
            cycle: RefreshCycleId(3),
            epoch: returned,
        },
        NOW,
    );
    let snapshot = state.snapshot();
    assert_eq!(state.epoch(), DeviceEpoch(3));
    assert!(snapshot.app.device.is_some());
    assert!(matches!(
        snapshot.app.availability,
        Availability::Available | Availability::Limited(_)
    ));
}

#[test]
fn older_epoch_events_are_still_rejected_after_adoption() {
    let mut state = ReducerState::new(NOW);
    let epoch = DeviceEpoch(2);
    state = reduce_state(
        &state,
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(2),
            epoch,
            result: passed(full_inventory(epoch)),
        },
        NOW,
    );
    let before = state.snapshot();

    let after_state = reduce_state(
        &state,
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(3),
            epoch: DeviceEpoch(1),
            result: passed(full_inventory(DeviceEpoch(1))),
        },
        NOW,
    );
    let after = after_state.snapshot();

    assert_eq!(after_state.epoch(), DeviceEpoch(2));
    assert_eq!(after.app.device, before.app.device);
}

fn network_rates(state: &ReducerState) -> (Option<u64>, Option<u64>) {
    let snapshot = state.snapshot();
    let network = snapshot.app.network.as_ref().expect("network evidence");
    (network.down_bytes_per_sec, network.up_bytes_per_sec)
}

#[test]
fn adapter_counters_become_rates_only_after_a_baseline() {
    let mut state = ReducerState::test_ready(NOW);
    let epoch = DeviceEpoch(1);

    // First sample: a baseline, never a fabricated rate.
    state = reduce_state(
        &state,
        adapter_finished(epoch, RefreshCycleId(1), Some(1_000), Some(500), NOW),
        NOW,
    );
    assert_eq!(network_rates(&state), (None, None));

    // Ten seconds later: 10 KB downloaded, 100 B uploaded.
    let later = NOW + Duration::from_secs(10);
    state = reduce_state(
        &state,
        adapter_finished(epoch, RefreshCycleId(2), Some(11_000), Some(600), later),
        later,
    );
    assert_eq!(network_rates(&state), (Some(1_000), Some(10)));
}

#[test]
fn counter_regression_resets_the_rate_baseline() {
    let mut state = ReducerState::test_ready(NOW);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        adapter_finished(epoch, RefreshCycleId(1), Some(1_000_000), Some(500), NOW),
        NOW,
    );
    // The counter jumped backwards: the adapter restarted, so no honest rate exists.
    let later = NOW + Duration::from_secs(10);
    state = reduce_state(
        &state,
        adapter_finished(epoch, RefreshCycleId(2), Some(4_000), Some(600), later),
        later,
    );
    assert_eq!(network_rates(&state), (None, None));

    // A fresh baseline from the new counter era computes normally again.
    let after = later + Duration::from_secs(10);
    state = reduce_state(
        &state,
        adapter_finished(epoch, RefreshCycleId(3), Some(14_000), Some(1_600), after),
        after,
    );
    assert_eq!(network_rates(&state), (Some(1_000), Some(100)));
}

#[test]
fn an_epoch_change_resets_the_rate_baseline() {
    let mut state = ReducerState::test_ready(NOW);
    state = reduce_state(
        &state,
        adapter_finished(
            DeviceEpoch(1),
            RefreshCycleId(1),
            Some(1_000),
            Some(500),
            NOW,
        ),
        NOW,
    );
    let later = NOW + Duration::from_secs(10);
    // A real epoch change replugs through a fresh inventory before the adapter stage re-runs.
    state = reduce_state(
        &state,
        BackendEvent::InventoryFinished {
            cycle: RefreshCycleId(2),
            epoch: DeviceEpoch(2),
            result: passed(full_inventory(DeviceEpoch(2))),
        },
        later,
    );
    state = reduce_state(
        &state,
        adapter_finished(
            DeviceEpoch(2),
            RefreshCycleId(2),
            Some(11_000),
            Some(600),
            later,
        ),
        later,
    );
    assert_eq!(network_rates(&state), (None, None));
}

#[test]
fn a_stale_previous_sample_beyond_the_ttl_resets_the_baseline() {
    let mut state = ReducerState::test_ready(NOW);
    state = reduce_state(
        &state,
        adapter_finished(
            DeviceEpoch(1),
            RefreshCycleId(1),
            Some(1_000),
            Some(500),
            NOW,
        ),
        NOW,
    );
    let stale = NOW + Duration::from_secs(31);
    state = reduce_state(
        &state,
        adapter_finished(
            DeviceEpoch(1),
            RefreshCycleId(2),
            Some(11_000),
            Some(600),
            stale,
        ),
        stale,
    );
    assert_eq!(network_rates(&state), (None, None));
}

#[test]
fn rate_tick_due_is_immediate_first_then_period_bounded() {
    let period = RATE_TICK_INTERVAL;
    assert!(
        rate_tick_due(None, NOW, period),
        "a runner that never ticked is due immediately"
    );
    assert!(!rate_tick_due(Some(NOW), NOW, period));
    assert!(!rate_tick_due(
        Some(NOW),
        NOW + period - Duration::from_millis(1),
        period
    ));
    assert!(rate_tick_due(Some(NOW), NOW + period, period));
    // A wall-clock step backwards must not stall the rates cadence.
    assert!(rate_tick_due(Some(NOW + period), NOW, period));
}

/// A rates-only tick recomputes throughput but must leave every safety-relevant evidence field
/// exactly as the last full refresh set it: `observed_at`, `evidence_revision`, and freshness never
/// move, only the publication revision advances so the UI repaints.
#[test]
fn rates_sampled_updates_rates_without_advancing_evidence() {
    let mut state = ReducerState::test_ready(NOW);
    let epoch = DeviceEpoch(1);
    // Establish the counter baseline through the adapter stage (first sample => no rate yet).
    state = reduce_state(
        &state,
        adapter_finished(epoch, RefreshCycleId(1), Some(1_000), Some(500), NOW),
        NOW,
    );
    assert_eq!(network_rates(&state), (None, None));
    let observed_before = state.snapshot().app.observed_at;
    let evidence_before = state.evidence_revision();
    let publication_before = state.publication_revision();

    // One second later a rates-only tick recomputes throughput from the same baseline logic.
    let tick = NOW + Duration::from_secs(1);
    state = reduce_state(
        &state,
        BackendEvent::RatesSampled {
            adapter_id: "{adapter}".into(),
            epoch,
            rx: Some(2_000),
            tx: Some(600),
            sampled_at: tick,
        },
        tick,
    );

    assert_eq!(network_rates(&state), (Some(1_000), Some(100)));
    assert_eq!(state.snapshot().app.observed_at, observed_before);
    assert_eq!(state.evidence_revision(), evidence_before);
    assert_eq!(state.publication_revision(), publication_before + 1);
    assert_eq!(state.snapshot().app.freshness, Freshness::Fresh);
}

/// A counter that jumps backwards during a rates-only tick (adapter restart / replug) must yield
/// honest `None` rates — never a stale or fabricated number — and still must not advance evidence.
#[test]
fn rate_tick_counter_regression_yields_none_rates() {
    let mut state = ReducerState::test_ready(NOW);
    let epoch = DeviceEpoch(1);
    state = reduce_state(
        &state,
        adapter_finished(epoch, RefreshCycleId(1), Some(1_000), Some(500), NOW),
        NOW,
    );
    // First a working rate, so the None below is provably the regression and not a missing baseline.
    let tick = NOW + Duration::from_secs(1);
    state = reduce_state(
        &state,
        BackendEvent::RatesSampled {
            adapter_id: "{adapter}".into(),
            epoch,
            rx: Some(2_000),
            tx: Some(600),
            sampled_at: tick,
        },
        tick,
    );
    assert_eq!(network_rates(&state), (Some(1_000), Some(100)));

    let observed_before = state.snapshot().app.observed_at;
    let evidence_before = state.evidence_revision();
    // rx jumps backwards below the previous sample.
    let regress = tick + Duration::from_secs(1);
    state = reduce_state(
        &state,
        BackendEvent::RatesSampled {
            adapter_id: "{adapter}".into(),
            epoch,
            rx: Some(500),
            tx: Some(600),
            sampled_at: regress,
        },
        regress,
    );
    assert_eq!(network_rates(&state), (None, None));
    assert_eq!(state.snapshot().app.observed_at, observed_before);
    assert_eq!(state.evidence_revision(), evidence_before);
}

/// End-to-end monitor proof: one due rates-only tick reads the bound adapter's counters and updates
/// the published network rates, while `observed_at`, evidence revision, and freshness stay pinned to
/// the 10 s refresh. A second immediate tick is not due (no busy sampling), and exactly one
/// read-only fetch runs.
#[test]
fn rate_tick_updates_rates_without_touching_observed_at_or_freshness() {
    let epoch = DeviceEpoch(1);
    let clock = Arc::new(FakeClock::new(NOW));
    let adapter = Arc::new(RateTickAdapter {
        observation: counted_adapter(epoch, Some(1_000), Some(500)),
        counters: Mutex::new(Some((1_000, 500))),
        reads: AtomicUsize::new(0),
    });
    let controller = Controller::new(
        ReducerState::new(NOW),
        Arc::new(FakeActionExecutor::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
    );
    let (handle, runner) = ControllerRunner::new(controller);
    let mut runner = runner.with_ports(MonitorPorts {
        inventory: Arc::new(StaticInventory {
            result: Ok(full_inventory(epoch)),
        }),
        at: Arc::new(StaticAt {
            result: Ok(AtObservation {
                availability: AtControlAvailability::Available,
                cellular: None,
            }),
        }),
        adapter: Arc::clone(&adapter) as Arc<dyn AdapterPort>,
        probe: Arc::new(StaticProbe {
            result: Ok(full_probe(epoch)),
            calls: AtomicUsize::new(0),
        }) as Arc<dyn NetworkProbePort>,
        hotspot: None,
        sms: None,
        device_tools: None,
    });

    // The first evidence refresh binds the adapter and lays down the rate baseline (no rate yet).
    assert!(runner.poll_monitoring_cadence());
    let before = handle.subscribe().borrow();
    let observed_before = before.app.observed_at;
    let revision_before = before.app.revision;
    let freshness_before = before.app.freshness;
    let network_before = before.app.network.as_ref().expect("bound network");
    assert_eq!(
        (
            network_before.down_bytes_per_sec,
            network_before.up_bytes_per_sec
        ),
        (None, None)
    );
    assert_eq!(freshness_before, Freshness::Fresh);

    // One second on the counters advance; a rates-only tick must pick them up as one sample.
    clock.advance_wall(RATE_TICK_INTERVAL);
    adapter.set_counters(Some((2_000, 600)));
    assert!(
        runner.poll_rate_cadence(),
        "the rates tick is due one period after the refresh"
    );
    // An immediate second tick must not be due: the cadence is a due-check, never a busy sample.
    assert!(!runner.poll_rate_cadence());

    let after = handle.subscribe().borrow();
    let network_after = after.app.network.as_ref().expect("bound network");
    assert_eq!(
        (
            network_after.down_bytes_per_sec,
            network_after.up_bytes_per_sec
        ),
        (Some(1_000), Some(100)),
        "1000 B down and 100 B up measured over the one-second tick"
    );
    // Freshness semantics stay pinned to the 10 s evidence refresh: the tick advanced none of them.
    assert_eq!(after.app.observed_at, observed_before);
    assert_eq!(after.app.revision, revision_before);
    assert_eq!(after.app.freshness, freshness_before);
    assert_eq!(
        adapter.reads.load(Ordering::SeqCst),
        1,
        "exactly one read-only counter fetch per due tick"
    );
}
