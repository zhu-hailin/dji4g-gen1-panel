//! Settings persistence and autostart outcome flows: the panel performs the writes and reports
//! terminal results back as closed `UiCommand` values; the reducer must transition honestly and
//! reject outcomes that no longer describe the current settings.

use std::time::SystemTime;

use dji4g_application::{
    AutostartApplyOutcome, AutostartKnownState, AutostartStatus, Controller, ErrorCode,
    FailureCode, SettingsPersistenceState, StableCode, UiCommand,
};

const NOW: SystemTime = SystemTime::UNIX_EPOCH;

fn code(stable: &'static str) -> FailureCode {
    FailureCode::new(
        ErrorCode::Internal,
        StableCode::try_from_static(stable).expect("static stable code"),
    )
}

fn handle(controller: &mut Controller, command: UiCommand) {
    controller
        .handle_command(command)
        .expect("the command is always accepted");
}

fn autostart(controller: &Controller) -> AutostartStatus {
    controller.snapshot().settings.autostart
}

fn persistence(controller: &Controller) -> SettingsPersistenceState {
    controller.snapshot().settings.persistence
}

#[test]
fn set_autostart_enters_saving_and_keeps_the_previous_observed_state() {
    let mut controller = Controller::for_test(NOW);
    controller.set_autostart_state(AutostartStatus::Ready(AutostartKnownState::Enabled));
    let revision_before = controller.snapshot().settings.revision;

    handle(&mut controller, UiCommand::SetAutostart(false));

    let revision_after = controller.snapshot().settings.revision;
    assert_eq!(revision_after, revision_before + 1);
    assert_eq!(
        autostart(&controller),
        AutostartStatus::Saving {
            desired_enabled: false,
            previous: Some(AutostartKnownState::Enabled),
        }
    );
}

#[test]
fn autostart_apply_success_reaches_the_observed_state() {
    let mut controller = Controller::for_test(NOW);
    handle(&mut controller, UiCommand::SetAutostart(true));

    handle(
        &mut controller,
        UiCommand::AutostartApplied(AutostartApplyOutcome {
            desired_enabled: true,
            observed: Ok(AutostartKnownState::Enabled),
        }),
    );

    assert_eq!(
        autostart(&controller),
        AutostartStatus::Ready(AutostartKnownState::Enabled)
    );
}

#[test]
fn autostart_apply_failure_reports_the_code_and_the_previous_state() {
    let mut controller = Controller::for_test(NOW);
    controller.set_autostart_state(AutostartStatus::Ready(AutostartKnownState::Enabled));
    handle(&mut controller, UiCommand::SetAutostart(false));

    handle(
        &mut controller,
        UiCommand::AutostartApplied(AutostartApplyOutcome {
            desired_enabled: false,
            observed: Err(code("autostart:registry_write_failed")),
        }),
    );

    assert_eq!(
        autostart(&controller),
        AutostartStatus::Failed {
            code: code("autostart:registry_write_failed"),
            previous: Some(AutostartKnownState::Enabled),
        }
    );
}

#[test]
fn autostart_outcome_for_a_different_direction_is_stale_and_ignored() {
    let mut controller = Controller::for_test(NOW);
    controller.set_autostart_state(AutostartStatus::Ready(AutostartKnownState::Disabled));
    handle(&mut controller, UiCommand::SetAutostart(true));

    // An outcome for a superseded request must neither resolve nor corrupt the pending save.
    handle(
        &mut controller,
        UiCommand::AutostartApplied(AutostartApplyOutcome {
            desired_enabled: false,
            observed: Ok(AutostartKnownState::Disabled),
        }),
    );

    assert_eq!(
        autostart(&controller),
        AutostartStatus::Saving {
            desired_enabled: true,
            previous: Some(AutostartKnownState::Disabled),
        }
    );
}

#[test]
fn a_new_toggle_after_a_failure_starts_saving_again_with_the_last_known_state() {
    let mut controller = Controller::for_test(NOW);
    controller.set_autostart_state(AutostartStatus::Ready(AutostartKnownState::Enabled));
    handle(&mut controller, UiCommand::SetAutostart(false));
    handle(
        &mut controller,
        UiCommand::AutostartApplied(AutostartApplyOutcome {
            desired_enabled: false,
            observed: Err(code("autostart:registry_write_failed")),
        }),
    );

    handle(&mut controller, UiCommand::SetAutostart(true));

    assert_eq!(
        autostart(&controller),
        AutostartStatus::Saving {
            desired_enabled: true,
            previous: Some(AutostartKnownState::Enabled),
        }
    );
}

#[test]
fn a_user_settings_change_persists_through_saving_to_clean() {
    let mut controller = Controller::for_test(NOW);
    assert_eq!(persistence(&controller), SettingsPersistenceState::Clean);

    handle(&mut controller, UiCommand::SetStartMinimized(true));
    let revision = controller.snapshot().settings.revision;
    assert_eq!(persistence(&controller), SettingsPersistenceState::Saving);

    handle(
        &mut controller,
        UiCommand::SettingsPersisted(dji4g_application::SettingsSaveOutcome {
            revision,
            result: Ok(()),
        }),
    );
    assert_eq!(persistence(&controller), SettingsPersistenceState::Clean);
}

#[test]
fn a_failed_settings_write_surfaces_the_stable_code() {
    let mut controller = Controller::for_test(NOW);
    handle(&mut controller, UiCommand::SetActiveProbe(false));
    let revision = controller.snapshot().settings.revision;

    handle(
        &mut controller,
        UiCommand::SettingsPersisted(dji4g_application::SettingsSaveOutcome {
            revision,
            result: Err(code("config:write_failed")),
        }),
    );

    assert_eq!(
        persistence(&controller),
        SettingsPersistenceState::Failed {
            code: code("config:write_failed"),
        }
    );
}

#[test]
fn a_stale_persistence_outcome_is_ignored() {
    let mut controller = Controller::for_test(NOW);
    handle(&mut controller, UiCommand::SetStartMinimized(true));
    let stale_revision = controller.snapshot().settings.revision;
    handle(&mut controller, UiCommand::SetActiveProbe(false));
    let current_revision = controller.snapshot().settings.revision;
    assert!(current_revision > stale_revision);

    handle(
        &mut controller,
        UiCommand::SettingsPersisted(dji4g_application::SettingsSaveOutcome {
            revision: stale_revision,
            result: Ok(()),
        }),
    );

    // The older write can no longer describe the pending settings, so `Saving` stays honest.
    assert_eq!(persistence(&controller), SettingsPersistenceState::Saving);

    handle(
        &mut controller,
        UiCommand::SettingsPersisted(dji4g_application::SettingsSaveOutcome {
            revision: current_revision,
            result: Ok(()),
        }),
    );
    assert_eq!(persistence(&controller), SettingsPersistenceState::Clean);
}

#[test]
fn a_no_op_settings_command_does_not_strand_the_persistence_state() {
    let mut controller = Controller::for_test(NOW);
    // Startup seeding on the same value must not mark a write pending: the panel triggers on the
    // settings revision, which a no-op never bumps, so `Saving` could never resolve.
    handle(&mut controller, UiCommand::SetStartMinimized(false));
    assert_eq!(persistence(&controller), SettingsPersistenceState::Clean);
    assert_eq!(controller.snapshot().settings.revision, 0);

    // Even with a write already pending, a no-op command must not resolve or reset it.
    handle(
        &mut controller,
        UiCommand::SetLanguage(dji4g_application::LanguageCode::EnUs),
    );
    handle(
        &mut controller,
        UiCommand::SetLanguage(dji4g_application::LanguageCode::EnUs),
    );
    assert_eq!(persistence(&controller), SettingsPersistenceState::Saving);
}

#[test]
fn startup_seeding_through_the_direct_setters_keeps_persistence_clean() {
    let mut controller = Controller::for_test(NOW);
    controller.set_language(dji4g_application::LanguageCode::EnUs);
    controller.set_start_minimized(true);
    controller.set_active_probe(false);
    controller.set_log_level(dji4g_application::LogLevel::Debug);
    controller.set_autostart_state(AutostartStatus::Ready(AutostartKnownState::Disabled));

    assert_eq!(persistence(&controller), SettingsPersistenceState::Clean);
}

#[test]
fn finishing_an_autostart_write_does_not_reset_the_persistence_state() {
    let mut controller = Controller::for_test(NOW);
    handle(&mut controller, UiCommand::SetAutostart(true));
    assert_eq!(persistence(&controller), SettingsPersistenceState::Saving);

    handle(
        &mut controller,
        UiCommand::AutostartApplied(AutostartApplyOutcome {
            desired_enabled: true,
            observed: Ok(AutostartKnownState::Enabled),
        }),
    );

    // The registry readback is not a settings value; only the config write resolves persistence.
    assert_eq!(persistence(&controller), SettingsPersistenceState::Saving);
}
