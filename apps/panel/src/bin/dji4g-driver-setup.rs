#![cfg_attr(windows, windows_subsystem = "windows")]
//! Offline driver installer with native confirmation instead of console input.
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
    if !check
        && argument.is_none()
        && !dji4g_windows_platform::confirm_message_box(
            None,
            "安装模块驱动",
            "将检查随程序附带的驱动，为缺驱动接口选择匹配的驱动包。Windows 也可能更新其他匹配同一驱动包的设备，不强制覆盖更优驱动。\n\n请先退出大疆 4G 面板（含托盘），再点击“是”。",
        )
    {
        std::process::exit(1223);
    }
    let result = run(check);
    if let Err(error) = &result {
        eprintln!("{error}");
    }
    if !check && (argument.as_deref() == Some("--install") || result.is_err()) {
        let message = match &result {
            Ok(report) => format!(
                "驱动检查步骤结束，请按以下结果操作。AT 通信和网络仍需单独验证。\n\n{report}"
            ),
            Err(error) => format!("驱动安装未完成，未强制重绑接口。\n\n{error}"),
        };
        dji4g_windows_platform::show_message_box("模块驱动", &message);
    }
    if result.is_err() {
        std::process::exit(1);
    }
}

fn run(check: bool) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let root = exe
        .parent()
        .ok_or("Missing executable directory")?
        .join("drivers");
    if !check && std::env::args().nth(1).as_deref() != Some("--install") {
        if let Some(pid) = panel_pid(std::env::args().nth(1).as_deref()) {
            dji4g_windows_platform::driver_setup::wait_for_panel_exit(
                pid,
                &exe.with_file_name("dji4g-panel.exe"),
                std::time::Duration::from_secs(30),
            )
            .map_err(|e| e.to_string())?;
        }
        // The elevated process repeats all validation after the native confirmation and UAC.
        let code = dji4g_windows_platform::driver_setup::elevate_current_driver_installer()
            .map_err(|e| format!("Windows 管理员授权或驱动安装启动失败：{e}"))?;
        // Preserve the elevated process result, including cancellation, for the outer setup.
        std::process::exit(code as i32);
    }
    let shell = dji4g_windows_platform::driver_setup_powershell().map_err(|e| e.to_string())?;
    let mode = match std::env::args().nth(1).as_deref() {
        Some("--check") => "check",
        Some("--plan") => "plan",
        _ => "install",
    };
    // Create a new file, never overwrite an elevated path that could be a symlink.
    // Starting evidence survives a child termination; PnPUtil output is kept for diagnosis.
    let mut log = if check {
        None
    } else {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let path = exe.with_file_name(format!("driver-setup-{stamp}-{}.log", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("无法创建安装日志，未开始安装：{e}"))?;
        writeln!(file, "Driver setup started; mode={mode}")
            .and_then(|()| file.flush())
            .map_err(|e| e.to_string())?;
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
            .map_err(|e| format!("安装日志写入失败，请检查设备实际状态：{e}\n{report}"))?;
        format!("\n日志：{}", path.display())
    } else {
        String::new()
    };
    let output = output.map_err(|e| format!("{e}{log_note}"))?;
    if check {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
    if output.status.success() {
        Ok(format!(
            "{}{log_note}",
            String::from_utf8_lossy(&output.stdout)
        ))
    } else {
        Err(format!("{report}{log_note}"))
    }
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
