//! Settings projection. Task 8 owns persistence; this page only renders its typed snapshot and
//! emits closed commands.

use dji4g_application::{
    AutostartKnownState, AutostartStatus, ControllerSnapshot, LanguageCode, LogLevel,
    SettingsPersistenceState, SettingsSnapshot, UiCommand,
};
use eframe::egui::{self, RichText, Ui};

use super::{meta_text, scale, wrapped_label};
use crate::localization::{
    Language, LocalizedText, TextKey, available_languages, language_code, language_key,
};

/// One catalog string in the language this page was rendered with.
fn t(language: crate::localization::Language, key: crate::localization::TextKey) -> String {
    crate::localization::LocalizedText::new(language, key).text
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsVm {
    pub title: LocalizedText,
    pub language: Language,
    pub language_options: &'static [Language],
    pub autostart: AutostartVm,
    pub start_minimized: bool,
    pub active_probe: bool,
    pub log_level: LogLevel,
    pub persistence: PersistenceVm,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutostartVm {
    pub status: LocalizedText,
    pub enabled: bool,
    pub toggle_enabled: bool,
    pub drift: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistenceVm {
    pub status: LocalizedText,
    pub saving: bool,
}

#[must_use]
pub fn settings_vm(snapshot: &ControllerSnapshot, language: Language) -> SettingsVm {
    settings_vm_from(&snapshot.settings, language)
}

#[must_use]
pub fn settings_vm_from(settings: &SettingsSnapshot, language: Language) -> SettingsVm {
    let selected_language = match settings.language {
        LanguageCode::ZhCn => Language::ZhCn,
        LanguageCode::ZhTw => Language::ZhTw,
        LanguageCode::EnUs => Language::EnUs,
    };
    let autostart = match &settings.autostart {
        AutostartStatus::Loading => AutostartVm {
            status: LocalizedText::new(language, TextKey::SettingsAutostartLoading),
            enabled: false,
            toggle_enabled: false,
            drift: false,
        },
        AutostartStatus::Ready(AutostartKnownState::Disabled) => AutostartVm {
            status: LocalizedText::new(language, TextKey::SettingsAutostartDisabled),
            enabled: false,
            toggle_enabled: true,
            drift: false,
        },
        AutostartStatus::Ready(AutostartKnownState::Enabled) => AutostartVm {
            status: LocalizedText::new(language, TextKey::SettingsSaved),
            enabled: true,
            toggle_enabled: true,
            drift: false,
        },
        AutostartStatus::Ready(AutostartKnownState::Drift) => AutostartVm {
            status: LocalizedText::new(language, TextKey::SettingsConfigDrift),
            enabled: false,
            toggle_enabled: true,
            drift: true,
        },
        AutostartStatus::Saving {
            desired_enabled, ..
        } => AutostartVm {
            status: LocalizedText::new(language, TextKey::SettingsAutostartSaving),
            enabled: *desired_enabled,
            toggle_enabled: false,
            drift: false,
        },
        AutostartStatus::Failed { code, previous } => AutostartVm {
            status: crate::localization::failure_text(code, language),
            enabled: matches!(previous, Some(AutostartKnownState::Enabled)),
            toggle_enabled: true,
            drift: matches!(previous, Some(AutostartKnownState::Drift)),
        },
    };
    let persistence = match &settings.persistence {
        SettingsPersistenceState::Clean => PersistenceVm {
            status: LocalizedText::new(language, TextKey::SettingsSaved),
            saving: false,
        },
        SettingsPersistenceState::Saving => PersistenceVm {
            status: LocalizedText::new(language, TextKey::StatusLoading),
            saving: true,
        },
        SettingsPersistenceState::Failed { code } => PersistenceVm {
            status: crate::localization::failure_text(code, language),
            saving: false,
        },
    };
    SettingsVm {
        title: LocalizedText::new(language, TextKey::SettingsTitle),
        language: selected_language,
        language_options: available_languages(),
        autostart,
        start_minimized: settings.start_minimized,
        active_probe: settings.active_probe,
        log_level: settings.log_level,
        persistence,
    }
}

/// The app dispatches collected commands after layout through its error-reporting send path.
#[must_use]
#[derive(Default)]
pub(crate) struct SettingsRenderOutput {
    #[cfg(test)]
    pub controls: Vec<egui::Response>,
    pub commands: Vec<UiCommand>,
}

/// Render controls and collect user intent without submitting it to the backend.
pub(crate) fn render(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    language: Language,
) -> SettingsRenderOutput {
    let vm = settings_vm(snapshot, language);
    super::components::page_heading(ui, &vm.title.text, "");
    ui.add_space(4.0);
    // Rows like the reference's `.setting-row`: label + description on the left, the control on
    // the right. Generous whitespace separates rows instead of hairlines so the page breathes.
    // Every control lives *inside* the row's right-to-left sub-layout: a spacer widget placed in
    // the row itself used to push the parent cursor past the row edge, which placed every
    // control outside the visible page.
    let mut controls = Vec::new();
    let mut commands = Vec::new();
    super::section_frame(ui, |ui| {
        ui.label(super::section_heading(t(
            language,
            crate::localization::TextKey::SettingsGeneral,
        )));
        controls.push(setting_row(
            ui,
            t(
                language,
                crate::localization::TextKey::SettingsInterfaceTheme,
            ),
            None,
            |ui| {
                let current = super::theme::theme_preference(snapshot.settings.theme);
                let mut selected = current;
                let combo = egui::ComboBox::from_id_salt("settings-theme")
                    .selected_text(super::theme::theme_label(selected, language))
                    .show_ui(ui, |ui| {
                        for choice in super::theme::THEME_CHOICES {
                            if ui
                                .selectable_label(
                                    choice == selected,
                                    super::theme::theme_label(choice, language),
                                )
                                .clicked()
                            {
                                selected = choice;
                            }
                        }
                    });
                if selected != current {
                    commands.push(UiCommand::SetTheme(super::theme::theme_code(selected)));
                }
                combo.response
            },
        ));
        controls.push(setting_row(
            ui,
            TextKey::SettingsLanguage.to_string(language),
            None,
            |ui| {
                // Same control as the theme row above, and the same shape: pick a value, and only a
                // real change emits a command. Each option is labelled with its own endonym, so
                // someone who cannot read the current language can still find theirs.
                let mut selected = vm.language;
                let combo = egui::ComboBox::from_id_salt("settings-language")
                    .selected_text(crate::localization::template(
                        language,
                        language_key(selected),
                    ))
                    .show_ui(ui, |ui| {
                        for choice in vm.language_options {
                            let label =
                                crate::localization::template(language, language_key(*choice));
                            if ui.selectable_label(*choice == selected, label).clicked() {
                                selected = *choice;
                            }
                        }
                    });
                if selected != vm.language {
                    commands.push(UiCommand::SetLanguage(language_code(selected)));
                }
                combo.response
            },
        ));
        controls.push(setting_row(
            ui,
            TextKey::SettingsActiveProbe.to_string(language),
            Some(TextKey::SettingsActiveProbeDescription.to_string(language)),
            |ui| {
                let mut active_probe = vm.active_probe;
                let response = super::components::switch(
                    ui,
                    &mut active_probe,
                    &t(language, crate::localization::TextKey::SettingsActiveProbe),
                    true,
                );
                if response.changed() {
                    commands.push(UiCommand::SetActiveProbe(active_probe));
                }
                response
            },
        ));
    });
    super::section_frame(ui, |ui| {
        ui.label(super::section_heading(t(
            language,
            crate::localization::TextKey::SettingsStartupTray,
        )));
        controls.push(setting_row(
            ui,
            TextKey::SettingsAutostart.to_string(language),
            Some(TextKey::SettingsAutostartDescription.to_string(language)),
            |ui| {
                let mut enabled = vm.autostart.enabled;
                let response = super::components::switch(
                    ui,
                    &mut enabled,
                    &t(language, crate::localization::TextKey::SettingsAutoStart),
                    vm.autostart.toggle_enabled,
                );
                if response.changed() {
                    commands.push(UiCommand::SetAutostart(enabled));
                }
                response
            },
        ));
        let status = meta_text(vm.autostart.status.text.clone());
        wrapped_label(
            ui,
            if vm.autostart.drift {
                status.color(super::StatusTone::Caution.color())
            } else {
                status
            },
        );
        controls.push(setting_row(
            ui,
            TextKey::SettingsStartMinimized.to_string(language),
            None,
            |ui| {
                let mut start_minimized = vm.start_minimized;
                let response = super::components::switch(
                    ui,
                    &mut start_minimized,
                    &t(language, crate::localization::TextKey::SettingsStartHidden),
                    true,
                );
                if response.changed() {
                    commands.push(UiCommand::SetStartMinimized(start_minimized));
                }
                response
            },
        ));
        ui.separator();
        ui.label(super::section_heading(t(
            language,
            crate::localization::TextKey::SettingsLogging,
        )));
        controls.push(setting_row(
            ui,
            TextKey::SettingsLogLevel.to_string(language),
            Some(TextKey::SettingsLogLevelRestart.to_string(language)),
            |ui| {
                let mut selected = vm.log_level;
                let combo = egui::ComboBox::from_id_salt("settings-log-level")
                    .selected_text(log_level_text(selected, language))
                    .show_ui(ui, |ui| {
                        for level in [
                            LogLevel::Error,
                            LogLevel::Warn,
                            LogLevel::Info,
                            LogLevel::Debug,
                        ] {
                            if ui
                                .selectable_label(
                                    level == selected,
                                    log_level_text(level, language),
                                )
                                .clicked()
                            {
                                selected = level;
                            }
                        }
                    });
                if selected != vm.log_level {
                    commands.push(UiCommand::SetLogLevel(selected));
                }
                combo.response
            },
        ));
        ui.separator();
        ui.label(super::section_heading(t(
            language,
            crate::localization::TextKey::SettingsAbout,
        )));
        wrapped_label(
            ui,
            crate::localization::format_positional(
                language,
                crate::localization::TextKey::SettingsVersion,
                &[env!("CARGO_PKG_VERSION")],
            ),
        );
        setting_block(
            ui,
            TextKey::SettingsPrivacy.to_string(language),
            TextKey::SettingsPrivacyDescription.to_string(language),
        );
        ui.add_space(12.0);
        wrapped_label(ui, meta_text(vm.persistence.status.text));
    });
    SettingsRenderOutput {
        #[cfg(test)]
        controls,
        commands,
    }
}

/// One setting row: label block on the left, the control placed inside a right-to-left
/// sub-layout on the right so it sits at the row's right edge and stays clickable. Returns the
/// control's response for layout regression tests.
fn setting_row(
    ui: &mut Ui,
    label: String,
    description: Option<String>,
    control: impl FnOnce(&mut Ui) -> egui::Response,
) -> egui::Response {
    ui.add_space(8.0);
    if ui.available_width() < 440.0 {
        ui.label(RichText::new(label).strong().color(scale::ink()));
        if let Some(description) = description {
            wrapped_label(ui, meta_text(description));
        }
        let response = control(ui);
        ui.add_space(8.0);
        ui.separator();
        return response;
    }
    let mut response = None;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width() * 0.55);
            ui.label(
                RichText::new(label)
                    .size(scale::HEADING)
                    .strong()
                    .color(scale::ink()),
            );
            if let Some(description) = description {
                wrapped_label(ui, meta_text(description));
            }
        });
        let inner = ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            response = Some(control(ui));
        });
        if response.is_none() {
            response = Some(inner.response);
        }
    });
    ui.add_space(8.0);
    response.expect("a setting row control always returns a response")
}

/// A full-width block without a control (privacy notice).
fn setting_block(ui: &mut Ui, label: String, description: String) {
    ui.add_space(8.0);
    ui.vertical(|ui| {
        ui.label(
            RichText::new(label)
                .size(scale::HEADING)
                .strong()
                .color(scale::ink()),
        );
        wrapped_label(ui, meta_text(description));
    });
    ui.add_space(8.0);
}

fn log_level_text(level: LogLevel, language: Language) -> String {
    LocalizedText::new(
        language,
        match level {
            LogLevel::Error => TextKey::LogLevelError,
            LogLevel::Warn => TextKey::LogLevelWarn,
            LogLevel::Info => TextKey::LogLevelInfo,
            LogLevel::Debug => TextKey::LogLevelDebug,
        },
    )
    .text
}

trait LocalizedKeyText {
    fn to_string(self, language: Language) -> String;
}

impl LocalizedKeyText for TextKey {
    fn to_string(self, language: Language) -> String {
        crate::localization::LocalizedText::new(language, self).text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dji4g_domain::{Availability, Freshness, HotspotStatus};
    use std::sync::Arc;

    fn snapshot() -> ControllerSnapshot {
        ControllerSnapshot {
            rates_sampled_at: None,
            module_network_check: None,
            host_network: dji4g_application::HostNetworkSnapshot::default(),
            publication_revision: 0,
            app: Arc::new(dji4g_domain::AppSnapshot {
                revision: 0,
                observed_at: std::time::SystemTime::UNIX_EPOCH,
                freshness: Freshness::Fresh,
                availability: Availability::Available,
                hotspot: HotspotStatus::Off,
                device: None,
                cellular: None,
                network: None,
                active_operation: None,
                issues: Vec::new(),
            }),
            diagnostics: dji4g_application::DiagnosticSet::new(dji4g_domain::DeviceEpoch(1)),
            prepared_action: None,
            operation: None,
            settings: SettingsSnapshot::default(),
            command_state: dji4g_application::CommandStateSnapshot::default(),
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

    /// Regression guard for the layout bug that pushed every settings control outside the
    /// visible page (a right-to-left spacer in the row): every interactive control must land
    /// inside the viewport and actually be hoverable.
    #[test]
    fn every_setting_control_lands_inside_the_visible_page() {
        for width in [320.0, 552.0] {
            for language in available_languages() {
                for autostart in [
                    AutostartStatus::Ready(AutostartKnownState::Disabled),
                    AutostartStatus::Ready(AutostartKnownState::Drift),
                    failed_autostart(Some(AutostartKnownState::Enabled)),
                ] {
                    let context = egui::Context::default();
                    super::super::apply_style(&context);
                    let mut snapshot = snapshot();
                    snapshot.settings.autostart = autostart;
                    let mut controls = Vec::new();
                    let viewport =
                        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 1400.0));
                    let _ = context.run(
                        egui::RawInput {
                            screen_rect: Some(viewport),
                            ..Default::default()
                        },
                        |context| {
                            egui::CentralPanel::default().show(context, |ui| {
                                let output = render(ui, &snapshot, *language);
                                assert!(output.commands.is_empty());
                                controls = output.controls;
                            });
                        },
                    );
                    assert_eq!(
                        controls.len(),
                        6,
                        "every setting row must render a control (theme, language, probe, \
                         autostart, start-minimized, log level)"
                    );
                    for response in &controls {
                        let rect = response.rect;
                        assert!(
                            rect.width() > 0.0 && rect.height() > 0.0,
                            "control has no usable area: {rect:?}"
                        );
                        assert!(
                            rect.left() >= viewport.left()
                                && rect.right() <= viewport.right() + 0.5,
                            "{language:?} control exceeds {width}px page: {rect:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn settings_explanations_and_confirmed_off_state_are_rendered() {
        for width in [320.0, 552.0] {
            for (language, disabled_text) in [
                (Language::ZhCn, "已关闭"),
                (Language::ZhTw, "已關閉"),
                (Language::EnUs, "Off"),
            ] {
                let context = egui::Context::default();
                super::super::apply_style(&context);
                let mut snapshot = snapshot();
                snapshot.settings.autostart = AutostartStatus::Ready(AutostartKnownState::Disabled);
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 1400.0),
                        )),
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            let _ = render(ui, &snapshot, language);
                        });
                    },
                );
                let texts: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) => Some(text.galley.text()),
                        _ => None,
                    })
                    .collect();
                assert!(texts.contains(&disabled_text));
                for key in [
                    TextKey::SettingsActiveProbeDescription,
                    TextKey::SettingsAutostartDescription,
                    TextKey::SettingsLogLevelRestart,
                    TextKey::SettingsPrivacyDescription,
                ] {
                    let expected = t(language, key);
                    assert!(
                        texts.contains(&expected.as_str()),
                        "{key:?} explanation must be visible in {language:?} at {width}px"
                    );
                }
            }
        }
    }

    fn failed_autostart(previous: Option<AutostartKnownState>) -> AutostartStatus {
        AutostartStatus::Failed {
            code: dji4g_application::FailureCode::new(
                dji4g_domain::ErrorCode::PermissionDenied,
                dji4g_application::StableCode::try_from_static("autostart:registry_write_failed")
                    .expect("safe code"),
            ),
            previous,
        }
    }

    #[test]
    fn failed_autostart_preserves_the_last_confirmed_state_and_error() {
        for previous in [AutostartKnownState::Enabled, AutostartKnownState::Disabled] {
            let settings = SettingsSnapshot {
                autostart: failed_autostart(Some(previous)),
                ..SettingsSnapshot::default()
            };
            let vm = settings_vm_from(&settings, Language::ZhCn);
            assert_eq!(
                vm.autostart.enabled,
                previous == AutostartKnownState::Enabled
            );
            assert!(vm.autostart.toggle_enabled);
            assert!(!vm.autostart.drift);
            let AutostartStatus::Failed { code, .. } = &settings.autostart else {
                unreachable!();
            };
            assert_eq!(
                vm.autostart.status,
                crate::localization::failure_text(code, Language::ZhCn)
            );
        }
    }

    #[test]
    fn retrying_failed_disable_collects_exactly_one_disable_command() {
        let context = egui::Context::default();
        super::super::apply_style(&context);
        let mut snapshot = snapshot();
        snapshot.settings.autostart = failed_autostart(Some(AutostartKnownState::Enabled));
        let mut control_id = None;
        let mut commands = Vec::new();
        for tick in 0..3 {
            if let Some(id) = control_id {
                context.memory_mut(|memory| memory.request_focus(id));
            }
            let events = if tick == 0 {
                Vec::new()
            } else {
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: tick == 1,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            };
            let _ = context.run(
                egui::RawInput {
                    events,
                    focused: true,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(552.0, 900.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        let output = render(ui, &snapshot, Language::ZhCn);
                        // Row order in the 常规/启动与托盘 sections: theme, language, probe,
                        // autostart, start-minimized, log level.
                        control_id = Some(output.controls[3].id);
                        if tick != 1 {
                            assert!(output.commands.is_empty());
                        }
                        commands.extend(output.commands);
                    });
                },
            );
        }
        assert!(matches!(
            commands.as_slice(),
            [UiCommand::SetAutostart(false)]
        ));
    }
}
