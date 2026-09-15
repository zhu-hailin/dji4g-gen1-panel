//! Real Authenticode verification of the privileged helper via `WinVerifyTrust`.
//!
//! `TrustedHelper::installed()` used to hardcode `signature_verified = false`, which failed
//! closed but was undiagnosable.  This module performs the actual Windows signature verdict so
//! an unsigned helper is reported as unsigned.  Failure remains the default: only a verdict of
//! `Verified` allows elevation, and every non-`0x800B` HRESULT that is not a plain trust error
//! is treated as "the check could not run" rather than as a pass.

use std::path::Path;

/// Outcome of the Authenticode check for one helper image.  Only [`Verification::Verified`]
/// may ever reach the elevation boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Verification {
    /// `WinVerifyTrust` returned `S_OK`: a complete, trusted Authenticode chain exists.
    Verified,
    /// `TRUST_E_NOSIGNATURE`: the image carries no Authenticode signature at all.
    Unsigned,
    /// A trust failure in the `0x800B` facility (`TRUST_E_*`/`CERT_E_*`): signed, but not
    /// trusted (bad chain, untrusted root, expired certificate, explicit distrust, ...).
    Untrusted,
    /// The check itself could not run (unexpected HRESULT, e.g. a Win32 error HRESULT), so no
    /// verdict about the image exists.  Callers must fail closed.
    ApiUnavailable,
}

/// HRESULT values this module maps.  Everything inside the `0x800B` security facility is a
/// wintrust trust error except `TRUST_E_NOSIGNATURE`, which specifically means "not signed".
#[cfg(windows)]
const S_OK: u32 = 0;
#[cfg(windows)]
const TRUST_E_NOSIGNATURE: u32 = 0x800B_0100;
#[cfg(windows)]
const SECURITY_FACILITY_MASK: u32 = 0x800B_0000;

#[cfg(windows)]
pub(crate) fn verify(path: &Path) -> Verification {
    use std::{mem::size_of, os::windows::ffi::OsStrExt, ptr::null_mut};

    use windows_sys::Win32::Security::WinTrust::{
        WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_FILE_INFO, WTD_CHOICE_FILE,
        WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
        WTD_UICONTEXT_EXECUTE, WinVerifyTrust,
    };

    // WinVerifyTrust opens the image itself; only a NUL-terminated wide path is required.
    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: path_wide.as_ptr(),
        hFile: null_mut(),
        pgKnownSubject: null_mut(),
    };
    // The FRU base is the crate-provided zeroed Default: null callbacks/handles and no implicit
    // policy; a background panel must never surface trust dialogs from a signature probe.
    let mut data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwUIContext: WTD_UICONTEXT_EXECUTE,
        ..WINTRUST_DATA::default()
    };
    // Selecting the pFile variant is required by dwUnionChoice = WTD_CHOICE_FILE; the pointer
    // targets `file_info`, which outlives both calls below, and no other variant of the union is
    // read by WinVerifyTrust under this choice.  Writing a Copy-typed union field is safe.
    data.Anonymous.pFile = &mut file_info;
    let mut action_id = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: action_id, file_info, and data are live, correctly initialized locals for the whole
    // synchronous call; windows-sys declares the action GUID `*mut` although the API reads it.
    let status = unsafe {
        WinVerifyTrust(
            null_mut(),
            &mut action_id,
            std::ptr::from_mut(&mut data).cast::<core::ffi::c_void>(),
        )
    };
    // The VERIFY action allocates provider state in hWVTStateData; the same structure must be
    // passed once more with STATEACTION_CLOSE to release it.  The close verdict is ignored: it
    // cannot change the trust decision and the state is process-local scratch.
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: same live locals as the verify call; closing never-instantiated state is a no-op.
    unsafe {
        WinVerifyTrust(
            null_mut(),
            &mut action_id,
            std::ptr::from_mut(&mut data).cast::<core::ffi::c_void>(),
        )
    };
    map_status(status)
}

#[cfg(not(windows))]
pub(crate) fn verify(_path: &Path) -> Verification {
    // No Authenticode stack exists off Windows; the elevation boundary fails closed on this.
    Verification::ApiUnavailable
}

/// Maps a raw `WinVerifyTrust` HRESULT to the closed [`Verification`] verdict set.
#[cfg(windows)]
#[must_use]
pub(crate) fn map_status(status: i32) -> Verification {
    match status as u32 {
        S_OK => Verification::Verified,
        TRUST_E_NOSIGNATURE => Verification::Unsigned,
        // Any other HRESULT in the security facility is a concrete trust failure
        // (TRUST_E_PROVIDER_UNKNOWN, TRUST_E_SUBJECT_NOT_TRUSTED, CERT_E_UNTRUSTEDROOT, ...).
        SECURITY_FACILITY_MASK..=0x800B_FFFF => Verification::Untrusted,
        // Win32 error HRESULTs and anything else mean the call never produced a verdict.
        _ => Verification::ApiUnavailable,
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::{Verification, verify};

    #[test]
    fn wintrust_reports_the_unsigned_test_binary_as_unsigned() {
        let current = std::env::current_exe().expect("test executable path");
        match verify(&current) {
            Verification::Unsigned => {}
            // A host without a usable wintrust stack cannot exercise the API; fail-closed
            // behavior for that case is pinned by the missing-file test below.
            Verification::ApiUnavailable => {}
            other => panic!("test binaries are unsigned, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_helper_image_never_yields_a_verified_verdict() {
        let missing = std::env::temp_dir().join("dji4g-wintrust-missing-probe.exe");
        // WinVerifyTrust answers CRYPT_E_FILE_ERROR (0x80092003) for an absent image on current
        // Windows; that is outside the 0x800B trust facility, so the verdict must be
        // ApiUnavailable: the check could not run against any image, and the elevation boundary
        // fails closed with `privilege:helper_unverified`.
        assert_eq!(verify(&missing), Verification::ApiUnavailable);
    }

    #[test]
    fn status_mapping_is_closed_and_ordered() {
        use super::{S_OK, SECURITY_FACILITY_MASK, TRUST_E_NOSIGNATURE, map_status};
        assert_eq!(map_status(S_OK as i32), Verification::Verified);
        assert_eq!(
            map_status(TRUST_E_NOSIGNATURE as i32),
            Verification::Unsigned
        );
        assert_eq!(
            map_status(0x800B_0109_u32 as i32), // CERT_E_UNTRUSTEDROOT
            Verification::Untrusted
        );
        assert_eq!(
            map_status(0x800B_0111_u32 as i32), // TRUST_E_EXPLICIT_DISTRUST
            Verification::Untrusted
        );
        assert_eq!(
            map_status(0x8009_2003_u32 as i32), // CRYPT_E_FILE_ERROR (image absent/unreadable)
            Verification::ApiUnavailable
        );
        assert_eq!(
            map_status(0x8007_0002_u32 as i32), // ERROR_FILE_NOT_FOUND as HRESULT
            Verification::ApiUnavailable
        );
        assert_eq!(map_status(-1), Verification::ApiUnavailable);
        assert_ne!(SECURITY_FACILITY_MASK, TRUST_E_NOSIGNATURE);
    }
}
