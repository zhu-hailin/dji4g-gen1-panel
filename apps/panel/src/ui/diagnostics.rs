//! Layered evidence diagnostics. The fixed order mirrors the reducer's nine checks.

use dji4g_application::{ControllerSnapshot, DiagnosticCheckId, UiCommand};
use dji4g_domain::{BoundDnsStatus, BoundPublicStatus, DefaultRouteOwner};
use eframe::egui::{self, RichText, Ui};

use super::{
    DiagnosticStateVm, DisplayValue, carrier_display_name, diagnostic_state_vm, scale,
    wrapped_label,
};
use crate::app::UiCommandSink;
use crate::localization::{Language, LocalizedText, TextKey, default_route_owner, diagnostic_id};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRowVm {
    pub id: DiagnosticCheckId,
    pub label: LocalizedText,
    pub state: DiagnosticStateVm,
    pub detail: Option<DisplayValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticsVm {
    pub title: LocalizedText,
    pub intro: LocalizedText,
    pub rows: Vec<DiagnosticRowVm>,
}

#[must_use]
pub fn diagnostics_vm(snapshot: &ControllerSnapshot, language: Language) -> DiagnosticsVm {
    let rows = snapshot
        .diagnostics
        .iter()
        .map(|check| DiagnosticRowVm {
            id: check.id,
            label: LocalizedText::new(language, diagnostic_id(check.id)),
            state: diagnostic_state_vm(&check.state, language),
            detail: detail_for(check.id, snapshot, language),
        })
        .collect();
    DiagnosticsVm {
        title: LocalizedText::new(language, TextKey::DiagnosticsTitle),
        intro: LocalizedText::new(language, TextKey::DiagnosticsIntro),
        rows,
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
            || Some(DisplayValue::new("未获取")),
            |device| {
                let problem = device
                    .problem_code
                    .map_or_else(|| "无问题代码".into(), |code| format!("问题代码 {code}"));
                Some(DisplayValue::new(problem))
            },
        ),
        DiagnosticCheckId::AtControl => app
            .device
            .as_ref()
            .and_then(|device| {
                device
                    .at_port
                    .as_ref()
                    .map(|port| DisplayValue::new(format!("端口 {port}")))
            })
            .or_else(|| Some(DisplayValue::new("未获取"))),
        DiagnosticCheckId::Cellular => cellular.map_or_else(
            || Some(DisplayValue::new("未获取")),
            |cellular| {
                let carrier = cellular
                    .carrier
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("运营商未获取");
                let rat = cellular
                    .radio_access_technology
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("制式未获取");
                Some(DisplayValue::new(format!(
                    "{} · {rat}",
                    carrier_display_name(carrier)
                )))
            },
        ),
        DiagnosticCheckId::WindowsAdapter => network.map_or_else(
            || Some(DisplayValue::new("未获取")),
            |network| Some(DisplayValue::copyable(preview_values(&network.addresses))),
        ),
        DiagnosticCheckId::BoundGateway => network.map_or_else(
            || Some(DisplayValue::new("未获取")),
            |network| Some(DisplayValue::copyable(preview_values(&network.gateways))),
        ),
        DiagnosticCheckId::BoundPublic => network.map_or_else(
            || Some(DisplayValue::new("未获取")),
            |network| Some(DisplayValue::new(bound_public_detail(network.bound_public))),
        ),
        DiagnosticCheckId::BoundDns => network.map_or_else(
            || Some(DisplayValue::new("未获取")),
            |network| Some(DisplayValue::new(bound_dns_detail(network.bound_dns))),
        ),
        DiagnosticCheckId::SystemRoute => network.map_or_else(
            || Some(DisplayValue::new("未获取")),
            |network| {
                let mut text =
                    LocalizedText::new(language, default_route_owner(network.system_default_route))
                        .text;
                if matches!(network.system_default_route, DefaultRouteOwner::VpnOrTun) {
                    text.push_str("；仅用于解释路由竞争，不否定模块绑定探测");
                }
                Some(DisplayValue::new(text))
            },
        ),
        DiagnosticCheckId::Hotspot => Some(DisplayValue::new(
            super::hotspot_vm(app.hotspot, language).status.text,
        )),
    }
}

fn preview_values(values: &[String]) -> String {
    if values.is_empty() {
        return "未获取".into();
    }
    let output = values
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join("、");
    if values.len() > 2 {
        format!("{output}（另有 {} 项）", values.len() - 2)
    } else {
        output
    }
}

fn bound_public_detail(value: BoundPublicStatus) -> String {
    match value {
        BoundPublicStatus::Succeeded => "已通过".into(),
        BoundPublicStatus::Incomplete => "尚未完成".into(),
        BoundPublicStatus::Failed { consecutive_cycles } => {
            format!("未通过（连续 {consecutive_cycles} 次）")
        }
    }
}

fn bound_dns_detail(value: BoundDnsStatus) -> String {
    match value {
        BoundDnsStatus::Succeeded => "已通过".into(),
        BoundDnsStatus::Failed => "解析失败".into(),
        BoundDnsStatus::Incomplete => "尚未完成".into(),
    }
}

pub(crate) fn render(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    language: Language,
    sink: &dyn UiCommandSink,
) {
    let vm = diagnostics_vm(snapshot, language);
    ui.heading(vm.title.text.clone());
    wrapped_label(
        ui,
        RichText::new(vm.intro.text.clone())
            .size(scale::RATE_AUX)
            .color(scale::SECONDARY),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        // Refresh is the primary action on this page, so it carries the bold weight.
        if ui
            .button(RichText::new(TextKey::ButtonRefresh.to_string(language)).strong())
            .clicked()
        {
            let _ = sink.try_send(UiCommand::Refresh);
        }
        if ui
            .button(TextKey::ButtonExportDiagnostics.to_string(language))
            .clicked()
        {
            let _ = sink.try_send(UiCommand::ExportDiagnostics);
        }
    });
    ui.add_space(16.0);
    super::section_frame(ui, |ui| {
        for row in &vm.rows {
            egui::CollapsingHeader::new(
                RichText::new(format!(
                    "{}    {} {}",
                    row.label.text,
                    row.state.tone.marker(),
                    row.state.label.text
                ))
                .color(row.state.tone.color()),
            )
            .id_salt(format!("diagnostic-{:?}", row.id))
            .show(ui, |ui| {
                if let Some(detail) = &row.state.detail {
                    wrapped_label(ui, super::detail_text(&detail.text));
                }
                if let Some(detail) = &row.detail {
                    wrapped_label(ui, super::detail_text(&detail.text));
                }
            });
            ui.separator();
        }
    });
}

trait LocalizedKeyText {
    fn to_string(self, language: Language) -> String;
}

impl LocalizedKeyText for TextKey {
    fn to_string(self, language: Language) -> String {
        crate::localization::LocalizedText::new(language, self).text
    }
}
