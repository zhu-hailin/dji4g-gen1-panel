//! Settings projection. Task 8 owns persistence; this page only renders its typed snapshot and
//! emits closed commands.

use dji4g_application::{
    AutostartKnownState, AutostartStatus, ControllerSnapshot, LanguageCode, LogLevel,
    SettingsPersistenceState, SettingsSnapshot, UiCommand,
};
use eframe::egui::{self, RichText, Ui};

use super::{meta_text, scale, wrapped_label};
use crate::app::UiCommandSink;
use crate::localization::{
    Language, LocalizedText, TextKey, available_languages, english_available,
};

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
        LanguageCode::EnUs if english_available() => Language::EnUs,
        LanguageCode::EnUs => Language::ZhCn,
    };
    let autostart = match &settings.autostart {
        AutostartStatus::Loading => AutostartVm {
            status: LocalizedText::new(language, TextKey::StatusLoading),
            enabled: false,
            toggle_enabled: false,
            drift: false,
        },
        AutostartStatus::Ready(AutostartKnownState::Disabled) => AutostartVm {
            status: LocalizedText::new(language, TextKey::ValueNotAvailable),
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
        AutostartStatus::Failed { code, .. } => AutostartVm {
            status: crate::localization::failure_text(code, language),
            enabled: false,
            toggle_enabled: true,
            drift: false,
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

/// Render the settings page. Returns the response of every interactive control, so tests can
/// assert they actually landed inside the visible page — a layout regression would otherwise
/// push them out of the clip rect and make the page unusable.
pub(crate) fn render(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    language: Language,
    sink: &dyn UiCommandSink,
) -> Vec<egui::Response> {
    let vm = settings_vm(snapshot, language);
    ui.heading(vm.title.text.clone());
    ui.add_space(4.0);
    // Rows like the reference's `.setting-row`: label + description on the left, the control on
    // the right. Generous whitespace separates rows instead of hairlines so the page breathes.
    // Every control lives *inside* the row's right-to-left sub-layout: a spacer widget placed in
    // the row itself used to push the parent cursor past the row edge, which placed every
    // control outside the visible page.
    let mut controls = Vec::new();
    super::section_frame(ui, |ui| {
        ui.label(super::section_heading("常规"));
        controls.push(setting_row(
            ui,
            TextKey::SettingsLanguage.to_string(language),
            Some("English 选项将在完整翻译审核后开放。".to_owned()),
            |ui| wrapped_label(ui, TextKey::LanguageZhCn.to_string(language)),
        ));
        controls.push(setting_row(
            ui,
            TextKey::SettingsActiveProbe.to_string(language),
            Some(TextKey::SettingsActiveProbeDescription.to_string(language)),
            |ui| {
                let mut active_probe = vm.active_probe;
                let response = ui.checkbox(&mut active_probe, "开 / 关");
                if response.changed() {
                    let _ = sink.try_send(UiCommand::SetActiveProbe(active_probe));
                }
                response
            },
        ));
    });
    super::section_frame(ui, |ui| {
        ui.label(super::section_heading("启动与托盘"));
        controls.push(setting_row(
            ui,
            TextKey::SettingsAutostart.to_string(language),
            Some(TextKey::SettingsAutostartDescription.to_string(language)),
            |ui| {
                let mut enabled = vm.autostart.enabled;
                let response = ui.add_enabled(
                    vm.autostart.toggle_enabled,
                    egui::Checkbox::new(&mut enabled, "开 / 关"),
                );
                if response.changed() {
                    let _ = sink.try_send(UiCommand::SetAutostart(enabled));
                }
                if !vm.autostart.status.text.is_empty() {
                    wrapped_label(ui, meta_text(vm.autostart.status.text.clone()));
                }
                if vm.autostart.drift {
                    wrapped_label(
                        ui,
                        RichText::new(TextKey::SettingsConfigDrift.to_string(language))
                            .color(super::StatusTone::Caution.color()),
                    );
                }
                response
            },
        ));
        controls.push(setting_row(
            ui,
            TextKey::SettingsStartMinimized.to_string(language),
            None,
            |ui| {
                let mut start_minimized = vm.start_minimized;
                let response = ui.checkbox(&mut start_minimized, "开 / 关");
                if response.changed() {
                    let _ = sink.try_send(UiCommand::SetStartMinimized(start_minimized));
                }
                response
            },
        ));
        ui.separator();
        ui.label(super::section_heading("日志"));
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
                    let _ = sink.try_send(UiCommand::SetLogLevel(selected));
                }
                combo.response
            },
        ));
        ui.separator();
        ui.label(super::section_heading("关于"));
        wrapped_label(
            ui,
            format!("DJI 一代 4G 面板 · v{}", env!("CARGO_PKG_VERSION")),
        );
        setting_block(
            ui,
            TextKey::SettingsPrivacy.to_string(language),
            Some(TextKey::SettingsPrivacyDescription.to_string(language)),
        );
        ui.add_space(12.0);
        wrapped_label(ui, meta_text(vm.persistence.status.text));
    });
    controls
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
    let mut response = None;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width() * 0.55);
            ui.label(
                RichText::new(label)
                    .size(scale::LABEL)
                    .strong()
                    .color(scale::INK),
            );
            if let Some(description) = description {
                wrapped_label(
                    ui,
                    RichText::new(description)
                        .size(scale::RATE_AUX)
                        .color(scale::SECONDARY),
                );
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
fn setting_block(ui: &mut Ui, label: String, description: Option<String>) {
    ui.add_space(8.0);
    ui.vertical(|ui| {
        ui.label(
            RichText::new(label)
                .size(scale::LABEL)
                .strong()
                .color(scale::INK),
        );
        if let Some(description) = description {
            wrapped_label(
                ui,
                RichText::new(description)
                    .size(scale::RATE_AUX)
                    .color(scale::SECONDARY),
            );
        }
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
    use std::sync::{Arc, Mutex};

    struct RecordingSink {
        commands: Mutex<Vec<UiCommand>>,
    }

    impl crate::app::UiCommandSink for RecordingSink {
        fn try_send(&self, command: UiCommand) -> Result<(), dji4g_application::UiSendError> {
            self.commands.lock().expect("sink lock").push(command);
            Ok(())
        }
    }

    fn snapshot() -> ControllerSnapshot {
        ControllerSnapshot {
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
            sms_inbox_failure: None,
            device_tools: Default::default(),
        }
    }

    /// Regression guard for the layout bug that pushed every settings control outside the
    /// visible page (a right-to-left spacer in the row): every interactive control must land
    /// inside the viewport and actually be hoverable.
    #[test]
    fn every_setting_control_lands_inside_the_visible_page() {
        let context = egui::Context::default();
        let sink = RecordingSink {
            commands: Mutex::new(Vec::new()),
        };
        let snapshot = snapshot();
        let mut controls = Vec::new();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(552.0, 900.0));
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    controls = render(ui, &snapshot, Language::ZhCn, &sink);
                });
            },
        );
        assert_eq!(
            controls.len(),
            5,
            "all five setting rows must render a control"
        );
        for response in &controls {
            let rect = response.rect;
            assert!(
                rect.width() > 0.0 && rect.height() > 0.0,
                "control has no usable area: {rect:?}"
            );
            assert!(
                rect.left() >= viewport.left() && rect.right() <= viewport.right() + 0.5,
                "control is outside the visible page: {rect:?}"
            );
        }
    }
}
