//! First-run guidance projects existing asynchronous diagnostic evidence; it opens no device.
use std::time::SystemTime;

use dji4g_application::{
    ControllerSnapshot, DiagnosticCheckId, DiagnosticCheckSnapshot, DiagnosticCheckState,
    UnexecutedReason,
};
use dji4g_domain::Freshness;
use eframe::egui::{self, RichText};

pub const OFFICIAL_DRIVER_GUIDANCE_URL: &str = "https://repair.dji.com/help/content?customId=01700008285&lang=en&paperDocType=ARTICLE&re=US&spaceId=17";
pub const OFFICIAL_SUPPORT_URL: &str = "https://www.dji.com/cn/support";

#[derive(Default)]
pub(crate) struct OnboardingState {
    pub open: bool,
    pub completed: bool,
    pub driver_fixture: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CheckState {
    Waiting,
    Running,
    Passed,
    Attention,
    Disabled,
    Expired,
}

impl CheckState {
    fn label(self) -> &'static str {
        match self {
            Self::Waiting => "等待检查 / 设备就绪",
            Self::Running => "正在检查…",
            Self::Passed => "已通过",
            Self::Attention => "需要查看原因",
            Self::Disabled => "已关闭，尚未验证",
            Self::Expired => "结果已过期，请刷新",
        }
    }
    fn tone(self) -> super::StatusTone {
        match self {
            Self::Passed => super::StatusTone::Positive,
            Self::Attention => super::StatusTone::Caution,
            Self::Running => super::StatusTone::Progress,
            _ => super::StatusTone::Neutral,
        }
    }
}

fn check_state(
    check: Option<&DiagnosticCheckSnapshot>,
    enabled: bool,
    now: SystemTime,
) -> CheckState {
    if !enabled {
        return CheckState::Disabled;
    }
    let Some(check) = check else {
        return CheckState::Waiting;
    };
    if check.freshness(now) == Freshness::Stale {
        return CheckState::Expired;
    }
    match &check.state {
        DiagnosticCheckState::Passed => CheckState::Passed,
        DiagnosticCheckState::Failed { .. } => CheckState::Attention,
        DiagnosticCheckState::Running { .. } => CheckState::Running,
        DiagnosticCheckState::Unexecuted {
            reason: UnexecutedReason::DisabledBySetting,
        } => CheckState::Disabled,
        DiagnosticCheckState::Expired => CheckState::Expired,
        DiagnosticCheckState::Unavailable { .. } | DiagnosticCheckState::Unexecuted { .. } => {
            CheckState::Waiting
        }
    }
}

pub(crate) fn checks(
    snapshot: &ControllerSnapshot,
    now: SystemTime,
) -> Vec<(&'static str, CheckState)> {
    use DiagnosticCheckId as Id;
    [
        (Id::UsbDevice, "USB 设备识别"),
        (Id::WindowsAdapter, "Windows 网卡"),
        (Id::AtControl, "AT 串口通信"),
        (Id::Cellular, "SIM 与蜂窝网络"),
        (Id::BoundPublic, "模块公网连接"),
        (Id::BoundDns, "模块 DNS 解析"),
    ]
    .into_iter()
    .map(|(id, label)| {
        let enabled =
            !matches!(id, Id::BoundPublic | Id::BoundDns) || snapshot.settings.active_probe;
        (
            label,
            check_state(Some(snapshot.diagnostics.get(id)), enabled, now),
        )
    })
    .collect()
}

pub(crate) enum OnboardingAction {
    None,
    Enter,
    Refresh,
    InstallBundledDriver,
}

fn bundled_driver_available() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
        .is_some_and(|dir| {
            dir.join("dji4g-driver-setup.exe").is_file() && dir.join("drivers/qcser.inf").is_file()
        })
}

pub(crate) fn render(
    ctx: &egui::Context,
    snapshot: &ControllerSnapshot,
    now: SystemTime,
    driver_fixture: Option<bool>,
) -> OnboardingAction {
    let rows = checks(snapshot, now);
    let ready = rows.iter().all(|(_, state)| *state == CheckState::Passed);
    let mut action = OnboardingAction::None;
    egui::TopBottomPanel::bottom("onboarding-actions")
        .frame(
            egui::Frame::none()
                .fill(egui::Color32::WHITE)
                .stroke(egui::Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgb(226, 230, 236),
                ))
                .inner_margin(16.0),
        )
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("跳过，直接进入面板").clicked() {
                    action = OnboardingAction::Enter;
                }
                if ui
                    .add(super::theme::primary_button(if ready {
                        "检查完成，开始使用"
                    } else {
                        "进入面板查看详情"
                    }))
                    .clicked()
                {
                    action = OnboardingAction::Enter;
                }
            });
            ui.label(super::meta_text(
                "跳过或进入后不再自动显示；可在设置中重新打开。",
            ));
        });
    egui::CentralPanel::default().frame(egui::Frame::central_panel(&ctx.style()).inner_margin(24.0)).show(ctx, |ui| {
        egui::ScrollArea::vertical().id_salt("onboarding-scroll").auto_shrink([false,false]).show(ui, |ui| {
            ui.label(RichText::new("欢迎使用 DJI 一代 4G 面板").size(24.0).strong());
            super::wrapped_label(ui,"连接模块后，面板会在后台检查连接情况。可以随时跳过，进入后继续查看检查结果。");
            ui.add_space(16.0);
            super::section_frame(ui, |ui| {
                ui.label(super::section_heading("1. 连接模块"));
                super::wrapped_label(ui,"插好 SIM 卡，使用支持数据传输的 USB 线连接模块与电脑。电脑原有驱动可能已经可用，无需先安装驱动。");
            });
            ui.add_space(12.0);
            super::section_frame(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(super::section_heading("2. 自动检查模块"));
                    if ui.add_enabled(!snapshot.serial_work_busy,egui::Button::new("重新检查")).clicked() { action = OnboardingAction::Refresh; }
                });
                for (label,state) in &rows {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(*label);
                        ui.colored_label(state.tone().color(),format!("{} {}",state.tone().marker(),state.label()));
                        if *state == CheckState::Running { ui.spinner(); }
                    });
                }
                ui.add_space(6.0);
                super::wrapped_label(ui,super::meta_text(if ready { "模块绑定的公网与 DNS 检查通过；电脑实际出口仍可能由 Wi-Fi 或 VPN 决定。" } else { "等待检查、未连接、证据过期或关闭主动联网检查，不等于驱动损坏。具体原因可进入面板查看。" }));
            });
            ui.add_space(12.0);
            super::section_frame(ui, |ui| {
                ui.label(super::section_heading("3. 需要驱动时再安装"));
                if driver_fixture.unwrap_or_else(bundled_driver_available) {
                    super::wrapped_label(ui,"此版本包含已校验的本地驱动包。已有接口正常时无需重复安装。");
                    if ui.add_enabled(!super::driver_setup::installation_busy(snapshot),egui::Button::new("使用内置驱动")).clicked() { action = OnboardingAction::InstallBundledDriver; }
                    super::wrapped_label(ui,super::meta_text("点击后先确认，再退出面板并显示 Windows 授权；安装后重新打开面板验证。"));
                } else {
                    super::wrapped_label(ui,"当前目录未检测到完整内置驱动包。可从官方渠道确认适配驱动，下载后按厂商说明安装。");
                }
                ui.horizontal_wrapped(|ui| {
                    ui.hyperlink_to("DJI 官方兼容与驱动说明",OFFICIAL_DRIVER_GUIDANCE_URL);
                    ui.hyperlink_to("联系 DJI 官方支持",OFFICIAL_SUPPORT_URL);
                });
                super::wrapped_label(ui,super::meta_text("官方网页仅在浏览器中打开，不会自动下载或安装。"));
            });
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_missing_and_disabled_evidence_is_never_a_failure() {
        let now = SystemTime::UNIX_EPOCH;
        let mut snapshot = dji4g_application::ReducerState::new(now).snapshot();
        assert!(
            checks(&snapshot, now)
                .iter()
                .all(|(_, state)| *state == CheckState::Waiting)
        );
        snapshot.settings.active_probe = false;
        let rows = checks(&snapshot, now);
        assert_eq!(rows[4].1, CheckState::Disabled);
        assert_eq!(rows[5].1, CheckState::Disabled);
        assert_eq!(check_state(None, true, now), CheckState::Waiting);
    }
    #[test]
    fn successful_checks_expire_instead_of_claiming_current_availability() {
        let now = SystemTime::now();
        let snapshot = dji4g_application::ReducerState::test_ready(now).snapshot();
        assert!(
            checks(&snapshot, now)
                .iter()
                .all(|(_, state)| *state == CheckState::Passed)
        );
        assert!(
            checks(&snapshot, now + std::time::Duration::from_secs(3600))
                .iter()
                .all(|(_, state)| *state == CheckState::Expired)
        );
    }
}
