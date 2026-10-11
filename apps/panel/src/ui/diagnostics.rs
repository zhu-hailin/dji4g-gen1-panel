//! Layered evidence diagnostics. The fixed order mirrors the reducer's nine checks.

use dji4g_application::{
    ControllerSnapshot, DiagnosticCheckId, DiagnosticCheckSnapshot, DiagnosticCheckState, UiCommand,
};
use dji4g_domain::{BoundDnsStatus, BoundPublicStatus, DefaultRouteOwner, Freshness};
use eframe::egui::{RichText, Ui};
use std::time::SystemTime;

use super::{
    DiagnosticStateVm, DisplayValue, carrier_display_name, diagnostic_state_vm, scale,
    wrapped_label,
};
use crate::app::UiCommandSink;
use crate::localization::{Language, LocalizedText, TextKey, default_route_owner, diagnostic_id};

/// One catalog string in the language this page was rendered with.
fn t(language: crate::localization::Language, key: crate::localization::TextKey) -> String {
    crate::localization::LocalizedText::new(language, key).text
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRowVm {
    pub id: DiagnosticCheckId,
    pub label: LocalizedText,
    pub state: DiagnosticStateVm,
    pub detail: Option<DisplayValue>,
    pub observation: Option<DisplayValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticsVm {
    pub title: LocalizedText,
    pub intro: LocalizedText,
    pub rows: Vec<DiagnosticRowVm>,
}

#[must_use]
pub fn diagnostics_vm(snapshot: &ControllerSnapshot, language: Language) -> DiagnosticsVm {
    diagnostics_vm_at(snapshot, language, SystemTime::now())
}

fn diagnostics_vm_at(
    snapshot: &ControllerSnapshot,
    language: Language,
    now: SystemTime,
) -> DiagnosticsVm {
    let rows = snapshot
        .diagnostics
        .iter()
        .map(|check| {
            diagnostic_row_vm(
                check,
                detail_for(check.id, snapshot, language),
                language,
                now,
            )
        })
        .collect();
    DiagnosticsVm {
        title: LocalizedText::new(language, TextKey::DiagnosticsTitle),
        intro: LocalizedText::new(language, TextKey::DiagnosticsIntro),
        rows,
    }
}

/// Evaluate time in the UI as well as the reducer: a delayed publication must not keep an old
/// passed result green. The original terminal conclusion and error detail remain available.
fn diagnostic_row_vm(
    check: &DiagnosticCheckSnapshot,
    detail: Option<DisplayValue>,
    language: Language,
    now: SystemTime,
) -> DiagnosticRowVm {
    let mut state = diagnostic_state_vm(&check.state, language);
    if check.freshness(now) == Freshness::Stale && check.state != DiagnosticCheckState::Expired {
        let mut expired = diagnostic_state_vm(&DiagnosticCheckState::Expired, language);
        expired.label.text = format!("{} · {}", expired.label.text, state.label.text);
        expired.detail = state.detail;
        state = expired;
    }
    let observation = if matches!(
        check.state,
        DiagnosticCheckState::Running { .. } | DiagnosticCheckState::Unexecuted { .. }
    ) {
        None
    } else {
        let age = check
            .observed_at
            .or(check.finished_at)
            .and_then(|at| now.duration_since(at).ok());
        Some(DisplayValue::new(match age {
            Some(age) => {
                crate::localization::format_text_in(
                    language,
                    TextKey::ObservedAgo,
                    &crate::localization::TextArgs::age(super::format_age(age, language).text),
                )
                .text
            }
            None => format!(
                "{} · {}",
                t(language, TextKey::FieldObservedAt),
                t(language, TextKey::ValueUnknown),
            ),
        }))
    };
    DiagnosticRowVm {
        id: check.id,
        label: LocalizedText::new(language, diagnostic_id(check.id)),
        state,
        detail,
        observation,
    }
}

fn detail_for(
    id: DiagnosticCheckId,
    snapshot: &ControllerSnapshot,
    language: Language,
) -> Option<DisplayValue> {
    let app = snapshot.app.as_ref();
    let network = app.network.as_ref();
    let cellular = app.cellular.as_ref();
    match id {
        DiagnosticCheckId::UsbDevice => app.device.as_ref().map_or_else(
            || {
                Some(DisplayValue::new(t(
                    language,
                    crate::localization::TextKey::ValueNotAvailable,
                )))
            },
            |device| {
                let problem = device.problem_code.map_or_else(
                    || t(language, crate::localization::TextKey::DiagNoProblemCode),
                    |code| {
                        crate::localization::format_positional(
                            language,
                            crate::localization::TextKey::DiagProblemCode,
                            &[&code.to_string()],
                        )
                    },
                );
                Some(DisplayValue::new(problem))
            },
        ),
        DiagnosticCheckId::AtControl => app
            .device
            .as_ref()
            .and_then(|device| {
                device.at_port.as_ref().map(|port| {
                    DisplayValue::new(crate::localization::format_positional(
                        language,
                        crate::localization::TextKey::DiagPort,
                        &[&port.to_string()],
                    ))
                })
            })
            .or_else(|| {
                Some(DisplayValue::new(t(
                    language,
                    crate::localization::TextKey::ValueNotAvailable,
                )))
            }),
        DiagnosticCheckId::Cellular => cellular.map_or_else(
            || {
                Some(DisplayValue::new(t(
                    language,
                    crate::localization::TextKey::ValueNotAvailable,
                )))
            },
            |cellular| {
                let carrier = cellular
                    .carrier
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map_or_else(
                        || t(language, crate::localization::TextKey::DiagCarrierNotRead),
                        str::to_owned,
                    );
                let rat = cellular
                    .radio_access_technology
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map_or_else(
                        || t(language, crate::localization::TextKey::DiagRatNotRead),
                        str::to_owned,
                    );
                Some(DisplayValue::new(format!(
                    "{} · {rat}",
                    carrier_display_name(&carrier, language)
                )))
            },
        ),
        DiagnosticCheckId::WindowsAdapter => network.map_or_else(
            || {
                Some(DisplayValue::new(t(
                    language,
                    crate::localization::TextKey::ValueNotAvailable,
                )))
            },
            |network| {
                Some(DisplayValue::copyable(preview_values(
                    &network.addresses,
                    language,
                )))
            },
        ),
        DiagnosticCheckId::BoundRoute => network.map_or_else(
            || {
                Some(DisplayValue::new(t(
                    language,
                    crate::localization::TextKey::ValueNotAvailable,
                )))
            },
            |network| {
                let gateways = preview_values(&network.gateways, language);
                Some(DisplayValue::new(crate::localization::format_positional(
                    language,
                    crate::localization::TextKey::DiagRouteProbeNote,
                    &[&gateways],
                )))
            },
        ),
        DiagnosticCheckId::BoundPublic => network.map_or_else(
            || {
                Some(DisplayValue::new(t(
                    language,
                    crate::localization::TextKey::ValueNotAvailable,
                )))
            },
            |network| {
                Some(DisplayValue::new(bound_public_detail(
                    network.bound_public,
                    language,
                )))
            },
        ),
        DiagnosticCheckId::BoundDns => network.map_or_else(
            || {
                Some(DisplayValue::new(t(
                    language,
                    crate::localization::TextKey::ValueNotAvailable,
                )))
            },
            |network| {
                Some(DisplayValue::new(bound_dns_detail(
                    network.bound_dns,
                    language,
                )))
            },
        ),
        DiagnosticCheckId::SystemRoute => network.map_or_else(
            || {
                Some(DisplayValue::new(t(
                    language,
                    crate::localization::TextKey::ValueNotAvailable,
                )))
            },
            |network| {
                let mut text =
                    LocalizedText::new(language, default_route_owner(network.system_default_route))
                        .text;
                if matches!(network.system_default_route, DefaultRouteOwner::VpnOrTun) {
                    text.push_str(&t(
                        language,
                        crate::localization::TextKey::DiagProxyInterfaceNote,
                    ));
                }
                Some(DisplayValue::new(text))
            },
        ),
        DiagnosticCheckId::Hotspot => Some(DisplayValue::new(
            super::hotspot_vm(app.hotspot, language).status.text,
        )),
    }
}

fn preview_values(values: &[String], language: Language) -> String {
    if values.is_empty() {
        return t(language, crate::localization::TextKey::ValueNotAvailable);
    }
    let output = values
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join("、");
    if values.len() > 2 {
        let more = (values.len() - 2).to_string();
        crate::localization::format_positional(
            language,
            crate::localization::TextKey::ValuePreviewMore,
            &[&output, &more],
        )
    } else {
        output
    }
}

fn bound_public_detail(value: BoundPublicStatus, language: Language) -> String {
    match value {
        BoundPublicStatus::Succeeded => t(language, crate::localization::TextKey::CheckPassed),
        BoundPublicStatus::Incomplete => t(language, crate::localization::TextKey::DiagIncomplete),
        BoundPublicStatus::Failed { consecutive_cycles } => {
            let count = consecutive_cycles.to_string();
            crate::localization::format_positional(
                language,
                crate::localization::TextKey::DiagFailedRepeatedly,
                &[&count],
            )
        }
    }
}

fn bound_dns_detail(value: BoundDnsStatus, language: Language) -> String {
    match value {
        BoundDnsStatus::Succeeded => t(language, crate::localization::TextKey::CheckPassed),
        BoundDnsStatus::Failed => t(language, crate::localization::TextKey::DiagResolutionFailed),
        BoundDnsStatus::Incomplete => t(language, crate::localization::TextKey::DiagIncomplete),
    }
}

pub(crate) fn render(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    language: Language,
    sink: &dyn UiCommandSink,
) {
    let vm = diagnostics_vm(snapshot, language);
    wrapped_label(
        ui,
        RichText::new(vm.intro.text.clone())
            .size(scale::RATE_AUX)
            .color(scale::secondary()),
    );
    ui.add_space(8.0);
    // The page's own action only. 刷新 is the panel-wide action and lives with the overview, next
    // to 更多; repeating it here put two identical 刷新 buttons on one screen.
    let mut export_requested = false;
    let export_label = TextKey::ButtonExportDiagnostics.to_string(language);
    if super::components::action_button(
        ui,
        &export_label,
        super::components::ButtonKind::Outlined,
        true,
        None,
    )
    .clicked()
    {
        export_requested = true;
    }
    if export_requested {
        let _ = sink.try_send(UiCommand::ExportDiagnostics);
    }
    ui.add_space(16.0);
    super::network_assistance::render(ui, snapshot, std::time::SystemTime::now(), language, sink);
    ui.add_space(12.0);
    super::section_frame(ui, |ui| {
        // Nine always-visible checks in two columns: a full-width single column left most of the
        // card empty, and the pairs read as a scan-friendly grid at desktop width.
        let half = vm.rows.len().div_ceil(2);
        let (left, right) = vm.rows.split_at(half);
        super::two_columns(ui, |ui| render_rows(ui, left), |ui| render_rows(ui, right));
    });
}

/// One column of checks. Every check is an always-visible row: the tone-coloured line is the row's
/// heading and its evidence is never hidden behind a disclosure triangle.
fn render_rows(ui: &mut Ui, rows: &[DiagnosticRowVm]) {
    for (index, row) in rows.iter().enumerate() {
        ui.label(
            super::section_heading(format!(
                "{}    {} {}",
                row.label.text,
                row.state.tone.marker(),
                row.state.label.text
            ))
            .color(row.state.tone.color()),
        );
        if let Some(detail) = &row.state.detail {
            wrapped_label(ui, super::detail_text(&detail.text));
        }
        if let Some(detail) = &row.detail {
            wrapped_label(ui, super::detail_text(&detail.text));
        }
        if let Some(observation) = &row.observation {
            wrapped_label(ui, super::meta_text(&observation.text));
        }
        if index + 1 < rows.len() {
            ui.separator();
        }
    }
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
    use dji4g_application::{FailureCode, RefreshCycleId, StableCode, UnexecutedReason};
    use dji4g_domain::{DeviceEpoch, ErrorCode};
    use std::time::Duration;

    fn check(state: DiagnosticCheckState) -> DiagnosticCheckSnapshot {
        DiagnosticCheckSnapshot {
            id: DiagnosticCheckId::BoundPublic,
            state,
            epoch: DeviceEpoch(1),
            started_at: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(900)),
            finished_at: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)),
            observed_at: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)),
            expires_at: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_030)),
        }
    }

    #[test]
    fn a_delayed_passed_snapshot_expires_using_the_render_clock() {
        let check = check(DiagnosticCheckState::Passed);
        let fresh = diagnostic_row_vm(
            &check,
            None,
            Language::EnUs,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_030),
        );
        assert_eq!(fresh.state.tone, super::super::StatusTone::Positive);
        assert_eq!(fresh.state.label.key, TextKey::CheckPassed);
        let stale = diagnostic_row_vm(
            &check,
            None,
            Language::EnUs,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_031),
        );
        assert_eq!(stale.state.tone, super::super::StatusTone::Caution);
        assert_eq!(stale.state.label.key, TextKey::CheckExpired);
        assert!(
            stale
                .state
                .label
                .text
                .starts_with(&t(Language::EnUs, TextKey::CheckExpired))
        );
        assert!(stale.state.label.text.contains(&fresh.state.label.text));
        assert_eq!(check.state, DiagnosticCheckState::Passed);
        assert_eq!(stale.observation.unwrap().text, "Updated 31 s ago");
    }

    #[test]
    fn stale_failed_results_keep_the_failure_and_evidence() {
        let check = check(DiagnosticCheckState::Failed {
            code: FailureCode::new(
                ErrorCode::Timeout,
                StableCode::try_from_static("probe:total_timeout").expect("safe code"),
            ),
        });
        let original = diagnostic_state_vm(&check.state, Language::ZhCn);
        let row = diagnostic_row_vm(
            &check,
            Some(DisplayValue::new("bound interface evidence")),
            Language::ZhCn,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_060),
        );
        assert_eq!(row.state.tone, super::super::StatusTone::Caution);
        assert_eq!(row.state.label.key, TextKey::CheckExpired);
        assert!(row.state.label.text.contains(&original.label.text));
        assert_eq!(row.state.detail, original.detail);
        assert_eq!(row.detail.unwrap().text, "bound interface evidence");
    }

    #[test]
    fn missing_or_future_observation_times_never_claim_a_recent_result() {
        for at in [
            None,
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(2_000)),
        ] {
            let mut check = check(DiagnosticCheckState::Passed);
            check.observed_at = at;
            check.finished_at = at;
            check.expires_at = None;
            let row = diagnostic_row_vm(
                &check,
                None,
                Language::ZhCn,
                SystemTime::UNIX_EPOCH + Duration::from_secs(1_060),
            );
            assert_eq!(row.state.tone, super::super::StatusTone::Caution);
            assert_eq!(row.observation.unwrap().text, "检测时间 · 未知");
        }
    }

    #[test]
    fn pending_checks_hide_old_times_and_completed_checks_fall_back_to_finished_at() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_060);
        for state in [
            DiagnosticCheckState::Running {
                cycle: RefreshCycleId(2),
            },
            DiagnosticCheckState::Unexecuted {
                reason: UnexecutedReason::NotScheduled,
            },
        ] {
            let row = diagnostic_row_vm(&check(state), None, Language::ZhCn, now);
            assert!(row.observation.is_none());
            assert_ne!(row.state.label.key, TextKey::CheckExpired);
        }
        let mut completed = check(DiagnosticCheckState::Expired);
        completed.observed_at = None;
        let row = diagnostic_row_vm(&completed, None, Language::ZhCn, now);
        assert_eq!(
            row.state.label.text,
            t(Language::ZhCn, TextKey::CheckExpired)
        );
        assert_eq!(row.observation.unwrap().text, "更新于 1 分钟前");
    }

    #[test]
    fn timed_diagnostic_rows_stay_inside_narrow_columns_in_every_language() {
        for width in [260.0, 360.0, 552.0, 760.0] {
            for language in [Language::ZhCn, Language::ZhTw, Language::EnUs] {
                let context = eframe::egui::Context::default();
                crate::ui::apply_style(&context);
                let viewport = eframe::egui::Rect::from_min_size(
                    eframe::egui::Pos2::ZERO,
                    eframe::egui::vec2(width, 1_200.0),
                );
                let rows: Vec<_> = DiagnosticCheckId::ORDERED
                    .into_iter()
                    .map(|id| {
                        let mut check = check(DiagnosticCheckState::Passed);
                        check.id = id;
                        diagnostic_row_vm(
                            &check,
                            None,
                            language,
                            SystemTime::UNIX_EPOCH + Duration::from_secs(1_060),
                        )
                    })
                    .collect();
                let _ = context.run(
                    eframe::egui::RawInput {
                        screen_rect: Some(viewport),
                        ..Default::default()
                    },
                    |context| {
                        eframe::egui::CentralPanel::default().show(context, |ui| {
                            render_rows(ui, &rows);
                            assert!(
                                ui.min_rect().right() <= viewport.right() + 0.5,
                                "diagnostic rows overflow: {width}, {language:?}, {:?}",
                                ui.min_rect(),
                            );
                        });
                    },
                );
            }
        }
    }
}
