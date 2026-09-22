//! First-connection checks reuse measured diagnostics, never infer driver absence from no IP.
use dji4g_application::{DiagnosticCheckSnapshot, DiagnosticCheckState};
use dji4g_domain::Freshness;
use std::time::SystemTime;

pub(crate) fn installation_busy(snapshot: &dji4g_application::ControllerSnapshot) -> bool {
    snapshot.serial_work_busy
        || snapshot
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GuideState {
    Detecting,
    Expired,
    NotRun,
    ProbeDisabled,
    Failed,
    Passed,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NextStepVm {
    pub state: GuideState,
    pub text: &'static str,
    pub destination: Option<crate::app::Page>,
}

pub(crate) fn next_step_vm(
    snapshot: &dji4g_application::ControllerSnapshot,
    now: SystemTime,
) -> NextStepVm {
    use crate::app::Page;
    use dji4g_application::DiagnosticCheckId as Id;
    for id in [
        Id::UsbDevice,
        Id::AtControl,
        Id::WindowsAdapter,
        Id::Cellular,
        Id::BoundPublic,
        Id::BoundDns,
    ] {
        if matches!(id, Id::BoundPublic | Id::BoundDns) && !snapshot.settings.active_probe {
            return NextStepVm {
                state: GuideState::ProbeDisabled,
                text: "主动联网检查已关闭，公网与 DNS 尚未验证；可在设置中开启。",
                destination: Some(Page::Settings),
            };
        }
        let state = snapshot
            .diagnostics
            .iter()
            .find(|check| check.id == id)
            .map(|check| current_state(check, now));
        match state {
            Some(DiagnosticCheckState::Passed) => continue,
            Some(DiagnosticCheckState::Running { .. }) => {
                return NextStepVm {
                    state: GuideState::Detecting,
                    text: "正在采集连接证据，请等待本轮检查完成；此时无需修改网络设置。",
                    destination: None,
                };
            }
            Some(DiagnosticCheckState::Expired) => {
                return NextStepVm {
                    state: GuideState::Expired,
                    text: "连接证据已过期，请刷新后再判断；旧结果不代表当前连接状态。",
                    destination: Some(Page::Diagnostics),
                };
            }
            Some(DiagnosticCheckState::Unexecuted {
                reason: dji4g_application::UnexecutedReason::DisabledBySetting,
            }) => {
                return NextStepVm {
                    state: GuideState::ProbeDisabled,
                    text: "此项检查已关闭，可在设置中开启；未检查不代表网络失败。",
                    destination: Some(Page::Settings),
                };
            }
            Some(DiagnosticCheckState::Unexecuted { .. }) | None => {
                return NextStepVm {
                    state: GuideState::NotRun,
                    text: "尚未完成此项检查，请先刷新。未识别设备不等于缺驱动。",
                    destination: Some(Page::Diagnostics),
                };
            }
            Some(
                DiagnosticCheckState::Failed { .. } | DiagnosticCheckState::Unavailable { .. },
            ) => {
                let text = match id {
                    Id::UsbDevice => {
                        "USB 检查未通过，请核对数据线、接口与设备。未识别设备不等于缺驱动。"
                    }
                    Id::AtControl | Id::WindowsAdapter => {
                        "模块接口检查未通过，请查看串口或网卡的具体原因；正常接口无需重装驱动。"
                    }
                    Id::Cellular => {
                        "SIM 或蜂窝注册检查未通过，请查看原因并核对卡状态、信号与运营商注册。"
                    }
                    _ => {
                        "模块绑定的公网或 DNS 检查未通过，请按具体失败项排查；无需反复切换 USB 模式。"
                    }
                };
                return NextStepVm {
                    state: GuideState::Failed,
                    text,
                    destination: Some(Page::Diagnostics),
                };
            }
        }
    }
    NextStepVm {
        state: GuideState::Passed,
        text: "模块公网与 DNS 检查已通过。系统实际出口仍可能由 Wi-Fi 或 VPN 决定。",
        destination: None,
    }
}

pub(crate) fn next_step(
    snapshot: &dji4g_application::ControllerSnapshot,
    now: SystemTime,
) -> &'static str {
    next_step_vm(snapshot, now).text
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
        let mut passed_checks = Vec::new();
        for (id, title) in [
            (Id::UsbDevice, "USB 识别"),
            (Id::WindowsAdapter, "网卡接口"),
            (Id::AtControl, "AT 串口"),
            (Id::Cellular, "SIM 与蜂窝网络"),
            (Id::BoundPublic, "模块公网"),
            (Id::BoundDns, "模块 DNS"),
        ] {
            if let Some(check) = snapshot.diagnostics.iter().find(|check| check.id == id) {
                let state = current_state(check, now);
                if state == DiagnosticCheckState::Passed {
                    passed_checks.push(title);
                    continue;
                }
                let vm = diagnostic_state_vm(&state, language);
                ui.horizontal_wrapped(|ui| {
                    ui.label(title);
                    ui.colored_label(vm.tone.color(), vm.label.text);
                    if let Some(detail) = vm.detail {
                        ui.label(detail.text);
                    }
                });
            }
        }
        if !passed_checks.is_empty() {
            eframe::egui::CollapsingHeader::new(format!(
                "已通过的检查（{} 项）",
                passed_checks.len()
            ))
            .show(ui, |ui| {
                for title in passed_checks {
                    ui.label(format!("✓ {title}"));
                }
            });
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
        eframe::egui::CollapsingHeader::new("驱动安装").default_open(driver_installation_expanded(snapshot, now)).show(ui, |ui| {
            let bundled = super::onboarding::bundled_driver_available();
            if bundled {
                wrapped_label(ui, "此离线版附带原始驱动资源，安装前会校验文件和签名。当前包不能覆盖所有接口（包括未匹配的 MI_04）；任何缺驱动接口无法匹配时，将在安装前停止。Windows 可能同时更新其他匹配该包的设备，不强制覆盖更优驱动。");
                wrapped_label(ui, meta_text("确认后面板会自动退出，再显示 Windows 管理员授权。安装结束或取消授权后会自动返回面板并显示结果；如提示重启，请先重启电脑。"));
                install_requested = ui.button("退出面板并安装驱动").clicked();
            } else {
                wrapped_label(ui, "此版本没有完整的离线驱动资源。请打开 Windows 设置 → Windows 更新 → 可选更新检查驱动，或联系 DJI 官方支持取得此模块的适配驱动；安装后点击“立即刷新”。");
            }
            ui.hyperlink_to("大疆官方兼容说明（第 21 项）", "https://repair.dji.com/help/content?customId=01700008285&lang=en&paperDocType=ARTICLE&re=US&spaceId=17");
            wrapped_label(ui, meta_text("网卡与 AT 串口可能需要不同驱动。安装结果返回后仍需验证 AT 与网络；已有功能正常时无需重复安装。Windows 更新不保证提供该模块的全部驱动。"));
            ui.hyperlink_to("联系 DJI 官方支持", super::onboarding::OFFICIAL_SUPPORT_URL);
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
    fn guidance_distinguishes_not_run_running_expired_and_passed() {
        use dji4g_application::{
            BackendEvent, CheckMask, ReducerState, RefreshCycleId, reduce_state,
        };
        let now = SystemTime::now();
        assert_eq!(
            next_step_vm(&ReducerState::new(now).snapshot(), now).state,
            GuideState::NotRun
        );
        let ready = ReducerState::test_ready(now);
        assert_eq!(
            next_step_vm(&ready.snapshot(), now).state,
            GuideState::Passed
        );
        assert_eq!(
            next_step_vm(&ready.snapshot(), now + Duration::from_secs(3600)).state,
            GuideState::Expired
        );
        let running = reduce_state(
            &ready,
            BackendEvent::RefreshStarted {
                cycle: RefreshCycleId(40),
                epoch: DeviceEpoch(1),
                scheduled: CheckMask::only(DiagnosticCheckId::AtControl),
            },
            now,
        );
        let guide = next_step_vm(&running.snapshot(), now);
        assert_eq!(guide.state, GuideState::Detecting);
        assert_eq!(guide.destination, None);
    }

    #[test]
    fn disabled_active_probes_direct_to_settings_not_network_repair() {
        let now = SystemTime::now();
        let mut snapshot = crate::demo::demo_snapshot(crate::demo::DemoScenario::Available, now);
        snapshot.settings.active_probe = false;
        assert!(next_step(&snapshot, now).contains("设置"));
        assert!(next_step(&snapshot, now).contains("关闭"));
    }

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

fn driver_installation_expanded(
    snapshot: &dji4g_application::ControllerSnapshot,
    now: SystemTime,
) -> bool {
    // A missing serial response is not proof of a missing driver. Code 28 is explicit PnP evidence.
    snapshot
        .app
        .device
        .as_ref()
        .is_some_and(|device| device.problem_code == Some(28))
        && snapshot.diagnostics.iter().any(|check| {
            check.id == dji4g_application::DiagnosticCheckId::UsbDevice
                && check.freshness(now) == Freshness::Fresh
        })
}
#[test]
fn interface_failure_alone_is_not_driver_installation_evidence() {
    use dji4g_application::{
        BackendEvent, CheckMask, CheckResult, DiagnosticCheckId, FailureCode, ReducerState,
        RefreshCycleId, reduce_state,
    };
    let now = SystemTime::now();
    let state = ReducerState::test_ready(now);
    let state = reduce_state(
        &state,
        BackendEvent::RefreshStarted {
            cycle: RefreshCycleId(90),
            epoch: dji4g_domain::DeviceEpoch(1),
            scheduled: CheckMask::only(DiagnosticCheckId::AtControl),
        },
        now,
    );
    let state = reduce_state(
        &state,
        BackendEvent::AtFinished {
            cycle: RefreshCycleId(90),
            epoch: dji4g_domain::DeviceEpoch(1),
            result: CheckResult::Failed {
                code: FailureCode::new(
                    dji4g_domain::ErrorCode::Timeout,
                    dji4g_application::StableCode::try_from_static("at:timeout").unwrap(),
                ),
                observed_at: now,
            },
        },
        now,
    );
    assert!(!driver_installation_expanded(&state.snapshot(), now));
}
