//! Bounded, length-prefixed JSON framing.

use std::{
    fmt,
    io::{self, Read, Write},
};

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::MAX_FRAME_BYTES;

#[derive(Clone, Eq, PartialEq, Error)]
pub enum FrameError {
    #[error("empty IPC frame")]
    EmptyFrame,
    #[error("IPC frame exceeds the bounded payload limit")]
    FrameTooLarge { length: usize },
    #[error("IPC frame is truncated")]
    Truncated { needed: usize, received: usize },
    #[error("IPC frame has a second payload")]
    SecondFrame,
    #[error("IPC payload is not UTF-8")]
    InvalidUtf8,
    #[error("IPC JSON payload is malformed")]
    Malformed,
    #[error("IPC JSON nesting is too deep")]
    DepthExceeded,
    #[error("IPC transport timed out")]
    Timeout,
    #[error("IPC transport disconnected")]
    Disconnected,
    #[error("IPC transport I/O failed")]
    Io,
}

impl fmt::Debug for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Lengths and native I/O details are intentionally omitted from diagnostics/logging.  The
        // stable variant is enough for both sides of the one-shot protocol.
        formatter.write_str(match self {
            Self::EmptyFrame => "EmptyFrame",
            Self::FrameTooLarge { .. } => "FrameTooLarge",
            Self::Truncated { .. } => "Truncated",
            Self::SecondFrame => "SecondFrame",
            Self::InvalidUtf8 => "InvalidUtf8",
            Self::Malformed => "Malformed",
            Self::DepthExceeded => "DepthExceeded",
            Self::Timeout => "Timeout",
            Self::Disconnected => "Disconnected",
            Self::Io => "Io",
        })
    }
}

#[must_use]
pub fn frame_error_code(error: &FrameError) -> &'static str {
    match error {
        FrameError::EmptyFrame => "ipc:empty_frame",
        FrameError::FrameTooLarge { .. } => "ipc:frame_too_large",
        FrameError::Truncated { .. } => "ipc:truncated",
        FrameError::SecondFrame => "ipc:second_frame",
        FrameError::InvalidUtf8 => "ipc:invalid_utf8",
        FrameError::Malformed => "ipc:malformed",
        FrameError::DepthExceeded => "ipc:depth_exceeded",
        FrameError::Timeout => "ipc:timeout",
        FrameError::Disconnected => "ipc:disconnected",
        FrameError::Io => "ipc:io",
    }
}

/// Serializes one JSON value and prepends its checked little-endian payload length.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, FrameError> {
    let payload = serde_json::to_vec(value).map_err(|_| FrameError::Malformed)?;
    encode_payload(&payload)
}

/// Adds a length prefix to an already serialized payload.  This is public for transport tests and
/// for native message-mode pipes; callers must still pass a payload no larger than 32 KiB.
pub fn encode_payload(payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    if payload.is_empty() {
        return Err(FrameError::EmptyFrame);
    }
    if payload.len() > MAX_FRAME_BYTES {
        return Err(FrameError::FrameTooLarge {
            length: payload.len(),
        });
    }
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Decodes exactly one length-prefixed frame.  Any bytes after the declared payload are treated as
/// a second frame so a caller cannot accidentally process two operations through one connection.
pub fn decode_frame<T: DeserializeOwned>(frame: &[u8]) -> Result<T, FrameError> {
    let payload = decode_payload(frame)?;
    decode_json_payload(payload)
}

/// Returns the single payload from a length-prefixed frame without deserializing it.
pub fn decode_payload(frame: &[u8]) -> Result<&[u8], FrameError> {
    if frame.len() < 4 {
        return if frame.is_empty() {
            Err(FrameError::EmptyFrame)
        } else {
            Err(FrameError::Truncated {
                needed: 4,
                received: frame.len(),
            })
        };
    }
    let length = u32::from_le_bytes(frame[..4].try_into().expect("four-byte prefix")) as usize;
    if length == 0 {
        return Err(FrameError::EmptyFrame);
    }
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::FrameTooLarge { length });
    }
    let expected = 4 + length;
    if frame.len() < expected {
        return Err(FrameError::Truncated {
            needed: expected,
            received: frame.len(),
        });
    }
    if frame.len() > expected {
        return Err(FrameError::SecondFrame);
    }
    Ok(&frame[4..expected])
}

fn decode_json_payload<T: DeserializeOwned>(payload: &[u8]) -> Result<T, FrameError> {
    if std::str::from_utf8(payload).is_err() {
        return Err(FrameError::InvalidUtf8);
    }
    enforce_depth(payload)?;
    serde_json::from_slice(payload).map_err(|_| FrameError::Malformed)
}

/// The JSON parser already rejects trailing values, but this explicit bounded scanner gives a
/// stable depth failure before serde recurses deeply on attacker-controlled input.
fn enforce_depth(payload: &[u8]) -> Result<(), FrameError> {
    const MAX_JSON_DEPTH: usize = 128;
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in payload {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > MAX_JSON_DEPTH {
                    return Err(FrameError::DepthExceeded);
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

/// Reads one bounded, length-prefixed frame from an ordinary blocking reader. Native overlapped
/// transports use the same length checks but supply their own deadline-aware implementation of
/// [`crate::PipeTransport`].
pub fn read_frame_from<R: Read>(reader: &mut R) -> Result<Vec<u8>, FrameError> {
    let mut prefix = [0_u8; 4];
    reader.read_exact(&mut prefix).map_err(map_io_error)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length == 0 {
        return Err(FrameError::EmptyFrame);
    }
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::FrameTooLarge { length });
    }
    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload).map_err(map_io_error)?;
    let mut frame = Vec::with_capacity(4 + length);
    frame.extend_from_slice(&prefix);
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn write_frame_to<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), FrameError> {
    let frame = encode_payload(payload)?;
    writer.write_all(&frame).map_err(map_io_error)
}

fn map_io_error(error: io::Error) -> FrameError {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => FrameError::Timeout,
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::NotConnected => FrameError::Disconnected,
        _ => FrameError::Io,
    }
}
