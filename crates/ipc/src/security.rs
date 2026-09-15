//! Peer and lifetime policy shared by the platform transport and pure state machine.

use std::time::{Duration, Instant};

use thiserror::Error;

use crate::{
    FrameError, Hash32, HelperRequestV1, OperationNonce, PeerRejectCode, RequestRejectCode,
    validate_request,
};

/// A monotonic operation deadline.  Wall-clock timestamps in a request are additional evidence,
/// never a substitute for this bound.
#[derive(Clone, Copy, Debug)]
pub struct Deadline {
    started: Instant,
    end: Instant,
}

impl Deadline {
    #[must_use]
    pub fn from_now(duration: Duration) -> Self {
        let duration = duration.min(crate::MAX_OPERATION_LIFETIME);
        let started = Instant::now();
        Self {
            started,
            end: started + duration,
        }
    }

    #[must_use]
    pub fn at(end: Instant) -> Self {
        let now = Instant::now();
        let end = end.min(now + crate::MAX_OPERATION_LIFETIME);
        Self { started: now, end }
    }

    #[must_use]
    pub fn remaining(self) -> Duration {
        self.end.saturating_duration_since(Instant::now())
    }

    #[must_use]
    pub fn expired(self) -> bool {
        Instant::now() >= self.end
    }

    #[must_use]
    pub fn elapsed(self) -> Duration {
        self.started.elapsed()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegrityLevel {
    Low,
    Medium,
    High,
    System,
}

impl IntegrityLevel {
    #[must_use]
    pub fn satisfies(self, minimum: Self) -> bool {
        let rank = |value| match value {
            Self::Low => 0_u8,
            Self::Medium => 1,
            Self::High => 2,
            Self::System => 3,
        };
        rank(self) >= rank(minimum)
    }
}

/// Only non-sensitive proof material is kept in this value.  SID and image path are represented by
/// fixed digests so accidental formatting cannot leak the full identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerIdentity {
    pub pid: u32,
    pub creation_time: u64,
    pub user_sid_hash: Hash32,
    pub session_id: u32,
    pub integrity: IntegrityLevel,
    pub image_hash: Hash32,
    pub remote: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerExpectation {
    pub pid: u32,
    pub creation_time: u64,
    pub user_sid_hash: Hash32,
    pub session_id: u32,
    pub minimum_integrity: IntegrityLevel,
    pub image_hash: Hash32,
}

impl PeerExpectation {
    pub fn verify(&self, peer: &PeerIdentity) -> Result<(), PeerRejectCode> {
        if peer.remote {
            return Err(PeerRejectCode::RemoteClient);
        }
        if peer.pid != self.pid {
            return Err(PeerRejectCode::PeerPidMismatch);
        }
        if peer.creation_time != self.creation_time {
            return Err(PeerRejectCode::PeerCreationChanged);
        }
        if peer.user_sid_hash != self.user_sid_hash {
            return Err(PeerRejectCode::UserMismatch);
        }
        if peer.session_id != self.session_id {
            return Err(PeerRejectCode::SessionMismatch);
        }
        if !peer.integrity.satisfies(self.minimum_integrity) {
            return Err(PeerRejectCode::IntegrityMismatch);
        }
        if peer.image_hash != self.image_hash {
            return Err(PeerRejectCode::PeerImageMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CurrentUserSession {
    pub user_sid_hash: Hash32,
    pub session_id: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessExpectation {
    pub pid: u32,
    pub creation_time: u64,
    pub user_sid_hash: Hash32,
    pub session_id: u32,
    pub minimum_integrity: IntegrityLevel,
    pub image_hash: Hash32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum TransportError {
    #[error("peer identity could not be inspected")]
    PeerUnavailable,
    #[error("peer identity was rejected")]
    PeerRejected(PeerRejectCode),
    #[error("transport timed out")]
    Timeout,
    #[error("transport disconnected")]
    Disconnected,
    #[error("transport failed")]
    Failed,
}

/// The platform-specific named pipe owns this trait; the protocol state machine never sees a raw
/// handle.  `try_read_frame` is a non-blocking probe used to reject a second frame before invoking
/// the operation handler.  A transport that cannot probe can safely return `Ok(None)`.
pub trait PipeTransport: Send {
    fn peer_identity(&self) -> Result<PeerIdentity, TransportError>;
    fn read_frame(&mut self, deadline: &Deadline) -> Result<Vec<u8>, FrameError>;
    fn write_frame(&mut self, payload: &[u8], deadline: &Deadline) -> Result<(), FrameError>;
    fn try_read_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        Ok(None)
    }
    fn close_once(&mut self);
}

pub fn validate_peer(
    peer: &PeerIdentity,
    expected: &PeerExpectation,
) -> Result<(), PeerRejectCode> {
    expected.verify(peer)
}

pub fn validate_request_for_peer(
    request: &HelperRequestV1,
    command_nonce: &OperationNonce,
    now: std::time::SystemTime,
) -> Result<(), RequestRejectCode> {
    validate_request(request, command_nonce, now)
}
