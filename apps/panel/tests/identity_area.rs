//! Integration guards for the 「身份信息」 area and the optional-feature status notes.
//!
//! These tests drive the public overview view-model builder with real snapshots plus the
//! correlated optional-probe view, pinning the masked defaults, the honest empty/failed wording,
//! and the rule that an uncorrelated probe record can never annotate the rows.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use dji4g_application::{
    CommandStateSnapshot, ControllerSnapshot, DiagnosticSet, SettingsSnapshot,
};
use dji4g_domain::{
    AppSnapshot, AttachState, Availability, CellularSnapshot, DeviceEpoch, FeatureStatus,
    Freshness, HotspotStatus, NumberLookup, PhoneNumber, RegistrationState, SimIdentity, SimState,
};
use dji4g_panel::feature_probe::FeatureProbeView;
use dji4g_panel::localization::{Language, TextKey, feature_status_note, template};
use dji4g_panel::ui::overview_vm_with_probes;

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn identity() -> SimIdentity {
    SimIdentity {
        iccid_masked: "8986…2345".to_owned(),
        fingerprint: [9; 8],
    }
}

fn snapshot(
    numbers: Option<NumberLookup>,
    sim_identity: Option<SimIdentity>,
) -> ControllerSnapshot {
    ControllerSnapshot {
        publication_revision: 7,
        app: Arc::new(AppSnapshot {
            revision: 7,
            observed_at: now(),
            freshness: Freshness::Fresh,
            availability: Availability::Available,
            hotspot: HotspotStatus::Off,
            device: None,
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
                sim_identity,
                numbers,
                temperature_celsius: None,
                temperature_status: FeatureStatus::NotProbed,
            }),
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

fn captured_view(numbers_status: FeatureStatus, iccid_status: FeatureStatus) -> FeatureProbeView {
    FeatureProbeView {
        captured: true,
        numbers_status,
        iccid_status,
        serving_cell_status: FeatureStatus::NotProbed,
        iccid_full: Some("89860123456789012345".to_owned()),
        serving_cell_raw: None,
        temperature_sensors: Vec::new(),
        temperature_raw: None,
    }
}

#[test]
fn reported_numbers_render_masked_without_annotations() {
    let snapshot = snapshot(
        Some(NumberLookup::Reported(vec![PhoneNumber::new(
            "+8613800138000",
            145,
        )])),
        Some(identity()),
    );
    let vm = overview_vm_with_probes(&snapshot, Language::ZhCn, None);
    assert_eq!(vm.identity.numbers, vec!["****8000"]);
    assert!(vm.identity.number_absent.is_none());
    assert!(vm.identity.number_note.is_none());
    assert_eq!(vm.identity.iccid_masked.as_deref(), Some("8986…2345"));
    // Without a correlated probe record the ICCID plaintext stays withheld.
    assert!(vm.identity.iccid_reveal_full.is_none());
}

#[test]
fn empty_cnum_ok_reads_as_not_provided_not_as_a_fault() {
    let snapshot = snapshot(Some(NumberLookup::Empty), None);
    let vm = overview_vm_with_probes(&snapshot, Language::ZhCn, None);
    let absent = vm
        .identity
        .number_absent
        .expect("empty reply is still explained");
    assert_eq!(
        absent.text,
        template(Language::ZhCn, TextKey::ValueNumberNotProvided)
    );
    assert_eq!(
        absent.text, "SIM/设备未提供本机号码",
        "the exact acceptance wording for an empty OK"
    );
}

#[test]
fn classified_failures_annotate_only_correlated_rows() {
    // A correlated record explains the failure categories the row itself cannot.
    let snapshot = snapshot(None, Some(identity()));
    let vm = overview_vm_with_probes(
        &snapshot,
        Language::ZhCn,
        Some(&captured_view(
            FeatureStatus::TransportFailure,
            FeatureStatus::UnsupportedConfirmed,
        )),
    );
    assert_eq!(
        vm.identity
            .number_absent
            .as_ref()
            .map(|value| value.text.as_str()),
        Some("未读取到本机号码"),
        "a failed read is never presented as an empty number"
    );
    let number_note = vm.identity.number_note.expect("timeout needs prose");
    assert!(number_note.text.contains("本次超时"));
    let iccid_note = vm.identity.iccid_note.expect("unsupported needs prose");
    assert!(iccid_note.text.contains("固件不支持"));
    assert_eq!(
        vm.identity.iccid_reveal_full.as_deref(),
        Some("89860123456789012345"),
        "correlated plaintext is handed out for an explicit user click"
    );
}

#[test]
fn uncorrelated_probe_record_cannot_annotate_these_rows() {
    let snapshot = snapshot(None, Some(identity()));
    let mut uncorrelated = captured_view(
        FeatureStatus::UnsupportedConfirmed,
        FeatureStatus::TransportFailure,
    );
    uncorrelated.captured = false;
    let vm = overview_vm_with_probes(&snapshot, Language::ZhCn, Some(&uncorrelated));
    assert!(vm.identity.number_note.is_none());
    assert!(vm.identity.iccid_note.is_none());
    assert!(
        vm.identity.iccid_reveal_full.is_none(),
        "plaintext must never leave through an uncorrelated record"
    );
}

#[test]
fn feature_status_note_covers_the_closed_categories() {
    assert_eq!(feature_status_note(FeatureStatus::Supported), None);
    assert_eq!(feature_status_note(FeatureStatus::Empty), None);
    assert_eq!(feature_status_note(FeatureStatus::NotProbed), None);
    assert_eq!(
        feature_status_note(FeatureStatus::UnsupportedConfirmed),
        Some(TextKey::FeatureStatusUnsupportedConfirmed)
    );
    assert_eq!(
        feature_status_note(FeatureStatus::FormatMismatch),
        Some(TextKey::FeatureStatusFormatMismatch)
    );
    assert_eq!(
        feature_status_note(FeatureStatus::TransportFailure),
        Some(TextKey::FeatureStatusTransportFailure)
    );
    assert_eq!(
        feature_status_note(FeatureStatus::TemporarilyUnavailable),
        Some(TextKey::FeatureStatusTemporarilyUnavailable)
    );
    for key in [
        TextKey::ValueNumberNotProvided,
        TextKey::ValueIccidNotRead,
        TextKey::FeatureStatusUnsupportedConfirmed,
        TextKey::FeatureStatusFormatMismatch,
        TextKey::FeatureStatusTransportFailure,
        TextKey::FeatureStatusTemporarilyUnavailable,
        TextKey::ServingCellLayoutProvisional,
    ] {
        let text = template(Language::ZhCn, key);
        assert!(!text.trim().is_empty());
        assert!(
            text.chars()
                .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
        );
    }
}
