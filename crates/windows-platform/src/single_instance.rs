//! Per-user/session single-instance ownership and a bounded activation channel.
//!
//! The wire format is deliberately smaller and stricter than a general serialization protocol:
//! one fixed frame, one fixed ACK, one request per connection.  No command line, path, device
//! identity, or user-controlled string crosses the activation boundary.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread,
    time::Duration,
};

use crate::PlatformError;

const MAGIC: [u8; 4] = *b"D4GP";
const ACK_MAGIC: [u8; 4] = *b"D4GA";
const PROTOCOL_VERSION: u16 = 1;
const FRAME_LENGTH: usize = 24;
const ACK_LENGTH: usize = 8;
const CHANNEL_CAPACITY: usize = 8;
const ACTIVATION_RETRIES: usize = 3;
const ACTIVATION_TIMEOUT: Duration = Duration::from_millis(300);
static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationRequest {
    Open,
    OpenAndRefresh,
}

impl ActivationRequest {
    const fn wire_value(self) -> u8 {
        match self {
            Self::Open => 1,
            Self::OpenAndRefresh => 2,
        }
    }

    const fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Open),
            2 => Some(Self::OpenAndRefresh),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivationFrame {
    pub protocol_version: u16,
    pub request: ActivationRequest,
    pub nonce: [u8; 16],
}

impl ActivationFrame {
    #[must_use]
    pub fn new(request: ActivationRequest) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request,
            nonce: next_nonce(),
        }
    }

    #[must_use]
    pub fn encode(self) -> [u8; FRAME_LENGTH] {
        let mut frame = [0_u8; FRAME_LENGTH];
        frame[..4].copy_from_slice(&MAGIC);
        frame[4..6].copy_from_slice(&self.protocol_version.to_le_bytes());
        frame[6] = self.request.wire_value();
        frame[8..].copy_from_slice(&self.nonce);
        frame
    }

    pub fn decode(frame: &[u8]) -> Result<Self, PlatformError> {
        if frame.len() != FRAME_LENGTH {
            return Err(protocol_error("single_instance:invalid_length"));
        }
        if frame[..4] != MAGIC {
            return Err(protocol_error("single_instance:invalid_magic"));
        }
        let protocol_version = u16::from_le_bytes([frame[4], frame[5]]);
        if protocol_version != PROTOCOL_VERSION {
            return Err(protocol_error("single_instance:unsupported_version"));
        }
        if frame[7] != 0 {
            return Err(protocol_error("single_instance:unknown_field"));
        }
        let request = ActivationRequest::from_wire(frame[6])
            .ok_or_else(|| protocol_error("single_instance:unknown_request"))?;
        let mut nonce = [0_u8; 16];
        nonce.copy_from_slice(&frame[8..]);
        Ok(Self {
            protocol_version,
            request,
            nonce,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActivationAck {
    Accepted,
    Rejected,
}

impl ActivationAck {
    fn encode(self) -> [u8; ACK_LENGTH] {
        let mut ack = [0_u8; ACK_LENGTH];
        ack[..4].copy_from_slice(&ACK_MAGIC);
        ack[4..6].copy_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        ack[6] = match self {
            Self::Accepted => 0,
            Self::Rejected => 1,
        };
        ack
    }

    fn decode(value: &[u8]) -> Result<Self, PlatformError> {
        if value.len() != ACK_LENGTH || value[..4] != ACK_MAGIC {
            return Err(protocol_error("single_instance:invalid_ack"));
        }
        if u16::from_le_bytes([value[4], value[5]]) != PROTOCOL_VERSION || value[7] != 0 {
            return Err(protocol_error("single_instance:invalid_ack"));
        }
        match value[6] {
            0 => Ok(Self::Accepted),
            1 => Ok(Self::Rejected),
            _ => Err(protocol_error("single_instance:invalid_ack")),
        }
    }
}

#[derive(Debug)]
pub enum AcquireResult {
    Primary(SingleInstance),
    Existing,
}

pub struct SingleInstance {
    #[cfg(windows)]
    _mutex: MutexHandle,
    #[cfg(windows)]
    pipe_name: String,
    requests: Receiver<ActivationRequest>,
    stop: Arc<AtomicBool>,
    #[cfg(windows)]
    worker: Option<thread::JoinHandle<()>>,
}

impl fmt::Debug for SingleInstance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SingleInstance")
            .field("scope", &"current-user/session")
            .finish_non_exhaustive()
    }
}

impl SingleInstance {
    pub fn acquire() -> Result<AcquireResult, PlatformError> {
        #[cfg(windows)]
        {
            let (mutex, pipe_name) = create_scoped_mutex()?;
            if mutex.existing {
                return Ok(AcquireResult::Existing);
            }
            let (sender, requests) = mpsc::sync_channel(CHANNEL_CAPACITY);
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let worker_pipe_name = pipe_name.clone();
            let worker = thread::Builder::new()
                .name("dji4g-activation".to_owned())
                .spawn(move || serve_pipe(&worker_pipe_name, sender, worker_stop))
                .map_err(|error| PlatformError {
                    code: "single_instance:worker_start_failed",
                    os_code: error
                        .raw_os_error()
                        .and_then(|value| u32::try_from(value).ok()),
                })?;
            Ok(AcquireResult::Primary(Self {
                _mutex: mutex,
                pipe_name,
                requests,
                stop,
                worker: Some(worker),
            }))
        }
        #[cfg(not(windows))]
        {
            Err(PlatformError {
                code: "single_instance:unsupported_platform",
                os_code: None,
            })
        }
    }

    pub fn activate_existing(request: ActivationRequest) -> Result<(), PlatformError> {
        #[cfg(windows)]
        {
            let pipe_name = scoped_pipe_name()?;
            let frame = ActivationFrame::new(request).encode();
            let mut last_error = None;
            for _ in 0..ACTIVATION_RETRIES {
                match send_pipe_frame(&pipe_name, &frame) {
                    Ok(()) => return Ok(()),
                    Err(error) => last_error = Some(error),
                }
                thread::sleep(ACTIVATION_TIMEOUT);
            }
            let _ = last_error;
            Err(PlatformError {
                code: "single_instance:activation_timeout",
                os_code: None,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = request;
            Err(PlatformError {
                code: "single_instance:unsupported_platform",
                os_code: None,
            })
        }
    }

    /// Receive at most one logical activation.  Repeated Open requests are coalesced; an
    /// OpenAndRefresh request always wins over a plain Open in the same polling turn.
    pub fn try_recv(&mut self) -> Option<ActivationRequest> {
        let first = match self.requests.try_recv() {
            Ok(value) => value,
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return None,
        };
        let mut merged = first;
        while let Ok(next) = self.requests.try_recv() {
            if matches!(next, ActivationRequest::OpenAndRefresh) {
                merged = ActivationRequest::OpenAndRefresh;
            }
        }
        Some(merged)
    }

    #[cfg(test)]
    fn from_requests(requests: Receiver<ActivationRequest>) -> Self {
        Self {
            requests,
            stop: Arc::new(AtomicBool::new(false)),
            #[cfg(windows)]
            _mutex: MutexHandle {
                handle: 0,
                existing: false,
            },
            #[cfg(windows)]
            pipe_name: String::new(),
            #[cfg(windows)]
            worker: None,
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        #[cfg(windows)]
        {
            // A bounded self-connect wakes a worker blocked in ConnectNamedPipe/ReadFile so the
            // owned thread can close its handle and terminate with the instance.
            wake_pipe(&self.pipe_name);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

#[must_use]
pub fn scoped_object_name(user_discriminator: &str, session_id: u32) -> String {
    let hash = stable_scope_hash(user_discriminator.as_bytes(), session_id);
    format!(r"Local\Dji4GPanel.Gen1.v1.{session_id:08x}.{hash:016x}")
}

#[must_use]
pub fn scoped_pipe_name_for(user_discriminator: &str, session_id: u32) -> String {
    format!(
        r"\\.\pipe\Dji4GPanel.Gen1.v1.{session_id:08x}.{hash:016x}",
        hash = stable_scope_hash(user_discriminator.as_bytes(), session_id)
    )
}

#[must_use]
pub fn stable_scope_hash(value: &[u8], session_id: u32) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64 ^ u64::from(session_id);
    for byte in value {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[must_use]
pub const fn activation_frame_length() -> usize {
    FRAME_LENGTH
}

fn next_nonce() -> [u8; 16] {
    let counter = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos() as u64);
    let first = counter ^ nanos.rotate_left(17);
    let second = nanos ^ counter.rotate_left(29);
    let mut nonce = [0_u8; 16];
    nonce[..8].copy_from_slice(&first.to_le_bytes());
    nonce[8..].copy_from_slice(&second.to_le_bytes());
    nonce
}

fn protocol_error(code: &'static str) -> PlatformError {
    PlatformError {
        code,
        os_code: None,
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct MutexHandle {
    handle: isize,
    existing: bool,
}

#[cfg(windows)]
impl Drop for MutexHandle {
    fn drop(&mut self) {
        if self.handle != 0 {
            // SAFETY: this is the one owned mutex handle returned by CreateMutexW.
            unsafe { single_instance_close_handle(self.handle as *mut std::ffi::c_void) };
            self.handle = 0;
        }
    }
}

#[cfg(windows)]
fn create_scoped_mutex() -> Result<(MutexHandle, String), PlatformError> {
    let scope =
        current_scope()?.ok_or_else(|| protocol_error("single_instance:scope_unavailable"))?;
    let name = scoped_object_name(&scope.0, scope.1);
    let wide = wide_null(&name);
    // SAFETY: the name is a bounded ASCII string with a terminating NUL; default security uses the
    // creator token DACL, and this process requests no cross-user access.
    let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 1, wide.as_ptr()) };
    if handle == 0 {
        return Err(protocol_error_with_os(
            "single_instance:mutex_create_failed",
            last_error(),
        ));
    }
    let existing = last_error() == ERROR_ALREADY_EXISTS;
    let pipe_name = scoped_pipe_name_for(&scope.0, scope.1);
    Ok((MutexHandle { handle, existing }, pipe_name))
}

#[cfg(windows)]
fn scoped_pipe_name() -> Result<String, PlatformError> {
    let scope =
        current_scope()?.ok_or_else(|| protocol_error("single_instance:scope_unavailable"))?;
    Ok(scoped_pipe_name_for(&scope.0, scope.1))
}

#[cfg(windows)]
fn current_scope() -> Result<Option<(String, u32)>, PlatformError> {
    let mut session = 0_u32;
    // SAFETY: the current process id is valid and `session` is writable output storage.
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(protocol_error_with_os(
            "single_instance:session_query_failed",
            last_error(),
        ));
    }
    let mut length = 256_u32;
    let mut name = vec![0_u16; length as usize];
    // SAFETY: the sizing buffer and length pointer are valid for GetUserNameW.
    if unsafe { GetUserNameW(name.as_mut_ptr(), &mut length) } == 0 {
        return Err(protocol_error_with_os(
            "single_instance:user_query_failed",
            last_error(),
        ));
    }
    name.truncate(length.saturating_sub(1) as usize);
    let name = String::from_utf16(&name)
        .map_err(|_| protocol_error("single_instance:user_query_invalid"))?;
    Ok(Some((name, session)))
}

#[cfg(windows)]
fn serve_pipe(name: &str, sender: SyncSender<ActivationRequest>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        let pipe = match create_pipe(name) {
            Ok(pipe) => pipe,
            Err(_) => return,
        };
        let connected = connect_pipe(pipe);
        if !connected && last_error() != ERROR_PIPE_CONNECTED {
            close_handle(pipe);
            continue;
        }
        let mut frame = [0_u8; FRAME_LENGTH];
        let accepted = read_exact(pipe, &mut frame)
            .ok()
            .and_then(|()| ActivationFrame::decode(&frame).ok())
            .is_some_and(|frame| sender.try_send(frame.request).is_ok());
        let ack = if accepted {
            ActivationAck::Accepted
        } else {
            ActivationAck::Rejected
        }
        .encode();
        let _ = write_all(pipe, &ack);
        // SAFETY: `pipe` is still the live server handle and the API has no retained pointers.
        let _ = unsafe { FlushFileBuffers(pipe) };
        disconnect_pipe(pipe);
        close_handle(pipe);
    }
}

#[cfg(windows)]
fn create_pipe(name: &str) -> Result<isize, PlatformError> {
    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_TYPE_BYTE: u32 = 0;
    const PIPE_READMODE_BYTE: u32 = 0;
    const PIPE_WAIT: u32 = 0;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
    const INVALID_HANDLE_VALUE: isize = -1;
    let wide = wide_null(name);
    // SAFETY: bounded static-name storage, one instance, small bounded buffers, and remote-client
    // rejection are all explicit.  The returned kernel handle is wrapped/closed by the caller.
    let handle = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64,
            64,
            500,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle == 0 {
        Err(protocol_error_with_os(
            "single_instance:pipe_create_failed",
            last_error(),
        ))
    } else {
        Ok(handle)
    }
}

#[cfg(windows)]
fn send_pipe_frame(name: &str, frame: &[u8; FRAME_LENGTH]) -> Result<(), PlatformError> {
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const OPEN_EXISTING: u32 = 3;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const INVALID_HANDLE_VALUE: isize = -1;
    let wide = wide_null(name);
    // SAFETY: the named pipe string is generated from the current scope and contains no input
    // path/command text; the resulting handle is closed below.
    let pipe = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            0,
        )
    };
    if pipe == INVALID_HANDLE_VALUE || pipe == 0 {
        return Err(protocol_error_with_os(
            "single_instance:activation_pipe_unavailable",
            last_error(),
        ));
    }
    let result = (|| {
        write_all(pipe, frame)?;
        let mut ack = [0_u8; ACK_LENGTH];
        read_exact(pipe, &mut ack)?;
        match ActivationAck::decode(&ack)? {
            ActivationAck::Accepted => Ok(()),
            ActivationAck::Rejected => Err(protocol_error("single_instance:activation_rejected")),
        }
    })();
    close_handle(pipe);
    result
}

#[cfg(windows)]
fn wake_pipe(name: &str) {
    let frame = [0_u8; FRAME_LENGTH];
    let _ = send_pipe_frame(name, &frame);
}

#[cfg(windows)]
fn connect_pipe(pipe: isize) -> bool {
    // SAFETY: `pipe` is the live handle returned by CreateNamedPipeW.
    unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) != 0 }
}

#[cfg(windows)]
fn disconnect_pipe(pipe: isize) {
    // SAFETY: `pipe` remains owned by the current server thread until close_handle below.
    unsafe { DisconnectNamedPipe(pipe) };
}

#[cfg(windows)]
fn read_exact(pipe: isize, buffer: &mut [u8]) -> Result<(), PlatformError> {
    let mut offset = 0_usize;
    while offset < buffer.len() {
        let mut read = 0_u32;
        // SAFETY: destination slice is valid for the bounded remaining length and handle is live.
        let ok = unsafe {
            ReadFile(
                pipe,
                buffer[offset..].as_mut_ptr().cast(),
                u32::try_from(buffer.len() - offset).unwrap_or(u32::MAX),
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || read == 0 {
            return Err(protocol_error_with_os(
                "single_instance:pipe_read_failed",
                last_error(),
            ));
        }
        offset = offset.saturating_add(read as usize);
    }
    Ok(())
}

#[cfg(windows)]
fn write_all(pipe: isize, buffer: &[u8]) -> Result<(), PlatformError> {
    let mut offset = 0_usize;
    while offset < buffer.len() {
        let mut written = 0_u32;
        // SAFETY: source slice is valid for the bounded remaining length and handle is live.
        let ok = unsafe {
            WriteFile(
                pipe,
                buffer[offset..].as_ptr().cast(),
                u32::try_from(buffer.len() - offset).unwrap_or(u32::MAX),
                &mut written,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || written == 0 {
            return Err(protocol_error_with_os(
                "single_instance:pipe_write_failed",
                last_error(),
            ));
        }
        offset = offset.saturating_add(written as usize);
    }
    Ok(())
}

#[cfg(windows)]
fn close_handle(handle: isize) {
    if handle != 0 && handle != -1 {
        // SAFETY: called exactly once for this owned kernel handle.
        unsafe { single_instance_close_handle(handle as *mut std::ffi::c_void) };
    }
}

#[cfg(windows)]
fn protocol_error_with_os(code: &'static str, os_code: u32) -> PlatformError {
    PlatformError {
        code,
        os_code: Some(os_code),
    }
}

#[cfg(windows)]
fn last_error() -> u32 {
    // SAFETY: GetLastError has no preconditions.
    unsafe { GetLastError() }
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

#[cfg(windows)]
const ERROR_ALREADY_EXISTS: u32 = 183;
#[cfg(windows)]
const ERROR_PIPE_CONNECTED: u32 = 535;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "CloseHandle"]
    fn single_instance_close_handle(handle: *mut std::ffi::c_void) -> i32;
    fn ConnectNamedPipe(pipe: isize, overlapped: *mut std::ffi::c_void) -> i32;
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *mut std::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: isize,
    ) -> isize;
    fn CreateMutexW(
        security_attributes: *mut std::ffi::c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> isize;
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buffer_size: u32,
        in_buffer_size: u32,
        default_timeout: u32,
        security_attributes: *mut std::ffi::c_void,
    ) -> isize;
    fn DisconnectNamedPipe(pipe: isize) -> i32;
    fn FlushFileBuffers(file: isize) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn GetLastError() -> u32;
    fn GetUserNameW(buffer: *mut u16, size: *mut u32) -> i32;
    fn ProcessIdToSessionId(process_id: u32, session_id: *mut u32) -> i32;
    fn ReadFile(
        file: isize,
        buffer: *mut std::ffi::c_void,
        bytes_to_read: u32,
        bytes_read: *mut u32,
        overlapped: *mut std::ffi::c_void,
    ) -> i32;
    fn WriteFile(
        file: isize,
        buffer: *const std::ffi::c_void,
        bytes_to_write: u32,
        bytes_written: *mut u32,
        overlapped: *mut std::ffi::c_void,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trip_is_one_fixed_versioned_request() {
        let frame = ActivationFrame::new(ActivationRequest::OpenAndRefresh);
        let decoded = ActivationFrame::decode(&frame.encode()).expect("decode");
        assert_eq!(decoded.protocol_version, PROTOCOL_VERSION);
        assert_eq!(decoded.request, ActivationRequest::OpenAndRefresh);
        assert_eq!(decoded.nonce, frame.nonce);
        assert_eq!(frame.encode().len(), FRAME_LENGTH);
    }

    #[test]
    fn bad_frames_are_rejected_without_parsing_extra_data() {
        let frame = ActivationFrame::new(ActivationRequest::Open).encode();
        for (index, value) in [(0, b'X'), (4, 2), (6, 3), (7, 1)] {
            let mut bad = frame.to_vec();
            bad[index] = value;
            assert!(ActivationFrame::decode(&bad).is_err());
        }
        assert!(ActivationFrame::decode(&frame[..FRAME_LENGTH - 1]).is_err());
        assert!(ActivationFrame::decode(&[0; FRAME_LENGTH + 1]).is_err());
    }

    #[test]
    fn scope_name_changes_with_user_or_session() {
        assert_ne!(scoped_object_name("alice", 1), scoped_object_name("bob", 1));
        assert_ne!(
            scoped_object_name("alice", 1),
            scoped_object_name("alice", 2)
        );
        assert!(scoped_object_name("alice", 1).starts_with(r"Local\"));
    }

    #[test]
    fn polling_coalesces_repeated_open_and_preserves_refresh() {
        let (sender, receiver) = mpsc::sync_channel(8);
        sender.send(ActivationRequest::Open).unwrap();
        sender.send(ActivationRequest::Open).unwrap();
        sender.send(ActivationRequest::OpenAndRefresh).unwrap();
        let mut instance = SingleInstance::from_requests(receiver);
        assert_eq!(instance.try_recv(), Some(ActivationRequest::OpenAndRefresh));
        assert_eq!(instance.try_recv(), None);
    }
}
