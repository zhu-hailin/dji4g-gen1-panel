//! One explicit, module-bound check. Host routing is explanatory only.
use super::StatusTone;
use crate::{app::PanelCommandSink, localization::Language};
use dji4g_application::{
    ControllerSnapshot, DefaultRouteDto, ModuleNetworkCheckPhase as Phase,
    ModuleNetworkCheckSnapshot, NetworkRepairKind, UiCommand,
};
use dji4g_domain::{ModuleNetworkVerdict as Verdict, NetworkEvidenceState as Evidence};
use eframe::egui::{self, RichText};
use std::time::SystemTime;

#[must_use]
pub fn conclusion(check: &ModuleNetworkCheckSnapshot) -> (StatusTone, &'static str) {
    match check.phase {
        Phase::Queued => (StatusTone::Progress, "已排队，等待当前任务结束后检查"),
        Phase::Running => (StatusTone::Progress, "正在检查模块网络…"),
        Phase::Stale => (StatusTone::Neutral, "结果已过期或设备已变化，请重新检查"),
        Phase::Finished => match check.verdict {
            Verdict::Usable => (
                StatusTone::Positive,
                "模块网络可用：本次公网与 DNS 验证通过",
            ),
            Verdict::DeviceMissing => {
                (StatusTone::Caution, "未识别到模块，请检查数据线和 USB 接口")
            }
            Verdict::AdapterIssue => (StatusTone::Caution, "模块网卡检查未通过，请查看具体证据"),
            Verdict::AddressRouteIssue => (StatusTone::Caution, "已识别网卡，但缺少可用地址或路由"),
            Verdict::LinkDown => (
                StatusTone::Caution,
                "模块网卡链路未连接，请检查 USB 连接与设备状态",
            ),
            Verdict::GatewayIssue => (
                StatusTone::Caution,
                "模块网关测试未通过，请查看地址与网关配置",
            ),
            Verdict::PublicProbeFailed => (
                StatusTone::Caution,
                "模块公网测试未通过，请查看蜂窝与公网证据",
            ),
            Verdict::DnsIssue => (StatusTone::Caution, "模块公网可达，但 DNS 解析未通过"),
            Verdict::Inconclusive => (
                StatusTone::Neutral,
                "尚不能判断模块能否上网，请查看未完成的检查",
            ),
        },
    }
}
fn evidence_label(state: Evidence) -> &'static str {
    match state {
        Evidence::NotRun => "未执行",
        Evidence::Running => "检查中",
        Evidence::Passed => "通过",
        Evidence::Failed => "未通过",
        Evidence::Unavailable => "无法获取",
        Evidence::Stale => "已过期",
    }
}
fn repair_label(kind: NetworkRepairKind) -> &'static str {
    match kind {
        NetworkRepairKind::RenewDhcp => "更新模块网卡 DHCP 租约",
        NetworkRepairKind::RestartAdapter => "重启模块网卡",
        NetworkRepairKind::AutomaticDns => "恢复自动 DNS",
    }
}

/// Returns true when the user asks to open the existing driver/repair guidance.
pub fn render(
    ui: &mut egui::Ui,
    snapshot: &ControllerSnapshot,
    now: SystemTime,
    language: Language,
    sink: &dyn PanelCommandSink,
) -> bool {
    let mut guidance = false;
    let confirm_id = egui::Id::new("module-network-probe-consent");
    let error_id = egui::Id::new("module-network-submit-error");
    let mut consent = ui
        .ctx()
        .data_mut(|d| d.get_temp::<bool>(confirm_id).unwrap_or(false));
    let mut error = ui
        .ctx()
        .data_mut(|d| d.get_temp::<bool>(error_id).unwrap_or(false));
    let active = snapshot
        .module_network_check
        .as_ref()
        .is_some_and(|c| c.active());
    super::section_frame(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(super::section_heading("模块网络检查"));
            if ui
                .add_enabled(
                    !active,
                    egui::Button::new("检查模块网络").min_size(egui::vec2(0.0, 32.0)),
                )
                .clicked()
            {
                if snapshot.settings.active_probe {
                    error = sink
                        .try_send(UiCommand::CheckModuleNetwork {
                            allow_probe_once: false,
                        })
                        .is_err();
                } else {
                    consent = true;
                }
            }
            if active {
                ui.spinner();
            }
        });
        if let Some(check) = snapshot.module_network_check.as_ref() {
            let (tone, text) = conclusion(check);
            super::wrapped_label(
                ui,
                RichText::new(format!("{} {text}", tone.marker())).color(tone.color()),
            );
            if let Some(outcome) = &check.operation_outcome {
                super::wrapped_label(
                    ui,
                    format!(
                        "操作结果：{}",
                        super::operation_outcome_text(outcome, language).text
                    ),
                );
                ui.label("下方为操作后的只读复检；不会自动重复修复。");
            }
            if check.phase == Phase::Running {
                let current = if check.evidence.device != Evidence::Passed {
                    "识别 USB 模块"
                } else if check.adapter.is_none() {
                    "读取串口、蜂窝与模块网卡"
                } else {
                    "验证模块网关、公网、DNS 与电脑出口"
                };
                super::wrapped_label(ui, format!("当前步骤：{current}"));
            }
            for repair in check.recommended_repairs() {
                let gate = super::action_availability::repair_action_availability(
                    snapshot,
                    repair.readiness_key(),
                    now,
                    language,
                );
                let enabled = check.fresh(
                    snapshot
                        .app
                        .device
                        .as_ref()
                        .map_or(dji4g_domain::DeviceEpoch(0), |d| d.epoch),
                    now,
                ) && gate.enabled;
                if ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(repair_label(repair)).min_size(egui::vec2(0.0, 32.0)),
                    )
                    .clicked()
                {
                    sink.prepare_network_repair_now(check.request_id, repair);
                }
                if !enabled {
                    super::wrapped_label(
                        ui,
                        super::meta_text(
                            gate.reason
                                .map(|r| r.text)
                                .unwrap_or_else(|| "检查结果已过期，请重新检查".into()),
                        ),
                    );
                }
            }
            if check.phase == Phase::Finished
                && matches!(check.verdict, Verdict::AdapterIssue | Verdict::Inconclusive)
                && check.evidence.device == Evidence::Passed
                && check.adapter.is_none()
            {
                super::wrapped_label(
                    ui,
                    "模块已识别，但尚未核实网卡接口。驱动、USB 网络模式或读取失败都可能有关。",
                );
                if ui
                    .add(
                        egui::Button::new("查看驱动与接口检查步骤").min_size(egui::vec2(0.0, 32.0)),
                    )
                    .clicked()
                {
                    guidance = true;
                }
            }
            if check.verdict == Verdict::PublicProbeFailed {
                super::wrapped_label(
                    ui,
                    "请确认 SIM 卡可用、蜂窝注册与套餐状态。一次超时不能说明驱动损坏。",
                );
            }
            if check.phase == Phase::Finished && check.evidence.public == Evidence::NotRun {
                super::wrapped_label(
                    ui,
                    "本次未验证公网连接。点击检查并允许一次联网探测，长期设置保持不变。",
                );
            }
            if let Some(probe) = check.probe.as_ref() {
                for route in &probe.route_choices {
                    let description = match route.owner {
                        Some(DefaultRouteDto::TargetAdapter) => "选择模块网卡",
                        Some(DefaultRouteDto::VpnOrTun) => "选择代理或 VPN；这本身不是故障",
                        Some(_) => "选择其他网卡（例如 Wi-Fi / 有线）；这本身不是故障",
                        None => "无法确认出口",
                    };
                    super::wrapped_label(
                        ui,
                        super::meta_text(format!(
                            "{:?} 对本次测试目标的路径：{description}。",
                            route.family
                        )),
                    );
                }
            }
            egui::CollapsingHeader::new("查看本轮证据与处理说明").id_salt("module-network-evidence").show(ui, |ui| {
                for (name, state) in [("USB 模块", check.evidence.device), ("网卡读取", check.evidence.adapter), ("链路", check.evidence.link),
                    ("地址与路由", check.evidence.address_route),
                    ("模块网关", check.evidence.gateway), ("模块公网", check.evidence.public), ("模块 DNS", check.evidence.dns)] {
                    ui.label(format!("{name}：{}", evidence_label(if check.phase == Phase::Stale { Evidence::Stale } else { state })));
                }
                if let Some(adapter) = check.adapter.as_ref() {
                    if let Some(details) = adapter.details.as_ref() {
                        ui.label(format!("链路：{}；IPv4 DHCP：{}", if details.link_up { "已连接" } else { "未连接" }, if details.dhcp_v4 { "启用" } else { "未启用" }));
                    }
                    ui.label(format!("可用协议：IPv4 {} / IPv6 {}", adapter.ipv4, adapter.ipv6));
                    if adapter.details.as_ref().is_some_and(|d| !d.dhcp_v4) { super::wrapped_label(ui, "检测到非 DHCP 配置，不会自动覆盖静态地址。请确认原有网络设置。"); }
                }
                for id in [dji4g_application::DiagnosticCheckId::AtControl, dji4g_application::DiagnosticCheckId::Cellular] {
                    let state = &check.diagnostics.get(id).state;
                    super::wrapped_label(ui, format!("{}：{}", if id == dji4g_application::DiagnosticCheckId::AtControl { "串口通信" } else { "SIM 与蜂窝" }, crate::localization::LocalizedText::new(language, crate::localization::diagnostic_state(state)).text));
                }
                super::wrapped_label(ui, super::meta_text("公网探测绑定模块网卡；电脑路径只代表本次固定测试目标，不能代表所有应用。未执行、无法获取和过期均不等于故障。"));
            });
        } else {
            super::wrapped_label(
                ui,
                "单独检查模块网卡能否上网，并说明电脑对测试目标选择的出口。",
            );
        }
        if consent {
            super::wrapped_label(
                ui,
                "后台联网探测已关闭。本次检查将向内置固定验证端点发送少量请求（公网与 DNS），不会修改长期设置。",
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(egui::Button::new("取消").min_size(egui::vec2(0.0, 32.0)))
                    .clicked()
                {
                    consent = false;
                }
                if ui
                    .add(egui::Button::new("仅检查本地信息").min_size(egui::vec2(0.0, 32.0)))
                    .clicked()
                {
                    error = sink
                        .try_send(UiCommand::CheckModuleNetwork {
                            allow_probe_once: false,
                        })
                        .is_err();
                    if !error {
                        consent = false;
                    }
                }
                if ui
                    .add(egui::Button::new("允许本次联网检查").min_size(egui::vec2(0.0, 32.0)))
                    .clicked()
                {
                    error = sink
                        .try_send(UiCommand::CheckModuleNetwork {
                            allow_probe_once: true,
                        })
                        .is_err();
                    if !error {
                        consent = false;
                    }
                }
            });
        }
        if error {
            super::wrapped_label(
                ui,
                RichText::new("检查请求未提交，请稍后重试。").color(StatusTone::Caution.color()),
            );
        }
    });
    ui.ctx().data_mut(|d| {
        d.insert_temp(confirm_id, consent);
        d.insert_temp(error_id, error);
    });
    guidance
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_and_stale_never_display_saved_success_as_current() {
        let mut check = ModuleNetworkCheckSnapshot::queued(1, dji4g_domain::DeviceEpoch(1), false);
        check.verdict = Verdict::Usable;
        assert!(conclusion(&check).1.contains("排队"));
        check.phase = Phase::Stale;
        assert!(conclusion(&check).1.contains("过期"));
        check.phase = Phase::Finished;
        check.verdict = Verdict::DnsIssue;
        assert!(conclusion(&check).1.contains("公网可达"));
        assert!(conclusion(&check).1.contains("DNS"));
    }
}
