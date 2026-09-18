//! First-connection checks reuse measured diagnostics, never infer driver absence from no IP.
use dji4g_application::{DiagnosticCheckSnapshot, DiagnosticCheckState};
use dji4g_domain::Freshness;
use std::time::SystemTime;

pub(crate) fn installation_busy(snapshot: &dji4g_application::ControllerSnapshot) -> bool {
    snapshot
        .sms_send
        .as_ref()
        .is_some_and(|send| send.phase != dji4g_application::SmsSendPhase::Finished)
        || matches!(
            snapshot.operation.as_ref().map(|op| &op.state),
            Some(dji4g_application::OperationState::Running { .. })
        )
        || snapshot.prepared_action.as_ref().is_some_and(|action| {
            matches!(
                action.state,
                dji4g_application::PreparedActionState::AwaitingConfirmation
            )
        })
}

fn current_state(check: &DiagnosticCheckSnapshot, now: SystemTime) -> DiagnosticCheckState {
    if check.freshness(now) == Freshness::Stale {
        DiagnosticCheckState::Expired
    } else {
        check.state.clone()
    }
}

pub(crate) fn next_step(
    snapshot: &dji4g_application::ControllerSnapshot,
    now: SystemTime,
) -> &'static str {
    use dji4g_application::DiagnosticCheckId as Id;
    let passed = |id| {
        snapshot.diagnostics.iter().any(|check| {
            check.id == id && current_state(check, now) == DiagnosticCheckState::Passed
        })
    };
    if !passed(Id::UsbDevice) {
        "先插入有效 SIM 卡，再用支持数据传输的 USB 线直连电脑。点击“立即刷新”。USB 检查失败时先查看诊断原因；未识别设备不等于缺驱动，持续失败请导出详细日志。"
    } else if !passed(Id::AtControl) || !passed(Id::WindowsAdapter) {
        "USB 已识别，继续检查 AT 串口和网卡。进入“修复”查看首次连接检查；确认缺驱动时选择“退出面板并安装驱动”，允许 Windows 管理员授权。完成后重新打开本程序并刷新；已正常的接口无需重装。"
    } else if !passed(Id::Cellular) {
        "电脑接口已通过，下一步检查 SIM 卡、信号和运营商注册状态。进入“诊断”查看原因，确认卡已激活且有流量，并把模块放在信号较好的位置。"
    } else if !passed(Id::BoundPublic) || !passed(Id::BoundDns) {
        "模块通信已通过，下一步确认模块网络能访问公网并解析域名。进入“诊断”查看 IP、网关、DNS；按明确的失败项处理，不要反复重装驱动或切换 USB 模式。"
    } else {
        "模块公网与 DNS 检查已通过。可以尝试浏览网页；如要收发短信，进入“短信”页按该页提示操作。电脑实际出口也可能由 Wi-Fi、VPN 等决定，可在“诊断”查看系统路由。"
    }
}

pub(crate) fn render_guide(
    ui: &mut eframe::egui::Ui,
    snapshot: &dji4g_application::ControllerSnapshot,
    now: SystemTime,
    language: crate::localization::Language,
) {
    use super::{diagnostic_state_vm, section_frame, section_heading, wrapped_label};
    use dji4g_application::DiagnosticCheckId as Id;
    section_frame(ui, |ui| {
        ui.label(section_heading("开始使用模块"));
        wrapped_label(
            ui,
            "1. 连接模块 → 2. 检查驱动与串口 → 3. 检查 SIM / 网络 → 4. 上网或短信",
        );
        for (id, title) in [
            (Id::UsbDevice, "USB 识别"),
            (Id::WindowsAdapter, "网卡接口"),
            (Id::AtControl, "AT 串口"),
            (Id::Cellular, "SIM 与蜂窝网络"),
            (Id::BoundPublic, "模块公网"),
            (Id::BoundDns, "模块 DNS"),
        ] {
            if let Some(check) = snapshot.diagnostics.iter().find(|check| check.id == id) {
                let vm = diagnostic_state_vm(&current_state(check, now), language);
                ui.horizontal_wrapped(|ui| {
                    ui.label(title);
                    ui.colored_label(vm.tone.color(), vm.label.text);
                    if let Some(detail) = vm.detail {
                        ui.label(detail.text);
                    }
                });
            }
        }
        ui.separator();
        wrapped_label(ui, next_step(snapshot, now));
        wrapped_label(
            ui,
            "长时间停在检测中：点击顶部“导出详细日志”，完成后“打开所在文件夹”，把该 TXT 文件交给协助排查的人。日志包含设备、驱动和网络信息，不包含短信正文。",
        );
    });
}

pub(crate) fn render(
    ui: &mut eframe::egui::Ui,
    snapshot: &dji4g_application::ControllerSnapshot,
    now: SystemTime,
    language: crate::localization::Language,
) -> bool {
    let mut install_requested = false;
    use super::{diagnostic_state_vm, meta_text, section_frame, section_heading, wrapped_label};
    use dji4g_application::DiagnosticCheckId;
    section_frame(ui, |ui| {
        ui.label(section_heading("首次连接检查"));
        wrapped_label(
            ui,
            "插入模块后，分别检查网卡和 AT 通信。无法上网或未获得 IP 并不一定是缺少驱动。",
        );
        for (id, title) in [
            (DiagnosticCheckId::UsbDevice, "1. USB 设备识别"),
            (DiagnosticCheckId::WindowsAdapter, "2. Windows 网卡"),
            (DiagnosticCheckId::AtControl, "3. AT 通信（短信与模块查询）"),
        ] {
            if let Some(check) = snapshot.diagnostics.iter().find(|check| check.id == id) {
                let state = current_state(check, now);
                let vm = diagnostic_state_vm(&state, language);
                ui.horizontal_wrapped(|ui| {
                    ui.label(title);
                    ui.colored_label(
                        vm.tone.color(),
                        format!("{} {}", vm.tone.marker(), vm.label.text),
                    );
                });
                if let Some(detail) = vm.detail {
                    wrapped_label(ui, meta_text(detail.text));
                }
            }
        }
        eframe::egui::CollapsingHeader::new("驱动安装").default_open(true).show(ui, |ui| {
            let installer = std::env::current_exe().ok().and_then(|exe| {
                exe.parent().map(|dir| dir.join("dji4g-driver-setup.exe"))
            });
            let bundled = installer.as_ref().is_some_and(|path| {
                path.is_file() && path.parent().is_some_and(|dir| dir.join("drivers/qcser.inf").is_file())
            });
            if bundled {
                wrapped_label(ui, "此离线版附带本机导出的原始签名驱动。安装器会校验文件，为缺驱动接口选择匹配包；Windows 可能同时更新其他匹配该包的设备，不强制覆盖更优驱动。");
                wrapped_label(ui, meta_text("确认后面板会自动退出，再显示 Windows 管理员授权。安装结束后重新打开独立程序，点击“立即刷新”验证；如提示重启，请先重启电脑。"));
                install_requested = ui.button("退出面板并安装驱动").clicked();
            } else {
                wrapped_label(ui, "安装资源不完整，请重新运行“大疆4G面板独立版.exe”。若仍失败，导出详细日志。");
            }
            ui.hyperlink_to("大疆官方兼容说明（第 21 项）", "https://repair.dji.com/help/content?customId=01700008285&lang=en&paperDocType=ARTICLE&re=US&spaceId=17");
            wrapped_label(ui, meta_text("网卡与 AT 串口可能需要不同驱动。安装后使用窗口顶部“刷新”重新检查；仅安装程序完成不代表模块已经可用。已有功能正常时无需重复安装。"));
            ui.hyperlink_to("移远官方驱动获取说明", "https://forums.quectel.com/t/how-to-get-driver-tools/38963");
            wrapped_label(ui, meta_text("该链接提供厂商获取渠道，不代表其中所有驱动都兼容大疆定制模块。"));
        });
    });
    install_requested
}

#[cfg(test)]
mod tests {
    use super::*;
    use dji4g_application::{DiagnosticCheckId, DiagnosticSet};
    use dji4g_domain::DeviceEpoch;
    use std::time::Duration;

    #[test]
    fn initial_guide_does_not_diagnose_missing_driver() {
        let now = SystemTime::now();
        let snapshot = dji4g_application::ReducerState::new(now).snapshot();
        assert!(next_step(&snapshot, now).contains("未识别设备不等于缺驱动"));
        assert!(!installation_busy(&snapshot));
    }

    #[test]
    fn expired_success_cannot_mark_setup_ready() {
        let set = DiagnosticSet::new(DeviceEpoch::default());
        let mut check = set
            .iter()
            .find(|c| c.id == DiagnosticCheckId::AtControl)
            .unwrap()
            .clone();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        check.state = DiagnosticCheckState::Passed;
        check.expires_at = Some(now - Duration::from_secs(1));
        assert_eq!(current_state(&check, now), DiagnosticCheckState::Expired);
        check.expires_at = Some(now + Duration::from_secs(1));
        assert_eq!(current_state(&check, now), DiagnosticCheckState::Passed);
    }
}
