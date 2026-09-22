use std::fs;

use dji4g_application::{
    AutostartKnownState, AutostartStatus, FailureCode, LanguageCode, LogLevel, SettingsSnapshot,
    StableCode,
};
use dji4g_panel::config::{
    ConfigLoadOutcome, ConfigPaths, ConfigStore, ConfigV1, StartupOptions, StdFileOps, SystemClock,
};

fn temp_root(label: &str) -> std::path::PathBuf {
    let root =
        std::env::temp_dir().join(format!("dji4g-panel-task8-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create test root");
    root
}

#[test]
fn config_defaults_are_safe_and_explicitly_versioned() {
    let config = ConfigV1::default();
    assert_eq!(config.language, dji4g_application::LanguageCode::ZhCn);
    assert!(!config.autostart);
    assert!(!config.start_minimized);
    assert!(config.active_probe);
    assert_eq!(config.log_level, LogLevel::Info);

    let encoded = ConfigStore::<StdFileOps, SystemClock>::encode(&config).expect("encode defaults");
    assert!(encoded.contains("schema_version = 1"));
    assert!(encoded.contains("language = \"zh-CN\""));
    assert!(encoded.contains("log_level = \"info\""));
}

#[test]
fn config_roundtrip_preserves_all_user_fields() {
    let root = temp_root("roundtrip");
    let paths = ConfigPaths::under_root(&root);
    let store = ConfigStore::new(paths.clone());
    let expected = ConfigV1 {
        onboarding_completed: true,
        sms_archive_enabled: true,
        language: dji4g_application::LanguageCode::ZhCn,
        autostart: true,
        start_minimized: true,
        active_probe: false,
        log_level: LogLevel::Debug,
    };

    store.save(&expected).expect("save config");
    let ConfigLoadOutcome::Loaded { config } = store.load().expect("load config") else {
        panic!("expected a loaded config");
    };
    assert_eq!(config, expected);
    assert_eq!(
        fs::read_to_string(paths.config_file).unwrap(),
        ConfigStore::<StdFileOps, SystemClock>::encode(&expected).unwrap()
    );
}

#[test]
fn corrupt_config_is_backed_up_before_safe_default_restore() {
    let root = temp_root("corrupt");
    let paths = ConfigPaths::under_root(&root);
    fs::create_dir_all(paths.config_file.parent().unwrap()).unwrap();
    let corrupt = b"schema_version = 2\nlanguage = \"zh-CN\"\n";
    fs::write(&paths.config_file, corrupt).unwrap();

    let store = ConfigStore::new(paths.clone());
    let outcome = store.load().expect("preserve corrupt config");
    let ConfigLoadOutcome::CorruptPreserved { config, backup } = outcome else {
        panic!("expected corrupt preservation");
    };
    assert_eq!(config, ConfigV1::default());
    assert!(backup.exists());
    assert_eq!(fs::read(backup).unwrap(), corrupt);
    let restored = fs::read_to_string(paths.config_file).unwrap();
    assert!(restored.contains("schema_version = 1"));
    assert!(!restored.contains("schema_version = 2"));
}

#[test]
fn missing_environment_is_not_replaced_with_working_directory() {
    let paths = ConfigPaths::from_environment_values(None, None);
    assert!(paths.is_err());
    assert_eq!(paths.unwrap_err().stable_code(), "config:path_unavailable");
}

#[test]
fn autostart_flag_forces_start_to_tray_without_mutating_config_intent() {
    let options = StartupOptions::parse(["--autostart".to_owned()]).expect("parse args");
    assert!(options.autostart);
    assert!(options.start_to_tray(true));
    assert!(options.start_to_tray(false));

    let normal = StartupOptions::default();
    assert!(!normal.autostart);
    assert!(!normal.start_to_tray(false));
    assert!(normal.start_to_tray(true));
}

#[test]
fn the_restart_handoff_takes_a_real_pid_and_refuses_anything_else() {
    let options = StartupOptions::parse(["--restart-after=4321".to_owned()]).expect("parse args");
    assert_eq!(options.restart_after, Some(4321));
    assert!(!options.autostart);
    assert_eq!(options.demo, None);

    // A restart waits on a process. A missing, empty, negative, zero or out-of-range pid would
    // make it wait on the wrong thing (or on nothing), so it is refused rather than defaulted.
    for argument in [
        "--restart-after=",
        "--restart-after=0",
        "--restart-after=-1",
        "--restart-after=abc",
        "--restart-after=4294967296",
        "--restart-after=12x",
    ] {
        let parsed = StartupOptions::parse([argument.to_owned()]);
        assert!(
            parsed.is_err(),
            "{argument} must not be accepted as a handoff pid"
        );
        assert_eq!(
            parsed.expect_err("refused").stable_code(),
            "config:startup_invalid_pid"
        );
    }

    // A plain start has no handoff.
    assert_eq!(StartupOptions::default().restart_after, None);
}

fn saving_settings(desired_enabled: bool) -> SettingsSnapshot {
    SettingsSnapshot {
        revision: 7,
        language: LanguageCode::EnUs,
        autostart: AutostartStatus::Saving {
            desired_enabled,
            previous: Some(AutostartKnownState::Disabled),
        },
        start_minimized: true,
        active_probe: false,
        log_level: LogLevel::Debug,
        persistence: dji4g_application::SettingsPersistenceState::Saving,
    }
}

#[test]
fn snapshot_settings_map_to_every_config_field() {
    let config = ConfigV1::from_settings(&saving_settings(true)).expect("intent is derivable");
    assert_eq!(config.language, LanguageCode::EnUs);
    assert!(config.autostart);
    assert!(config.start_minimized);
    assert!(!config.active_probe);
    assert_eq!(config.log_level, LogLevel::Debug);

    let disabling = ConfigV1::from_settings(&saving_settings(false)).expect("intent is derivable");
    assert!(!disabling.autostart);

    let mut confirmed = saving_settings(true);
    confirmed.autostart = AutostartStatus::Ready(AutostartKnownState::Enabled);
    assert!(
        ConfigV1::from_settings(&confirmed)
            .expect("confirmed state is derivable")
            .autostart
    );
    confirmed.autostart = AutostartStatus::Ready(AutostartKnownState::Disabled);
    assert!(
        !ConfigV1::from_settings(&confirmed)
            .expect("confirmed state is derivable")
            .autostart
    );
}

#[test]
fn autostart_states_without_a_derivable_intent_are_never_persisted() {
    // Guessing a bool for these states could overwrite the stored intent and hide real drift.
    for autostart in [
        AutostartStatus::Loading,
        AutostartStatus::Ready(AutostartKnownState::Drift),
        AutostartStatus::Failed {
            code: FailureCode::new(
                dji4g_domain::ErrorCode::Internal,
                StableCode::try_from_static("autostart:registry_write_failed").expect("safe code"),
            ),
            previous: Some(AutostartKnownState::Disabled),
        },
    ] {
        let mut settings = saving_settings(true);
        settings.autostart = autostart;
        assert!(
            ConfigV1::from_settings(&settings).is_none(),
            "{settings:?} must not produce a document"
        );
    }
}

#[test]
fn mapped_snapshot_survives_a_save_reload_roundtrip_under_root() {
    let root = temp_root("mapped-roundtrip");
    let store = ConfigStore::new(ConfigPaths::under_root(&root));
    let expected = ConfigV1::from_settings(&saving_settings(true)).expect("intent is derivable");

    store.save(&expected).expect("save mapped config");
    let ConfigLoadOutcome::Loaded { config } = store.load().expect("load mapped config") else {
        panic!("expected a loaded config");
    };
    assert_eq!(config, expected);
}
