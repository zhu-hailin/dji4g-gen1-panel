//! End-to-end diagnostic export through the production write path.
//!
//! The tests mirror `logging_privacy.rs`: every exported byte is checked for the redaction
//! invariants, and the atomic replace contract is exercised against a temp profile root built
//! with `ConfigPaths::under_root`.

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use dji4g_application::{
    CommandStateSnapshot, ControllerSnapshot, DeviceEpoch, DiagnosticCheckId, SettingsSnapshot,
    UiCommand, UiSendError,
};
use dji4g_domain::{
    AdapterState, AppSnapshot, AttachState, Availability, BoundDnsStatus, BoundPublicStatus,
    CellularSnapshot, DefaultRouteOwner, DeviceSnapshot, FeatureStatus, Freshness, HotspotStatus,
    NetworkSnapshot, ProtocolCoverage, RegistrationState, SimState, SmsEncoding, SmsInboxSummary,
    SmsMessage, SmsStatus, SmsStorageId, StableDeviceIdentity,
};
use dji4g_panel::app::{PanelApp, UiCommandSink};
use dji4g_panel::config::ConfigPaths;
use dji4g_panel::diagnostics_export::{
    REDACTED_APN, REDACTED_DEVICE_ID, REPORT_FILE_NAME, REPORT_JSON_FILE_NAME, build,
    check_id_name, write_export,
};
use dji4g_panel::localization::{Language, LocalizedText, TextKey};

const CONTAINER_ID: &str = "container-9f3a1c7e5b2d";
const DEVICE_INSTANCE_ID: &str = r"USB\VID_2CA3&PID_4006\5&1a2b3c4d&0&2";
const APN: &str = "internet.carrier-apn.example";

struct RecordingSink {
    forwarded: Mutex<Vec<UiCommand>>,
}

impl UiCommandSink for RecordingSink {
    fn try_send(&self, command: UiCommand) -> Result<(), UiSendError> {
        self.forwarded.lock().expect("sink lock").push(command);
        Ok(())
    }
}

fn temp_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("dji4g-panel-export-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create test root");
    root
}

fn fixture() -> Arc<ControllerSnapshot> {
    Arc::new(ControllerSnapshot {
        module_network_check: None,
        host_network: dji4g_application::HostNetworkSnapshot::default(),
        publication_revision: 7,
        app: Arc::new(AppSnapshot {
            revision: 7,
            observed_at: UNIX_EPOCH + Duration::from_secs(1_699_999_970),
            freshness: Freshness::Fresh,
            availability: Availability::Available,
            hotspot: HotspotStatus::Off,
            device: Some(DeviceSnapshot {
                epoch: DeviceEpoch(3),
                identity: StableDeviceIdentity {
                    container_id: CONTAINER_ID.to_owned(),
                    device_instance_id: DEVICE_INSTANCE_ID.to_owned(),
                    vid: 0x2CA3,
                    pid: 0x4006,
                },
                problem_code: Some(28),
                at_port: Some("COM3".to_owned()),
                adapter_id: Some("device-adapter-internal-id".to_owned()),
            }),
            cellular: Some(CellularSnapshot {
                sim: SimState::Ready,
                registration: RegistrationState::RegisteredHome,
                attached: AttachState::Attached,
                carrier: Some("中国移动".to_owned()),
                radio_access_technology: Some("LTE".to_owned()),
                signal_rssi_dbm: Some(-71),
                apn: Some(APN.to_owned()),
                pdp_address: Some("10.11.12.13".to_owned()),
                firmware: None,
                pdp_state: Some("active".to_owned()),
                serving_cell: None,
                sim_identity: None,
                numbers: None,
                temperature_celsius: None,
                temperature_status: dji4g_domain::FeatureStatus::NotProbed,
            }),
            network: Some(NetworkSnapshot {
                adapter_id: "network-adapter-internal-id".to_owned(),
                addresses: vec!["192.168.42.11".to_owned()],
                gateways: vec!["192.168.42.1".to_owned()],
                dns_servers: vec!["192.168.42.1".to_owned()],
                adapter_state: AdapterState::UsableAddressAndRoute,
                bound_public: BoundPublicStatus::Succeeded,
                bound_dns: BoundDnsStatus::Succeeded,
                protocol_coverage: ProtocolCoverage::AllRequiredFamilies,
                system_default_route: DefaultRouteOwner::TargetAdapter,
                down_bytes_per_sec: None,
                up_bytes_per_sec: None,
            }),
            active_operation: None,
            issues: Vec::new(),
        }),
        diagnostics: dji4g_application::DiagnosticSet::new(DeviceEpoch(3)),
        prepared_action: None,
        operation: None,
        settings: SettingsSnapshot::default(),
        command_state: CommandStateSnapshot::default(),
        action_readiness: Vec::new(),
        feedback: None,
        sim_epoch: 0,
        feature_status: None,
        adapter_metrics: None,
        timeline: Default::default(),
        sms_inbox: Default::default(),
        sms_messages: Vec::new(),
        sms_send: None,
        sms_delete: None,
        serial_work_busy: false,
        sms_refresh_pending: false,
        sms_read_phase: None,
        sms_read_progress: 0,
        sms_read_report: None,
        sms_inbox_failure: None,
        device_tools: Default::default(),
    })
}

#[test]
fn export_writes_both_files_with_all_checks_and_redacted_content() {
    let root = temp_root("write");
    let paths = ConfigPaths::under_root(&root);
    let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);

    write_export(&paths.exports_dir, &fixture(), now).expect("export writes");

    let human_path = paths.exports_dir.join(REPORT_FILE_NAME);
    let json_path = paths.exports_dir.join(REPORT_JSON_FILE_NAME);
    assert!(human_path.exists());
    assert!(json_path.exists());
    let human = fs::read_to_string(&human_path).expect("human report is utf-8");
    let json = fs::read_to_string(&json_path).expect("json report is utf-8");
    assert!(!human.is_empty());
    assert!(!json.is_empty());

    for id in DiagnosticCheckId::ORDERED {
        assert!(
            human.contains(check_id_name(id)),
            "human report is missing the id for {id:?}"
        );
    }
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("json report parses");
    assert_eq!(parsed["checks"].as_array().expect("checks array").len(), 9);

    for document in [&human, &json] {
        assert!(!document.contains(CONTAINER_ID));
        assert!(!document.contains(DEVICE_INSTANCE_ID));
        assert!(!document.contains(APN));
        assert!(document.contains(REDACTED_DEVICE_ID));
        assert!(document.contains(REDACTED_APN));
    }
}

#[test]
fn re_export_atomically_replaces_the_previous_report_without_temporary_leftovers() {
    let root = temp_root("overwrite");
    let paths = ConfigPaths::under_root(&root);
    let snapshot = fixture();
    let first = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let second = first + Duration::from_secs(60);

    write_export(&paths.exports_dir, &snapshot, first).expect("first export");
    write_export(&paths.exports_dir, &snapshot, second).expect("second export");

    // The canonical file holds exactly the newest document; nothing was appended and no
    // temporary sibling survived the replaces.
    let human = fs::read_to_string(paths.exports_dir.join(REPORT_FILE_NAME)).unwrap();
    assert_eq!(human, build(&snapshot, second).human);
    assert!(human.contains("2023-11-14T22:14:20Z"));
    let entries = fs::read_dir(&paths.exports_dir)
        .expect("exports dir")
        .count();
    assert_eq!(entries, 2);
}

#[test]
fn panel_app_exports_ui_side_and_never_forwards_the_command_to_the_controller() {
    let root = temp_root("panel");
    let paths = ConfigPaths::under_root(&root);
    let sink = Arc::new(RecordingSink {
        forwarded: Mutex::new(Vec::new()),
    });
    let mut app = PanelApp::from_snapshot(fixture(), Arc::clone(&sink) as Arc<dyn UiCommandSink>);

    // Without a resolved export directory the failure is the stable path code, surfaced as the
    // closed zh-CN toast text.
    app.export_diagnostics(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
    let expected_failure =
        LocalizedText::new(Language::ZhCn, TextKey::DiagnosticsExportFailed).text;
    assert_eq!(app.toast_text().map(str::to_owned), Some(expected_failure));

    // The successful path writes both files and shows the fixed, truthful location text.
    app.set_exports_dir(Some(paths.exports_dir.clone()));
    app.export_diagnostics(UNIX_EPOCH + Duration::from_secs(1_700_000_060));
    let expected_success =
        LocalizedText::new(Language::ZhCn, TextKey::DiagnosticsExportSuccess).text;
    assert_eq!(app.toast_text().map(str::to_owned), Some(expected_success));
    assert!(paths.exports_dir.join(REPORT_FILE_NAME).exists());
    assert!(paths.exports_dir.join(REPORT_JSON_FILE_NAME).exists());

    // The same request sent through the normal command path is intercepted before dispatch:
    // the controller sink must not see it at all.
    app.send(UiCommand::ExportDiagnostics);
    let forwarded = sink.forwarded.lock().expect("sink lock");
    assert!(
        !forwarded
            .iter()
            .any(|command| matches!(command, UiCommand::ExportDiagnostics)),
        "ExportDiagnostics must never reach the controller queue"
    );
    assert!(paths.exports_dir.join(REPORT_FILE_NAME).exists());
}

#[test]
fn exported_sms_section_is_counts_only_and_never_carries_message_content() {
    let root = temp_root("sms");
    let paths = ConfigPaths::under_root(&root);
    let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let mut snapshot = (*fixture()).clone();
    snapshot.sms_inbox = SmsInboxSummary {
        message_count: 3,
        unread_count: 2,
        capacity: Some((3, 30)),
        status: FeatureStatus::Supported,
        has_incomplete: true,
        evicted: 0,
    };

    write_export(&paths.exports_dir, &snapshot, now).expect("export writes");

    let human = fs::read_to_string(paths.exports_dir.join(REPORT_FILE_NAME)).expect("human");
    let json = fs::read_to_string(paths.exports_dir.join(REPORT_JSON_FILE_NAME)).expect("json");
    assert!(human.contains("消息数：3"));
    assert!(human.contains("未读数：2"));
    assert!(human.contains("已用 3 / 总数 30"));
    assert!(human.contains("存在未完整接收的长短信"));
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("json parses");
    assert_eq!(parsed["sms"]["message_count"], 3);
    assert_eq!(parsed["sms"]["unread_count"], 2);
    assert_eq!(parsed["sms"]["status"], "supported");
    for document in [&human, &json] {
        assert!(
            !document.contains("\"sender\""),
            "message sender field must never be exported"
        );
        assert!(
            !document.contains("\"body\""),
            "message body field must never be exported"
        );
    }
}

#[test]
fn serialized_sms_messages_redact_sender_and_body_even_outside_the_export() {
    // Defence in depth: even if a future caller serializes a stored message anyway, the domain
    // type emits markers instead of content.
    let message = SmsMessage::new(
        1,
        SmsStorageId("SM".to_owned()),
        1,
        0,
        "+8613800138000",
        "private message body",
        SmsEncoding::Gsm7,
        SmsStatus::Received,
    );
    let json = serde_json::to_string(&message).expect("message serializes");
    assert!(!json.contains("+8613800138000"), "sender leaked");
    assert!(!json.contains("private message body"), "body leaked");
    assert!(json.contains("[REDACTED]"));
}
