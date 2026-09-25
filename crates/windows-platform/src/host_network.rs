//! Read-only host network observation; no DJI device or serial port is required.

use std::{path::PathBuf, time::SystemTime};

use dji4g_domain::{HostNetworkObservation, HostProxyMode};

use crate::{PlatformError, observe_host_adapters};

#[cfg(windows)]
pub fn roaming_app_data() -> Result<PathBuf, PlatformError> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_RoamingAppData, SHGetKnownFolderPath},
    };
    let mut path = std::ptr::null_mut();
    let result = unsafe {
        SHGetKnownFolderPath(&FOLDERID_RoamingAppData, 0, std::ptr::null_mut(), &mut path)
    };
    if result < 0 || path.is_null() {
        return Err(PlatformError {
            code: "host:known_folder_unavailable",
            os_code: Some(result as u32),
        });
    }
    let mut length = 0_usize;
    // SHGetKnownFolderPath returns a NUL-terminated CoTaskMem allocation.
    while length < 32768 && unsafe { *path.add(length) } != 0 {
        length += 1;
    }
    if length == 32768 {
        unsafe { CoTaskMemFree(path.cast()) };
        return Err(PlatformError {
            code: "host:known_folder_invalid",
            os_code: None,
        });
    }
    let value = std::ffi::OsString::from_wide(unsafe { std::slice::from_raw_parts(path, length) });
    unsafe { CoTaskMemFree(path.cast()) };
    Ok(PathBuf::from(value))
}

#[cfg(not(windows))]
pub fn roaming_app_data() -> Result<PathBuf, PlatformError> {
    Err(PlatformError {
        code: "host:unsupported_platform",
        os_code: None,
    })
}

#[cfg(windows)]
fn system_proxy_mode() -> Result<HostProxyMode, PlatformError> {
    use windows_sys::Win32::{
        Foundation::GlobalFree,
        Networking::WinHttp::{
            WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WinHttpGetIEProxyConfigForCurrentUser,
        },
    };
    let mut value = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
    if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut value) } == 0 {
        return Err(PlatformError {
            code: "host:proxy_settings_unavailable",
            os_code: None,
        });
    }
    let auto_detect = value.fAutoDetect != 0;
    let auto_config = !value.lpszAutoConfigUrl.is_null();
    let manual = !value.lpszProxy.is_null();
    for pointer in [
        value.lpszAutoConfigUrl,
        value.lpszProxy,
        value.lpszProxyBypass,
    ] {
        if !pointer.is_null() {
            unsafe { GlobalFree(pointer.cast()) };
        }
    }
    Ok(match (auto_detect, auto_config, manual) {
        (false, false, false) => HostProxyMode::Disabled,
        (false, false, true) => HostProxyMode::Manual,
        (false, true, false) => HostProxyMode::AutoConfig,
        (true, false, false) => HostProxyMode::AutoDetect,
        _ => HostProxyMode::Mixed,
    })
}

#[cfg(not(windows))]
fn system_proxy_mode() -> Result<HostProxyMode, PlatformError> {
    Err(PlatformError {
        code: "host:unsupported_platform",
        os_code: None,
    })
}

/// A failed adapter/route inventory prevents declaring a configured interface missing.  Proxy
/// settings can be unavailable independently; they then appear as Unknown, not Disabled.
pub fn inspect_host_network() -> Result<HostNetworkObservation, PlatformError> {
    let (adapters, default_routes) = observe_host_adapters()?;
    Ok(HostNetworkObservation {
        adapters,
        default_routes,
        system_proxy: system_proxy_mode().unwrap_or(HostProxyMode::Unknown),
        binding: None,
        proxy_inspection_complete: false,
        proxy_error_code: None,
        observed_at: SystemTime::now(),
    })
}
