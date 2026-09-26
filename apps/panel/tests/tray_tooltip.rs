//! The tray tooltip is the only always-visible surface while the window hides to the tray, so it
//! must track the availability verdict: pushed once per change, never spammed per snapshot.

use std::{sync::Arc, time::SystemTime};

use dji4g_application::{ControllerSnapshot, DeviceEpoch, DiagnosticSet, UiCommand, UiSendError};
use dji4g_domain::{AppSnapshot, Availability, Freshness, HotspotStatus, LimitedReason};
use dji4g_panel::app::{PanelApp, PanelInputs, UiCommandSink};
use dji4g_panel::tray::{MemoryTrayBackend, TrayController, TrayLabels};
use eframe::egui;

const NOW: SystemTime = SystemTime::UNIX_EPOCH;

struct NoopSink;

impl UiCommandSink for NoopSink {
    fn try_send(&self, _command: UiCommand) -> Result<(), UiSendError> {
        Ok(())
    }
}

fn snapshot(availability: Availability, freshness: Freshness) -> ControllerSnapshot {
    ControllerSnapshot {
        module_network_check: None,
        host_network: dji4g_application::HostNetworkSnapshot::default(),
        publication_revision: 1,
        app: Arc::new(AppSnapshot {
            revision: 1,
            observed_at: NOW,
            freshness,
            availability,
            hotspot: HotspotStatus::Off,
            device: None,
            cellular: None,
            network: None,
            active_operation: None,
            issues: Vec::new(),
        }),
        diagnostics: DiagnosticSet::new(DeviceEpoch(1)),
        prepared_action: None,
        operation: None,
        settings: Default::default(),
        command_state: Default::default(),
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

fn harness() -> (
    PanelApp,
    dji4g_application::sync::watch::Sender<Arc<ControllerSnapshot>>,
    egui::Context,
) {
    let (snapshot_tx, snapshot_rx) = dji4g_application::sync::watch::channel(Arc::new(snapshot(
        Availability::Detecting,
        Freshness::Unknown,
    )));
    let tray = TrayController::initialize(MemoryTrayBackend::default(), TrayLabels::zh_cn())
        .expect("memory tray initialises");
    let mut app = PanelApp::headless(PanelInputs::new(
        snapshot_rx,
        Arc::new(NoopSink),
        None,
        None,
    ));
    app.attach_tray(tray);
    (app, snapshot_tx, egui::Context::default())
}

#[test]
fn the_tray_tooltip_tracks_availability_and_deduplicates() {
    let (mut app, snapshot_tx, ctx) = harness();
    assert!(app.last_tray_tooltip().is_none());

    // First change pushes the current verdict once.
    snapshot_tx
        .send(Arc::new(snapshot(
            Availability::Detecting,
            Freshness::Unknown,
        )))
        .expect("publish detecting again");
    app.receive_latest_nonblocking(&ctx);
    let detecting = app
        .last_tray_tooltip()
        .expect("the verdict must push once")
        .to_owned();
    assert!(
        detecting.contains("正在检测") || detecting.contains("检测中"),
        "{detecting}"
    );

    // The same verdict again: the text must not churn (set_tooltip is deduped by content).
    snapshot_tx
        .send(Arc::new(snapshot(
            Availability::Detecting,
            Freshness::Unknown,
        )))
        .expect("publish detecting a third time");
    app.receive_latest_nonblocking(&ctx);
    assert_eq!(app.last_tray_tooltip(), Some(detecting).as_deref());

    // Detecting -> Limited: exactly one push, with the short verdict appended to the base label.
    snapshot_tx
        .send(Arc::new(snapshot(
            Availability::Limited(LimitedReason::AtControlUnavailable),
            Freshness::Fresh,
        )))
        .expect("publish limited");
    app.receive_latest_nonblocking(&ctx);
    let tooltip = app
        .last_tray_tooltip()
        .expect("limited verdict must push")
        .to_owned();
    assert!(tooltip.starts_with("DJI 一代 4G 面板："), "{tooltip}");
    assert!(tooltip.contains("受限"), "{tooltip}");
    assert!(
        tooltip.chars().count() <= 127,
        "tray tooltips cap at 127 UTF-16 units"
    );

    // Limited -> Available: one more push with the new verdict.
    snapshot_tx
        .send(Arc::new(snapshot(
            Availability::Available,
            Freshness::Fresh,
        )))
        .expect("publish available");
    app.receive_latest_nonblocking(&ctx);
    let tooltip = app
        .last_tray_tooltip()
        .expect("available verdict must push")
        .to_owned();
    assert!(tooltip.contains("可用"), "{tooltip}");
    assert!(!tooltip.contains("受限"), "{tooltip}");
}
