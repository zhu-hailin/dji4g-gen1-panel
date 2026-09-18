#![cfg_attr(windows, windows_subsystem = "windows")]
//! A single-file per-user installer containing the entire reviewed offline payload.
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
include!(concat!(env!("OUT_DIR"), "/offline_bundle.rs"));
const INSTALL: &str = include_str!("../../../../packaging/scripts/install-offline-app.ps1");

fn main() {
    if let Err(error) = run() {
        dji4g_windows_platform::show_message_box("安装未完成", &error);
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    if PAYLOAD.is_empty() {
        return Err("此构建未包含安装资源，请使用完整安装包。".into());
    }
    if !dji4g_windows_platform::confirm_message_box(
        None,
        "安装大疆 4G 面板",
        "将为当前用户安装程序、离线驱动资源，并创建桌面快捷方式。\n\n安装程序本身不会修改系统驱动，完成后可选择安装驱动。\n\n是否继续？",
    ) {
        return Ok(());
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let staging = std::env::temp_dir().join(format!("dji4g-setup-{}-{nonce}", std::process::id()));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let zip = staging.join("payload.zip");
    fs::write(&zip, PAYLOAD).map_err(|e| e.to_string())?;
    let shell = dji4g_windows_platform::driver_setup_powershell().map_err(|e| e.to_string())?;
    let mut command = Command::new(shell);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let result = command
        .env_remove("PSModulePath")
        .args(["-NoProfile", "-NonInteractive", "-Command", INSTALL])
        .env("DJI4G_SETUP_PAYLOAD", &zip)
        .env("DJI4G_SETUP_HASH", HASH)
        .output()
        .map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&zip);
    let _ = fs::remove_dir(&staging);
    if !result.status.success() {
        return Err(format!(
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    let installed = String::from_utf8(result.stdout).map_err(|e| e.to_string())?;
    let directory = std::path::PathBuf::from(installed.trim());
    if !directory.join("dji4g-panel.exe").is_file() {
        return Err("安装文件验证失败。".into());
    }
    if dji4g_windows_platform::confirm_message_box(
        None,
        "安装完成",
        "程序和离线驱动已安装，桌面快捷方式已创建。\n\n现在安装模块驱动吗？需要管理员确认。\n已有驱动可选“否”，直接打开程序。",
    ) {
        let status = Command::new(directory.join("dji4g-driver-setup.exe"))
            .status()
            .map_err(|e| e.to_string())?;
        driver_result(status.code())?;
    }
    Command::new(directory.join("dji4g-panel.exe"))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn driver_result(code: Option<i32>) -> Result<(), String> {
    match code {
        Some(0) => Ok(()),
        Some(1223) => Err("驱动安装已取消。程序文件已安装，但未确认模块驱动可用。".into()),
        _ => Err(format!(
            "驱动安装未完成（退出码：{code:?}）。程序文件已安装，但不会自动打开面板。\n\n若安全软件有拦截，请保留报告中的检测名称与文件路径。不要关闭防护；请将报告交给开发者核查。"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::driver_result;

    #[test]
    fn failed_or_cancelled_driver_setup_stops_app_launch() {
        for code in [Some(1), Some(1223), Some(64), None] {
            assert!(driver_result(code).is_err(), "must stop for {code:?}");
        }
    }

    #[test]
    fn successful_driver_setup_allows_app_launch() {
        assert!(driver_result(Some(0)).is_ok());
    }
}
