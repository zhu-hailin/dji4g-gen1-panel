//! Windows login-start policy and the narrow HKCU Run adapter.
//!
//! The policy is intentionally separate from the application settings file: a configured intent
//! is not reported as enabled until the exact value written by this executable is observed again.
//! Package identity is a separate backend boundary and never silently falls back to HKCU Run.

use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::PlatformError;

const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE_NAME: &str = "Dji4GPanel";
const MAX_COMMAND_UNITS: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunValueType {
    RegSz,
    Other(u32),
}

impl RunValueType {
    #[must_use]
    pub const fn reg_sz() -> Self {
        Self::RegSz
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunValue {
    pub value_type: RunValueType,
    /// UTF-16 units without the storage-only trailing NUL.
    pub raw_utf16: Vec<u16>,
}

impl RunValue {
    #[must_use]
    pub fn new(value_type: RunValueType, raw_utf16: Vec<u16>) -> Self {
        Self {
            value_type,
            raw_utf16,
        }
    }

    #[must_use]
    pub fn reg_sz(raw_utf16: Vec<u16>) -> Self {
        Self::new(RunValueType::RegSz, raw_utf16)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutostartObservedState {
    Disabled,
    Enabled,
    Drift,
}

pub trait RunValueBackend: Clone + Send + Sync + 'static {
    fn read(&self) -> Result<Option<RunValue>, PlatformError>;
    fn write(&self, value: &RunValue) -> Result<(), PlatformError>;
    fn delete(&self) -> Result<(), PlatformError>;
}

#[derive(Clone, Debug)]
enum AutostartBackend<B> {
    Run(B),
    /// The StartupTask implementation is intentionally kept behind this boundary until the
    /// packaged manifest/runtime is available.  Crucially, this state returns an error rather
    /// than writing a second HKCU registration.
    StartupTask,
}

pub struct AutostartControl<B = WindowsRunValueBackend> {
    executable: PathBuf,
    expected: Vec<u16>,
    backend: AutostartBackend<B>,
}

impl<B: RunValueBackend> AutostartControl<B> {
    pub fn with_backend(executable: PathBuf, backend: B) -> Result<Self, PlatformError> {
        let expected = expected_command(&executable)?;
        Ok(Self {
            executable,
            expected,
            backend: AutostartBackend::Run(backend),
        })
    }

    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    #[must_use]
    pub fn expected(&self) -> &[u16] {
        &self.expected
    }

    pub fn status(&self) -> Result<AutostartObservedState, PlatformError> {
        match &self.backend {
            AutostartBackend::Run(backend) => {
                Ok(classify_value(backend.read()?.as_ref(), &self.expected))
            }
            AutostartBackend::StartupTask => Err(startup_task_error()),
        }
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<AutostartObservedState, PlatformError> {
        match &self.backend {
            AutostartBackend::StartupTask => Err(startup_task_error()),
            AutostartBackend::Run(backend) if enabled => {
                backend.write(&RunValue::reg_sz(self.expected.clone()))?;
                Ok(classify_value(backend.read()?.as_ref(), &self.expected))
            }
            AutostartBackend::Run(backend) => {
                let current = backend.read()?;
                // Never delete a same-name value unless its bytes match the exact command that
                // this process owns.  Drift is observable and recoverable by an explicit enable.
                if classify_value(current.as_ref(), &self.expected)
                    != AutostartObservedState::Enabled
                {
                    return Ok(classify_value(current.as_ref(), &self.expected));
                }
                backend.delete()?;
                Ok(classify_value(backend.read()?.as_ref(), &self.expected))
            }
        }
    }
}

impl AutostartControl<WindowsRunValueBackend> {
    /// Select the package StartupTask backend when this process has package identity; otherwise
    /// select the fixed current-user Run value.
    pub fn for_current_process() -> Result<Self, PlatformError> {
        let executable = std::env::current_exe().map_err(|error| PlatformError {
            code: "autostart:current_exe_failed",
            os_code: error
                .raw_os_error()
                .and_then(|value| u32::try_from(value).ok()),
        })?;
        let expected = expected_command(&executable)?;
        if current_process_has_package_identity()? {
            Ok(Self {
                executable,
                expected,
                backend: AutostartBackend::StartupTask,
            })
        } else {
            Ok(Self {
                executable,
                expected,
                backend: AutostartBackend::Run(WindowsRunValueBackend),
            })
        }
    }

    #[must_use]
    pub const fn uses_startup_task(&self) -> bool {
        matches!(self.backend, AutostartBackend::StartupTask)
    }
}

impl<B> fmt::Debug for AutostartControl<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AutostartControl")
            .field("executable", &"<redacted>")
            .field("backend", &self.backend_kind())
            .finish()
    }
}

impl<B> AutostartControl<B> {
    fn backend_kind(&self) -> &'static str {
        match &self.backend {
            AutostartBackend::Run(_) => "run",
            AutostartBackend::StartupTask => "startup_task",
        }
    }
}

#[must_use]
pub fn classify_value(actual: Option<&RunValue>, expected: &[u16]) -> AutostartObservedState {
    let Some(actual) = actual else {
        return AutostartObservedState::Disabled;
    };
    if actual.value_type != RunValueType::RegSz
        || actual.raw_utf16.contains(&0)
        || String::from_utf16(&actual.raw_utf16).is_err()
    {
        return AutostartObservedState::Drift;
    }
    if actual.raw_utf16 == expected {
        AutostartObservedState::Enabled
    } else {
        AutostartObservedState::Drift
    }
}

pub fn expected_command(executable: &Path) -> Result<Vec<u16>, PlatformError> {
    let mut result = quote_windows_arg(executable.as_os_str())?;
    result.extend(" --autostart".encode_utf16());
    if result.len() > MAX_COMMAND_UNITS {
        return Err(PlatformError {
            code: "autostart:path_too_long",
            os_code: None,
        });
    }
    Ok(result)
}

/// Quote exactly one Windows command-line argument.  Only the closing run of backslashes needs
/// doubling before the closing quote; embedded quotes and NULs are rejected rather than escaped.
pub fn quote_windows_arg(value: &OsStr) -> Result<Vec<u16>, PlatformError> {
    #[cfg(windows)]
    use std::os::windows::ffi::OsStrExt;

    #[cfg(windows)]
    let units = value.encode_wide().collect::<Vec<_>>();
    #[cfg(not(windows))]
    let units = value.to_string_lossy().encode_utf16().collect::<Vec<_>>();
    if units.is_empty()
        || units
            .iter()
            .any(|unit| *unit == 0 || *unit == u16::from(b'"'))
    {
        return Err(PlatformError {
            code: "autostart:path_invalid",
            os_code: None,
        });
    }
    let trailing_slashes = units
        .iter()
        .rev()
        .take_while(|unit| **unit == u16::from(b'\\'))
        .count();
    let mut result = Vec::with_capacity(units.len() + trailing_slashes + 2);
    result.push(u16::from(b'"'));
    result.extend(units);
    result.extend(std::iter::repeat_n(u16::from(b'\\'), trailing_slashes));
    result.push(u16::from(b'"'));
    Ok(result)
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryRunValueBackend {
    value: Arc<Mutex<Option<RunValue>>>,
}

impl InMemoryRunValueBackend {
    #[must_use]
    pub fn with(value: RunValue) -> Self {
        Self {
            value: Arc::new(Mutex::new(Some(value))),
        }
    }

    #[must_use]
    pub fn read_value(&self) -> Option<RunValue> {
        self.value.lock().expect("memory registry lock").clone()
    }
}

impl RunValueBackend for InMemoryRunValueBackend {
    fn read(&self) -> Result<Option<RunValue>, PlatformError> {
        Ok(self.read_value())
    }

    fn write(&self, value: &RunValue) -> Result<(), PlatformError> {
        *self.value.lock().expect("memory registry lock") = Some(value.clone());
        Ok(())
    }

    fn delete(&self) -> Result<(), PlatformError> {
        *self.value.lock().expect("memory registry lock") = None;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsRunValueBackend;

impl RunValueBackend for WindowsRunValueBackend {
    fn read(&self) -> Result<Option<RunValue>, PlatformError> {
        read_run_value()
    }

    fn write(&self, value: &RunValue) -> Result<(), PlatformError> {
        write_run_value(value)
    }

    fn delete(&self) -> Result<(), PlatformError> {
        delete_run_value()
    }
}

fn startup_task_error() -> PlatformError {
    PlatformError {
        code: "autostart:startup_task_unavailable",
        os_code: None,
    }
}

#[cfg(windows)]
fn current_process_has_package_identity() -> Result<bool, PlatformError> {
    const APPMODEL_ERROR_NO_PACKAGE: u32 = 15_700;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    let mut length = 0_u32;
    // SAFETY: the sizing call intentionally passes a null package-name buffer and a valid output
    // length pointer, matching GetCurrentPackageFullName's documented contract.
    let status = unsafe { GetCurrentPackageFullName(&mut length, std::ptr::null_mut()) };
    if status == APPMODEL_ERROR_NO_PACKAGE {
        return Ok(false);
    }
    if status != ERROR_INSUFFICIENT_BUFFER || length == 0 || length > 32 * 1024 {
        return Err(PlatformError {
            code: "autostart:package_identity_failed",
            os_code: Some(status),
        });
    }
    let mut name = vec![0_u16; length as usize];
    // SAFETY: `name` has exactly the capacity requested by the sizing call and remains live for
    // the duration of the second call.
    let status = unsafe { GetCurrentPackageFullName(&mut length, name.as_mut_ptr()) };
    if status == 0 {
        Ok(true)
    } else {
        Err(PlatformError {
            code: "autostart:package_identity_failed",
            os_code: Some(status),
        })
    }
}

#[cfg(not(windows))]
fn current_process_has_package_identity() -> Result<bool, PlatformError> {
    Ok(false)
}

#[cfg(windows)]
fn read_run_value() -> Result<Option<RunValue>, PlatformError> {
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_MORE_DATA: i32 = 234;
    const REG_SZ: u32 = 1;
    let key_name = wide_null(RUN_SUBKEY);
    let value_name = wide_null(RUN_VALUE_NAME);
    let mut key = std::ptr::null_mut();
    // SAFETY: static HKCU/key/value names are NUL-terminated and output handle is valid storage.
    let status = unsafe {
        autostart_reg_open_key_ex_w(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if status != 0 {
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        return Err(reg_error("autostart:registry_open_failed", status));
    }
    let result = (|| {
        let mut capacity = 256_usize;
        loop {
            if capacity > 32 * 1024 {
                return Err(PlatformError {
                    code: "autostart:registry_value_invalid",
                    os_code: None,
                });
            }
            let mut data = vec![0_u8; capacity];
            let mut value_type = 0_u32;
            let mut byte_len = data.len() as u32;
            // SAFETY: key/name/buffer/output lengths remain live and writable for this call.
            let status = unsafe {
                autostart_reg_query_value_ex_w(
                    key,
                    value_name.as_ptr(),
                    std::ptr::null_mut(),
                    &mut value_type,
                    data.as_mut_ptr(),
                    &mut byte_len,
                )
            };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            if status == ERROR_MORE_DATA {
                capacity = capacity.saturating_mul(2);
                continue;
            }
            if status != 0 || byte_len as usize > data.len() || byte_len % 2 != 0 {
                return Err(reg_error("autostart:registry_read_failed", status));
            }
            data.truncate(byte_len as usize);
            let raw = data
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>();
            let Some((&0, content)) = raw.split_last() else {
                return Err(PlatformError {
                    code: "autostart:registry_value_invalid",
                    os_code: None,
                });
            };
            if content.is_empty() || content.contains(&0) || String::from_utf16(content).is_err() {
                return Err(PlatformError {
                    code: "autostart:registry_value_invalid",
                    os_code: None,
                });
            }
            return Ok(Some(RunValue {
                value_type: if value_type == REG_SZ {
                    RunValueType::RegSz
                } else {
                    RunValueType::Other(value_type)
                },
                raw_utf16: content.to_vec(),
            }));
        }
    })();
    // SAFETY: key is owned by this function and is closed once on every successful open.
    unsafe { autostart_reg_close_key(key) };
    result
}

#[cfg(not(windows))]
fn read_run_value() -> Result<Option<RunValue>, PlatformError> {
    Err(PlatformError {
        code: "autostart:unsupported_platform",
        os_code: None,
    })
}

#[cfg(windows)]
fn write_run_value(value: &RunValue) -> Result<(), PlatformError> {
    const KEY_SET_VALUE: u32 = 0x0002;
    const REG_SZ: u32 = 1;
    let key_name = wide_null(RUN_SUBKEY);
    let value_name = wide_null(RUN_VALUE_NAME);
    let mut key = std::ptr::null_mut();
    // SAFETY: static key name and output storage are valid; only HKCU and KEY_SET_VALUE are used.
    let status = unsafe {
        autostart_reg_open_key_ex_w(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if status != 0 {
        return Err(reg_error("autostart:registry_open_failed", status));
    }
    let mut units = value.raw_utf16.clone();
    units.push(0);
    let bytes = units
        .iter()
        .flat_map(|unit| unit.to_le_bytes())
        .collect::<Vec<_>>();
    // SAFETY: key/name/data are valid for the exact byte length and are not retained by the API.
    let status = unsafe {
        autostart_reg_set_value_ex_w(
            key,
            value_name.as_ptr(),
            0,
            REG_SZ,
            bytes.as_ptr(),
            u32::try_from(bytes.len()).unwrap_or(u32::MAX),
        )
    };
    // SAFETY: key is owned by this function and closed exactly once.
    unsafe { autostart_reg_close_key(key) };
    if status == 0 {
        Ok(())
    } else {
        Err(reg_error("autostart:registry_write_failed", status))
    }
}

#[cfg(not(windows))]
fn write_run_value(_value: &RunValue) -> Result<(), PlatformError> {
    Err(PlatformError {
        code: "autostart:unsupported_platform",
        os_code: None,
    })
}

#[cfg(windows)]
fn delete_run_value() -> Result<(), PlatformError> {
    const KEY_SET_VALUE: u32 = 0x0002;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    let key_name = wide_null(RUN_SUBKEY);
    let value_name = wide_null(RUN_VALUE_NAME);
    let mut key = std::ptr::null_mut();
    // SAFETY: static key name and output storage are valid; only HKCU and KEY_SET_VALUE are used.
    let status = unsafe {
        autostart_reg_open_key_ex_w(
            HKEY_CURRENT_USER,
            key_name.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if status != 0 {
        return if status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(reg_error("autostart:registry_open_failed", status))
        };
    }
    // SAFETY: key and static value name are valid; only this fixed value is addressed.
    let status = unsafe { autostart_reg_delete_value_w(key, value_name.as_ptr()) };
    // SAFETY: key is owned by this function and closed exactly once.
    unsafe { autostart_reg_close_key(key) };
    if status == 0 || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(reg_error("autostart:registry_delete_failed", status))
    }
}

#[cfg(not(windows))]
fn delete_run_value() -> Result<(), PlatformError> {
    Err(PlatformError {
        code: "autostart:unsupported_platform",
        os_code: None,
    })
}

#[cfg(windows)]
fn atomic_replace_file(temporary: &Path, destination: &Path) -> Result<(), PlatformError> {
    const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
    const ERROR_FILE_NOT_FOUND: u32 = 2;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    const REPLACEFILE_WRITE_THROUGH: u32 = 0x0000_0001;
    let temporary = wide_path(temporary);
    let destination = wide_path(destination);
    // SAFETY: destination is an owned NUL-terminated UTF-16 path.
    let attributes = unsafe { GetFileAttributesW(destination.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        let error = last_error();
        if error != ERROR_FILE_NOT_FOUND {
            return Err(PlatformError {
                code: "config:replace_stat_failed",
                os_code: Some(error),
            });
        }
        // SAFETY: both paths are valid NUL-terminated UTF-16 strings; flags request write-through
        // replacement without first deleting the canonical file.
        let ok = unsafe {
            MoveFileExW(
                temporary.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        return if ok != 0 {
            Ok(())
        } else {
            Err(PlatformError {
                code: "config:replace_failed",
                os_code: Some(last_error()),
            })
        };
    }
    // SAFETY: source/destination are same-volume sibling paths owned by this operation; the
    // destination is replaced atomically by ReplaceFileW and no prior delete occurs.
    let ok = unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            temporary.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(PlatformError {
            code: "config:replace_failed",
            os_code: Some(last_error()),
        })
    }
}

#[cfg(not(windows))]
fn atomic_replace_file(temporary: &Path, destination: &Path) -> Result<(), PlatformError> {
    std::fs::rename(temporary, destination).map_err(|error| PlatformError {
        code: "config:replace_failed",
        os_code: error
            .raw_os_error()
            .and_then(|value| u32::try_from(value).ok()),
    })
}

pub fn atomic_replace_file_public(
    temporary: &Path,
    destination: &Path,
) -> Result<(), PlatformError> {
    atomic_replace_file(temporary, destination)
}

fn reg_error(code: &'static str, os_code: i32) -> PlatformError {
    PlatformError {
        code,
        os_code: u32::try_from(os_code).ok(),
    }
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

#[cfg(windows)]
fn wide_path(value: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.as_os_str().encode_wide().chain([0]).collect()
}

#[cfg(windows)]
fn last_error() -> u32 {
    // SAFETY: GetLastError has no preconditions and returns the calling thread's last status.
    unsafe { GetLastError() }
}

#[cfg(windows)]
const HKEY_CURRENT_USER: *mut std::ffi::c_void = -2_147_483_647_isize as *mut std::ffi::c_void;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentPackageFullName(
        package_full_name_length: *mut u32,
        package_full_name: *mut u16,
    ) -> u32;
    fn GetFileAttributesW(file_name: *const u16) -> u32;
    fn GetLastError() -> u32;
    fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
    fn ReplaceFileW(
        replaced_file_name: *const u16,
        replacement_file_name: *const u16,
        backup_file_name: *const u16,
        replace_flags: u32,
        exclude: *mut std::ffi::c_void,
        reserved: *mut std::ffi::c_void,
    ) -> i32;
}

#[cfg(windows)]
#[link(name = "advapi32")]
unsafe extern "system" {
    #[link_name = "RegCloseKey"]
    fn autostart_reg_close_key(key: *mut std::ffi::c_void) -> i32;
    #[link_name = "RegDeleteValueW"]
    fn autostart_reg_delete_value_w(key: *mut std::ffi::c_void, value_name: *const u16) -> i32;
    #[link_name = "RegOpenKeyExW"]
    fn autostart_reg_open_key_ex_w(
        key: *mut std::ffi::c_void,
        sub_key: *const u16,
        options: u32,
        sam_desired: u32,
        result: *mut *mut std::ffi::c_void,
    ) -> i32;
    #[link_name = "RegQueryValueExW"]
    fn autostart_reg_query_value_ex_w(
        key: *mut std::ffi::c_void,
        value_name: *const u16,
        reserved: *mut u32,
        value_type: *mut u32,
        data: *mut u8,
        data_size: *mut u32,
    ) -> i32;
    #[link_name = "RegSetValueExW"]
    fn autostart_reg_set_value_ex_w(
        key: *mut std::ffi::c_void,
        value_name: *const u16,
        reserved: u32,
        value_type: u32,
        data: *const u8,
        data_size: u32,
    ) -> i32;
}
