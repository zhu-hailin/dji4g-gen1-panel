#![cfg_attr(windows, windows_subsystem = "windows")]
//! The unelevated outer process owns the return path. The elevated helper only installs.
use dji4g_windows_platform::driver_setup::{self, DriverSetupOutcome as Outcome};
use std::{io::Write, process::Command};

const SCRIPT: &str = include_str!("../../../../packaging/scripts/local-driver-install.ps1");

fn panel_pid(argument: Option<&str>) -> Option<u32> {
    argument?
        .strip_prefix("--wait-for-panel=")?
        .parse::<u32>()
        .ok()
        .filter(|pid| *pid != 0 && *pid != std::process::id())
}

fn main() {
    let argument = std::env::args().nth(1);
    let check = matches!(argument.as_deref(), Some("--check" | "--plan"));
    if std::env::args().len() > 2
        || !(matches!(
            argument.as_deref(),
            None | Some("--check" | "--plan" | "--install")
        ) || panel_pid(argument.as_deref()).is_some())
    {
        eprintln!("Usage: dji4g-driver-setup.exe [--check | --plan | --install]");
        std::process::exit(64);
    }
    if check {
        let result = run_script(argument.as_deref().unwrap());
        if let Err(error) = &result {
            eprintln!("{error}");
        }
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    if argument.as_deref() == Some("--install") {
        let (outcome, note) = match run_script("--install") {
            Ok(report) => report,
            Err(error) => (Outcome::Failed, error),
        };
        dji4g_windows_platform::show_message_box(
            "模块驱动检查结果",
            &format!("{}\n\n{note}\n\n点击“确定”返回面板。", outcome.message()),
        );
        std::process::exit(outcome.exit_code() as i32);
    }
    if let Some(pid) = panel_pid(argument.as_deref()) {
        let waited = std::env::current_exe().and_then(|exe| {
            driver_setup::wait_for_panel_exit(
                pid,
                &exe.with_file_name("dji4g-panel.exe"),
                std::time::Duration::from_secs(30),
            )
        });
        if waited.is_err() {
            // Do not open a duplicate panel while the existing one may still own the serial port.
            dji4g_windows_platform::show_message_box(
                "尚未开始安装",
                "面板尚未完全退出，或无法核实正在运行的面板。请返回原面板；关闭托盘中的面板后再尝试安装。未执行驱动安装。",
            );
            std::process::exit(1);
        }
    }
    if argument.is_none()
        && !dji4g_windows_platform::confirm_message_box(
            None,
            "安装模块驱动",
            "将校验随程序附带的驱动，仅为缺驱动接口选择匹配包。Windows 可能更新其他匹配同一驱动包的设备，不强制覆盖更优驱动。\n\n请先退出大疆 4G 面板（含托盘）。点击“是”后申请管理员授权，结束后自动返回面板。",
        )
    {
        return_to_panel(Outcome::Cancelled);
        std::process::exit(1223);
    }
    let outcome = match driver_setup::elevate_current_driver_installer() {
        Ok(code) => Outcome::from_exit_code(code),
        Err(error) => {
            let result = if error.raw_os_error() == Some(1223) {
                Outcome::Cancelled
            } else {
                Outcome::Failed
            };
            dji4g_windows_platform::show_message_box("管理员授权未完成", result.message());
            result
        }
    };
    return_to_panel(outcome);
    std::process::exit(outcome.exit_code() as i32);
}

fn return_to_panel(outcome: Outcome) {
    if driver_setup::reopen_panel(outcome).is_err() {
        dji4g_windows_platform::show_message_box(
            "请打开面板继续",
            &format!(
                "{}\n\n未能自动返回。请从桌面正常打开“大疆 4G 面板”，在设置中打开首次连接引导。",
                outcome.message()
            ),
        );
    }
}

fn run_script(mode_argument: &str) -> Result<(Outcome, String), String> {
    let exe =
        std::env::current_exe().map_err(|_| "无法确定程序位置，请重新打开完整程序。".to_owned())?;
    let root = exe.parent().ok_or("无法确定程序目录。")?.join("drivers");
    let shell = dji4g_windows_platform::driver_setup_powershell()
        .map_err(|_| "无法启动 Windows 驱动检查组件，请联系技术支持。".to_owned())?;
    let mode = match mode_argument {
        "--check" => "check",
        "--plan" => "plan",
        _ => "install",
    };
    let check = mode != "install";
    let mut log = if check {
        None
    } else {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "系统时间异常，未开始安装。")?
            .as_nanos();
        let path = exe.with_file_name(format!("driver-setup-{stamp}-{}.log", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "无法创建安装日志，未开始安装。请将完整程序放在可写目录后重试。")?;
        writeln!(file, "Driver setup started; mode={mode}")
            .and_then(|()| file.flush())
            .map_err(|_| "无法写入日志，未开始安装。")?;
        Some((path, file))
    };
    let mut command = Command::new(shell);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .env_remove("PSModulePath")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("DJI4G_DRIVER_ROOT", root)
        .env("DJI4G_DRIVER_MODE", mode)
        .output();
    let report = match &output {
        Ok(value) => format!(
            "Exit: {}\n{}\n{}",
            value.status,
            String::from_utf8_lossy(&value.stdout),
            String::from_utf8_lossy(&value.stderr)
        ),
        Err(error) => format!("Failed to launch driver check: {error}"),
    };
    let log_note = if let Some((path, file)) = &mut log {
        writeln!(file, "{report}")
            .and_then(|()| file.flush())
            .map_err(|_| {
                format!(
                    "安装日志写入失败，请检查设备实际状态。日志位置：{}",
                    path.display()
                )
            })?;
        format!("详细安装日志：{}", path.display())
    } else {
        String::new()
    };
    let output = output.map_err(|_| format!("Windows 驱动检查未能启动。{log_note}"))?;
    if check {
        print!("{}", String::from_utf8_lossy(&output.stdout));
        return if output.status.success() {
            Ok((Outcome::Ready, String::new()))
        } else {
            Err(report)
        };
    }
    Ok((
        Outcome::from_script_output(
            &String::from_utf8_lossy(&output.stdout),
            output.status.success(),
        ),
        log_note,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handoff_accepts_only_nonzero_process_id() {
        assert_eq!(panel_pid(Some("--wait-for-panel=42")), Some(42));
        for value in [
            "--wait-for-panel=0",
            "--wait-for-panel=-1",
            "--wait-for-panel=4294967296",
            "--wait-for-panel=42 --install",
            "--install",
        ] {
            assert_eq!(panel_pid(Some(value)), None);
        }
        assert_eq!(
            panel_pid(Some(&format!("--wait-for-panel={}", std::process::id()))),
            None
        );
    }
}
