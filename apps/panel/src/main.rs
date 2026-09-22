#![cfg_attr(
    all(windows, not(test), not(debug_assertions)),
    windows_subsystem = "windows"
)]
#![forbid(unsafe_code)]

use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::SystemTime;

use dji4g_application::{
    AutostartKnownState, AutostartStatus, ControllerRunner, ErrorCode, PortError,
};
use eframe::egui;

use dji4g_panel::app::{
    NativeSettingsBackend, PANEL_MIN_SIZE, PANEL_WINDOW_SIZE, PanelApp, PanelInputs,
    SettingsBackend, map_autostart_state,
};
use dji4g_panel::config::{ConfigLoadOutcome, ConfigPaths, ConfigStore, StartupOptions};
use dji4g_panel::localization::{Language, LocalizedText, TextKey};
use dji4g_panel::logging::{LoggingConfig, LoggingGuard, init_logging};
use dji4g_panel::runtime::ProductionComposition;
use dji4g_panel::tray::{NativeTrayBackend, TrayController, TrayError, TrayLabels};
use dji4g_windows_platform::{AcquireResult, ActivationRequest, AutostartControl, SingleInstance};

#[cfg(debug_assertions)]
use dji4g_panel::demo::{DemoScenario, demo_snapshot};

/// How long a restarted panel waits for the process it replaces to release the single-instance
/// mutex. Long enough for a graceful shutdown (the tray worker and the serial actor both get
/// their own bounded close), short enough that a wedged shutdown still ends up in front of the
/// user as an activated window rather than a process that never starts.
const RESTART_HANDOFF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The 256 px brand icon shown on the window, taskbar, and Alt-Tab switcher.
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/brand/icon.png");

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let startup = match StartupOptions::parse(args.clone()) {
        Ok(value) => value,
        Err(error) => {
            eprintln!(
                "{}",
                LocalizedText::new(Language::ZhCn, TextKey::ErrorUnsupported)
            );
            let _ = error;
            return;
        }
    };
    #[cfg(debug_assertions)]
    let demo = match startup.demo.as_deref().and_then(DemoScenario::parse) {
        Some(value) => Some(value),
        None if startup.demo.is_some() => {
            eprintln!(
                "{}",
                LocalizedText::new(Language::ZhCn, TextKey::DemoInvalidScenario)
            );
            return;
        }
        None => None,
    };
    #[cfg(not(debug_assertions))]
    if startup.demo.is_some() {
        eprintln!(
            "{}",
            LocalizedText::new(Language::ZhCn, TextKey::DemoRejectedRelease)
        );
        return;
    }

    let mut notices = Vec::new();
    let (paths, config, config_store) = match ConfigPaths::from_environment() {
        Ok(paths) => {
            let store = ConfigStore::new(paths.clone());
            match store.load() {
                Ok(ConfigLoadOutcome::Missing { config })
                | Ok(ConfigLoadOutcome::Loaded { config }) => (Some(paths), config, Some(store)),
                Ok(ConfigLoadOutcome::CorruptPreserved { config, .. }) => {
                    notices.push(TextKey::SettingsCorruptConfig);
                    (Some(paths), config, Some(store))
                }
                Err(_) => {
                    notices.push(TextKey::SettingsReadFailed);
                    // The stored document could not be read (or a corrupt one could not be
                    // restored), so the session runs on defaults; later user-driven saves stay
                    // enabled and report their own terminal result.
                    (
                        Some(paths),
                        dji4g_panel::config::ConfigV1::default(),
                        Some(store),
                    )
                }
            }
        }
        Err(_) => {
            notices.push(TextKey::SettingsPathUnavailable);
            (None, dji4g_panel::config::ConfigV1::default(), None)
        }
    };

    let _logging_guard: Option<LoggingGuard> =
        paths.as_ref().and_then(|paths| {
            match init_logging(LoggingConfig {
                directory: paths.log_dir.clone(),
                level: config.log_level,
                ..LoggingConfig::default()
            }) {
                Ok(guard) => Some(guard),
                Err(_) => {
                    notices.push(TextKey::LoggingInitFailed);
                    None
                }
            }
        });

    // A restart hands the single-instance mutex over: wait (bounded) for the process this one
    // replaces, then take it. If the wait fails — it exited and its pid was reused, or it is
    // still busy — the ordinary single-instance path below decides, which at worst activates the
    // running instance instead of starting a second one.
    if let Some(previous) = startup.restart_after {
        if let Ok(exe) = std::env::current_exe() {
            let _ = dji4g_windows_platform::driver_setup::wait_for_panel_exit(
                previous,
                &exe,
                RESTART_HANDOFF_TIMEOUT,
            );
        }
    }

    let instance = match SingleInstance::acquire() {
        Ok(AcquireResult::Primary(instance)) => Some(instance),
        Ok(AcquireResult::Existing) => {
            // A concurrent manual launch can win the instance mutex while the installer is
            // finishing. Do not silently lose its closed result when activating that window.
            if let Some(outcome) = startup.driver_setup_result {
                dji4g_windows_platform::show_message_box("模块驱动安装结果", outcome.message());
            }
            let request = if startup.autostart {
                ActivationRequest::OpenAndRefresh
            } else {
                ActivationRequest::Open
            };
            if SingleInstance::activate_existing(request).is_err() {
                eprintln!(
                    "{}",
                    LocalizedText::new(Language::ZhCn, TextKey::SingleInstanceActivationFailed)
                );
            }
            return;
        }
        Err(_) => {
            eprintln!(
                "{}",
                LocalizedText::new(Language::ZhCn, TextKey::ErrorInternal)
            );
            return;
        }
    };

    // One autostart control per process: the same handle produces the startup drift readback and
    // later serves the settings toggle through the panel's settings backend.
    let autostart_control = AutostartControl::for_current_process();
    let autostart_status = match &autostart_control {
        Ok(control) => match control.status() {
            Ok(observed) => {
                let actual = map_autostart_state(observed);
                let expected = if config.autostart {
                    AutostartKnownState::Enabled
                } else {
                    AutostartKnownState::Disabled
                };
                if actual == expected {
                    AutostartStatus::Ready(actual)
                } else {
                    AutostartStatus::Ready(AutostartKnownState::Drift)
                }
            }
            Err(error) => AutostartStatus::Failed {
                code: failure_code(error.code),
                previous: None,
            },
        },
        Err(error) => AutostartStatus::Failed {
            code: failure_code(error.code),
            previous: None,
        },
    };
    // The backend bundles both settings side effects; when either handle is missing, pending
    // changes fail with a stable code instead of being claimed as saved.
    let settings_backend: Option<Arc<dyn SettingsBackend>> =
        match (config_store, autostart_control.ok()) {
            (Some(store), Some(control)) => {
                Some(Arc::new(NativeSettingsBackend::new(store, control)))
            }
            _ => None,
        };

    let now = SystemTime::now();
    let composition = ProductionComposition::new(now);
    // The probe record is shared with the UI before the composition is consumed.
    let feature_probe = composition.feature_probe_state();
    let (mut controller, ports) = composition.into_parts();
    controller.set_language(config.language);
    controller.set_start_minimized(config.start_minimized);
    controller.set_active_probe(config.active_probe);
    controller.set_log_level(config.log_level);
    controller.set_autostart_state(autostart_status);
    let (handle, runner) = ControllerRunner::new(controller);
    #[cfg(debug_assertions)]
    let runner = if demo.is_some() {
        runner
    } else {
        runner.with_ports(ports)
    };
    #[cfg(not(debug_assertions))]
    let runner = runner.with_ports(ports);
    #[cfg(debug_assertions)]
    let demo_active = demo.is_some();
    #[cfg(debug_assertions)]
    let snapshot = demo.map_or_else(
        || Arc::new(runner.controller().snapshot()),
        |scenario| Arc::new(demo_snapshot(scenario, now)),
    );
    let _runner_thread = std::thread::Builder::new()
        .name("dji4g-controller".to_owned())
        .spawn(move || run_runner(runner));
    #[cfg(debug_assertions)]
    let snapshot_rx = if demo.is_some() {
        let (_snapshot_tx, snapshot_rx) = dji4g_application::sync::watch::channel(snapshot);
        snapshot_rx
    } else {
        handle.subscribe()
    };
    #[cfg(not(debug_assertions))]
    let snapshot_rx = handle.subscribe();
    // The startup scan must be scheduled in release builds as well.  It used to be gated behind
    // `debug_assertions`, so a packaged panel never scanned at all and stayed permanently in its
    // initial 「正在检测」 state with no recorded evidence, even with the module plugged in and
    // working.  A demo scenario still must not drive the real ports.
    #[cfg(debug_assertions)]
    if demo.is_none() {
        let _ = handle.try_send(dji4g_application::UiCommand::Refresh);
    }
    #[cfg(not(debug_assertions))]
    let _ = handle.try_send(dji4g_application::UiCommand::Refresh);
    // A demo session drives a fabricated snapshot: its settings must never be written into the
    // real user configuration, so the settings backend is detached there.
    #[cfg(debug_assertions)]
    let settings_backend = if demo_active { None } else { settings_backend };
    let inputs = PanelInputs::new(
        snapshot_rx,
        Arc::new(handle),
        paths.as_ref().map(|paths| paths.exports_dir.clone()),
        settings_backend,
    )
    // The demo session drives fabricated snapshots: its optional probes never run, so the shared
    // record stays detached exactly like the settings backend above.
    .with_feature_probe(feature_probe)
    // Confirmation and result boxes are the system's own MessageBoxW dialogs.
    .with_dialog_backend(Arc::new(
        dji4g_panel::native_dialog::NativeMessageBoxBackend,
    ));

    let mut tray_error: Option<TrayError> = None;
    let tray = match TrayController::initialize(NativeTrayBackend::default(), TrayLabels::zh_cn()) {
        Ok(tray) => Some(tray),
        Err(error) => {
            tray_error = Some(error);
            None
        }
    };
    let start_hidden = config.onboarding_completed
        && startup.start_to_tray(config.start_minimized)
        && tray.is_some();
    #[cfg(debug_assertions)]
    let configure_first_run = !demo_active;
    #[cfg(not(debug_assertions))]
    let configure_first_run = true;

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(PANEL_WINDOW_SIZE)
        .with_min_inner_size(PANEL_MIN_SIZE)
        .with_resizable(true)
        .with_visible(!start_hidden);
    // The brand PNG is a repository asset; a decode failure degrades to the platform default
    // icon instead of failing the launch.
    if let Ok(icon) = eframe::icon_data::from_png_bytes(APP_ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }
    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let result = eframe::run_native(
        "DJI 一代 4G 面板",
        native_options,
        Box::new(move |cc| {
            let mut app = PanelApp::new(inputs, cc);
            if configure_first_run {
                app.configure_onboarding(&config);
                if let Some(paths) = &paths {
                    app.configure_archive(
                        paths.log_dir.with_file_name("sms-history.dat"),
                        config.sms_archive_enabled,
                    );
                }
                if let Some(outcome) = startup.driver_setup_result {
                    app.configure_driver_setup_result(outcome);
                }
            }
            if let Some(tray) = tray {
                app.attach_tray(tray);
            }
            if let Some(instance) = instance {
                app.attach_single_instance(instance);
            }
            if let Some(error) = tray_error {
                app.set_tray_error(error);
            }
            for notice in notices {
                app.set_notice(notice);
            }
            Ok(Box::new(app))
        }),
    );
    if result.is_err() {
        // The native backend error is intentionally not forwarded verbatim: normal user-facing
        // output must stay inside the closed zh-CN catalog.
        eprintln!(
            "{}",
            LocalizedText::new(Language::ZhCn, TextKey::ErrorInternal)
        );
    }
}

fn failure_code(code: &'static str) -> dji4g_application::FailureCode {
    PortError::new(ErrorCode::Internal, code).code
}

fn run_runner(runner: ControllerRunner) {
    let mut future = Box::pin(runner.run());
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    loop {
        if let Poll::Ready(()) = future.as_mut().poll(&mut context) {
            break;
        }
        std::thread::yield_now();
    }
}
