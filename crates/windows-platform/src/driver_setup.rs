//! Native, fixed-action UAC launch for the sibling offline driver installer.
//! This does not change the helper's separate signature/IPC policy.

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
