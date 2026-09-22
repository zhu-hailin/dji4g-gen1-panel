//! Panel-owned settings side effects: the snapshot-drain path must perform at most one
//! `config.toml` write and one autostart registration per settings revision, and report their
//! terminal results back as closed `UiCommand` values instead of claiming success.

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use dji4g_application::{
    AutostartApplyOutcome, AutostartKnownState, AutostartStatus, ControllerSnapshot, DiagnosticSet,
    LanguageCode, LogLevel, SettingsPersistenceState, SettingsSaveOutcome, SettingsSnapshot,
    UiCommand, UiSendError as ApplicationUiSendError,
};
use dji4g_domain::{AppSnapshot, Availability, DeviceEpoch, Freshness, HotspotStatus};
use dji4g_panel::app::{PanelApp, PanelInputs, SettingsBackend, UiCommandSink};
use dji4g_panel::config::{ConfigError, ConfigV1};
use dji4g_windows_platform::{AutostartObservedState, PlatformError};

fn initial_snapshot() -> Arc<ControllerSnapshot> {
    Arc::new(ControllerSnapshot {
        publication_revision: 0,
        app: Arc::new(AppSnapshot {
            revision: 0,
            observed_at: SystemTime::UNIX_EPOCH,
            freshness: Freshness::Unknown,
            availability: Availability::Detecting,
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
        settings: SettingsSnapshot {
            revision: 0,
            autostart: AutostartStatus::Ready(AutostartKnownState::Disabled),
            ..SettingsSnapshot::default()
        },
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
        sms_inbox_failure: None,
        device_tools: Default::default(),
    })
}

fn saving_snapshot(revision: u64, desired_enabled: bool) -> Arc<ControllerSnapshot> {
    let mut snapshot = (*initial_snapshot()).clone();
    snapshot.settings = SettingsSnapshot {
        revision,
        autostart: AutostartStatus::Saving {
            desired_enabled,
            previous: Some(AutostartKnownState::Disabled),
        },
        start_minimized: true,
        active_probe: false,
        log_level: LogLevel::Debug,
        language: LanguageCode::ZhCn,
        persistence: SettingsPersistenceState::Saving,
    };
    Arc::new(snapshot)
}

#[derive(Default)]
struct BackendCalls {
    saves: Mutex<Vec<ConfigV1>>,
    toggles: Mutex<Vec<bool>>,
}

struct RecordingBackend {
    calls: Arc<BackendCalls>,
    save_result: Result<(), ConfigError>,
    toggle_result: Result<AutostartObservedState, PlatformError>,
}

impl SettingsBackend for RecordingBackend {
    fn save_config(&self, config: &ConfigV1) -> Result<(), ConfigError> {
        self.calls
            .saves
            .lock()
            .expect("save call lock")
            .push(config.clone());
        self.save_result.clone()
    }

    fn set_autostart(&self, enabled: bool) -> Result<AutostartObservedState, PlatformError> {
        self.calls
            .toggles
            .lock()
            .expect("toggle call lock")
            .push(enabled);
        self.toggle_result.clone()
    }
}

#[derive(Default)]
struct RecordingSink {
    commands: Mutex<Vec<UiCommand>>,
}

impl UiCommandSink for RecordingSink {
    fn try_send(&self, command: UiCommand) -> Result<(), ApplicationUiSendError> {
        self.commands
            .lock()
            .expect("sink command lock")
            .push(command);
        Ok(())
    }
}

struct Harness {
    app: PanelApp,
    snapshot_tx: dji4g_application::sync::watch::Sender<Arc<ControllerSnapshot>>,
    sink: Arc<RecordingSink>,
    calls: Arc<BackendCalls>,
    context: eframe::egui::Context,
}

fn harness(
    save_result: Result<(), ConfigError>,
    toggle_result: Result<AutostartObservedState, PlatformError>,
) -> Harness {
    let (snapshot_tx, snapshot_rx) = dji4g_application::sync::watch::channel(initial_snapshot());
    let sink = Arc::new(RecordingSink::default());
    let calls = Arc::new(BackendCalls::default());
    let backend = RecordingBackend {
        calls: Arc::clone(&calls),
        save_result,
        toggle_result,
    };
    let app = PanelApp::headless(PanelInputs::new(
        snapshot_rx,
        sink.clone() as Arc<dyn UiCommandSink>,
        None,
        Some(Arc::new(backend)),
    ));
    Harness {
        app,
        snapshot_tx,
        sink,
        calls,
        context: eframe::egui::Context::default(),
    }
}

fn recorded_commands(sink: &RecordingSink) -> Vec<UiCommand> {
    sink.commands.lock().expect("sink command lock").clone()
}

fn save_outcomes(commands: &[UiCommand]) -> Vec<SettingsSaveOutcome> {
    commands
        .iter()
        .filter_map(|command| match command {
            UiCommand::SettingsPersisted(outcome) => Some(outcome.clone()),
            _ => None,
        })
        .collect()
}

fn autostart_outcomes(commands: &[UiCommand]) -> Vec<AutostartApplyOutcome> {
    commands
        .iter()
        .filter_map(|command| match command {
            UiCommand::AutostartApplied(outcome) => Some(outcome.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_pending_toggle_drives_one_save_and_one_registration_per_revision() {
    let mut harness = harness(Ok(()), Ok(AutostartObservedState::Enabled));

    harness
        .snapshot_tx
        .send(saving_snapshot(1, true))
        .expect("publish saving snapshot");
    harness.app.receive_latest_nonblocking(&harness.context);

    // All five fields come from the snapshot; the autostart intent comes from `Saving`.
    assert_eq!(
        harness
            .calls
            .saves
            .lock()
            .expect("save call lock")
            .as_slice(),
        &[ConfigV1 {
            onboarding_completed: false,
            language: LanguageCode::ZhCn,
            autostart: true,
            start_minimized: true,
            active_probe: false,
            log_level: LogLevel::Debug,
        }]
    );
    assert_eq!(
        harness
            .calls
            .toggles
            .lock()
            .expect("toggle call lock")
            .as_slice(),
        &[true]
    );
    let commands = recorded_commands(&harness.sink);
    let saves = save_outcomes(&commands);
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0].revision, 1);
    assert_eq!(saves[0].result, Ok(()));
    let applies = autostart_outcomes(&commands);
    assert_eq!(applies.len(), 1);
    assert_eq!(
        *applies.first().expect("one apply outcome"),
        AutostartApplyOutcome {
            desired_enabled: true,
            observed: Ok(AutostartKnownState::Enabled),
        }
    );

    // The same revision observed again (a publication-only change while the outcome is still in
    // flight) must not re-write the registry or the config file.
    harness
        .snapshot_tx
        .send(saving_snapshot(1, true))
        .expect("republish the same revision");
    harness.app.receive_latest_nonblocking(&harness.context);
    assert_eq!(harness.calls.saves.lock().expect("save call lock").len(), 1);
    assert_eq!(
        harness
            .calls
            .toggles
            .lock()
            .expect("toggle call lock")
            .len(),
        1
    );
    assert_eq!(recorded_commands(&harness.sink).len(), 2);
}

#[test]
fn ordinary_settings_updates_preserve_completed_onboarding() {
    let mut harness = harness(Ok(()), Ok(AutostartObservedState::Enabled));
    harness.app.configure_onboarding(&ConfigV1 {
        onboarding_completed: true,
        ..ConfigV1::default()
    });
    harness.snapshot_tx.send(saving_snapshot(1, true)).unwrap();
    harness.app.receive_latest_nonblocking(&harness.context);
    let saves = harness.calls.saves.lock().unwrap();
    assert_eq!(saves.len(), 1);
    assert!(saves[0].onboarding_completed);
    assert!(saves[0].autostart);
    assert!(!saves[0].active_probe);
}

#[test]
fn a_new_revision_with_the_same_desired_value_is_attempted_again() {
    let mut harness = harness(Ok(()), Ok(AutostartObservedState::Enabled));

    harness
        .snapshot_tx
        .send(saving_snapshot(1, true))
        .expect("publish first toggle");
    harness.app.receive_latest_nonblocking(&harness.context);
    harness
        .snapshot_tx
        .send(saving_snapshot(2, true))
        .expect("publish a second toggle of the same direction");
    harness.app.receive_latest_nonblocking(&harness.context);

    assert_eq!(harness.calls.saves.lock().expect("save call lock").len(), 2);
    assert_eq!(
        harness
            .calls
            .toggles
            .lock()
            .expect("toggle call lock")
            .len(),
        2
    );
}

#[test]
fn a_failed_config_write_reports_the_stable_code_once_and_is_not_retried() {
    let mut harness = harness(
        Err(ConfigError::with_os_code("config:write_failed", 5)),
        Ok(AutostartObservedState::Enabled),
    );

    harness
        .snapshot_tx
        .send(saving_snapshot(1, true))
        .expect("publish saving snapshot");
    harness.app.receive_latest_nonblocking(&harness.context);
    harness
        .snapshot_tx
        .send(saving_snapshot(1, true))
        .expect("republish the same revision");
    harness.app.receive_latest_nonblocking(&harness.context);

    assert_eq!(harness.calls.saves.lock().expect("save call lock").len(), 1);
    let commands = recorded_commands(&harness.sink);
    let saves = save_outcomes(&commands);
    assert_eq!(saves.len(), 1);
    let error = saves[0]
        .result
        .as_ref()
        .expect_err("the scripted save failed");
    assert_eq!(error.stable().as_str(), "config:write_failed");
    // The registry write is independent of the config write and still applies exactly once.
    assert_eq!(autostart_outcomes(&commands).len(), 1);
}

#[test]
fn a_failed_registration_reports_the_platform_code() {
    let mut harness = harness(
        Ok(()),
        Err(PlatformError {
            code: "autostart:registry_write_failed",
            os_code: Some(5),
        }),
    );

    harness
        .snapshot_tx
        .send(saving_snapshot(1, true))
        .expect("publish saving snapshot");
    harness.app.receive_latest_nonblocking(&harness.context);

    let applies = autostart_outcomes(&recorded_commands(&harness.sink));
    assert_eq!(applies.len(), 1);
    let error = applies[0].observed.as_ref().expect_err("scripted failure");
    assert_eq!(error.stable().as_str(), "autostart:registry_write_failed");
    assert_eq!(error.category(), dji4g_domain::ErrorCode::Internal);
}

#[test]
fn without_a_backend_pending_changes_fail_with_honest_stable_codes() {
    let (snapshot_tx, snapshot_rx) = dji4g_application::sync::watch::channel(initial_snapshot());
    let sink = Arc::new(RecordingSink::default());
    let mut app = PanelApp::headless(PanelInputs::new(
        snapshot_rx,
        sink.clone() as Arc<dyn UiCommandSink>,
        None,
        None,
    ));
    let context = eframe::egui::Context::default();

    snapshot_tx
        .send(saving_snapshot(1, true))
        .expect("publish saving snapshot");
    app.receive_latest_nonblocking(&context);

    let commands = recorded_commands(&sink);
    let saves = save_outcomes(&commands);
    assert_eq!(saves.len(), 1);
    assert_eq!(
        saves[0]
            .result
            .as_ref()
            .expect_err("no backend means no save")
            .stable()
            .as_str(),
        "config:path_unavailable"
    );
    let applies = autostart_outcomes(&commands);
    assert_eq!(applies.len(), 1);
    assert_eq!(
        applies[0]
            .observed
            .as_ref()
            .expect_err("no backend means no registration")
            .stable()
            .as_str(),
        "autostart:unsupported_platform"
    );

    // One honest failure per revision, not a retry storm.
    snapshot_tx
        .send(saving_snapshot(1, true))
        .expect("republish the same revision");
    app.receive_latest_nonblocking(&context);
    assert_eq!(recorded_commands(&sink).len(), 2);
}

#[test]
fn a_freshly_loaded_snapshot_is_never_written_back() {
    let mut harness = harness(Ok(()), Ok(AutostartObservedState::Enabled));

    // No snapshot change at all: nothing pending, nothing attempted.
    harness.app.receive_latest_nonblocking(&harness.context);
    assert!(
        harness
            .calls
            .saves
            .lock()
            .expect("save call lock")
            .is_empty()
    );
    assert!(
        harness
            .calls
            .toggles
            .lock()
            .expect("toggle call lock")
            .is_empty()
    );
    assert!(recorded_commands(&harness.sink).is_empty());

    // Republishing the identical startup snapshot must not persist it either: the initial
    // revision seeded the guard so a loaded config is never clobbered (in particular not a
    // config that just failed to parse).
    harness
        .snapshot_tx
        .send(initial_snapshot())
        .expect("republish startup snapshot");
    harness.app.receive_latest_nonblocking(&harness.context);
    assert!(
        harness
            .calls
            .saves
            .lock()
            .expect("save call lock")
            .is_empty()
    );
    assert!(recorded_commands(&harness.sink).is_empty());
}

#[test]
fn the_service_loop_never_blocks_the_drain_path() {
    // A smoke check that the drain keeps working across many publishes with pending settings.
    let mut harness = harness(Ok(()), Ok(AutostartObservedState::Enabled));
    for revision in 1..=8 {
        harness
            .snapshot_tx
            .send(saving_snapshot(revision, revision % 2 == 1))
            .expect("publish toggle");
        harness.app.receive_latest_nonblocking(&harness.context);
    }
    assert_eq!(harness.calls.saves.lock().expect("save call lock").len(), 8);
    assert_eq!(
        harness
            .calls
            .toggles
            .lock()
            .expect("toggle call lock")
            .len(),
        8
    );
    assert_eq!(
        save_outcomes(&recorded_commands(&harness.sink))
            .iter()
            .map(|outcome| outcome.revision)
            .collect::<Vec<_>>(),
        (1..=8).collect::<Vec<_>>()
    );
}
