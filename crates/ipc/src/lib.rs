#![forbid(unsafe_code)]

//! Versioned IPC contracts shared by the panel and helper.

mod client;
mod framing;
mod protocol;
mod security;
mod server;

pub use client::{ClientError, PipeClient};
pub use framing::{
    FrameError, decode_frame, decode_payload, encode_frame, encode_payload, frame_error_code,
    read_frame_from, write_frame_to,
};
pub use protocol::*;
pub use security::{
    CurrentUserSession, Deadline, IntegrityLevel, PeerExpectation, PeerIdentity, PipeTransport,
    ProcessExpectation, TransportError, validate_peer, validate_request_for_peer,
};
pub use server::{PipeServer, ServerError, deadline_from_instant};

pub const PROTOCOL_VERSION_V1: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 32 * 1024;
pub const MAX_OPERATION_LIFETIME: std::time::Duration = std::time::Duration::from_secs(60);
pub const MAX_IO_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
pub const MAX_CLOCK_SKEW: std::time::Duration = std::time::Duration::from_secs(5);
