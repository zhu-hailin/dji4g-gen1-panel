#![deny(unsafe_op_in_unsafe_fn)]

//! Windows platform adapters for the DJI 4G panel.

mod adapter;
pub mod autostart;
mod device_tools;
pub mod driver_setup;
pub mod host_network;
pub mod hotspot;
mod pnp;
mod privilege;
mod privilege_wintrust;
mod probe;
pub mod proxy_clients;
pub mod repair;
mod serial;
pub mod single_instance;
pub mod sms;
pub mod sms_archive_crypto;
mod sms_history;
pub use sms_history::sms_list_controlled;
pub mod tray;

pub use adapter::*;

/// Resolve an executable that ships with Windows, independently of PATH and user-supplied
/// environment variables.
///
/// `relative` is relative to the system directory (`C:\Windows\System32` by default), e.g.
/// `"schtasks.exe"` or `"WindowsPowerShell/v1.0/powershell.exe"`.
#[cfg(windows)]
pub fn system_executable(relative: &str) -> std::io::Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = vec![0u16; 32768];
    // SAFETY: the writable buffer holds exactly the advertised number of UTF-16 units.
    let length = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(std::io::Error::last_os_error());
    }
    Ok(std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length])).join(relative))
}

#[cfg(not(windows))]
pub fn system_executable(_relative: &str) -> std::io::Result<std::path::PathBuf> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Resolve the OS shell independently of PATH and user-supplied environment variables.
#[cfg(windows)]
pub fn driver_setup_powershell() -> std::io::Result<std::path::PathBuf> {
    system_executable("WindowsPowerShell/v1.0/powershell.exe")
}

#[cfg(not(windows))]
pub fn driver_setup_powershell() -> std::io::Result<std::path::PathBuf> {
    Err(std::io::ErrorKind::Unsupported.into())
}
pub use autostart::atomic_replace_file_public as atomic_replace_file;
pub use autostart::*;
pub use device_tools::*;
pub use hotspot::*;
pub use pnp::*;
pub use privilege::*;
pub use probe::*;
pub use repair::*;
pub use serial::*;
pub use single_instance::*;
pub use sms::*;
mod sms_delete;
pub use sms_delete::sms_delete_checked;
mod sms_transaction;
pub use sms_transaction::{SmsSubmitReceipt, sms_send_controlled};
pub use tray::{
    NativeTray, TrayError as NativeTrayError, TrayEvent, TrayLabels as NativeTrayLabels,
};

/// Show a system-default modal message box (MB_OK, information icon).  Blocking by design: the
/// caller keeps the UI modal until the user dismisses it.  Text is plain UTF-8 in, UTF-16 out.
#[cfg(windows)]
pub fn show_message_box(title: &str, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MessageBoxW,
    };

    let mut wide_title: Vec<u16> = title.encode_utf16().collect();
    wide_title.push(0);
    let mut wide_text: Vec<u16> = text.encode_utf16().collect();
    wide_text.push(0);
    // SAFETY: both wide strings are NUL-terminated for the duration of the modal call; no owner
    // window is required for an informational box.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide_text.as_ptr(),
            wide_title.as_ptr(),
            MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
        );
    }
}

/// Show a system-default modal confirm box (Yes/No via `TaskDialogIndirect`).  Blocking by
/// design: the caller keeps the UI modal until the user answers.  Returns `true` for Yes,
/// `false` for No, Cancel, or any failure to present the box.  This is the official native
/// confirmation surface: the same task dialog every Windows application uses, localized by the
/// system.  `owner` is the raw `HWND` of the panel window (when available): the box is then
/// modal to it, so Windows itself disables the panel for the duration and a second click cannot
/// stack another box.  `TDF_ALLOW_DIALOG_CANCELLATION` keeps the title-bar close (X) and Escape
/// live even though the common buttons are only Yes/No; either dismisses the box without
/// approving.
#[cfg(windows)]
pub fn confirm_message_box(owner: Option<isize>, title: &str, text: &str) -> bool {
    use std::mem::{size_of, zeroed};

    use windows_sys::Win32::Foundation::{HWND, S_OK};
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows_sys::Win32::UI::Controls::{
        TASKDIALOGCONFIG, TD_WARNING_ICON, TDCBF_NO_BUTTON, TDCBF_YES_BUTTON,
        TDF_ALLOW_DIALOG_CANCELLATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDNO, IDYES};

    /// The export is only guaranteed when the process binds to the v6 common controls; a static
    /// `windows-sys` import would make the whole executable fail to start on machines without
    /// that binding (`STATUS_ENTRYPOINT_NOT_FOUND`). Resolving it lazily keeps the app
    /// launchable everywhere and lets this function fall back to the classic message box.
    type TaskDialogIndirect = unsafe extern "system" fn(
        *const TASKDIALOGCONFIG,
        *mut i32,
        *mut i32,
        *mut windows_sys::core::BOOL,
    ) -> windows_sys::core::HRESULT;

    fn task_dialog_indirect() -> Option<TaskDialogIndirect> {
        let library = "comctl32.dll\0".encode_utf16().collect::<Vec<u16>>();
        // SAFETY: `library` is NUL-terminated for the call. The module handle is deliberately
        // never freed: the process is long-lived and every later confirm reuses it.
        let module = unsafe { LoadLibraryW(library.as_ptr()) };
        if module.is_null() {
            return None;
        }
        // SAFETY: the name literal is NUL-terminated; the returned pointer is only used through
        // the cast below while comctl32 stays loaded.
        let name = b"TaskDialogIndirect\0";
        let entry = unsafe { GetProcAddress(module, name.as_ptr().cast()) }?;
        // SAFETY: the export has the TaskDialogIndirect ABI; GetProcAddress returns a generic
        // untyped FARPROC that this cast reinterprets as the typed callable.
        Some(unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, TaskDialogIndirect>(entry)
        })
    }

    let parent = owner.map_or(std::ptr::null_mut(), |hwnd| hwnd as HWND);
    let mut wide_title: Vec<u16> = title.encode_utf16().collect();
    wide_title.push(0);
    let mut wide_text: Vec<u16> = text.encode_utf16().collect();
    wide_text.push(0);

    if let Some(task_dialog) = task_dialog_indirect() {
        // The result and the default button take the classic ID* values (IDYES/IDNO/IDCANCEL),
        // not the TDCBF_* display flags: TDCBF_YES_BUTTON (2) collides with IDCANCEL, and a
        // non-matching nDefaultButton would fall back to the first button (Yes).
        let mut config: TASKDIALOGCONFIG = unsafe { zeroed() };
        config.cbSize = size_of::<TASKDIALOGCONFIG>() as u32;
        config.hwndParent = parent;
        config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION;
        config.dwCommonButtons = TDCBF_YES_BUTTON | TDCBF_NO_BUTTON;
        config.pszWindowTitle = wide_title.as_ptr();
        config.pszMainInstruction = wide_title.as_ptr();
        config.pszContent = wide_text.as_ptr();
        // Keep the focus on No so a stray Enter cannot approve a device-changing operation,
        // matching the MB_YESNO | MB_DEFBUTTON2 semantics of the old message box.
        config.nDefaultButton = IDNO;
        config.Anonymous1.pszMainIcon = TD_WARNING_ICON;

        let mut clicked = 0_i32;
        // SAFETY: both wide strings are NUL-terminated for the duration of the modal call, and
        // TaskDialogIndirect copies them before returning. The owner, when present, is the
        // panel's own still-alive window; ownership only makes the box modal to it, and a
        // missing owner falls back to the desktop.
        let result = unsafe {
            task_dialog(
                &config,
                &mut clicked,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if result == S_OK {
            return clicked == IDYES;
        }
        // The task dialog failed to present itself; fall through to the message box below.
    }
    message_box_yes_no(parent, &wide_title, &wide_text)
}

/// Classic `MessageBoxW` Yes/No confirm box. Its title-bar close (X) is disabled for the
/// MB_YESNO style on modern Windows, so `MB_CANCELBUTTON` (style bit 0x08000000; windows-sys
/// 0.61 does not generate the named constant) keeps an explicit Cancel button available. No is
/// the default button.
#[cfg(windows)]
fn message_box_yes_no(
    parent: windows_sys::Win32::Foundation::HWND,
    wide_title: &[u16],
    wide_text: &[u16],
) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_SETFOREGROUND, MB_YESNO, MessageBoxW,
    };
    const MB_CANCELBUTTON: u32 = 0x0800_0000;

    // SAFETY: both wide strings are NUL-terminated for the duration of the modal call; the
    // optional owner only participates in modality.
    let result = unsafe {
        MessageBoxW(
            parent,
            wide_text.as_ptr(),
            wide_title.as_ptr(),
            MB_YESNO | MB_CANCELBUTTON | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND,
        )
    };
    result == IDYES
}

#[cfg(not(windows))]
pub fn show_message_box(_title: &str, _text: &str) {}

#[cfg(not(windows))]
pub fn confirm_message_box(_owner: Option<isize>, _title: &str, _text: &str) -> bool {
    false
}
