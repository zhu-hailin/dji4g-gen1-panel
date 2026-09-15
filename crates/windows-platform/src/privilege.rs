//! Authenticated one-shot helper transport and elevation boundary.
//!
//! This module owns the small amount of Win32 FFI needed for a per-operation named pipe and
//! `runas` launch.  Repair dispatch is deliberately delegated to the typed executor in
//! `repair.rs`; the helper never accepts a raw command, path, COM name, or display name.

use std::{
    fmt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use dji4g_ipc::{
    ClientError, Deadline, FrameError, Hash32, HelperActionV1, HelperRequestV1, HelperResponseV1,
    HelperResultV1, MAX_IO_WAIT, MAX_OPERATION_LIFETIME, OperationNonce, PeerExpectation,
    PeerIdentity, PeerRejectCode, PipeClient, PipeName, PipeTransport, ProtocolVersion,
    RequestRejectCode, SupportedProfileV1, TransportError, validate_request,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrivilegeError {
    UnsupportedPlatform,
    Protocol(RequestRejectCode),
    Peer(PeerRejectCode),
    Timeout,
    OperationCancelled,
    /// The helper image carries no Authenticode signature (`TRUST_E_NOSIGNATURE`).
    HelperUnsigned,
    /// The helper image is signed but its trust chain was rejected (`TRUST_E_*`/`CERT_E_*`).
    HelperUntrusted,
    /// The Authenticode check could not run, so no verdict about the helper exists.
    HelperVerificationUnavailable,
    LaunchFailed,
    Internal,
}

impl fmt::Display for PrivilegeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedPlatform => "privilege:unsupported_platform",
            Self::Protocol(_) => "privilege:protocol_rejected",
            Self::Peer(_) => "privilege:peer_rejected",
            Self::Timeout => "privilege:timeout",
            Self::OperationCancelled => "privilege:operation_cancelled",
            Self::HelperUnsigned => "privilege:helper_unsigned",
            Self::HelperUntrusted => "privilege:helper_untrusted",
            Self::HelperVerificationUnavailable => "privilege:helper_unverified",
            Self::LaunchFailed => "privilege:launch_failed",
            Self::Internal => "privilege:internal",
        })
    }
}

impl std::error::Error for PrivilegeError {}

/// An installed helper proof.  The constructor is intentionally explicit: a portable or unsigned
/// path is not considered trusted merely because it has the right filename.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedHelper {
    canonical_path: PathBuf,
    image_hash: Hash32,
    signature_verified: bool,
}

impl TrustedHelper {
    /// Finds the sibling helper, verifies its Authenticode signature with `WinVerifyTrust`, and
    /// refuses to elevate unless installation/signature policy is proven.  Development and
    /// portable binaries therefore fail closed with a diagnosable reason (unsigned, untrusted,
    /// or unverifiable).
    pub fn installed() -> Result<Self, PrivilegeError> {
        let current = std::env::current_exe().map_err(|_| PrivilegeError::HelperUntrusted)?;
        let sibling = current
            .parent()
            .ok_or(PrivilegeError::HelperUntrusted)?
            .join("dji4g-helper.exe");
        let verification = crate::privilege_wintrust::verify(&sibling);
        Self::from_path_with_verification(sibling, verification)
    }

    /// Builds a proof after the installer/signature verifier has established the protected path.
    /// This function only accepts a canonical executable beneath Program Files and an explicit
    /// `signature_verified` bit; callers must not pass user-controlled paths or infer the bit from
    /// a filename.
    pub fn from_path_with_signature(
        path: impl AsRef<Path>,
        signature_verified: bool,
    ) -> Result<Self, PrivilegeError> {
        let verification = if signature_verified {
            crate::privilege_wintrust::Verification::Verified
        } else {
            crate::privilege_wintrust::Verification::Untrusted
        };
        Self::from_path_with_verification(path, verification)
    }

    /// Shared constructor: the signature verdict is resolved into its specific error first so the
    /// diagnosable reason wins over the (weaker) path-shape checks, and the digest step is only
    /// ever reached for a verified image in a protected location.
    fn from_path_with_verification(
        path: impl AsRef<Path>,
        verification: crate::privilege_wintrust::Verification,
    ) -> Result<Self, PrivilegeError> {
        match verification {
            crate::privilege_wintrust::Verification::Verified => {}
            crate::privilege_wintrust::Verification::Unsigned => {
                return Err(PrivilegeError::HelperUnsigned);
            }
            crate::privilege_wintrust::Verification::Untrusted => {
                return Err(PrivilegeError::HelperUntrusted);
            }
            crate::privilege_wintrust::Verification::ApiUnavailable => {
                return Err(PrivilegeError::HelperVerificationUnavailable);
            }
        }
        let canonical_path =
            std::fs::canonicalize(path).map_err(|_| PrivilegeError::HelperUntrusted)?;
        if !is_protected_install_path(&canonical_path) {
            return Err(PrivilegeError::HelperUntrusted);
        }
        Ok(Self {
            image_hash: digest_file(&canonical_path)?,
            canonical_path,
            signature_verified: true,
        })
    }

    /// Test/integration seam for a verifier that already checked the exact canonical path and
    /// Authenticode chain.  It still applies the same protected-directory check.
    pub fn from_verified_install(path: impl AsRef<Path>) -> Result<Self, PrivilegeError> {
        Self::from_path_with_signature(path, true)
    }

    /// Development-mode proof for a portable, unsigned build: the sibling helper image next to
    /// this process, canonicalized and digested but deliberately **not** signature-verified.
    ///
    /// This constructor is only ever reached when [`is_dev_build`] is true — i.e. this panel
    /// process itself carries no Authenticode signature — so a signed installation can never
    /// take this path and the signed production boundary stays fail-closed.
    pub fn dev_sibling() -> Result<Self, PrivilegeError> {
        let current = std::env::current_exe().map_err(|_| PrivilegeError::HelperUntrusted)?;
        let sibling = current
            .parent()
            .ok_or(PrivilegeError::HelperUntrusted)?
            .join("dji4g-helper.exe");
        let canonical_path =
            std::fs::canonicalize(&sibling).map_err(|_| PrivilegeError::HelperUntrusted)?;
        Ok(Self {
            image_hash: digest_file(&canonical_path)?,
            canonical_path,
            signature_verified: false,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.canonical_path
    }

    #[must_use]
    pub const fn image_hash(&self) -> Hash32 {
        self.image_hash
    }

    #[must_use]
    pub const fn signature_verified(&self) -> bool {
        self.signature_verified
    }
}

fn is_protected_install_path(path: &Path) -> bool {
    let Some(program_files) = std::env::var_os("ProgramFiles") else {
        return false;
    };
    let Ok(program_files) = std::fs::canonicalize(program_files) else {
        return false;
    };
    path.starts_with(program_files)
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
}

/// Whether this process image carries no Authenticode signature — the portable development
/// build. This gates the documented development-mode helper exemption: an unsigned panel may
/// elevate its unsigned sibling helper with an explicit warning, while a signed installation
/// can never downgrade the trust boundary.
#[must_use]
pub fn is_dev_build() -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    !matches!(
        crate::privilege_wintrust::verify(&current),
        crate::privilege_wintrust::Verification::Verified
    )
}

fn digest_file(path: &Path) -> Result<Hash32, PrivilegeError> {
    let bytes = std::fs::read(path).map_err(|_| PrivilegeError::HelperUntrusted)?;
    Ok(digest_bytes(&bytes))
}

fn digest_bytes(bytes: &[u8]) -> Hash32 {
    // SHA-256 is used for process-image and identity equality. It is not a signature verifier;
    // installer/signing proof and Windows token checks still provide the trust decision.
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bfe8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut data = bytes.to_vec();
    let bit_len = (data.len() as u64).saturating_mul(8);
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());
    let mut state = [
        0x6a09_e667_u32,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    for chunk in data.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, word) in words[..16].iter_mut().enumerate() {
            *word = u32::from_be_bytes([
                chunk[index * 4],
                chunk[index * 4 + 1],
                chunk[index * 4 + 2],
                chunk[index * 4 + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h) = (
            state[0], state[1], state[2], state[3], state[4], state[5], state[6], state[7],
        );
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut digest = [0_u8; 32];
    for (index, word) in state.into_iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    Hash32::from_bytes(digest)
}

#[cfg(not(windows))]
pub fn launch_elevated_helper(
    _helper: &TrustedHelper,
    _request: HelperRequestV1,
    _now: SystemTime,
) -> Result<HelperResponseV1, PrivilegeError> {
    Err(PrivilegeError::UnsupportedPlatform)
}

#[cfg(not(windows))]
pub fn run_helper_once(
    _pipe: PipeName,
    _nonce: OperationNonce,
    _protocol: ProtocolVersion,
) -> Result<(), PrivilegeError> {
    Err(PrivilegeError::UnsupportedPlatform)
}

#[cfg(not(windows))]
pub struct WindowsNamedPipeServer;

#[cfg(not(windows))]
pub struct WindowsNamedPipeClient;

#[cfg(windows)]
mod native {
    use super::*;
    use crate::{
        DjiDevice, RepairAction, RepairError, WindowsDeviceInventory, WindowsNativeRepairBackend,
        WindowsRepairExecutor,
    };
    use dji4g_at_protocol::{Apn, PdpContextId, VerifiedUsbNetProfile};
    use dji4g_domain::DnsProfile;
    use dji4g_domain::{BeforeStateHash, DeviceEpoch, OperationOutcome};
    use std::{
        mem::{size_of, zeroed},
        ptr::{null, null_mut},
    };

    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
            LocalFree,
        },
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            GetTokenInformation, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER,
            TokenIntegrityLevel, TokenSessionId, TokenUser,
        },
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE,
            OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
        },
        System::{
            IO::{CancelIoEx, GetOverlappedResultEx, OVERLAPPED},
            Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
                GetNamedPipeClientProcessId, GetNamedPipeServerProcessId, PIPE_READMODE_MESSAGE,
                PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT, PeekNamedPipe,
            },
            Threading::{
                CreateEventW, GetCurrentProcess, GetProcessId, GetProcessTimes, OpenProcess,
                OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
                WaitForSingleObject,
            },
        },
        UI::{
            Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };

    const ERROR_IO_PENDING: u32 = 997;
    const ERROR_MORE_DATA: u32 = 234;
    const ERROR_PIPE_CONNECTED: u32 = 535;
    const ERROR_BROKEN_PIPE: u32 = 109;
    const ERROR_CANCELLED: u32 = 1223;
    const ERROR_NO_DATA: u32 = 232;
    const PROCESS_QUERY_LIMITED_INFORMATION_FLAGS: u32 = PROCESS_QUERY_LIMITED_INFORMATION;

    struct OwnedHandle(HANDLE);

    impl OwnedHandle {
        fn new(handle: HANDLE) -> Result<Self, PrivilegeError> {
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                Err(PrivilegeError::Internal)
            } else {
                Ok(Self(handle))
            }
        }

        #[must_use]
        const fn raw(&self) -> HANDLE {
            self.0
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: this handle is created by a successful Win32 call and uniquely owned here.
            unsafe { CloseHandle(self.0) };
        }
    }

    // SAFETY: an OwnedHandle has unique ownership and all operations borrow it synchronously.
    unsafe impl Send for OwnedHandle {}

    struct SecurityDescriptor {
        ptr: *mut core::ffi::c_void,
    }

    impl Drop for SecurityDescriptor {
        fn drop(&mut self) {
            if !self.ptr.is_null() {
                // SAFETY: pointer is returned by ConvertStringSecurityDescriptorToSecurityDescriptorW
                // and is released exactly once with LocalFree.
                unsafe { LocalFree(self.ptr) };
            }
        }
    }

    fn user_sid_material(process: HANDLE) -> Result<Vec<u8>, PrivilegeError> {
        let mut token = null_mut();
        // SAFETY: process is a live query handle and token points to writable HANDLE storage.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(PrivilegeError::Internal);
        }
        let token = OwnedHandle::new(token)?;
        let mut required = 0_u32;
        // SAFETY: first call only asks for required length and passes a null output buffer.
        unsafe { GetTokenInformation(token.raw(), TokenUser, null_mut(), 0, &mut required) };
        if required == 0 || required as usize > 64 * 1024 {
            return Err(PrivilegeError::Internal);
        }
        let mut buffer = vec![0_u8; required as usize];
        // SAFETY: buffer is exactly the size requested by the token API and remains alive for call.
        if unsafe {
            GetTokenInformation(
                token.raw(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(PrivilegeError::Internal);
        }
        // SAFETY: TOKEN_USER is the documented layout returned for TokenUser and buffer is aligned
        // sufficiently for the platform allocator; copying only the SID bytes avoids borrowing it.
        let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        let sid = user.User.Sid;
        // SAFETY: GetLengthSid reads the self-describing SID returned by the token query.
        let sid_length = unsafe { windows_sys::Win32::Security::GetLengthSid(sid) };
        if sid_length == 0 || sid_length > required {
            return Err(PrivilegeError::Internal);
        }
        // SAFETY: the SID belongs to the still-live token information buffer.
        Ok(unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), sid_length as usize) }.to_vec())
    }

    fn sid_text(process: HANDLE) -> Result<Vec<u16>, PrivilegeError> {
        let bytes = user_sid_material(process)?;
        let mut text = null_mut();
        // SAFETY: bytes contains a valid token SID and text points to writable PWSTR storage.
        if unsafe { ConvertSidToStringSidW(bytes.as_ptr().cast_mut().cast(), &mut text) } == 0 {
            return Err(PrivilegeError::Internal);
        }
        let mut length = 0_usize;
        // SAFETY: text is a NUL-terminated string allocated by the Windows API.
        unsafe {
            while *text.add(length) != 0 {
                length += 1;
                if length > 256 {
                    LocalFree(text.cast());
                    return Err(PrivilegeError::Internal);
                }
            }
            let result = std::slice::from_raw_parts(text, length).to_vec();
            LocalFree(text.cast());
            Ok(result)
        }
    }

    fn current_session(process: HANDLE) -> Result<u32, PrivilegeError> {
        let mut token = null_mut();
        // SAFETY: process and output token storage are valid for this synchronous query.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(PrivilegeError::Internal);
        }
        let token = OwnedHandle::new(token)?;
        let mut session = 0_u32;
        let mut returned = 0_u32;
        // SAFETY: session is writable u32 storage of the documented size.
        if unsafe {
            GetTokenInformation(
                token.raw(),
                TokenSessionId,
                (&mut session as *mut u32).cast(),
                size_of::<u32>() as u32,
                &mut returned,
            )
        } == 0
        {
            return Err(PrivilegeError::Internal);
        }
        Ok(session)
    }

    fn integrity_level(process: HANDLE) -> Result<dji4g_ipc::IntegrityLevel, PrivilegeError> {
        let mut token = null_mut();
        // SAFETY: process and output token storage are valid for this synchronous query.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(PrivilegeError::Internal);
        }
        let token = OwnedHandle::new(token)?;
        let mut required = 0_u32;
        // SAFETY: first call queries the required TOKEN_MANDATORY_LABEL buffer size.
        unsafe {
            GetTokenInformation(
                token.raw(),
                TokenIntegrityLevel,
                null_mut(),
                0,
                &mut required,
            )
        };
        if required == 0 || required > 4096 {
            return Err(PrivilegeError::Internal);
        }
        let mut buffer = vec![0_u8; required as usize];
        // SAFETY: buffer has the exact size returned by the token API.
        if unsafe {
            GetTokenInformation(
                token.raw(),
                TokenIntegrityLevel,
                buffer.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(PrivilegeError::Internal);
        }
        // SAFETY: TOKEN_MANDATORY_LABEL is the documented returned layout.
        let label = unsafe { &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
        // SAFETY: the SID is owned by the still-live buffer and contains a RID at the documented
        // final sub-authority position.
        let count = unsafe { *label.Label.Sid.cast::<u8>().add(1) };
        if count == 0 {
            return Err(PrivilegeError::Internal);
        }
        let rid_ptr = unsafe { label.Label.Sid.cast::<u32>().add(2 + count as usize - 1) };
        let rid = unsafe { std::ptr::read_unaligned(rid_ptr) };
        Ok(match rid {
            0x0000_1000..=0x0000_1fff => dji4g_ipc::IntegrityLevel::Low,
            0x0000_2000..=0x0000_2fff => dji4g_ipc::IntegrityLevel::Medium,
            0x0000_3000..=0x0000_3fff => dji4g_ipc::IntegrityLevel::High,
            _ => dji4g_ipc::IntegrityLevel::System,
        })
    }

    fn process_image(process: HANDLE) -> Result<PathBuf, PrivilegeError> {
        let mut buffer = vec![0_u16; 32_768];
        let mut length = buffer.len() as u32;
        // SAFETY: process is a query handle and buffer/length are writable as documented.
        if unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) } == 0
        {
            return Err(PrivilegeError::Internal);
        }
        buffer.truncate(length as usize);
        String::from_utf16(&buffer)
            .ok()
            .map(PathBuf::from)
            .and_then(|path| std::fs::canonicalize(path).ok())
            .ok_or(PrivilegeError::Internal)
    }

    fn process_creation_time(process: HANDLE) -> Result<u64, PrivilegeError> {
        // SAFETY: FILETIME is a plain C value whose every field is initialized by Windows.
        let mut creation = unsafe { zeroed() };
        let mut exit = unsafe { zeroed() };
        let mut kernel = unsafe { zeroed() };
        let mut user = unsafe { zeroed() };
        // SAFETY: all FILETIME pointers refer to writable local storage and process is live.
        if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
            == 0
        {
            return Err(PrivilegeError::Internal);
        }
        Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
    }

    fn process_expectation(
        pid: u32,
        image_hash: Hash32,
    ) -> Result<dji4g_ipc::ProcessExpectation, PrivilegeError> {
        // SAFETY: pid comes from ShellExecuteEx or the named pipe API and access is query-only.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION_FLAGS, 0, pid) };
        let process = OwnedHandle::new(process)?;
        let sid = user_sid_material(process.raw())?;
        Ok(dji4g_ipc::ProcessExpectation {
            pid,
            creation_time: process_creation_time(process.raw())?,
            user_sid_hash: digest_bytes(&sid),
            session_id: current_session(process.raw())?,
            minimum_integrity: dji4g_ipc::IntegrityLevel::Medium,
            image_hash,
        })
    }

    fn peer_identity(pid: u32, remote: bool) -> Result<PeerIdentity, PrivilegeError> {
        // SAFETY: pid comes from the kernel-reported pipe peer, not user input.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION_FLAGS, 0, pid) };
        let process = OwnedHandle::new(process)?;
        let sid = user_sid_material(process.raw())?;
        let image = process_image(process.raw())?;
        Ok(PeerIdentity {
            pid,
            creation_time: process_creation_time(process.raw())?,
            user_sid_hash: digest_bytes(&sid),
            session_id: current_session(process.raw())?,
            integrity: integrity_level(process.raw())?,
            image_hash: digest_file(&image)?,
            remote,
        })
    }

    fn make_security_descriptor(process: HANDLE) -> Result<SecurityDescriptor, PrivilegeError> {
        let sid = sid_text(process)?;
        let mut sddl = Vec::with_capacity(32 + sid.len());
        sddl.extend("D:P(A;;GA;;;".encode_utf16());
        sddl.extend(&sid);
        sddl.extend(")(A;;GA;;;BA)(A;;GA;;;SY)".encode_utf16());
        sddl.push(0);
        let mut descriptor = null_mut();
        // SAFETY: sddl is NUL-terminated and descriptor points to writable storage.  Windows owns
        // the returned descriptor, released by SecurityDescriptor::drop.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(PrivilegeError::Internal);
        }
        Ok(SecurityDescriptor { ptr: descriptor })
    }

    fn timeout_millis(deadline: &Deadline) -> u32 {
        deadline
            .remaining()
            .min(MAX_IO_WAIT)
            .as_millis()
            .max(1)
            .min(u32::MAX as u128) as u32
    }

    fn wait_overlapped(
        handle: HANDLE,
        overlapped: &OVERLAPPED,
        deadline: &Deadline,
    ) -> Result<u32, FrameError> {
        let mut transferred = 0_u32;
        // SAFETY: handle and OVERLAPPED remain live until this bounded wait returns; event is owned
        // by the OVERLAPPED operation and is closed by the caller after completion.
        let success = unsafe {
            GetOverlappedResultEx(
                handle,
                overlapped,
                &mut transferred,
                timeout_millis(deadline),
                0,
            )
        };
        if success != 0 {
            return Ok(transferred);
        }
        let error = unsafe { GetLastError() };
        if error == 258 {
            // SAFETY: cancellation targets only this operation on this owned pipe handle.
            unsafe { CancelIoEx(handle, overlapped) };
            return Err(FrameError::Timeout);
        }
        Err(if error == ERROR_BROKEN_PIPE || error == ERROR_NO_DATA {
            FrameError::Disconnected
        } else if error == ERROR_MORE_DATA {
            FrameError::SecondFrame
        } else {
            FrameError::Io
        })
    }

    fn read_message_overlapped(
        handle: HANDLE,
        output: &mut [u8],
        deadline: &Deadline,
    ) -> Result<usize, FrameError> {
        let event = OwnedHandle::new(unsafe { CreateEventW(null(), 1, 0, null()) })
            .map_err(|_| FrameError::Io)?;
        // SAFETY: OVERLAPPED is initialized by Windows before completion.
        let mut overlapped: OVERLAPPED = unsafe { zeroed() };
        overlapped.hEvent = event.raw();
        let mut transferred = 0_u32;
        // SAFETY: output is a bounded writable buffer and remains live until the operation ends.
        let success = unsafe {
            ReadFile(
                handle,
                output.as_mut_ptr(),
                output.len() as u32,
                &mut transferred,
                &mut overlapped,
            )
        };
        if success == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_MORE_DATA {
                return Err(FrameError::FrameTooLarge {
                    length: output.len() + 1,
                });
            }
            if error != ERROR_IO_PENDING {
                return Err(if error == ERROR_BROKEN_PIPE || error == ERROR_NO_DATA {
                    FrameError::Disconnected
                } else {
                    FrameError::Io
                });
            }
            transferred = wait_overlapped(handle, &overlapped, deadline)?;
        }
        if transferred == 0 {
            return Err(FrameError::Disconnected);
        }
        Ok(transferred as usize)
    }

    fn write_all_overlapped(
        handle: HANDLE,
        input: &[u8],
        deadline: &Deadline,
    ) -> Result<(), FrameError> {
        let mut offset = 0_usize;
        while offset < input.len() {
            // SAFETY: null event attributes, manual reset, initially non-signaled, unnamed event.
            let event = OwnedHandle::new(unsafe { CreateEventW(null(), 1, 0, null()) })
                .map_err(|_| FrameError::Io)?;
            // SAFETY: OVERLAPPED is initialized by Windows before completion.
            let mut overlapped: OVERLAPPED = unsafe { zeroed() };
            overlapped.hEvent = event.raw();
            let mut transferred = 0_u32;
            // SAFETY: input slice remains live through bounded completion and is read-only to Win32.
            let success = unsafe {
                WriteFile(
                    handle,
                    input[offset..].as_ptr(),
                    (input.len() - offset) as u32,
                    &mut transferred,
                    &mut overlapped,
                )
            };
            if success == 0 {
                let error = unsafe { GetLastError() };
                if error != ERROR_IO_PENDING {
                    return Err(FrameError::Io);
                }
                transferred = wait_overlapped(handle, &overlapped, deadline)?;
            }
            if transferred == 0 {
                return Err(FrameError::Disconnected);
            }
            offset = offset.saturating_add(transferred as usize);
        }
        Ok(())
    }

    pub struct WindowsPipeTransport {
        handle: OwnedHandle,
        server_end: bool,
        closed: bool,
    }

    // SAFETY: the transport is exclusively owned by one state-machine instance; all Win32 calls
    // borrow the handle synchronously and no handle is shared between threads.
    unsafe impl Send for WindowsPipeTransport {}

    impl PipeTransport for WindowsPipeTransport {
        fn peer_identity(&self) -> Result<PeerIdentity, TransportError> {
            let mut pid = 0_u32;
            let ok = if self.server_end {
                // SAFETY: self.handle is a live connected named pipe and pid is writable storage.
                unsafe { GetNamedPipeClientProcessId(self.handle.raw(), &mut pid) }
            } else {
                // SAFETY: self.handle is a live connected named pipe and pid is writable storage.
                unsafe { GetNamedPipeServerProcessId(self.handle.raw(), &mut pid) }
            };
            if ok == 0 || pid == 0 {
                return Err(TransportError::PeerUnavailable);
            }
            peer_identity(pid, false).map_err(|_| TransportError::PeerUnavailable)
        }

        fn read_frame(&mut self, deadline: &Deadline) -> Result<Vec<u8>, FrameError> {
            // Message-mode pipes preserve the panel's single WriteFile call. Read the complete
            // bounded message in one operation; reading a four-byte prefix first would make
            // Windows return ERROR_MORE_DATA for every valid message.
            let mut frame = vec![0_u8; dji4g_ipc::MAX_FRAME_BYTES + 4];
            let size = read_message_overlapped(self.handle.raw(), &mut frame, deadline)?;
            frame.truncate(size);
            dji4g_ipc::decode_payload(&frame).map_err(|error| error.clone())?;
            Ok(frame)
        }

        fn write_frame(&mut self, payload: &[u8], deadline: &Deadline) -> Result<(), FrameError> {
            write_all_overlapped(self.handle.raw(), payload, deadline)
        }

        fn try_read_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
            let mut available = 0_u32;
            // SAFETY: query-only peek with no output buffer; available is writable storage.
            if unsafe {
                PeekNamedPipe(
                    self.handle.raw(),
                    null_mut(),
                    0,
                    null_mut(),
                    &mut available,
                    null_mut(),
                )
            } == 0
            {
                let error = unsafe { GetLastError() };
                return if error == ERROR_NO_DATA || error == ERROR_BROKEN_PIPE {
                    Ok(None)
                } else {
                    Err(FrameError::Io)
                };
            }
            if available == 0 {
                return Ok(None);
            }
            self.read_frame(&Deadline::from_now(Duration::from_millis(1)))
                .map(Some)
        }

        fn close_once(&mut self) {
            if self.closed {
                return;
            }
            self.closed = true;
            if self.server_end {
                // SAFETY: idempotent disconnect on this owned server endpoint; Drop closes handle.
                unsafe { DisconnectNamedPipe(self.handle.raw()) };
            }
        }
    }

    impl Drop for WindowsPipeTransport {
        fn drop(&mut self) {
            self.close_once();
        }
    }

    pub struct WindowsNamedPipeServer {
        handle: OwnedHandle,
        name: PipeName,
        user_sid_hash: Hash32,
        session_id: u32,
        panel_image_hash: Hash32,
    }

    impl WindowsNamedPipeServer {
        pub fn create_for_current_user(nonce: &OperationNonce) -> Result<Self, PrivilegeError> {
            let _ = nonce;
            let name = PipeName::random().map_err(|_| PrivilegeError::Internal)?;
            let current = unsafe { GetCurrentProcess() };
            let sid = user_sid_material(current)?;
            let descriptor = make_security_descriptor(current)?;
            let attributes = windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
                nLength: size_of::<windows_sys::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.ptr,
                bInheritHandle: 0,
            };
            let name_w: Vec<u16> = name
                .as_str()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: name/attributes point to live buffers; flags request one overlapped message
            // instance and reject remote clients; returned handle is immediately wrapped by RAII.
            let handle = unsafe {
                CreateNamedPipeW(
                    name_w.as_ptr(),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                    PIPE_TYPE_MESSAGE
                        | PIPE_READMODE_MESSAGE
                        | PIPE_WAIT
                        | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    (dji4g_ipc::MAX_FRAME_BYTES + 4) as u32,
                    (dji4g_ipc::MAX_FRAME_BYTES + 4) as u32,
                    MAX_IO_WAIT.as_millis() as u32,
                    &attributes,
                )
            };
            let handle = OwnedHandle::new(handle)?;
            let image_hash =
                digest_file(&std::env::current_exe().map_err(|_| PrivilegeError::Internal)?)?;
            Ok(Self {
                handle,
                name,
                user_sid_hash: digest_bytes(&sid),
                session_id: current_session(current)?,
                panel_image_hash: image_hash,
            })
        }

        #[must_use]
        pub fn name(&self) -> &PipeName {
            &self.name
        }

        pub fn accept_one(
            self,
            expected_helper: &dji4g_ipc::ProcessExpectation,
            deadline: Deadline,
        ) -> Result<WindowsPipeTransport, PrivilegeError> {
            let event = OwnedHandle::new(unsafe { CreateEventW(null(), 1, 0, null()) })?;
            let mut overlapped: OVERLAPPED = unsafe { zeroed() };
            overlapped.hEvent = event.raw();
            // SAFETY: self.handle is a unique pipe server endpoint and overlapped points to live
            // event storage; completion is bounded below.
            let connected = unsafe { ConnectNamedPipe(self.handle.raw(), &mut overlapped) };
            if connected == 0 {
                let error = unsafe { GetLastError() };
                if error != ERROR_PIPE_CONNECTED && error != ERROR_IO_PENDING {
                    return Err(PrivilegeError::Internal);
                }
                if error != ERROR_PIPE_CONNECTED {
                    wait_overlapped(self.handle.raw(), &overlapped, &deadline).map_err(
                        |error| {
                            if error == FrameError::Timeout {
                                PrivilegeError::Timeout
                            } else {
                                PrivilegeError::Internal
                            }
                        },
                    )?;
                }
            }
            let transport = WindowsPipeTransport {
                handle: self.handle,
                server_end: true,
                closed: false,
            };
            let peer = transport
                .peer_identity()
                .map_err(|_| PrivilegeError::Internal)?;
            let expected = PeerExpectation {
                pid: expected_helper.pid,
                creation_time: expected_helper.creation_time,
                user_sid_hash: expected_helper.user_sid_hash,
                session_id: expected_helper.session_id,
                minimum_integrity: expected_helper.minimum_integrity,
                image_hash: expected_helper.image_hash,
            };
            expected.verify(&peer).map_err(PrivilegeError::Peer)?;
            Ok(transport)
        }

        #[must_use]
        pub const fn panel_image_hash(&self) -> Hash32 {
            self.panel_image_hash
        }

        #[must_use]
        pub const fn user_sid_hash(&self) -> Hash32 {
            self.user_sid_hash
        }

        #[must_use]
        pub const fn session_id(&self) -> u32 {
            self.session_id
        }
    }

    pub struct WindowsNamedPipeClient {
        transport: WindowsPipeTransport,
    }

    impl WindowsNamedPipeClient {
        pub fn connect_once(name: &PipeName, deadline: Deadline) -> Result<Self, PrivilegeError> {
            let name_w: Vec<u16> = name
                .as_str()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let wait = timeout_millis(&deadline);
            // SAFETY: name_w is a valid NUL-terminated generated pipe name; wait is bounded.
            if unsafe { windows_sys::Win32::System::Pipes::WaitNamedPipeW(name_w.as_ptr(), wait) }
                == 0
            {
                let error = unsafe { GetLastError() };
                return Err(if error == 1460 {
                    PrivilegeError::Timeout
                } else {
                    PrivilegeError::Internal
                });
            }
            // SAFETY: generated name, query/write access only, no inheritance, and no template.
            let handle = unsafe {
                CreateFileW(
                    name_w.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    FILE_SHARE_NONE,
                    null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    null_mut(),
                )
            };
            Ok(Self {
                transport: WindowsPipeTransport {
                    handle: OwnedHandle::new(handle)?,
                    server_end: false,
                    closed: false,
                },
            })
        }

        pub fn verify_server(&self, expected: &PeerExpectation) -> Result<(), PrivilegeError> {
            let peer = self
                .transport
                .peer_identity()
                .map_err(|_| PrivilegeError::Internal)?;
            expected.verify(&peer).map_err(PrivilegeError::Peer)
        }

        pub fn into_transport(self) -> WindowsPipeTransport {
            self.transport
        }
    }

    pub(super) fn launch_elevated_helper(
        helper: &TrustedHelper,
        request: HelperRequestV1,
        now: SystemTime,
    ) -> Result<HelperResponseV1, PrivilegeError> {
        validate_request(&request, &request.nonce, now).map_err(PrivilegeError::Protocol)?;
        let nonce = request.nonce.clone();
        let server = WindowsNamedPipeServer::create_for_current_user(&nonce)?;
        let mut pipe_name = server.name().as_str().to_owned();
        let mut nonce_hex = nonce.to_hex();
        let mut parameters: Vec<u16> = format!(
            "--pipe {pipe_name} --nonce {nonce_hex} --protocol {}",
            request.version.as_u16()
        )
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
        pipe_name.clear();
        nonce_hex.clear();
        let path: Vec<u16> = helper
            .path()
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let verb: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();
        let mut execute_info: SHELLEXECUTEINFOW = unsafe { zeroed() };
        execute_info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        execute_info.fMask = SEE_MASK_NOCLOSEPROCESS;
        execute_info.lpVerb = verb.as_ptr();
        execute_info.lpFile = path.as_ptr();
        execute_info.lpParameters = parameters.as_ptr();
        execute_info.nShow = SW_SHOWNORMAL;
        // SAFETY: all pointers target live, NUL-terminated buffers; no caller-controlled action or
        // path enters lpParameters; ShellExecuteExW returns an owned process handle.
        let launched = unsafe { ShellExecuteExW(&mut execute_info) };
        parameters.fill(0);
        if launched == 0 {
            return if unsafe { GetLastError() } == ERROR_CANCELLED {
                Err(PrivilegeError::OperationCancelled)
            } else {
                Err(PrivilegeError::LaunchFailed)
            };
        }
        let process = OwnedHandle::new(execute_info.hProcess)?;
        let pid = unsafe { GetProcessId(process.raw()) };
        if pid == 0 {
            return Err(PrivilegeError::LaunchFailed);
        }
        let mut expected = process_expectation(pid, helper.image_hash())?;
        expected.minimum_integrity = dji4g_ipc::IntegrityLevel::High;
        let deadline = Deadline::from_now(MAX_OPERATION_LIFETIME);
        let transport = server.accept_one(&expected, deadline)?;
        let mut client = PipeClient::new(transport);
        let response = client
            .request_once(&request, deadline)
            .map_err(map_client_error)?;
        let _ = now;
        // SAFETY: waiting on the process handle returned by ShellExecuteExW is bounded and ensures
        // a successful response does not leave a resident elevated helper.
        let wait = unsafe {
            WaitForSingleObject(
                process.raw(),
                deadline.remaining().as_millis().min(u32::MAX as u128) as u32,
            )
        };
        if wait == 258 {
            return Err(PrivilegeError::Timeout);
        }
        if wait == u32::MAX {
            return Err(PrivilegeError::Internal);
        }
        Ok(response)
    }

    pub(super) fn run_helper_once(
        pipe: PipeName,
        nonce: OperationNonce,
        protocol: ProtocolVersion,
    ) -> Result<(), PrivilegeError> {
        let deadline = Deadline::from_now(MAX_OPERATION_LIFETIME);
        let client = WindowsNamedPipeClient::connect_once(&pipe, deadline)?;
        let server = client
            .transport
            .peer_identity()
            .map_err(|_| PrivilegeError::Internal)?;
        let panel_path = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join("dji4g-panel.exe")))
            .and_then(|path| std::fs::canonicalize(path).ok())
            .ok_or(PrivilegeError::Peer(PeerRejectCode::ServerImageMismatch))?;
        let panel_pid = server.pid;
        let panel_expectation = process_expectation(panel_pid, digest_file(&panel_path)?)?;
        client.verify_server(&PeerExpectation {
            pid: panel_expectation.pid,
            creation_time: panel_expectation.creation_time,
            user_sid_hash: panel_expectation.user_sid_hash,
            session_id: panel_expectation.session_id,
            minimum_integrity: dji4g_ipc::IntegrityLevel::Medium,
            image_hash: panel_expectation.image_hash,
        })?;
        let mut transport = client.into_transport();
        let frame = transport.read_frame(&deadline).map_err(|error| {
            if error == FrameError::Timeout {
                PrivilegeError::Timeout
            } else {
                PrivilegeError::Protocol(RequestRejectCode::MalformedFrame)
            }
        })?;
        let request: HelperRequestV1 = dji4g_ipc::decode_frame(&frame)
            .map_err(|_| PrivilegeError::Protocol(RequestRejectCode::MalformedFrame))?;
        let response = if request.version != protocol {
            HelperResponseV1 {
                version: ProtocolVersion::V1,
                request_id: request.request_id,
                result: HelperResultV1::Rejected {
                    code: RequestRejectCode::UnsupportedVersion,
                },
            }
        } else if let Err(code) = dji4g_ipc::validate_request(&request, &nonce, SystemTime::now()) {
            HelperResponseV1 {
                version: ProtocolVersion::V1,
                request_id: request.request_id,
                result: HelperResultV1::Rejected { code },
            }
        } else if transport
            .try_read_frame()
            .map_err(|_| PrivilegeError::Protocol(RequestRejectCode::SecondFrame))?
            .is_some()
        {
            // A second frame is a protocol violation.  Reject before inventory enumeration so a
            // malicious client cannot turn one authenticated connection into two operations.
            HelperResponseV1 {
                version: ProtocolVersion::V1,
                request_id: request.request_id,
                result: HelperResultV1::Rejected {
                    code: RequestRejectCode::SecondFrame,
                },
            }
        } else {
            inspect_or_reject(request)?
        };
        let response_frame = dji4g_ipc::encode_frame(&response)
            .map_err(|_| PrivilegeError::Protocol(RequestRejectCode::MalformedFrame))?;
        transport
            .write_frame(&response_frame, &deadline)
            .map_err(|_| PrivilegeError::Timeout)?;
        Ok(())
    }

    fn inspect_or_reject(request: HelperRequestV1) -> Result<HelperResponseV1, PrivilegeError> {
        if !matches!(request.target.profile, SupportedProfileV1::DjiGen1) {
            return Ok(HelperResponseV1 {
                version: ProtocolVersion::V1,
                request_id: request.request_id,
                result: HelperResultV1::Rejected {
                    code: RequestRejectCode::UnsupportedDevice,
                },
            });
        }
        let inventory = WindowsDeviceInventory
            .scan_now()
            .map_err(|_| PrivilegeError::Internal)?;
        let devices = inventory.devices();
        let device = match devices {
            [] => {
                return Ok(HelperResponseV1 {
                    version: ProtocolVersion::V1,
                    request_id: request.request_id,
                    result: HelperResultV1::Rejected {
                        code: RequestRejectCode::TargetNotFound,
                    },
                });
            }
            [device] => device,
            _ => {
                return Ok(HelperResponseV1 {
                    version: ProtocolVersion::V1,
                    request_id: request.request_id,
                    result: HelperResultV1::Rejected {
                        code: RequestRejectCode::TargetAmbiguous,
                    },
                });
            }
        };
        let identity = authoritative_identity_hash(device);
        if identity != request.target.identity_hash {
            return Ok(HelperResponseV1 {
                version: ProtocolVersion::V1,
                request_id: request.request_id,
                result: HelperResultV1::Rejected {
                    code: RequestRejectCode::TargetIdentityChanged,
                },
            });
        }
        if request.target.before_state_hash.is_zero() {
            return Ok(HelperResponseV1 {
                version: ProtocolVersion::V1,
                request_id: request.request_id,
                result: HelperResultV1::Completed(dji4g_ipc::OperationResultV1::Failed {
                    code: dji4g_ipc::OperationCode::BeforeStateChanged,
                    rollback: dji4g_ipc::RollbackResultV1::NotAttempted,
                }),
            });
        }
        if matches!(request.action, HelperActionV1::InspectTarget) {
            return Ok(HelperResponseV1 {
                version: ProtocolVersion::V1,
                request_id: request.request_id,
                result: HelperResultV1::Inspected {
                    state_hash: identity,
                },
            });
        }

        // State-changing requests are decoded into the same closed typed action enum used by the
        // in-process executor.  The executor performs its own fresh scan and action-specific
        // before-hash check immediately before dispatch, so the helper cannot become a bypass of
        // the normal repair safety boundary.
        let action = helper_action_to_repair(request.action.clone())
            .map_err(|_| PrivilegeError::Protocol(RequestRejectCode::UnknownAction))?;
        let result = execute_repair_once(&request, action);
        Ok(HelperResponseV1 {
            version: ProtocolVersion::V1,
            request_id: request.request_id,
            result: HelperResultV1::Completed(result),
        })
    }

    fn helper_action_to_repair(action: HelperActionV1) -> Result<RepairAction, RequestRejectCode> {
        match action {
            HelperActionV1::InspectTarget => Err(RequestRejectCode::UnknownAction),
            HelperActionV1::RenewDhcp => Ok(RepairAction::RefreshDhcp),
            HelperActionV1::ApplyDnsProfile { profile } => Ok(RepairAction::ApplyDnsProfile {
                profile: match profile {
                    dji4g_ipc::DnsProfileV1::Automatic => DnsProfile::Automatic,
                    dji4g_ipc::DnsProfileV1::Static { servers } => DnsProfile::Static {
                        servers: servers.as_slice().to_vec(),
                    },
                },
            }),
            HelperActionV1::RestartAdapter => Ok(RepairAction::RestartAdapter),
            HelperActionV1::ReenumerateDevice => Ok(RepairAction::ReenumerateDevice),
            HelperActionV1::RestartModule => Ok(RepairAction::RestartModule),
            HelperActionV1::EditApn { cid, apn } => Ok(RepairAction::SetApn {
                cid: PdpContextId::try_from(cid.get())
                    .map_err(|_| RequestRejectCode::UnknownAction)?,
                apn: Apn::try_from(apn.as_str()).map_err(|_| RequestRejectCode::UnknownAction)?,
            }),
            HelperActionV1::SetUsbNetProfile { profile } => Ok(RepairAction::SetUsbNetProfile {
                profile: match profile {
                    dji4g_ipc::UsbNetProfileV1::DjiNdis => VerifiedUsbNetProfile::DjiNdis,
                    dji4g_ipc::UsbNetProfileV1::Ecm => VerifiedUsbNetProfile::Ecm,
                },
            }),
            HelperActionV1::ToggleHotspot { enabled } => {
                Ok(RepairAction::ToggleHotspot { enabled })
            }
        }
    }

    fn execute_repair_once(
        request: &HelperRequestV1,
        action: RepairAction,
    ) -> dji4g_ipc::OperationResultV1 {
        let executor = WindowsRepairExecutor::new(WindowsNativeRepairBackend::with_epoch(
            DeviceEpoch(request.target.epoch),
        ));
        let plan = match executor.prepare(action) {
            Ok(plan) => plan,
            Err(error) => return failed_from_repair_error(error),
        };
        if plan.epoch() != DeviceEpoch(request.target.epoch) {
            return dji4g_ipc::OperationResultV1::Failed {
                code: dji4g_ipc::OperationCode::EpochChanged,
                rollback: dji4g_ipc::RollbackResultV1::NotAttempted,
            };
        }
        if plan.target_identity_hash() != *request.target.identity_hash.as_bytes() {
            return dji4g_ipc::OperationResultV1::Failed {
                code: dji4g_ipc::OperationCode::TargetIdentityChanged,
                rollback: dji4g_ipc::RollbackResultV1::NotAttempted,
            };
        }
        let requested_before = BeforeStateHash(*request.target.before_state_hash.as_bytes());
        if plan.before_state_hash() != requested_before {
            return dji4g_ipc::OperationResultV1::Failed {
                code: dji4g_ipc::OperationCode::BeforeStateChanged,
                rollback: dji4g_ipc::RollbackResultV1::NotAttempted,
            };
        }
        map_repair_result(executor.execute_once(&plan))
    }

    fn map_repair_result(result: crate::RepairResult) -> dji4g_ipc::OperationResultV1 {
        match result.outcome() {
            OperationOutcome::Applied { after_state_hash } => {
                dji4g_ipc::OperationResultV1::Applied {
                    after_state_hash: dji4g_ipc::Hash32::from_bytes(after_state_hash.0),
                    verified_at: dji4g_ipc::UnixMillis::from_system_time(SystemTime::now())
                        .unwrap_or(dji4g_ipc::UnixMillis(0)),
                }
            }
            OperationOutcome::Failed { code, rollback } => dji4g_ipc::OperationResultV1::Failed {
                code: map_error_code(*code),
                rollback: map_rollback(*rollback),
            },
            OperationOutcome::OutcomeUnknown { code } => {
                dji4g_ipc::OperationResultV1::OutcomeUnknown {
                    code: map_error_code(*code),
                }
            }
        }
    }

    fn failed_from_repair_error(error: RepairError) -> dji4g_ipc::OperationResultV1 {
        dji4g_ipc::OperationResultV1::Failed {
            code: map_error_code(error.code()),
            rollback: dji4g_ipc::RollbackResultV1::NotAttempted,
        }
    }

    fn map_error_code(error: dji4g_domain::ErrorCode) -> dji4g_ipc::OperationCode {
        match error {
            dji4g_domain::ErrorCode::PermissionDenied => dji4g_ipc::OperationCode::PermissionDenied,
            dji4g_domain::ErrorCode::DeviceRemoved => dji4g_ipc::OperationCode::TargetNotFound,
            dji4g_domain::ErrorCode::DeviceIdentityChanged => {
                dji4g_ipc::OperationCode::TargetIdentityChanged
            }
            dji4g_domain::ErrorCode::EvidenceExpired => {
                dji4g_ipc::OperationCode::BeforeStateChanged
            }
            dji4g_domain::ErrorCode::Timeout => dji4g_ipc::OperationCode::Timeout,
            dji4g_domain::ErrorCode::Unsupported => dji4g_ipc::OperationCode::UnsupportedDevice,
            dji4g_domain::ErrorCode::CapabilityUnavailable => {
                dji4g_ipc::OperationCode::AtPortUnavailable
            }
            dji4g_domain::ErrorCode::OperationCancelled => {
                dji4g_ipc::OperationCode::OperationCancelled
            }
            dji4g_domain::ErrorCode::VerificationFailed => {
                dji4g_ipc::OperationCode::VerificationFailed
            }
            dji4g_domain::ErrorCode::RollbackFailed => dji4g_ipc::OperationCode::RollbackFailed,
            dji4g_domain::ErrorCode::ProbeFailed
            | dji4g_domain::ErrorCode::DnsFailed
            | dji4g_domain::ErrorCode::Internal => dji4g_ipc::OperationCode::Internal,
        }
    }

    fn map_rollback(value: dji4g_domain::RollbackOutcome) -> dji4g_ipc::RollbackResultV1 {
        match value {
            dji4g_domain::RollbackOutcome::NotRequired => dji4g_ipc::RollbackResultV1::NotRequired,
            dji4g_domain::RollbackOutcome::Applied => dji4g_ipc::RollbackResultV1::Applied,
            dji4g_domain::RollbackOutcome::Failed { code } => dji4g_ipc::RollbackResultV1::Failed {
                code: map_error_code(code),
            },
            dji4g_domain::RollbackOutcome::NotAttempted => {
                dji4g_ipc::RollbackResultV1::NotAttempted
            }
        }
    }

    fn authoritative_identity_hash(device: &DjiDevice) -> Hash32 {
        Hash32::from_bytes(crate::repair::authoritative_identity_hash(device))
    }

    fn map_client_error(error: ClientError) -> PrivilegeError {
        match error {
            ClientError::Frame(FrameError::Timeout) => PrivilegeError::Timeout,
            ClientError::Frame(_) => PrivilegeError::Protocol(RequestRejectCode::MalformedFrame),
            ClientError::PeerRejected(code) => PrivilegeError::Peer(code),
            ClientError::Transport(TransportError::PeerRejected(code)) => {
                PrivilegeError::Peer(code)
            }
            ClientError::Transport(_) => PrivilegeError::Internal,
            ClientError::AlreadyUsed | ClientError::RequestIdMismatch => {
                PrivilegeError::Protocol(RequestRejectCode::ProtocolRejected)
            }
        }
    }

    #[cfg(test)]
    mod repair_router_tests {
        use super::*;

        #[test]
        fn every_wire_repair_action_maps_to_a_typed_action_without_raw_text() {
            let actions = [
                HelperActionV1::RenewDhcp,
                HelperActionV1::ApplyDnsProfile {
                    profile: dji4g_ipc::DnsProfileV1::Automatic,
                },
                HelperActionV1::RestartAdapter,
                HelperActionV1::ReenumerateDevice,
                HelperActionV1::RestartModule,
                HelperActionV1::EditApn {
                    cid: dji4g_ipc::PdpContextIdV1::new(1).expect("valid CID"),
                    apn: dji4g_ipc::ValidatedApnV1::try_from("safe.example".to_owned())
                        .expect("valid APN"),
                },
                HelperActionV1::SetUsbNetProfile {
                    profile: dji4g_ipc::UsbNetProfileV1::DjiNdis,
                },
                HelperActionV1::ToggleHotspot { enabled: true },
            ];
            for wire_action in actions {
                let action = helper_action_to_repair(wire_action).expect("closed mapping");
                let debug = format!("{action:?}");
                assert!(!debug.contains("safe.example"));
            }
        }

        #[test]
        fn inspect_is_not_a_repair_router_input() {
            assert!(matches!(
                helper_action_to_repair(HelperActionV1::InspectTarget),
                Err(RequestRejectCode::UnknownAction)
            ));
        }
    }
}

#[cfg(windows)]
pub use native::{WindowsNamedPipeClient, WindowsNamedPipeServer};

#[cfg(windows)]
pub fn launch_elevated_helper(
    helper: &TrustedHelper,
    request: HelperRequestV1,
    now: SystemTime,
) -> Result<HelperResponseV1, PrivilegeError> {
    native::launch_elevated_helper(helper, request, now)
}

#[cfg(windows)]
pub fn run_helper_once(
    pipe: PipeName,
    nonce: OperationNonce,
    protocol: ProtocolVersion,
) -> Result<(), PrivilegeError> {
    native::run_helper_once(pipe, nonce, protocol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_helper_never_accepts_user_writable_or_unsigned_path() {
        let path = std::env::temp_dir().join("dji4g-helper.exe");
        assert_eq!(
            TrustedHelper::from_path_with_signature(path, true),
            Err(PrivilegeError::HelperUntrusted)
        );
    }

    #[cfg(windows)]
    #[test]
    fn installed_fails_closed_with_a_signature_verdict_reason() {
        // Cargo test harnesses run from a deps directory that has no sibling helper image, so
        // `WinVerifyTrust` answers CRYPT_E_FILE_ERROR and the boundary reports
        // `privilege:helper_unverified`.  When the unsigned dev helper exists next to the panel
        // (cargo dev layout, unsigned MSIX package), `WinVerifyTrust` answers
        // `TRUST_E_NOSIGNATURE` and the boundary reports `privilege:helper_unsigned` — pinned
        // exactly by `wintrust_reports_the_unsigned_test_binary_as_unsigned` and
        // `verification_verdicts_map_to_their_specific_privilege_errors` below.
        let error = TrustedHelper::installed().expect_err("unsigned dev helper must fail closed");
        assert!(
            matches!(
                error.to_string().as_str(),
                "privilege:helper_unsigned" | "privilege:helper_unverified"
            ),
            "expected the diagnosable signature reason, got {error}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn verification_verdicts_map_to_their_specific_privilege_errors() {
        use crate::privilege_wintrust::Verification;
        // The verdict check runs before canonicalization, so the path shape is irrelevant here.
        let absent = std::env::temp_dir().join("dji4g-verdict-mapping-probe.exe");
        assert_eq!(
            TrustedHelper::from_path_with_verification(absent.clone(), Verification::Unsigned),
            Err(PrivilegeError::HelperUnsigned)
        );
        assert_eq!(
            TrustedHelper::from_path_with_verification(absent.clone(), Verification::Untrusted),
            Err(PrivilegeError::HelperUntrusted)
        );
        assert_eq!(
            TrustedHelper::from_path_with_verification(absent, Verification::ApiUnavailable),
            Err(PrivilegeError::HelperVerificationUnavailable)
        );
    }

    #[cfg(windows)]
    #[test]
    fn every_privilege_error_display_is_a_stable_lowercase_code() {
        for error in [
            PrivilegeError::UnsupportedPlatform,
            PrivilegeError::HelperUnsigned,
            PrivilegeError::HelperUntrusted,
            PrivilegeError::HelperVerificationUnavailable,
            PrivilegeError::LaunchFailed,
            PrivilegeError::Internal,
        ] {
            let text = error.to_string();
            assert!(
                text.starts_with("privilege:")
                    && text
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch == ':' || ch == '_'),
                "unstable display for {error:?}: {text}"
            );
        }
    }

    #[test]
    fn digest_is_fixed_width_and_stable_without_logging_material() {
        let first = digest_bytes(b"sensitive-path");
        assert_eq!(first, digest_bytes(b"sensitive-path"));
        assert!(!format!("{first:?}").contains("sensitive-path"));
    }

    #[test]
    fn digest_uses_sha256_for_image_and_identity_proofs() {
        assert_eq!(
            digest_bytes(b"abc").as_bytes(),
            &[
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
    }
}
