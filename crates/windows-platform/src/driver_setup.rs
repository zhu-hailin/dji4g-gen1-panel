//! Native, fixed-action UAC launch for the sibling offline driver installer.
//! This does not change the helper's separate signature/IPC policy.

/// Closed advisory result passed from the installer to a fresh, unelevated panel.
/// This is not evidence of AT, SMS or network readiness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DriverSetupOutcome {
    Ready,
    RestartRequired,
    RestartRequiredAfterFailure,
    Cancelled,
    UnsupportedInterface,
    NotReady,
    Disconnected,
    ValidationFailed,
    Failed,
}

impl DriverSetupOutcome {
    pub fn code(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::RestartRequired => "restart-required",
            Self::RestartRequiredAfterFailure => "restart-required-after-failure",
            Self::Cancelled => "cancelled",
            Self::UnsupportedInterface => "unsupported-interface",
            Self::NotReady => "not-ready",
            Self::Disconnected => "disconnected",
            Self::ValidationFailed => "validation-failed",
            Self::Failed => "failed",
        }
    }
    pub fn parse_argument(argument: &str) -> Option<Self> {
        Self::parse_code(argument.strip_prefix("--driver-setup-result=")?)
    }
    fn parse_code(code: &str) -> Option<Self> {
        [
            Self::Ready,
            Self::RestartRequired,
            Self::RestartRequiredAfterFailure,
            Self::Cancelled,
            Self::UnsupportedInterface,
            Self::NotReady,
            Self::Disconnected,
            Self::ValidationFailed,
            Self::Failed,
        ]
        .into_iter()
        .find(|outcome| outcome.code() == code)
    }
    pub fn argument(self) -> String {
        format!("--driver-setup-result={}", self.code())
    }
    pub fn exit_code(self) -> u32 {
        match self {
            Self::Ready => 0,
            Self::RestartRequired => 3010,
            Self::RestartRequiredAfterFailure => 24,
            Self::Cancelled => 1223,
            Self::UnsupportedInterface => 20,
            Self::NotReady => 21,
            Self::Disconnected => 22,
            Self::ValidationFailed => 23,
            Self::Failed => 1,
        }
    }
    pub fn from_exit_code(code: u32) -> Self {
        [
            Self::Ready,
            Self::RestartRequired,
            Self::RestartRequiredAfterFailure,
            Self::Cancelled,
            Self::UnsupportedInterface,
            Self::NotReady,
            Self::Disconnected,
            Self::ValidationFailed,
        ]
        .into_iter()
        .find(|outcome| outcome.exit_code() == code)
        .unwrap_or(Self::Failed)
    }
    /// Only a single terminal marker paired with the correct process status is accepted.
    pub fn from_script_output(output: &str, success: bool) -> Self {
        let mut markers = output
            .lines()
            .filter_map(|line| line.strip_prefix("DRIVER_SETUP_RESULT="));
        let result = markers.next().and_then(Self::parse_code);
        if markers.next().is_some() {
            return Self::Failed;
        }
        match result {
            Some(outcome) if success == matches!(outcome, Self::Ready | Self::RestartRequired) => {
                outcome
            }
            _ => Self::Failed,
        }
    }
    pub fn allows_recheck(self) -> bool {
        matches!(self, Self::Ready | Self::NotReady | Self::Disconnected)
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Ready => {
                "Windows 中的驱动接口检查已通过。面板将重新检查 USB、网卡和 AT 通信；短信及上网是否可用，请以新的检查结果为准。"
            }
            Self::RestartRequired => {
                "Windows 要求重启电脑，本次还不能确认驱动可用。请先保存工作并重启电脑，再打开面板检查模块。不要重复安装。"
            }
            Self::RestartRequiredAfterFailure => {
                "驱动安装的部分操作失败，且 Windows 已要求重启电脑；本次不能确认驱动可用。请保留安装日志，先保存工作并重启电脑，再打开面板检查。不要重复安装；重启后仍异常时，把日志交给技术支持。"
            }
            Self::Cancelled => {
                "已取消驱动安装或 Windows 管理员授权，未开始安装。你可以继续使用面板；确实需要安装时，点击“使用内置驱动”，并在 Windows 授权窗口选择“是”。"
            }
            Self::UnsupportedInterface => {
                "当前驱动包没有为某个缺驱动接口找到唯一匹配项（例如 MI_04），本次未安装任何驱动。请先通过 Windows 更新查找适配驱动，或联系 DJI 官方支持提供该模块的匹配驱动。重复安装本包不能补齐该接口。"
            }
            Self::NotReady => {
                "驱动检查后仍有接口异常，暂时不能确认可用。请查看面板的新检查结果；若仍异常，打开设备管理器查看原因，并把安装日志交给技术支持。不要反复安装。"
            }
            Self::Disconnected => {
                "没有检测到已连接的大疆一代模块，或模块在安装后暂时断开。请插稳支持数据传输的 USB 线，等待模块识别，再点击“重新检查”。"
            }
            Self::ValidationFailed => {
                "安装资源缺失、校验未通过，或与当前 Windows 不兼容，本次未开始安装。请重新取得完整的原始驱动版程序，或联系 DJI 官方支持；不要修改驱动文件。"
            }
            Self::Failed => {
                "安装未完成，不能确认设备状态。请查看本次安装日志，并在面板中重新检查；如需协助，把日志交给技术支持。"
            }
        }
    }
}

/// Opens only Windows Update settings; it does not request installation or elevation.
#[cfg(windows)]
pub fn open_windows_update() -> std::io::Result<()> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let uri: Vec<u16> = "ms-settings:windowsupdate\0".encode_utf16().collect();
    // SAFETY: fixed terminated strings remain valid for the synchronous shell call.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            uri.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        Err(std::io::Error::other(
            "无法打开 Windows 更新，请从系统设置中打开。",
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn open_windows_update() -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Fail closed if launched as administrator: never inherit that token into the panel.
#[cfg(windows)]
pub fn reopen_panel(outcome: DriverSetupOutcome) -> std::io::Result<()> {
    use std::{
        mem::size_of,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    use windows_sys::Win32::{
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    let mut raw = std::ptr::null_mut();
    // SAFETY: current process is valid and raw receives an owned token handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: successful OpenProcessToken returned an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0;
    // SAFETY: output is a correctly sized TOKEN_ELEVATION buffer.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    if elevation.TokenIsElevated != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "安装器以管理员身份运行，请从桌面正常打开面板。",
        ));
    }
    let exe = std::env::current_exe()?.canonicalize()?;
    let sibling = verified_panel_sibling(&exe)?;
    std::process::Command::new(sibling)
        .arg(outcome.argument())
        .spawn()?;
    Ok(())
}

#[cfg(any(windows, test))]
fn verified_panel_sibling(installer: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let parent = installer.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    let panel = parent.join("dji4g-panel.exe").canonicalize()?;
    if panel.parent() != Some(parent) || !panel.is_file() {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    Ok(panel)
}

#[cfg(not(windows))]
pub fn reopen_panel(_: DriverSetupOutcome) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Wait only for the known sibling panel; never terminate a user process.
#[cfg(windows)]
pub fn wait_for_panel_exit(
    pid: u32,
    expected: &std::path::Path,
    timeout: std::time::Duration,
) -> std::io::Result<()> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{ERROR_INVALID_PARAMETER, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            WaitForSingleObject,
        },
    };
    if pid == 0 || pid == std::process::id() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    // SAFETY: read/query/synchronize only; no termination or modification rights.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | 0x00100000, 0, pid) };
    if raw.is_null() {
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            Ok(())
        } else {
            Err(error)
        };
    }
    // SAFETY: OpenProcess returned an owned handle, closed exactly once by OwnedHandle.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    // A terminated process can retain a PID/handle while its image name is no longer queryable.
    // No action is performed on it, so already-signaled handles need no identity query.
    if unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } == WAIT_OBJECT_0 {
        return Ok(());
    }
    let mut name = vec![0u16; 32768];
    let mut size = name.len() as u32;
    // SAFETY: writable buffer and size are valid for this call.
    if unsafe {
        QueryFullProcessImageNameW(handle.as_raw_handle(), 0, name.as_mut_ptr(), &mut size)
    } == 0
    {
        let error = std::io::Error::last_os_error();
        if unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } == WAIT_OBJECT_0 {
            return Ok(());
        }
        return Err(error);
    }
    let actual = std::fs::canonicalize(String::from_utf16_lossy(&name[..size as usize]))?;
    let expected = std::fs::canonicalize(expected)?;
    if !actual
        .to_string_lossy()
        .eq_ignore_ascii_case(&expected.to_string_lossy())
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "等待对象不是同目录面板，未安装驱动",
        ));
    }
    // SAFETY: handle remains valid throughout the bounded wait.
    match unsafe {
        WaitForSingleObject(
            handle.as_raw_handle(),
            timeout.as_millis().min(u32::MAX as u128 - 1) as u32,
        )
    } {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "面板未能在 30 秒内退出，未安装驱动。请退出托盘中的面板后重试。",
        )),
        _ => Err(std::io::Error::last_os_error()),
    }
}

#[cfg(not(windows))]
pub fn wait_for_panel_exit(
    _: u32,
    _: &std::path::Path,
    _: std::time::Duration,
) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn wait_validates_identity_times_out_without_killing_and_observes_exit() {
        use std::os::windows::process::CommandExt;
        let shell = crate::driver_setup_powershell().unwrap();
        let mut child = std::process::Command::new(&shell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 10",
            ])
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        let wrong = wait_for_panel_exit(
            child.id(),
            &std::env::current_exe().unwrap(),
            std::time::Duration::ZERO,
        );
        let timeout = wait_for_panel_exit(child.id(), &shell, std::time::Duration::from_millis(10));
        let still_alive = child.try_wait().unwrap().is_none();
        child.kill().unwrap();
        child.wait().unwrap();
        let exited = wait_for_panel_exit(child.id(), &shell, std::time::Duration::from_secs(1));
        assert_eq!(
            wrong.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert_eq!(timeout.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(still_alive);
        assert!(exited.is_ok());
    }
    #[test]
    fn rejects_self_and_zero_without_waiting() {
        for pid in [0, std::process::id()] {
            assert_eq!(
                wait_for_panel_exit(
                    pid,
                    &std::env::current_exe().unwrap(),
                    std::time::Duration::ZERO
                )
                .unwrap_err()
                .kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
    }
}

#[cfg(windows)]
pub fn elevate_current_driver_installer() -> std::io::Result<u32> {
    use std::{mem::size_of, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
        UI::{
            Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };
    let exe = std::env::current_exe()?;
    let path: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let args: Vec<u16> = "--install\0".encode_utf16().collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: verb.as_ptr(),
        lpFile: path.as_ptr(),
        lpParameters: args.as_ptr(),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    // SAFETY: all strings are terminated and alive throughout the synchronous launch.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if info.hProcess.is_null() {
        return Err(std::io::Error::other(
            "Windows returned no installer process",
        ));
    }
    // Wait for the user's elevated installer, including its result dialog. Never report
    // cancellation or a security-product termination as success, or forcibly kill it.
    // SAFETY: ShellExecuteExW returned an owned process handle; it is closed below.
    let result = unsafe {
        if WaitForSingleObject(info.hProcess, INFINITE) != WAIT_OBJECT_0 {
            Err(std::io::Error::last_os_error())
        } else {
            let mut code = 0;
            if GetExitCodeProcess(info.hProcess, &mut code) == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(code)
            }
        }
    };
    // SAFETY: this is the only close of the owned handle.
    unsafe { CloseHandle(info.hProcess) };
    result
}

#[cfg(not(windows))]
pub fn elevate_current_driver_installer() -> std::io::Result<u32> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(test)]
mod outcome_tests {
    use super::*;
    #[test]
    fn handoff_round_trips_closed_results_and_rejects_arbitrary_arguments() {
        for outcome in [
            DriverSetupOutcome::Ready,
            DriverSetupOutcome::RestartRequired,
            DriverSetupOutcome::RestartRequiredAfterFailure,
            DriverSetupOutcome::Cancelled,
            DriverSetupOutcome::UnsupportedInterface,
            DriverSetupOutcome::NotReady,
            DriverSetupOutcome::Disconnected,
            DriverSetupOutcome::ValidationFailed,
            DriverSetupOutcome::Failed,
        ] {
            assert_eq!(
                DriverSetupOutcome::parse_argument(&outcome.argument()),
                Some(outcome)
            );
            assert_eq!(
                DriverSetupOutcome::from_exit_code(outcome.exit_code()),
                outcome
            );
            assert!(!outcome.message().is_empty());
        }
        for arg in [
            "ready",
            "--driver-setup-result=unknown",
            "--driver-setup-result=ready --install",
            "--driver-setup-result=READY",
        ] {
            assert!(DriverSetupOutcome::parse_argument(arg).is_none());
        }
        assert_eq!(
            DriverSetupOutcome::from_exit_code(0xc0000005),
            DriverSetupOutcome::Failed
        );
    }
    #[test]
    fn successful_process_without_complete_report_never_means_installed() {
        use DriverSetupOutcome as O;
        assert_eq!(
            O::from_script_output("DRIVER_SETUP_RESULT=ready\r\n", true),
            O::Ready
        );
        assert_eq!(
            O::from_script_output("DRIVER_SETUP_RESULT=restart-required\n", true),
            O::RestartRequired
        );
        assert_eq!(
            O::from_script_output("DRIVER_SETUP_RESULT=unsupported-interface\n", false),
            O::UnsupportedInterface
        );
        assert_eq!(
            O::from_script_output("DRIVER_SETUP_RESULT=ready\n", false),
            O::Failed
        );
        assert_eq!(
            O::from_script_output("DRIVER_SETUP_RESULT=not-ready\n", true),
            O::Failed
        );
        for text in [
            "",
            "Installation finished",
            "DRIVER_SETUP_RESULT=bogus",
            "DRIVER_SETUP_RESULT=ready\nDRIVER_SETUP_RESULT=ready",
            "prefix DRIVER_SETUP_RESULT=ready",
        ] {
            assert_eq!(O::from_script_output(text, true), O::Failed);
        }
    }
    #[test]
    fn cancelled_restart_and_unsupported_do_not_schedule_automatic_recheck() {
        use DriverSetupOutcome as O;
        for outcome in [
            O::RestartRequired,
            O::RestartRequiredAfterFailure,
            O::Cancelled,
            O::UnsupportedInterface,
            O::ValidationFailed,
            O::Failed,
        ] {
            assert!(!outcome.allows_recheck());
        }
        for outcome in [O::Ready, O::NotReady, O::Disconnected] {
            assert!(outcome.allows_recheck());
        }
    }
    #[test]
    fn panel_return_path_requires_existing_fixed_sibling() {
        let dir = std::env::temp_dir().join(format!(
            "dji4g-handoff-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let installer = dir.join("dji4g-driver-setup.exe");
        assert!(verified_panel_sibling(&installer).is_err());
        let panel = dir.join("dji4g-panel.exe");
        std::fs::write(&panel, b"test file, never executed").unwrap();
        assert_eq!(verified_panel_sibling(&installer).unwrap(), panel);
        std::fs::remove_file(panel).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}

#[test]
fn partial_failure_with_restart_has_distinct_failed_process_marker() {
    let outcome =
        DriverSetupOutcome::parse_argument("--driver-setup-result=restart-required-after-failure")
            .expect("closed partial-restart result");
    assert_eq!(outcome.exit_code(), 24);
    assert_eq!(DriverSetupOutcome::from_exit_code(24), outcome);
    assert_eq!(
        DriverSetupOutcome::from_script_output(
            "DRIVER_SETUP_RESULT=restart-required-after-failure\n",
            false
        ),
        outcome
    );
    assert_eq!(
        DriverSetupOutcome::from_script_output(
            "DRIVER_SETUP_RESULT=restart-required-after-failure\n",
            true
        ),
        DriverSetupOutcome::Failed
    );
    assert!(!outcome.allows_recheck());
    assert!(outcome.message().contains("部分操作失败"));
    assert!(outcome.message().contains("重启"));
}
