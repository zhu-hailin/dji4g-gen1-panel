//! Safe, confirmation-first repair controls. This module only emits closed UiCommand values.

use std::time::SystemTime;

use dji4g_application::{ActionReadinessKey, ControlledRepairRequest, ControllerSnapshot};
use dji4g_domain::{ActionKind, DnsProfile, Freshness, HotspotStatus};
use eframe::egui::{self, RichText, Ui};
use std::net::{IpAddr, Ipv4Addr};

use super::{field_label, meta_text, scale, section_frame, section_heading, wrapped_label};
use crate::app::PanelCommandSink;
use crate::localization::{Language, LocalizedText, TextKey, failure_text};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairActionVm {
    pub action: ActionKind,
    pub title: LocalizedText,
    pub enabled: bool,
    /// Honest reason a disabled action cannot be prepared, shown as the button tooltip.
    pub disabled_reason: Option<LocalizedText>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairsVm {
    pub title: LocalizedText,
    pub notice: LocalizedText,
    pub driver_notice: LocalizedText,
    pub actions: Vec<RepairActionVm>,
}

#[must_use]
pub fn repairs_vm(snapshot: &ControllerSnapshot, now: SystemTime, language: Language) -> RepairsVm {
    let app = snapshot.app.as_ref();
    // While an operation is running the controller refuses new prepares (PrepareError::Busy);
    // reflect that honestly instead of leaving clickable buttons that silently no-op.
    let operation_running = matches!(
        snapshot
            .operation
            .as_ref()
            .map(|operation| &operation.state),
        Some(dji4g_application::OperationState::Running { .. })
    );
    let operation_running = operation_running
        || snapshot
            .sms_send
            .as_ref()
            .is_some_and(|send| send.phase != dji4g_application::SmsSendPhase::Finished);
    let target_available = app.device.is_some()
        && app.freshness == Freshness::Fresh
        && !matches!(
            app.availability,
            dji4g_domain::Availability::NotDetected | dji4g_domain::Availability::UnsupportedDevice
        );
    let readiness_of = |key: ActionReadinessKey| -> bool {
        snapshot
            .action_readiness
            .iter()
            .find(|entry| entry.key == key)
            .is_some_and(|entry| entry.ready.is_ok())
    };
    let readiness_reason = |key: ActionReadinessKey| -> Option<LocalizedText> {
        snapshot
            .action_readiness
            .iter()
            .find(|entry| entry.key == key)
            .and_then(|entry| entry.ready.as_ref().err())
            .map(|code| failure_text(code, language))
    };
    let base_enabled = target_available && !operation_running;
    // One shared builder so every action gets the same readiness-derived enablement and reason.
    let build = |action: ActionKind, key: ActionReadinessKey, _title: TextKey| {
        let ready = readiness_of(key);
        let reason = if operation_running {
            Some(LocalizedText::new(language, TextKey::CommandFeedbackBusy))
        } else if app.device.is_none() {
            Some(LocalizedText::new(
                language,
                TextKey::AvailabilityNotDetectedReason,
            ))
        } else if app.freshness != Freshness::Fresh {
            Some(LocalizedText::new(language, TextKey::ErrorEvidenceExpired))
        } else if ready {
            None
        } else {
            readiness_reason(key)
        };
        let title = crate::localization::action_text(&action, language);
        RepairActionVm {
            action,
            title,
            enabled: base_enabled && ready,
            disabled_reason: reason,
        }
    };
    let hotspot_enabled = matches!(
        app.hotspot,
        HotspotStatus::Off
            | HotspotStatus::On { .. }
            | HotspotStatus::Starting
            | HotspotStatus::Stopping
    );
    let mut actions = vec![
        build(
            ActionKind::RenewDhcp,
            ActionReadinessKey::RenewDhcp,
            TextKey::ActionRenewDhcp,
        ),
        build(
            ActionKind::ApplyDnsProfile {
                profile: DnsProfile::Automatic,
            },
            ActionReadinessKey::ApplyDnsProfile,
            TextKey::ActionApplyDnsAutomatic,
        ),
        build(
            ActionKind::ApplyDnsProfile {
                profile: DnsProfile::Static {
                    servers: vec![
                        IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
                        IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
                    ],
                },
            },
            ActionReadinessKey::ApplyDnsProfile,
            TextKey::ActionApplyDnsStatic,
        ),
        build(
            ActionKind::ToggleHotspot {
                enabled: !matches!(
                    app.hotspot,
                    HotspotStatus::On { .. } | HotspotStatus::Starting
                ),
            },
            ActionReadinessKey::ToggleHotspot,
            if matches!(
                app.hotspot,
                HotspotStatus::On { .. } | HotspotStatus::Starting
            ) {
                TextKey::ActionDisableHotspot
            } else {
                TextKey::ActionEnableHotspot
            },
        ),
        build(
            ActionKind::SetVerifiedUsbNetworkProfile {
                profile: dji4g_domain::UsbNetworkProfile::DjiNdis,
            },
            ActionReadinessKey::SetUsbNetworkProfile,
            TextKey::ActionSetUsbProfileDjiNdis,
        ),
        build(
            ActionKind::RestartAdapter,
            ActionReadinessKey::RestartAdapter,
            TextKey::ActionRestartAdapter,
        ),
        build(
            ActionKind::EditApn {
                cid: 1,
                apn: String::new(),
            },
            ActionReadinessKey::EditApn,
            TextKey::ActionEditApn,
        ),
        build(
            ActionKind::ReenumerateDevice,
            ActionReadinessKey::ReenumerateDevice,
            TextKey::ActionReenumerateDevice,
        ),
        build(
            ActionKind::RestartModule,
            ActionReadinessKey::RestartModule,
            TextKey::ActionRestartModule,
        ),
        build(
            ActionKind::SetVerifiedUsbNetworkProfile {
                profile: dji4g_domain::UsbNetworkProfile::Ecm,
            },
            ActionReadinessKey::SetUsbNetworkProfile,
            TextKey::ActionSetUsbProfileEcm,
        ),
    ];
    // The hotspot toggle's own extra gate: only a resolvable hotspot state can be toggled.
    if let Some(action) = actions
        .iter_mut()
        .find(|entry| matches!(entry.action, ActionKind::ToggleHotspot { .. }))
    {
        action.enabled = action.enabled && hotspot_enabled && !operation_running;
    }
    if !target_available || operation_running {
        actions.iter_mut().for_each(|action| action.enabled = false);
    }
    let _ = now;
    RepairsVm {
        title: LocalizedText::new(language, TextKey::RepairsTitle),
        notice: LocalizedText::new(language, TextKey::RepairsReadOnlyNotice),
        driver_notice: LocalizedText::new(language, TextKey::RepairsDriverNotIncluded),
        actions,
    }
}

pub(crate) fn render(
    ui: &mut Ui,
    snapshot: &ControllerSnapshot,
    _now: SystemTime,
    language: Language,
    sink: &dyn PanelCommandSink,
) {
    let vm = repairs_vm(snapshot, SystemTime::now(), language);
    ui.heading(vm.title.text.clone());
    wrapped_label(
        ui,
        RichText::new(vm.notice.text.clone())
            .size(scale::RATE_AUX)
            .color(scale::SECONDARY),
    );
    wrapped_label(ui, meta_text(vm.driver_notice.text.clone()));
    let (usb_actions, other_actions): (Vec<_>, Vec<_>) =
        vm.actions.into_iter().partition(|action| {
            matches!(
                action.action,
                ActionKind::SetVerifiedUsbNetworkProfile { .. }
            )
        });
    section_frame(ui, |ui| {
        ui.label(section_heading("电脑网卡模式"));
        ui.hyperlink_to("查看大疆官方使用说明", "https://dl.djicdn.com/downloads/DJI_Mavic_3/DJI_Cellular_Dongle_LTE_USB_Modem_User_Guide_v1.0.pdf");
        wrapped_label(
            ui,
            "部分一代模块保留原厂固件即可用作电脑网卡。先检查驱动和当前网络状态；已经能上网时无需切换。",
        );
        wrapped_label(
            ui,
            meta_text(
                "下方操作只切换 USB 网络配置，不刷写固件。DJI NDIS 配置需要匹配的 Windows 驱动；ECM 配置的兼容性取决于系统与驱动。",
            ),
        );
        wrapped_label(
            ui,
            meta_text(
                "切换会中断连接并可能重新枚举设备。仅适用于本项目已验证的设备配置；AT 端口不可用时，请先处理驱动问题。",
            ),
        );
        for action in usb_actions {
            render_action_button(ui, action, language, sink);
        }
    });
    // No nested scroll area: the shell already scrolls the page, so the grouped actions share
    // one scrollbar instead of fighting each other for height. Sections follow the domain risk
    // metadata, never a list index, so a high-risk action can never hide in the low-risk group.
    let (low_risk, interrupting): (Vec<_>, Vec<_>) = other_actions
        .into_iter()
        .partition(|action| !is_interrupting(&action.action));
    section_frame(ui, |ui| {
        ui.label(section_heading("低风险与网络恢复"));
        ui.vertical(|ui| {
            // Even gaps in both axes so a wrapped group still reads as an aligned block.
            ui.spacing_mut().item_spacing =
                egui::vec2(scale::CONTROL_GAP[0], scale::CONTROL_GAP[1]);
            for action in low_risk {
                render_action_button(ui, action, language, sink);
            }
        });
    });
    section_frame(ui, |ui| {
        ui.label(section_heading("会中断连接"));
        for action in &interrupting {
            if matches!(action.action, ActionKind::EditApn { .. }) {
                render_apn_editor(ui, action.clone(), language, sink);
            }
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing =
                egui::vec2(scale::CONTROL_GAP[0], scale::CONTROL_GAP[1]);
            for action in interrupting {
                if matches!(action.action, ActionKind::EditApn { .. }) {
                    continue;
                }
                render_action_button(ui, action, language, sink);
            }
        });
    });
}

fn is_interrupting(action: &ActionKind) -> bool {
    matches!(
        dji4g_application::action_disruption(action),
        Some(
            dji4g_domain::DisruptionLevel::ConnectionInterrupting
                | dji4g_domain::DisruptionLevel::DeviceReenumeration
        )
    )
}

/// One repair button: disabled buttons carry the precise prerequisite reason as a tooltip so a
/// silent no-op is impossible.
fn render_action_button(
    ui: &mut Ui,
    action: RepairActionVm,
    _language: Language,
    sink: &dyn PanelCommandSink,
) {
    let button = ui.add_enabled(action.enabled, egui::Button::new(action.title.text.clone()));
    if !action.enabled {
        if let Some(reason) = action.disabled_reason {
            button.clone().on_hover_text(reason.text.clone());
            wrapped_label(ui, meta_text(reason.text));
        }
    }
    if button.clicked() {
        if let Ok(request) = ControlledRepairRequest::try_from_action(action.action.clone()) {
            sink.prepare_repair_now(request);
        }
    }
}

/// Keep the editable APN/CID only in egui's transient UI memory.  It is converted to the typed
/// application request at the button edge and is never copied into a controller snapshot or an
/// audit/debug value.  A fixed CID default is intentional: the platform still proves that the
/// selected context exists, is complete, and is inactive before any write.
fn render_apn_editor(
    ui: &mut Ui,
    action: RepairActionVm,
    language: Language,
    sink: &dyn PanelCommandSink,
) {
    let cid_id = egui::Id::new("repair.apn.cid");
    let apn_id = egui::Id::new("repair.apn.value");
    let mut cid = ui.data(|data| data.get_temp::<u8>(cid_id)).unwrap_or(1);
    let mut value = ui
        .data(|data| data.get_temp::<String>(apn_id))
        .unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.label(field_label("PDP 上下文"));
        ui.add(egui::DragValue::new(&mut cid).range(1..=16));
        ui.label(field_label("新 APN"));
        // Height matches the neighbouring buttons so the input row does not read as shorter
        // than the controls beside it; the row wraps, so the width cannot overflow.
        ui.add_sized(
            [160.0, 26.0],
            egui::TextEdit::singleline(&mut value).password(true),
        );
    });
    ui.data_mut(|data| {
        data.insert_temp(cid_id, cid);
        data.insert_temp(apn_id, value.clone());
    });
    let request = ControlledRepairRequest::try_apn(cid, value.as_str()).ok();
    let button = ui.add_enabled(
        action.enabled && request.is_some(),
        egui::Button::new(
            crate::localization::format_text_in(
                language,
                TextKey::ActionEditApn,
                &crate::localization::TextArgs::cid(cid),
            )
            .text,
        ),
    );
    if !action.enabled {
        if let Some(reason) = action.disabled_reason {
            button.clone().on_hover_text(reason.text.clone());
            wrapped_label(ui, meta_text(reason.text));
        }
    }
    if button.clicked() {
        if let Some(request) = request {
            sink.prepare_repair_now(request);
        }
    }
}
