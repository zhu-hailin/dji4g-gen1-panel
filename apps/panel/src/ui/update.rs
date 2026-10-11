//! Optional updater entry and status view, matching the header's circular GitHub control.
use crate::{
    localization::{Language, TextKey, format_positional, template},
    update::{UpdateCheckStatus, UpdateError, UpdateState},
};
use eframe::egui::{self, RichText};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Check,
    Open,
    Download,
    Install,
}
fn status_key(state: &UpdateState) -> TextKey {
    match state {
        UpdateState::Checking => TextKey::UpdateChecking,
        UpdateState::Downloading { .. } => TextKey::UpdateDownloading,
        UpdateState::Verifying => TextKey::UpdateVerifying,
        UpdateState::Ready => TextKey::UpdateReady,
        UpdateState::Installing => TextKey::UpdatePreparing,
        UpdateState::Failed { error } => match error {
            UpdateError::Checksum => TextKey::UpdateFailedChecksum,
            UpdateError::Integrity => TextKey::UpdateFailedIntegrity,
            UpdateError::Updater | UpdateError::Startup => TextKey::UpdateFailedUpdater,
            UpdateError::Unsupported => TextKey::UpdateUnsupported,
            _ => TextKey::UpdateFailedNetwork,
        },
        _ => TextKey::UpdateTitle,
    }
}
/// A permanent settings entry makes checks and their outcomes discoverable even
/// when no update icon is present on the overview.
pub fn settings_entry(
    ui: &mut egui::Ui,
    state: &UpdateState,
    check_status: UpdateCheckStatus,
    can_check: bool,
    language: Language,
) -> Action {
    let mut action = Action::None;
    super::section_frame(ui, |ui| {
        ui.label(super::section_heading(template(
            language,
            TextKey::UpdateTitle,
        )));
        ui.label(super::meta_text(format_positional(
            language,
            TextKey::UpdateCurrentVersion,
            &[env!("CARGO_PKG_VERSION")],
        )));
        let key = match check_status {
            UpdateCheckStatus::NotChecked => TextKey::UpdateNotChecked,
            UpdateCheckStatus::Checking => TextKey::UpdateChecking,
            UpdateCheckStatus::UpToDate => TextKey::UpdateUpToDate,
            UpdateCheckStatus::Failed => TextKey::UpdateCheckFailed,
            UpdateCheckStatus::Available => TextKey::UpdateTitle,
        };
        if check_status != UpdateCheckStatus::Available {
            super::wrapped_label(ui, super::meta_text(template(language, key)));
        }
        match state {
            UpdateState::Available { version } => {
                super::wrapped_label(
                    ui,
                    super::meta_text(format_positional(
                        language,
                        TextKey::UpdateAvailable,
                        &[version],
                    )),
                );
            }
            UpdateState::Idle | UpdateState::Checking => {}
            _ => {
                super::wrapped_label(ui, super::meta_text(template(language, status_key(state))));
            }
        }
        ui.horizontal_wrapped(|ui| {
            if super::components::action_button(
                ui,
                template(language, TextKey::UpdateCheckNow),
                super::components::ButtonKind::Outlined,
                can_check,
                Some(template(language, TextKey::UpdateBusy)),
            )
            .clicked()
            {
                action = Action::Check;
            }
            if !matches!(state, UpdateState::Idle | UpdateState::Checking)
                && super::components::action_button(
                    ui,
                    template(language, TextKey::UpdateView),
                    super::components::ButtonKind::Tonal,
                    true,
                    None,
                )
                .clicked()
            {
                action = Action::Open;
            }
            if check_status == UpdateCheckStatus::Checking {
                super::components::loading_spinner(ui);
            }
        });
    });
    action
}
pub fn indicator(
    ui: &mut egui::Ui,
    state: &UpdateState,
    language: Language,
) -> Option<egui::Response> {
    if matches!(state, UpdateState::Idle | UpdateState::Checking) {
        return None;
    }
    let tooltip = match state {
        UpdateState::Available { version } => {
            format_positional(language, TextKey::UpdateAvailable, &[version])
        }
        _ => template(language, status_key(state)).to_owned(),
    };
    let size = super::scale::CONTROL_H;
    let response = ui.add_sized(
        [size, size],
        egui::Button::new("")
            .rounding(egui::Rounding::same(size / 2.0))
            .stroke(egui::Stroke::NONE),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &tooltip)
    });
    let color = if response.hovered() || response.has_focus() {
        super::scale::accent()
    } else {
        super::scale::secondary()
    };
    let stroke = egui::Stroke::new(1.8_f32, color);
    let c = response.rect.center();
    ui.painter().line_segment(
        [c + egui::vec2(0.0, -7.0), c + egui::vec2(0.0, 4.0)],
        stroke,
    );
    ui.painter().line_segment(
        [c + egui::vec2(-4.0, 0.0), c + egui::vec2(0.0, 4.0)],
        stroke,
    );
    ui.painter()
        .line_segment([c + egui::vec2(4.0, 0.0), c + egui::vec2(0.0, 4.0)], stroke);
    ui.painter().line_segment(
        [c + egui::vec2(-6.0, 7.0), c + egui::vec2(6.0, 7.0)],
        stroke,
    );
    if response.has_focus() {
        ui.painter().circle_stroke(
            c,
            size / 2.0 - 1.0,
            egui::Stroke::new(2.0_f32, super::scale::accent()),
        );
    }
    Some(
        response
            .on_hover_text(tooltip)
            .on_hover_cursor(egui::CursorIcon::PointingHand),
    )
}
fn size_text(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}
pub fn progress_window(
    ctx: &egui::Context,
    state: &UpdateState,
    language: Language,
    open: &mut bool,
    can_install: bool,
) -> Action {
    let mut action = Action::None;
    egui::Window::new(template(language, TextKey::UpdateTitle))
        .id(egui::Id::new("panel-update-progress"))
        .open(open)
        .collapsible(false)
        .resizable(false)
        .default_width(360.0)
        .show(ctx, |ui| {
            match state {
                UpdateState::Available { version } => {
                    ui.label(format_positional(
                        language,
                        TextKey::UpdateAvailable,
                        &[version],
                    ));
                }
                UpdateState::Downloading { downloaded, total } => {
                    ui.label(template(language, TextKey::UpdateDownloading));
                    let fraction = if *total == 0 {
                        0.0
                    } else {
                        (*downloaded as f64 / *total as f64).clamp(0.0, 1.0) as f32
                    };
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .show_percentage()
                            .desired_width(300.0),
                    );
                    ui.label(format_positional(
                        language,
                        TextKey::UpdateProgress,
                        &[&size_text(*downloaded), &size_text(*total)],
                    ));
                }
                _ => {
                    ui.label(RichText::new(template(language, status_key(state))).color(
                        if matches!(state, UpdateState::Failed { .. }) {
                            super::scale::danger()
                        } else {
                            super::scale::ink()
                        },
                    ));
                }
            }
            if matches!(
                state,
                UpdateState::Downloading { .. } | UpdateState::Verifying | UpdateState::Installing
            ) {
                super::components::loading_spinner(ui);
            }
            if matches!(state, UpdateState::Ready) {
                ui.label(template(language, TextKey::UpdateRestartHint));
                if !can_install {
                    ui.label(template(language, TextKey::UpdateBusy));
                }
                if ui
                    .add_enabled(
                        can_install,
                        egui::Button::new(template(language, TextKey::UpdateInstallRestart)),
                    )
                    .clicked()
                {
                    action = Action::Install;
                }
            }
            let download_label = match state {
                UpdateState::Available { .. } => Some(TextKey::UpdateDownload),
                UpdateState::Failed { error } if *error != UpdateError::Unsupported => {
                    Some(TextKey::UpdateRetry)
                }
                _ => None,
            };
            if download_label.is_some_and(|key| ui.button(template(language, key)).clicked()) {
                action = Action::Download;
            }
        });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_version_has_no_icon_but_new_version_is_clickable_without_browser() {
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        for tick in 0..3 {
            let events = if tick == 0 {
                vec![]
            } else {
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed: tick == 1,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            let output = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        assert!(indicator(ui, &UpdateState::Idle, Language::ZhCn).is_none());
                        let response = indicator(
                            ui,
                            &UpdateState::Available {
                                version: "0.1.15".into(),
                            },
                            Language::ZhCn,
                        )
                        .expect("new version needs an entry");
                        rect = response.rect;
                        assert_eq!(response.clicked(), tick == 2);
                    });
                },
            );
            assert!(output.platform_output.open_url.is_none());
        }
    }
    #[test]
    fn worker_states_keep_ui_entry_visible_and_never_open_browser() {
        let ctx = egui::Context::default();
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for state in [
                    UpdateState::Downloading {
                        downloaded: 1,
                        total: 10,
                    },
                    UpdateState::Verifying,
                    UpdateState::Ready,
                    UpdateState::Installing,
                    UpdateState::Failed {
                        error: crate::update::UpdateError::Integrity,
                    },
                ] {
                    assert!(indicator(ui, &state, Language::ZhCn).is_some());
                }
            });
        });
        assert!(output.platform_output.open_url.is_none());
    }
}
