//! First-connection checks reuse measured diagnostics, never infer driver absence from no IP.
use dji4g_application::{DiagnosticCheckSnapshot, DiagnosticCheckState};
use dji4g_domain::Freshness;
use std::time::SystemTime;

fn current_state(check: &DiagnosticCheckSnapshot, now: SystemTime) -> DiagnosticCheckState {
    if check.freshness(now) == Freshness::Stale {
        DiagnosticCheckState::Expired
    } else {
        check.state.clone()
    }
}

pub(crate) fn render(
    ui: &mut eframe::egui::Ui,
    snapshot: &dji4g_application::ControllerSnapshot,
    now: SystemTime,
    language: crate::localization::Language,
) {
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
                wrapped_label(ui, "此离线版附带本机导出的原始签名驱动。安装器会校验文件，仅处理硬件 ID 匹配的缺驱动接口；正常接口不会强制重装。");
                wrapped_label(ui, meta_text("安装时请关闭本程序，按安装窗口提示确认。完成后重新打开程序验证 AT 与网络。未匹配的接口需单独处理。"));
                if ui.button("启动离线驱动安装器").clicked() {
                    if let Some(path) = &installer {
                        let mut command = std::process::Command::new(path);
                        let error = command.spawn().err().map(|error| format!("无法启动安装器：{error}"));
                        ui.data_mut(|data| data.insert_temp(eframe::egui::Id::new("driver.setup.error"), error));
                    }
                }
                if let Some(Some(error)) = ui.data(|data| data.get_temp::<Option<String>>(eframe::egui::Id::new("driver.setup.error"))) {
                    wrapped_label(ui, error);
                }
            } else {
                wrapped_label(ui, "安装资源不完整，请重新运行“大疆4G面板完整安装包.exe”。");
            }
            ui.hyperlink_to("大疆官方兼容说明（第 21 项）", "https://repair.dji.com/help/content?customId=01700008285&lang=en&paperDocType=ARTICLE&re=US&spaceId=17");
            wrapped_label(ui, meta_text("网卡与 AT 串口可能需要不同驱动。安装后使用窗口顶部“刷新”重新检查；仅安装程序完成不代表模块已经可用。已有功能正常时无需重复安装。"));
            ui.hyperlink_to("移远官方驱动获取说明", "https://forums.quectel.com/t/how-to-get-driver-tools/38963");
            wrapped_label(ui, meta_text("该链接提供厂商获取渠道，不代表其中所有驱动都兼容大疆定制模块。"));
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use dji4g_application::{DiagnosticCheckId, DiagnosticSet};
    use dji4g_domain::DeviceEpoch;
    use std::time::Duration;

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
