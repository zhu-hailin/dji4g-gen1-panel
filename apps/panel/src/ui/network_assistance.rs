//! A small, evidence-limited guide for computer/agent network conflicts.

use std::time::SystemTime;

use dji4g_application::{ControllerSnapshot, HostNetworkPhase, UiCommand};
use dji4g_domain::{
    Availability, HostNetworkFinding, HostProxyMode, IpFamily, host_observation_is_fresh,
};
use eframe::egui::{self, RichText, Ui};

use crate::{app::UiCommandSink, localization::Language};

use super::{StatusTone, section_frame, wrapped_label};

fn copy(language: Language, zh: &'static str, en: &'static str) -> &'static str {
    match language {
        Language::ZhCn => zh,
        Language::EnUs => en,
    }
}

#[must_use]
pub fn brief(
    snapshot: &ControllerSnapshot,
    now: SystemTime,
    language: Language,
) -> (StatusTone, String) {
    let host = &snapshot.host_network;
    if matches!(
        host.phase,
        HostNetworkPhase::Checking
            | HostNetworkPhase::Preparing
            | HostNetworkPhase::Applying
            | HostNetworkPhase::Restoring
    ) {
        return (
            StatusTone::Progress,
            copy(
                language,
                "正在检查电脑网络与代理设置…",
                "Checking computer network and proxy settings…",
            )
            .into(),
        );
    }
    if host.phase == HostNetworkPhase::AwaitingRestart {
        return (
            StatusTone::Caution,
            copy(
                language,
                "代理配置已修改；重启代理后请重新检查。",
                "Proxy configuration changed. Restart the client, then check again.",
            )
            .into(),
        );
    }
    if let Some(code) = &host.error_code {
        return (
            StatusTone::Caution,
            format!(
                "{} ({code})",
                copy(
                    language,
                    "本次电脑网络检查或修复未完成",
                    "Computer network check or repair did not finish"
                )
            ),
        );
    }
    let Some(observation) = &host.observation else {
        return (
            StatusTone::Neutral,
            copy(
                language,
                "电脑网络与代理尚未检查。",
                "Computer network and proxy have not been checked.",
            )
            .into(),
        );
    };
    if !host_observation_is_fresh(observation.observed_at, now) {
        return (
            StatusTone::Caution,
            copy(
                language,
                "电脑网络信息已过期，请重新检查。",
                "Computer network information is stale. Check again.",
            )
            .into(),
        );
    }
    match host.finding {
        Some(HostNetworkFinding::MissingBoundInterface) => {
            let module = if snapshot.app.availability == Availability::Available {
                copy(language, "模块连接正常；", "The module connection passed; ")
            } else {
                ""
            };
            (
                StatusTone::Negative,
                format!(
                    "{module}{}",
                    copy(
                        language,
                        "代理软件指定的出口网卡已不存在。",
                        "the proxy client references a missing network adapter."
                    )
                ),
            )
        }
        Some(HostNetworkFinding::BoundInterfaceDown) => (
            StatusTone::Caution,
            copy(
                language,
                "代理指定的网卡仍在电脑上，但当前未连接或已禁用。",
                "The proxy-bound adapter exists but is down or disabled.",
            )
            .into(),
        ),
        Some(HostNetworkFinding::BindingAmbiguous | HostNetworkFinding::EvidenceIncomplete) => (
            StatusTone::Caution,
            copy(
                language,
                "暂时无法确定代理出口问题，请查看处理步骤。",
                "The proxy outlet could not be determined; see guidance.",
            )
            .into(),
        ),
        _ => {
            let tun = snapshot.app.network.as_ref().is_some_and(|network| {
                matches!(
                    network.system_default_route,
                    dji4g_domain::DefaultRouteOwner::VpnOrTun
                )
            });
            if tun {
                (StatusTone::Neutral, copy(language, "电脑正在通过代理或 VPN 接口联网；模块通路单独检查。", "A proxy or VPN interface is in use; the module path is checked separately.").into())
            } else {
                (
                    StatusTone::Neutral,
                    copy(
                        language,
                        "未发现已知的固定出口网卡问题。",
                        "No known fixed-outlet adapter problem was found.",
                    )
                    .into(),
                )
            }
        }
    }
}

pub(crate) fn render_brief(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    now: SystemTime,
    language: Language,
) -> bool {
    let (tone, message) = brief(snapshot, now, language);
    let mut open = false;
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(tone.color(), format!("{} {message}", tone.marker()));
        if ui
            .button(copy(language, "电脑网络详情", "Computer network details"))
            .clicked()
        {
            open = true;
        }
    });
    open
}

fn send(ui: &mut Ui, sink: &dyn UiCommandSink, command: UiCommand) {
    let id = egui::Id::new("host-network-send-error");
    match sink.try_send(command) {
        Ok(()) => ui.data_mut(|data| data.remove::<String>(id)),
        Err(_) => ui.data_mut(|data| data.insert_temp(id, "命令未提交，请稍后重试。".to_owned())),
    }
}

pub(crate) fn render(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    now: SystemTime,
    language: Language,
    sink: &dyn UiCommandSink,
) {
    let host = &snapshot.host_network;
    section_frame(ui, |ui| {
        ui.heading(copy(
            language,
            "电脑网络与代理",
            "Computer network and proxy",
        ));
        let (tone, message) = brief(snapshot, now, language);
        wrapped_label(
            ui,
            RichText::new(format!("{} {message}", tone.marker())).color(tone.color()),
        );
        if snapshot.app.device.is_none() {
            wrapped_label(
                ui,
                copy(
                    language,
                    "未连接模块，仍可检查电脑网络与代理设置。",
                    "No module is connected. Computer and proxy checks remain available.",
                ),
            );
        }
        ui.horizontal_wrapped(|ui| {
            let busy = matches!(
                host.phase,
                HostNetworkPhase::Checking
                    | HostNetworkPhase::Preparing
                    | HostNetworkPhase::Applying
                    | HostNetworkPhase::Restoring
            );
            if ui
                .add_enabled(
                    !busy,
                    egui::Button::new(copy(language, "重新检查", "Check again")),
                )
                .clicked()
            {
                send(ui, sink, UiCommand::InspectHostNetwork);
            }
            if host.finding == Some(HostNetworkFinding::MissingBoundInterface)
                && host
                    .observation
                    .as_ref()
                    .is_some_and(|value| host_observation_is_fresh(value.observed_at, now))
                && host
                    .observation
                    .as_ref()
                    .and_then(|value| value.binding.as_ref())
                    .is_some_and(|binding| binding.repairable)
                && host.phase == HostNetworkPhase::Ready
                && let Some(finding_id) = host.finding_id
                && ui
                    .button(copy(language, "查看修复方案", "Review repair"))
                    .clicked()
            {
                send(ui, sink, UiCommand::PrepareProxyRepair { finding_id });
            }
        });
        if let Some(error) =
            ui.data(|data| data.get_temp::<String>(egui::Id::new("host-network-send-error")))
        {
            wrapped_label(ui, RichText::new(error).color(StatusTone::Negative.color()));
        }
        if host.preview.is_none()
            && host.result.is_none()
            && let Some(observation) = &host.observation
        {
            ui.separator();
            let mode = match observation.system_proxy {
                HostProxyMode::Disabled => copy(language, "未开启", "Off"),
                HostProxyMode::Manual => copy(language, "手动代理", "Manual"),
                HostProxyMode::AutoConfig => copy(
                    language,
                    "自动配置脚本；未执行脚本",
                    "Automatic script; script not executed",
                ),
                HostProxyMode::AutoDetect => {
                    copy(language, "自动检测；尚未验证", "Auto-detect; not verified")
                }
                HostProxyMode::Mixed => copy(
                    language,
                    "多种代理设置；需进一步确认",
                    "Multiple proxy settings; needs review",
                ),
                HostProxyMode::Unknown => copy(language, "未获取", "Unavailable"),
            };
            wrapped_label(
                ui,
                format!(
                    "{}：{mode}",
                    copy(language, "Windows 系统代理", "Windows system proxy")
                ),
            );
            for family in [IpFamily::V4, IpFamily::V6] {
                let aliases = observation
                    .default_routes
                    .iter()
                    .filter(|route| route.family == family)
                    .filter_map(|route| {
                        observation
                            .adapters
                            .iter()
                            .find(|adapter| adapter.luid == route.luid)
                    })
                    .map(|adapter| adapter.alias.as_str())
                    .collect::<Vec<_>>();
                if !aliases.is_empty() {
                    wrapped_label(
                        ui,
                        format!(
                            "{}：{}",
                            if family == IpFamily::V4 {
                                "IPv4"
                            } else {
                                "IPv6"
                            },
                            aliases.join("、")
                        ),
                    );
                }
            }
            if let Some(binding) = &observation.binding {
                wrapped_label(
                    ui,
                    format!(
                        "{}：{}",
                        copy(language, "代理指定网卡", "Proxy-bound adapter"),
                        binding.interface_alias
                    ),
                );
                wrapped_label(
                    ui,
                    format!(
                        "Clash Verge Rev {}",
                        binding.version.as_deref().unwrap_or("版本未确认")
                    ),
                );
                if !binding.repairable {
                    wrapped_label(
                        ui,
                        copy(
                            language,
                            "此配置或版本暂不支持自动修改，请在代理软件中检查出站接口设置。",
                            "This configuration cannot be changed automatically. Review the outbound interface in the proxy client.",
                        ),
                    );
                }
            }
            if let Some(code) = &observation.proxy_error_code {
                wrapped_label(
                    ui,
                    format!(
                        "{} ({code})",
                        copy(
                            language,
                            "代理配置无法可靠读取",
                            "Proxy configuration could not be read reliably"
                        )
                    ),
                );
            }
        }
        if let Some(preview) = &host.preview {
            ui.separator();
            wrapped_label(ui, "Clash Verge Rev v2.5.5");
            wrapped_label(
                ui,
                format!(
                    "{}“{}”{}",
                    copy(
                        language,
                        "将取消代理软件对",
                        "Remove the proxy client's fixed outlet for "
                    ),
                    preview.interface_alias,
                    copy(
                        language,
                        "的固定出口设置。之后可能使用 Wi-Fi、有线网络或其他可用出口。将先备份原配置。请先完整退出 Clash Verge Rev 及相关核心。",
                        ". It may then use Wi-Fi, Ethernet or another available outlet. The original will be backed up. Exit Clash Verge Rev and its core first."
                    )
                ),
            );
            let expired = now > preview.expires_at;
            if expired {
                wrapped_label(
                    ui,
                    RichText::new(copy(
                        language,
                        "修复方案已过期，请重新检查后再查看方案。",
                        "This repair plan expired. Check again before preparing a new plan.",
                    ))
                    .color(StatusTone::Caution.color()),
                );
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(copy(language, "保留原设置", "Keep original settings"))
                    .clicked()
                {
                    send(
                        ui,
                        sink,
                        UiCommand::CancelProxyRepair {
                            plan_id: preview.plan_id,
                        },
                    );
                }
                if ui
                    .add_enabled(
                        !expired,
                        egui::Button::new(copy(language, "备份并修复", "Back up and repair")),
                    )
                    .clicked()
                {
                    send(
                        ui,
                        sink,
                        UiCommand::ConfirmProxyRepair {
                            plan_id: preview.plan_id,
                        },
                    );
                }
            });
        }
        if let Some(result) = &host.result {
            wrapped_label(
                ui,
                copy(
                    language,
                    "配置已修改。重启代理后点击“重新检查”；配置改动不等于互联网已经恢复。",
                    "Configuration changed. Restart the proxy and check again; this does not prove Internet access.",
                ),
            );
            if ui
                .button(copy(
                    language,
                    "恢复原配置",
                    "Restore original configuration",
                ))
                .clicked()
            {
                send(
                    ui,
                    sink,
                    UiCommand::RestoreProxyRepair {
                        backup_id: result.backup_id,
                    },
                );
            }
        }
        egui::CollapsingHeader::new(copy(language, "查看处理步骤", "Troubleshooting steps")).show(ui, |ui| {
            wrapped_label(ui, copy(language, "先检查模块与 SIM；若模块公网检查通过，但代理仍报找不到网卡，请在代理软件中检查“出站接口”是否指向已移除或改名的网卡。多网卡用户可能有意固定出口，修改前确认用途。配置来源不明确时，请在代理软件中手动调整。", "Check the module and SIM. If the module path passes but the proxy reports a missing interface, review its outbound-interface setting. A fixed outlet may be intentional on computers with multiple adapters. Use the proxy client to edit ambiguous configurations."));
        });
    });
}
