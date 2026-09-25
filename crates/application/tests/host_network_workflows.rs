use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

use dji4g_application::{
    ActionRequest, Controller, ControllerRunner, FakeActionExecutor, FakeClock, HostNetworkPhase,
    HostNetworkPort, PortError, PortFuture, ProxyRepairPreview, ProxyRepairResult, ReducerState,
    UiCommand, UiSendError,
};
use dji4g_domain::{HostAdapter, HostNetworkObservation, HostProxyMode, ProxyBinding, ProxyClient};

struct FakeHost {
    calls: AtomicUsize,
    now: SystemTime,
}

#[test]
fn host_operation_and_network_changing_repair_cannot_start_together() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000);
    let mut controller = Controller::for_test(now);
    controller
        .handle_command(UiCommand::InspectHostNetwork)
        .unwrap();
    assert_eq!(
        controller.handle_command(UiCommand::PrepareAction {
            request: ActionRequest::RestartAdapter,
        }),
        Err(UiSendError::QueueFull)
    );
}

impl HostNetworkPort for FakeHost {
    fn inspect(&self) -> PortFuture<'_, Result<HostNetworkObservation, PortError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(HostNetworkObservation {
                adapters: vec![HostAdapter {
                    guid: "{wifi}".into(),
                    luid: 1,
                    alias: "Wi-Fi".into(),
                    up: true,
                }],
                default_routes: vec![],
                system_proxy: HostProxyMode::Manual,
                binding: Some(ProxyBinding {
                    client: ProxyClient::ClashVergeRev,
                    version: Some("2.5.5".into()),
                    interface_alias: "removed adapter".into(),
                    repairable: true,
                }),
                proxy_inspection_complete: true,
                proxy_error_code: None,
                observed_at: self.now,
            })
        })
    }
    fn prepare_repair(&self, _: u64) -> PortFuture<'_, Result<ProxyRepairPreview, PortError>> {
        Box::pin(async move {
            Ok(ProxyRepairPreview {
                plan_id: 9,
                interface_alias: "removed adapter".into(),
                expires_at: self.now + Duration::from_secs(60),
            })
        })
    }
    fn apply_repair(&self, _: u64) -> PortFuture<'_, Result<ProxyRepairResult, PortError>> {
        Box::pin(async {
            Ok(ProxyRepairResult {
                backup_id: 12,
                changed: true,
            })
        })
    }
    fn restore_repair(&self, _: u64) -> PortFuture<'_, Result<(), PortError>> {
        Box::pin(async { Ok(()) })
    }
}

fn settle(runner: &mut ControllerRunner) {
    assert!(runner.poll_host_requests());
    for _ in 0..200 {
        if runner.poll_host_requests() {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("host worker never finished");
}

#[test]
fn host_diagnosis_runs_without_a_dji_device_and_repair_requires_ids() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let clock = Arc::new(FakeClock::new(now));
    let controller = Controller::new(
        ReducerState::new(now),
        Arc::new(FakeActionExecutor::new()),
        clock,
    );
    let fake = Arc::new(FakeHost {
        calls: AtomicUsize::new(0),
        now,
    });
    let (handle, runner) = ControllerRunner::new(controller);
    let mut runner = runner.with_host_network_port(fake.clone());
    handle.try_send(UiCommand::InspectHostNetwork).unwrap();
    runner.poll_commands();
    settle(&mut runner);
    let snapshot = runner.controller().snapshot();
    assert!(snapshot.app.device.is_none());
    assert_eq!(snapshot.host_network.phase, HostNetworkPhase::Ready);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    let finding_id = snapshot.host_network.finding_id.unwrap();
    handle
        .try_send(UiCommand::PrepareProxyRepair {
            finding_id: finding_id + 1,
        })
        .unwrap();
    runner.poll_commands();
    assert_eq!(
        runner.controller().snapshot().host_network.phase,
        HostNetworkPhase::Ready
    );
    handle
        .try_send(UiCommand::PrepareProxyRepair { finding_id })
        .unwrap();
    runner.poll_commands();
    settle(&mut runner);
    assert_eq!(
        runner.controller().snapshot().host_network.phase,
        HostNetworkPhase::AwaitingConfirmation
    );
    handle
        .try_send(UiCommand::ConfirmProxyRepair { plan_id: 9 })
        .unwrap();
    runner.poll_commands();
    settle(&mut runner);
    assert_eq!(
        runner.controller().snapshot().host_network.phase,
        HostNetworkPhase::AwaitingRestart
    );
}
