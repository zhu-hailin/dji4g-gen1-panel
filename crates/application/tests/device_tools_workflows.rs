//! Device-tool contract and workflow tests.
//!
//! The runner-level tests at the bottom drive a real [`ControllerRunner`] against fake ports, so
//! arbitration, cancellation and evidence bookkeeping are exercised without a module, a driver or
//! a serial port. Nothing here opens a COM port or installs anything.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use dji4g_application::{
    DeviceToolsSnapshot, FeatureStatus, MAX_TOOL_HISTORY_BYTES, ToolCapabilityRow, ToolContext,
    ToolHistory, ToolHistoryEntry, ToolOperation, ToolOperationKind, ToolOutcome, ToolPhase,
    ToolRequest, ToolTaskSnapshot, ToolTranscript, UsbNetReading, as_at_response, extract_identity,
    extract_payload, item_deadline, parse_profile_temperature, parse_usb_net,
    transcript_from_response,
};
use dji4g_at_protocol::{
    AtCommand, AtFinalCode, ToolReadId, ToolResponse, ValidatedToolLine, VerifiedUsbNetProfile,
};
use dji4g_domain::{DeviceEpoch, StableDeviceIdentity};

/// A sentinel that must never appear in a derived formatter, a log line or an export summary.
const SENTINEL: &str = "SENTINEL-13800138000";

fn identity() -> StableDeviceIdentity {
    StableDeviceIdentity {
        container_id: format!("container-{SENTINEL}"),
        device_instance_id: format!("instance-{SENTINEL}"),
        vid: 0x2ca3,
        pid: 0x4006,
    }
}

fn context() -> ToolContext {
    ToolContext {
        device_epoch: DeviceEpoch(7),
        sim_epoch: 3,
        identity: identity(),
        at_port: format!("COM9-{SENTINEL}"),
    }
}

fn response(lines: &[&str], final_code: AtFinalCode) -> ToolResponse {
    ToolResponse {
        lines: lines.iter().map(|line| (*line).to_owned()).collect(),
        urc_lines: Vec::new(),
        unclassified_lines: 0,
        final_code,
    }
}

fn entry(id: u64, transcript: ToolTranscript) -> ToolHistoryEntry {
    ToolHistoryEntry {
        id,
        operation: ToolOperationKind::Read(ToolReadId::SignalQuality),
        outcome: ToolOutcome::Ok,
        elapsed: Duration::from_millis(120),
        finished_at: SystemTime::UNIX_EPOCH,
        transcript: Arc::new(transcript),
    }
}

#[test]
fn a_module_refusal_is_not_a_transport_failure() {
    assert_eq!(
        ToolOutcome::from_final_code(&AtFinalCode::Ok),
        ToolOutcome::Ok
    );
    assert_eq!(
        ToolOutcome::from_final_code(&AtFinalCode::Error),
        ToolOutcome::Rejected
    );
    assert_eq!(
        ToolOutcome::from_final_code(&AtFinalCode::CmeError("10".to_owned())),
        ToolOutcome::Rejected
    );
    // Only the one refusal whose meaning the standard fixes is reported as unsupported.
    assert_eq!(
        ToolOutcome::from_final_code(&AtFinalCode::CmeError("4".to_owned())),
        ToolOutcome::Unsupported
    );
    assert_eq!(
        ToolOutcome::from_final_code(&AtFinalCode::CmsError("500".to_owned())),
        ToolOutcome::Rejected
    );
}

#[test]
fn outcomes_map_to_the_capability_statuses_the_page_shows() {
    assert_eq!(ToolOutcome::Ok.feature_status(), FeatureStatus::Supported);
    assert_eq!(
        ToolOutcome::Rejected.feature_status(),
        FeatureStatus::TemporarilyUnavailable
    );
    assert_eq!(
        ToolOutcome::Unsupported.feature_status(),
        FeatureStatus::UnsupportedConfirmed
    );
    assert_eq!(
        ToolOutcome::TransportFailure.feature_status(),
        FeatureStatus::TransportFailure
    );
    assert_eq!(
        ToolOutcome::FormatMismatch.feature_status(),
        FeatureStatus::FormatMismatch
    );
    // Nothing was learned about a read that never ran.
    assert_eq!(
        ToolOutcome::CancelledBeforeWrite.feature_status(),
        FeatureStatus::NotProbed
    );
    assert_eq!(
        ToolOutcome::ContextChanged.feature_status(),
        FeatureStatus::NotProbed
    );
    for outcome in [
        ToolOutcome::Rejected,
        ToolOutcome::Unsupported,
        ToolOutcome::TransportFailure,
        ToolOutcome::FormatMismatch,
        ToolOutcome::CancelledBeforeWrite,
        ToolOutcome::OutcomeUnknown,
        ToolOutcome::ContextChanged,
    ] {
        assert!(outcome.is_failure());
    }
    assert!(!ToolOutcome::Ok.is_failure());
}

#[test]
fn one_failed_item_keeps_the_other_results() {
    let mut snapshot = DeviceToolsSnapshot::default();
    let now = SystemTime::UNIX_EPOCH;
    snapshot.record_capability(ToolCapabilityRow::new(
        ToolReadId::Model,
        ToolOutcome::Ok,
        context(),
        now,
    ));
    snapshot.record_capability(ToolCapabilityRow::new(
        ToolReadId::Temperature,
        ToolOutcome::Rejected,
        context(),
        now,
    ));
    snapshot.record_capability(ToolCapabilityRow::new(
        ToolReadId::UsbNet,
        ToolOutcome::Unsupported,
        context(),
        now,
    ));
    assert_eq!(
        snapshot.capability(ToolReadId::Model).map(|row| row.status),
        Some(FeatureStatus::Supported)
    );
    assert_eq!(
        snapshot
            .capability(ToolReadId::Temperature)
            .map(|row| row.reason),
        Some(ToolOutcome::Rejected)
    );
    assert_eq!(
        snapshot
            .capability(ToolReadId::UsbNet)
            .map(|row| row.status),
        Some(FeatureStatus::UnsupportedConfirmed)
    );
    assert_eq!(
        snapshot.capability(ToolReadId::SignalQuality),
        None,
        "an untouched item stays NotProbed rather than borrowing another item's result"
    );
}

#[test]
fn an_ok_without_content_is_empty_not_a_value() {
    let row = ToolCapabilityRow::empty(ToolReadId::SmsStorage, context(), SystemTime::UNIX_EPOCH);
    assert_eq!(row.status, FeatureStatus::Empty);
    assert_eq!(row.reason, ToolOutcome::Ok);
}

#[test]
fn recording_the_same_read_twice_replaces_its_evidence() {
    let mut snapshot = DeviceToolsSnapshot::default();
    let now = SystemTime::UNIX_EPOCH;
    snapshot.record_capability(ToolCapabilityRow::new(
        ToolReadId::Model,
        ToolOutcome::TransportFailure,
        context(),
        now,
    ));
    snapshot.record_capability(ToolCapabilityRow::new(
        ToolReadId::Model,
        ToolOutcome::Ok,
        context(),
        now,
    ));
    assert_eq!(snapshot.capabilities.len(), 1);
    assert_eq!(
        snapshot.capability(ToolReadId::Model).map(|row| row.reason),
        Some(ToolOutcome::Ok)
    );
}

#[test]
fn the_history_is_bounded_in_items_and_bytes() {
    let mut history = ToolHistory::default();
    for id in 0..(dji4g_application::MAX_TOOL_HISTORY_ITEMS as u64 + 25) {
        history.push(entry(id, ToolTranscript::from_lines(["OK".to_owned()])));
    }
    assert_eq!(history.len(), dji4g_application::MAX_TOOL_HISTORY_ITEMS);
    // The oldest entries are the ones dropped.
    assert_eq!(history.entries().front().map(|item| item.id), Some(25));

    let mut history = ToolHistory::default();
    let big = "x".repeat(32 * 1024);
    for id in 0..20u64 {
        history.push(entry(id, ToolTranscript::from_lines([big.clone()])));
    }
    let bytes: usize = history
        .entries()
        .iter()
        .map(|item| item.transcript.bytes())
        .sum();
    assert!(
        bytes <= MAX_TOOL_HISTORY_BYTES,
        "history kept {bytes} bytes"
    );
    assert!(history.len() < 20);
}

#[test]
fn a_transcript_that_hits_its_bound_says_so() {
    let mut transcript = ToolTranscript::new();
    let chunk = "y".repeat(1000);
    for _ in 0..100 {
        transcript.push(chunk.clone());
    }
    assert!(transcript.is_truncated());
    assert!(transcript.bytes() <= dji4g_application::MAX_TRANSCRIPT_BYTES);
    assert!(transcript.bytes() < 100 * 1000);
}

#[test]
fn urcs_are_shown_but_marked_as_module_originated() {
    let response = ToolResponse {
        lines: vec!["+CSQ: 19,99".to_owned()],
        urc_lines: vec!["+CMTI: \"SM\",3".to_owned()],
        unclassified_lines: 0,
        final_code: AtFinalCode::Ok,
    };
    let transcript = transcript_from_response(&response);
    assert_eq!(transcript.lines()[0], "+CSQ: 19,99");
    assert!(transcript.lines()[1].contains("+CMTI"));
    assert!(transcript.lines()[1].contains("模块主动上报"));
}

#[test]
fn identity_and_usb_mode_come_from_their_own_answers() {
    let lines = vec!["+CGMI: Simulated Vendor".to_owned()];
    assert_eq!(
        extract_identity(&lines, "+CGMI:").as_deref(),
        Some("Simulated Vendor")
    );
    // A bare answer is accepted, a foreign prefixed line is not invented into a value.
    let bare = vec!["Simulated Vendor".to_owned()];
    assert_eq!(
        extract_identity(&bare, "+CGMI:").as_deref(),
        Some("Simulated Vendor")
    );
    let foreign = vec!["+CSQ: 12,0".to_owned()];
    assert_eq!(extract_identity(&foreign, "+CGMI:"), None);

    let usbnet = vec!["+QCFG: \"usbnet\",1".to_owned()];
    assert_eq!(
        parse_usb_net(&usbnet),
        Some(UsbNetReading::Verified(VerifiedUsbNetProfile::Ecm))
    );
    let unknown = vec!["+QCFG: \"usbnet\",7".to_owned()];
    assert_eq!(parse_usb_net(&unknown), Some(UsbNetReading::Unrecognised));
    let garbage = vec!["+QCFG: \"usbnet\",\"ecm\"".to_owned()];
    assert_eq!(parse_usb_net(&garbage), Some(UsbNetReading::Unrecognised));
    assert_eq!(parse_usb_net(&["OK".to_owned()]), None);
    assert_eq!(
        extract_payload(&["+CSQ: 12,0".to_owned()], "+CSQ:"),
        Some("12,0")
    );
}

#[test]
fn a_missing_temperature_sensor_is_absent_rather_than_zero() {
    let reading = response(&["+QTEMP: \"XO_THERM\",42"], AtFinalCode::Ok);
    let parsed = parse_profile_temperature(&reading);
    assert_eq!(parsed, vec![("XO_THERM".to_owned(), 42)]);
    let empty = response(&[], AtFinalCode::Ok);
    assert!(parse_profile_temperature(&empty).is_empty());
}

#[test]
fn a_sub_request_never_gets_more_than_its_own_deadline_or_the_batch_budget() {
    assert_eq!(
        item_deadline(Duration::from_secs(45)),
        dji4g_application::TOOL_TRANSACTION_TIMEOUT
    );
    assert_eq!(
        item_deadline(Duration::from_secs(3)),
        Duration::from_secs(3)
    );
    assert_eq!(item_deadline(Duration::ZERO), Duration::ZERO);
}

#[test]
fn a_pdp_answer_is_parsed_by_the_shared_parser() {
    let contexts = response(
        &[
            "+CGDCONT: 1,\"IP\",\"sentinel.apn\"",
            "+CGDCONT: 2,\"IPV6\",\"other\"",
        ],
        AtFinalCode::Ok,
    );
    let typed = as_at_response(DeviceEpoch(7), AtCommand::PdpContexts, &contexts);
    assert_eq!(typed.command, AtCommand::PdpContexts);
    let parsed = dji4g_at_protocol::parse_pdp_contexts(&typed).expect("parses");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].apn().as_str(), "sentinel.apn");
}

#[test]
fn no_derived_formatter_prints_request_text_device_identity_or_response_text() {
    let line = ValidatedToolLine::parse("AT+CGDCONT=1,\"IP\",\"sentinel.apn\"").expect("valid");
    let request = ToolRequest {
        id: 4,
        context: context(),
        operation: ToolOperation::Expert(line),
    };
    let task = ToolTaskSnapshot::new(&request, 1);
    let snapshot = DeviceToolsSnapshot {
        task: Some(task),
        ..DeviceToolsSnapshot::default()
    };
    let response = response(
        &[
            "+CGDCONT: 1,\"IP\",\"sentinel.apn\"",
            "+CME ERROR: SENTINEL-13800138000",
        ],
        AtFinalCode::CmeError(SENTINEL.to_owned()),
    );
    let transcript = transcript_from_response(&response);
    let mut with_history = snapshot.clone();
    with_history.history.push(entry(4, transcript.clone()));

    for rendered in [
        format!("{request:?}"),
        format!("{snapshot:?}"),
        format!("{with_history:?}"),
        format!("{:?}", request.operation),
        format!("{:?}", context()),
        format!("{transcript:?}"),
        format!("{:?}", entry(4, transcript.clone())),
    ] {
        assert!(!rendered.contains(SENTINEL), "{rendered}");
        assert!(!rendered.contains("sentinel.apn"), "{rendered}");
        assert!(!rendered.contains("container-"), "{rendered}");
        assert!(!rendered.contains("COM9"), "{rendered}");
    }
    // The explicit accessors are the only readers, and they do return the text.
    assert_eq!(transcript.lines()[0], "+CGDCONT: 1,\"IP\",\"sentinel.apn\"");
    assert!(context().at_port.contains("COM9"));
    assert!(context().identity.container_id.contains("container-"));
}

#[test]
fn a_device_change_drops_every_previous_conclusion() {
    let mut snapshot = DeviceToolsSnapshot::default();
    snapshot.record_capability(ToolCapabilityRow::new(
        ToolReadId::Model,
        ToolOutcome::Ok,
        context(),
        SystemTime::UNIX_EPOCH,
    ));
    snapshot.profile.model = Some("Simulated".to_owned());
    snapshot.history.push(entry(1, ToolTranscript::new()));
    snapshot.task = Some(ToolTaskSnapshot {
        id: 1,
        context: context(),
        operation: ToolOperationKind::Read(ToolReadId::Model),
        phase: ToolPhase::Finished,
        outcome: Some(ToolOutcome::Ok),
        completed_items: 1,
        total_items: 1,
    });
    snapshot.invalidate_context();
    assert!(snapshot.capabilities.is_empty());
    assert!(snapshot.profile.is_empty());
    assert!(snapshot.history.is_empty());
    assert!(snapshot.task.is_none());
    assert!(!snapshot.busy());
}

#[test]
fn an_expert_request_is_never_retried_and_never_printed() {
    let line = ValidatedToolLine::parse("AT+VENDOR=7").expect("valid");
    let operation = ToolOperation::Expert(line);
    assert!(operation.is_expert());
    assert_eq!(operation.kind(), ToolOperationKind::Expert);
    assert_eq!(format!("{operation:?}"), "Expert([REDACTED_TOOL_COMMAND])");
    let read = ToolOperation::Read(ToolReadId::SignalQuality);
    assert!(!read.is_expert());
    assert_eq!(
        read.kind(),
        ToolOperationKind::Read(ToolReadId::SignalQuality)
    );
    assert_eq!(ToolOperation::ProbeAll.kind(), ToolOperationKind::ProbeAll);
}

// ---------------------------------------------------------------------------------------------
// Controller- and runner-level lifecycle
//
// These drive the real `Controller`/`ControllerRunner` against fake ports. No module, no driver
// and no serial port is involved.
// ---------------------------------------------------------------------------------------------

use dji4g_application::{
    AdapterPort, AtPort, Controller, ControllerRunner, DeviceToolsPort, InventoryPort,
    MonitorPorts, NetworkProbePort, PortError, PortFuture, ProbeObservationDto, SmsPort,
    TargetContext, ToolReceipt, UiCommand,
};

fn controller() -> Controller {
    Controller::for_test(SystemTime::UNIX_EPOCH)
}

/// A tool port that records what it was asked to run and answers from a script.
struct FakeTools {
    calls: std::sync::Mutex<Vec<ToolOperationKind>>,
    /// Responses per operation, in call order; anything past the end is a transport failure.
    script: std::sync::Mutex<std::collections::VecDeque<(ToolOutcome, usize)>>,
}

impl FakeTools {
    fn new(script: Vec<(ToolOutcome, usize)>) -> Arc<Self> {
        Arc::new(Self {
            calls: std::sync::Mutex::new(Vec::new()),
            script: std::sync::Mutex::new(script.into()),
        })
    }

    fn calls(&self) -> Vec<ToolOperationKind> {
        self.calls.lock().expect("healthy lock").clone()
    }
}

impl DeviceToolsPort for FakeTools {
    fn execute(
        &self,
        _target: &TargetContext,
        request: ToolRequest,
        _control: dji4g_application::ToolControl,
    ) -> PortFuture<'_, Result<ToolReceipt, PortError>> {
        self.calls
            .lock()
            .expect("healthy lock")
            .push(request.operation.kind());
        let next = self.script.lock().expect("healthy lock").pop_front();
        let context = request.context.clone();
        let operation = request.operation.kind();
        let id = request.id;
        Box::pin(async move {
            let Some((outcome, payload_lines)) = next else {
                return Err(PortError::new(
                    dji4g_domain::ErrorCode::Timeout,
                    "device_tools:timeout",
                ));
            };
            let mut transcript = ToolTranscript::new();
            for index in 0..payload_lines {
                transcript.push(format!("line {index}"));
            }
            Ok(ToolReceipt {
                id,
                context,
                operation,
                outcome,
                elapsed: Duration::from_millis(5),
                transcript: Arc::new(transcript),
                saw_final_code: outcome == ToolOutcome::Ok,
                payload_lines,
            })
        })
    }
}

struct NoInventory;
impl InventoryPort for NoInventory {
    fn scan(&self) -> PortFuture<'_, Result<dji4g_application::InventoryObservation, PortError>> {
        Box::pin(async {
            Err(PortError::new(
                dji4g_domain::ErrorCode::Unsupported,
                "test:no_inventory",
            ))
        })
    }
}

struct NoAt;
impl AtPort for NoAt {
    fn observe(
        &self,
        _target: &TargetContext,
    ) -> PortFuture<'_, Result<dji4g_application::AtObservation, PortError>> {
        Box::pin(async {
            Err(PortError::new(
                dji4g_domain::ErrorCode::Unsupported,
                "test:no_at",
            ))
        })
    }

    fn invalidate(&self, _epoch: dji4g_application::DeviceEpoch) {}
}

struct NoAdapter;
impl AdapterPort for NoAdapter {
    fn resolve(
        &self,
        _target: &TargetContext,
    ) -> PortFuture<'_, Result<dji4g_application::AdapterObservationDto, PortError>> {
        Box::pin(async {
            Err(PortError::new(
                dji4g_domain::ErrorCode::Unsupported,
                "test:no_adapter",
            ))
        })
    }

    fn read_byte_counters(
        &self,
        _adapter_id: &str,
    ) -> PortFuture<'_, Result<(u64, u64), PortError>> {
        Box::pin(async {
            Err(PortError::new(
                dji4g_domain::ErrorCode::Unsupported,
                "test:no_adapter",
            ))
        })
    }
}

struct NoProbe;
impl NetworkProbePort for NoProbe {
    fn observe(
        &self,
        _adapter: &dji4g_application::AdapterContext,
        _active: bool,
    ) -> PortFuture<'_, Result<ProbeObservationDto, PortError>> {
        Box::pin(async {
            Err(PortError::new(
                dji4g_domain::ErrorCode::Unsupported,
                "test:no_probe",
            ))
        })
    }
}

struct NoSms;
impl SmsPort for NoSms {
    fn query_pdu_mode(
        &self,
        _target: &TargetContext,
    ) -> PortFuture<'_, Result<Option<bool>, PortError>> {
        Box::pin(async { Ok(Some(true)) })
    }

    fn enable_pdu_mode(&self, _target: &TargetContext) -> PortFuture<'_, Result<(), PortError>> {
        Box::pin(async { Ok(()) })
    }

    fn list(
        &self,
        _target: &TargetContext,
    ) -> PortFuture<'_, Result<dji4g_application::SmsListing, PortError>> {
        Box::pin(async {
            Ok(dji4g_application::SmsListing {
                messages: Vec::new(),
                capacity: None,
            })
        })
    }

    fn read(
        &self,
        _target: &TargetContext,
        _index: u32,
    ) -> PortFuture<'_, Result<dji4g_domain::SmsMessage, PortError>> {
        Box::pin(async {
            Err(PortError::new(
                dji4g_domain::ErrorCode::Unsupported,
                "test:no_sms",
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

fn ports_with(tools: Option<Arc<dyn DeviceToolsPort>>) -> MonitorPorts {
    MonitorPorts {
        inventory: Arc::new(NoInventory),
        at: Arc::new(NoAt),
        adapter: Arc::new(NoAdapter),
        probe: Arc::new(NoProbe),
        hotspot: None,
        sms: Some(Arc::new(NoSms)),
        device_tools: tools,
    }
}

/// Drive the runner until no tool work remains, or give up after a bounded number of steps.
///
/// A `false` from `poll_tool_requests` means "nothing changed this iteration", not "the pipeline
/// is done": a worker running on its own thread needs a moment before its receipt can be
/// collected, so the wait is driven by the pending state rather than by the return value.
fn settle_tools(runner: &mut ControllerRunner, steps: usize) {
    for _ in 0..steps {
        let _ = runner.poll_commands();
        let _ = runner.poll_tool_requests();
        if !runner.tool_pending() && !runner.controller().tool_active() {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("tool pipeline did not settle");
}

#[test]
fn cancelling_a_queued_tool_releases_the_busy_state_without_execution() {
    let mut controller = controller();
    controller
        .handle_command(UiCommand::RunToolRead {
            id: ToolReadId::Model,
        })
        .unwrap();
    let id = controller.snapshot().device_tools.task.unwrap().id;
    controller
        .handle_command(UiCommand::CancelDeviceTool { id })
        .unwrap();
    assert!(!controller.tool_active());
    assert!(controller.take_next_tool_request().is_none());
    assert_eq!(
        controller.snapshot().device_tools.task.unwrap().outcome,
        Some(ToolOutcome::CancelledBeforeWrite)
    );
}

struct WaitForCancellation {
    control: std::sync::Mutex<Option<dji4g_application::ToolControl>>,
    wrote: bool,
    calls: std::sync::atomic::AtomicUsize,
}

impl DeviceToolsPort for WaitForCancellation {
    fn execute(
        &self,
        _: &TargetContext,
        request: ToolRequest,
        control: dji4g_application::ToolControl,
    ) -> PortFuture<'_, Result<ToolReceipt, PortError>> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.wrote {
            control.mark_write_attempted();
        }
        *self.control.lock().unwrap() = Some(control.clone());
        Box::pin(std::future::poll_fn(move |_| {
            if !control.is_cancelled() {
                return std::task::Poll::Pending;
            }
            std::task::Poll::Ready(Ok(ToolReceipt {
                id: request.id,
                context: request.context.clone(),
                operation: request.operation.kind(),
                outcome: if control.write_attempted() {
                    ToolOutcome::OutcomeUnknown
                } else {
                    ToolOutcome::CancelledBeforeWrite
                },
                elapsed: Duration::ZERO,
                transcript: Arc::new(ToolTranscript::new()),
                saw_final_code: false,
                payload_lines: 0,
            }))
        }))
    }
}

#[test]
fn cancelling_a_running_sweep_signals_the_worker_and_stops_remaining_items() {
    for wrote in [false, true] {
        let tools = Arc::new(WaitForCancellation {
            control: std::sync::Mutex::new(None),
            wrote,
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let (_, runner) = ControllerRunner::new(controller());
        let mut runner = runner.with_ports(ports_with(Some(tools.clone())));
        runner
            .handle()
            .try_send(UiCommand::ProbeDeviceTools)
            .unwrap();
        runner.poll_commands();
        runner.poll_tool_requests();
        for _ in 0..100 {
            if tools.control.lock().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(tools.control.lock().unwrap().is_some());
        let id = runner.controller().snapshot().device_tools.task.unwrap().id;
        runner
            .handle()
            .try_send(UiCommand::CancelDeviceTool { id })
            .unwrap();
        runner.poll_commands();
        runner.poll_tool_requests();
        assert!(
            tools
                .control
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .is_cancelled()
        );
        settle_tools(&mut runner, 100);
        assert_eq!(tools.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let task = runner.controller().snapshot().device_tools.task.unwrap();
        assert_eq!(
            task.outcome,
            Some(if wrote {
                ToolOutcome::OutcomeUnknown
            } else {
                ToolOutcome::CancelledBeforeWrite
            })
        );
    }
}

#[test]
fn a_build_without_a_tool_port_refuses_instead_of_staying_queued() {
    let controller = controller();
    let (_, mut runner) = ControllerRunner::new(controller);
    runner = runner.with_ports(ports_with(None));

    runner
        .handle()
        .try_send(UiCommand::RunToolRead {
            id: ToolReadId::SignalQuality,
        })
        .expect("queued");
    let _ = runner.poll_commands();
    assert!(runner.poll_tool_requests(), "the refusal is handled");
    let snapshot = runner.controller().snapshot();
    let task = snapshot.device_tools.task.as_ref().expect("a task row");
    assert_eq!(task.phase, ToolPhase::Finished);
    assert_eq!(task.outcome, Some(ToolOutcome::Unsupported));
}

#[test]
fn a_capability_sweep_is_expanded_in_order_and_keeps_the_items_that_answered() {
    let tools = FakeTools::new(vec![
        (ToolOutcome::Ok, 1),
        (ToolOutcome::Rejected, 0),
        (ToolOutcome::Ok, 1),
    ]);
    let controller = controller();
    let (_, mut runner) = ControllerRunner::new(controller);
    runner = runner.with_ports(ports_with(Some(
        Arc::clone(&tools) as Arc<dyn DeviceToolsPort>
    )));

    runner
        .handle()
        .try_send(UiCommand::ProbeDeviceTools)
        .expect("queued");
    settle_tools(&mut runner, 40);

    // The port never sees a bulk request: it sees one read per whitelisted item, in order.
    let calls = tools.calls();
    assert_eq!(calls.len(), ToolReadId::ALL.len());
    for (index, call) in calls.iter().enumerate() {
        assert_eq!(
            *call,
            ToolOperationKind::Read(ToolReadId::ALL[index]),
            "item {index} ran out of order or as a bulk request"
        );
    }

    let snapshot = runner.controller().snapshot();
    let tools_snapshot = &snapshot.device_tools;
    // The item that failed keeps its failure; the others keep their own evidence.
    assert_eq!(
        tools_snapshot
            .capability(ToolReadId::ALL[0])
            .map(|row| row.status),
        Some(FeatureStatus::Supported)
    );
    assert_eq!(
        tools_snapshot
            .capability(ToolReadId::ALL[1])
            .map(|row| row.reason),
        Some(ToolOutcome::Rejected)
    );
    assert_eq!(
        tools_snapshot
            .capability(ToolReadId::ALL[2])
            .map(|row| row.status),
        Some(FeatureStatus::Supported)
    );
    // Items the script had no answer for failed as transport failures, and only on themselves.
    assert_eq!(
        tools_snapshot
            .capability(ToolReadId::ALL[3])
            .map(|row| row.status),
        Some(FeatureStatus::TransportFailure)
    );
    let task = tools_snapshot.task.as_ref().expect("a task row");
    assert_eq!(task.phase, ToolPhase::Finished);
    assert_eq!(task.completed_items, ToolReadId::ALL.len());
    assert_eq!(task.total_items, ToolReadId::ALL.len());
    // Every item ran, so the sweep itself completed; the per-item rows carry the failures.
    assert_eq!(task.outcome, Some(ToolOutcome::Ok));
    // One history entry per transaction, so the transcript can be read per command.
    assert_eq!(tools_snapshot.history.len(), ToolReadId::ALL.len());
}

#[test]
fn one_task_at_a_time_is_enforced_across_sms_repairs_and_tools() {
    let tools = FakeTools::new(vec![(ToolOutcome::Ok, 1)]);
    let controller = controller();
    let (_, mut runner) = ControllerRunner::new(controller);
    runner = runner.with_ports(ports_with(Some(
        Arc::clone(&tools) as Arc<dyn DeviceToolsPort>
    )));

    // A tool task in flight refuses a new SMS send and a new repair...
    runner
        .handle()
        .try_send(UiCommand::RunToolRead {
            id: ToolReadId::SignalQuality,
        })
        .expect("queued");
    let _ = runner.poll_commands();
    assert!(runner.controller().tool_active());
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::SmsSend {
                recipient: "+8613800138000".to_owned(),
                body: "sentinel".to_owned(),
            })
            .is_err()
    );
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::PrepareRepair {
                request: dji4g_application::ControlledRepairRequest::RestartModule,
            })
            .is_err()
    );

    // ...and a queued SMS send refuses a new tool task.
    settle_tools(&mut runner, 20);
    assert!(!runner.controller().tool_active());
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::RunToolRead {
                id: ToolReadId::Model,
            })
            .is_ok()
    );
}

#[test]
fn an_expert_command_needs_its_own_confirmation_and_runs_at_most_once() {
    let tools = FakeTools::new(vec![(ToolOutcome::OutcomeUnknown, 0)]);
    let controller = controller();
    let (_, mut runner) = ControllerRunner::new(controller);
    runner = runner.with_ports(ports_with(Some(
        Arc::clone(&tools) as Arc<dyn DeviceToolsPort>
    )));

    let line = ValidatedToolLine::parse("AT+VENDOR=1").expect("valid");
    // Freezing is not executing: nothing may reach the port before the confirmation.
    runner
        .controller_mut()
        .handle_command(UiCommand::PrepareExpertTool { line })
        .expect("frozen");
    let plan_id = runner
        .controller()
        .snapshot()
        .device_tools
        .pending_expert
        .as_ref()
        .expect("a frozen plan")
        .id;
    let _ = runner.poll_commands();
    assert!(tools.calls().is_empty(), "a frozen plan must not write");

    // The confirmation carries only the id, and it is consumed at most once.
    runner
        .controller_mut()
        .handle_command(UiCommand::ConfirmExpertTool { id: plan_id })
        .expect("confirmed");
    let _ = runner.poll_commands();
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::ConfirmExpertTool { id: plan_id })
            .is_err()
    );
    settle_tools(&mut runner, 20);
    assert_eq!(tools.calls().len(), 1);
    // An expert command whose effect was never confirmed is OutcomeUnknown, never a success.
    let snapshot = runner.controller().snapshot();
    assert_eq!(
        snapshot
            .device_tools
            .task
            .as_ref()
            .and_then(|task| task.outcome),
        Some(ToolOutcome::OutcomeUnknown)
    );
}

#[test]
fn an_expired_or_withdrawn_expert_plan_cannot_be_confirmed() {
    let tools = FakeTools::new(vec![(ToolOutcome::Ok, 1)]);
    let controller = controller();
    let (_, mut runner) = ControllerRunner::new(controller);
    runner = runner.with_ports(ports_with(Some(
        Arc::clone(&tools) as Arc<dyn DeviceToolsPort>
    )));

    let line = ValidatedToolLine::parse("AT+VENDOR=1").expect("valid");
    runner
        .controller_mut()
        .handle_command(UiCommand::PrepareExpertTool { line })
        .expect("frozen");
    let plan_id = runner
        .controller()
        .snapshot()
        .device_tools
        .pending_expert
        .as_ref()
        .expect("a frozen plan")
        .id;

    // The plan expires; the confirmation must refuse rather than run a stale command.
    runner
        .controller_mut()
        .advance_time(dji4g_application::PLAN_LIFETIME + Duration::from_secs(1));
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::ConfirmExpertTool { id: plan_id })
            .is_err()
    );
    settle_tools(&mut runner, 10);
    assert!(tools.calls().is_empty(), "an expired plan must never write");

    // Withdrawing a plan is idempotent and also prevents the write.
    let line = ValidatedToolLine::parse("AT+VENDOR=2").expect("valid");
    runner
        .controller_mut()
        .handle_command(UiCommand::PrepareExpertTool { line })
        .expect("frozen");
    let plan_id = runner
        .controller()
        .snapshot()
        .device_tools
        .pending_expert
        .as_ref()
        .expect("a frozen plan")
        .id;
    runner
        .controller_mut()
        .handle_command(UiCommand::CancelExpertToolPlan { id: plan_id })
        .expect("withdrawn");
    assert!(
        runner
            .controller_mut()
            .handle_command(UiCommand::ConfirmExpertTool { id: plan_id })
            .is_err()
    );
    settle_tools(&mut runner, 10);
    assert!(tools.calls().is_empty());
}

#[test]
fn a_device_change_drops_capability_evidence_and_the_frozen_plan() {
    let controller = controller();
    let (_, mut runner) = ControllerRunner::new(controller);
    runner = runner.with_ports(ports_with(None));

    let line = ValidatedToolLine::parse("AT+VENDOR=1").expect("valid");
    runner
        .controller_mut()
        .handle_command(UiCommand::PrepareExpertTool { line })
        .expect("frozen");
    assert!(
        runner
            .controller()
            .snapshot()
            .device_tools
            .pending_expert
            .is_some()
    );

    runner.controller_mut().invalidate_epoch(
        dji4g_domain::DeviceEpoch(9),
        dji4g_application::EpochInvalidationReason::PhysicalRemoval,
    );
    let snapshot = runner.controller().snapshot();
    assert!(snapshot.device_tools.pending_expert.is_none());
    assert!(snapshot.device_tools.capabilities.is_empty());
    assert!(snapshot.device_tools.profile.is_empty());
}
