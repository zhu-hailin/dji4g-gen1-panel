//! Native, fixed-action UAC launch for the sibling offline driver installer.
//! This does not change the helper's separate signature/IPC policy.

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
