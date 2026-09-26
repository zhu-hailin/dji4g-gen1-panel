//! Integration guards for the 温度 row and the 「模块温度」 area of the overview.
//!
//! These tests pin the evidence a reading travels with: the reported channel count in report
//! order, the raw `+QTEMP:` line the module actually sent, and the rule that none of it can
//! describe a snapshot the record does not belong to.  The real device answers `AT+QTEMP` with an
//! unnamed positional list (`+QTEMP: 57,51,51`), so that layout is the fixture throughout.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use dji4g_application::{
    CommandStateSnapshot, ControllerSnapshot, DiagnosticSet, SettingsSnapshot,
};
use dji4g_at_protocol::SensorTemperature;
use dji4g_domain::{
    AppSnapshot, AttachState, Availability, CellularSnapshot, DeviceEpoch, FeatureStatus,
    Freshness, HotspotStatus, RegistrationState, SimState,
};
use dji4g_panel::feature_probe::FeatureProbeView;
use dji4g_panel::localization::{Language, TextKey, template};
use dji4g_panel::ui::overview_vm_with_probes;

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn device() -> dji4g_domain::DeviceSnapshot {
    dji4g_domain::DeviceSnapshot {
        epoch: DeviceEpoch(1),
        identity: dji4g_domain::StableDeviceIdentity {
            container_id: "container".to_owned(),
            device_instance_id: "instance".to_owned(),
            vid: 0x2ca3,
            pid: 0x4006,
        },
        problem_code: None,
        at_port: Some("COM7".to_owned()),
        adapter_id: None,
    }
}

fn cell(temperature: Option<i16>, status: FeatureStatus) -> CellularSnapshot {
    CellularSnapshot {
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
        sim_identity: None,
        numbers: None,
        temperature_celsius: temperature,
        temperature_status: status,
    }
}

fn snapshot(cellular: CellularSnapshot) -> ControllerSnapshot {
    ControllerSnapshot {
        module_network_check: None,
        host_network: dji4g_application::HostNetworkSnapshot::default(),
        publication_revision: 7,
        app: Arc::new(AppSnapshot {
            revision: 7,
            observed_at: now(),
            freshness: Freshness::Fresh,
            availability: Availability::Available,
            hotspot: HotspotStatus::Off,
            device: Some(device()),
            cellular: Some(cellular),
            network: None,
            active_operation: None,
            issues: Vec::new(),
        }),
        diagnostics: DiagnosticSet::new(DeviceEpoch(1)),
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
    }
}

fn dji_readings() -> Vec<SensorTemperature> {
    vec![
        SensorTemperature {
            name: None,
            celsius: 57,
        },
        SensorTemperature {
            name: None,
            celsius: 51,
        },
        SensorTemperature {
            name: None,
            celsius: 51,
        },
    ]
}

fn probe_view(sensors: Vec<SensorTemperature>, raw: Option<String>) -> FeatureProbeView {
    FeatureProbeView {
        captured: true,
        numbers_status: FeatureStatus::NotProbed,
        iccid_status: FeatureStatus::NotProbed,
        serving_cell_status: FeatureStatus::NotProbed,
        iccid_full: None,
        serving_cell_raw: None,
        temperature_sensors: sensors,
        temperature_raw: raw,
    }
}

#[test]
fn the_positional_temperature_layout_reaches_the_row_with_its_evidence() {
    let snapshot = snapshot(cell(Some(57), FeatureStatus::Supported));
    let probes = probe_view(dji_readings(), Some("+QTEMP: 57,51,51".to_owned()));
    let vm = overview_vm_with_probes(&snapshot, Language::ZhCn, Some(&probes));
    assert_eq!(vm.temperature.text, "57 °C");
    assert_eq!(vm.temperature_celsius, Some(57));
    assert_eq!(
        vm.temperature_note.map(|note| note.text),
        Some(template(Language::ZhCn, TextKey::TemperatureSensorNote).to_owned())
    );
    assert_eq!(vm.temperature_raw.as_deref(), Some("+QTEMP: 57,51,51"));
    assert!(
        vm.temperature_sensors_note
            .is_some_and(|note| note.text.contains("57 / 51 / 51")),
        "the reported channels travel with the reading"
    );
}

#[test]
fn an_unreadable_answer_stays_visible_beside_its_classified_failure() {
    // The reported symptom: the device answered and the build could not read the layout.  The row
    // must show the failure category *and* the bytes, so the next round has evidence.
    let snapshot = snapshot(cell(None, FeatureStatus::FormatMismatch));
    let probes = probe_view(Vec::new(), Some("+QTEMP: \"modem\",\"pa\"".to_owned()));
    let vm = overview_vm_with_probes(&snapshot, Language::ZhCn, Some(&probes));
    assert_eq!(
        vm.temperature.text,
        template(Language::ZhCn, TextKey::TemperatureNotRead)
    );
    let note = vm
        .temperature_note
        .expect("a classified failure keeps its note");
    assert!(note.text.contains("格式不匹配"), "{}", note.text);
    assert_eq!(
        vm.temperature_raw.as_deref(),
        Some("+QTEMP: \"modem\",\"pa\"")
    );
    assert!(
        vm.temperature_sensors_note.is_none(),
        "no readings, no list"
    );
}

#[test]
fn an_uncorrelated_record_never_describes_the_temperature_rows() {
    // The value comes from the snapshot, the evidence from the probe record: the two must describe
    // the same observation, or nothing is annotated.
    let snapshot = snapshot(cell(Some(59), FeatureStatus::Supported));
    let probes = probe_view(dji_readings(), Some("+QTEMP: 57,51,51".to_owned()));
    let mut stale = probes.clone();
    stale.captured = false;
    let vm = overview_vm_with_probes(&snapshot, Language::ZhCn, Some(&stale));
    assert_eq!(vm.temperature.text, "59 °C");
    assert_eq!(vm.temperature_raw, None);
    assert_eq!(vm.temperature_sensors_note, None);
    // Without a probe record at all the row keeps its own reading and claims nothing more.
    let vm = overview_vm_with_probes(&snapshot, Language::ZhCn, None);
    assert_eq!(vm.temperature.text, "59 °C");
    assert_eq!(vm.temperature_raw, None);
    assert_eq!(vm.temperature_sensors_note, None);
    assert_eq!(vm.temperature_celsius, Some(59));
}

#[test]
fn no_cellular_evidence_stays_未获取_instead_of_a_reading() {
    let mut snapshot = snapshot(cell(None, FeatureStatus::NotProbed));
    let mut app = (*snapshot.app).clone();
    app.cellular = None;
    snapshot.app = Arc::new(app);
    let vm = overview_vm_with_probes(&snapshot, Language::ZhCn, None);
    assert_eq!(
        vm.temperature.text,
        template(Language::ZhCn, TextKey::ValueNotAvailable)
    );
    assert_eq!(vm.temperature_celsius, None);
    assert!(vm.temperature_note.is_none());
    assert!(vm.temperature_raw.is_none());
}
