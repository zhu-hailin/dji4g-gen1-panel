//! One-client/one-request server state machine.

use std::time::{Instant, SystemTime};

use thiserror::Error;

use crate::{
    Deadline, FrameError, HelperRequestV1, HelperResponseV1, OperationNonce, PeerExpectation,
    PeerRejectCode, PipeTransport, RequestRejectCode, decode_frame, encode_frame, validate_request,
};

pub struct PipeServer<T: PipeTransport> {
    transport: T,
    consumed: bool,
}

impl<T: PipeTransport> PipeServer<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self {
            transport,
            consumed: false,
        }
    }

    /// Handles exactly one peer and one request.  The transport is dropped (and therefore closed)
    /// on every return path.  A second-frame probe happens before the handler is invoked, so it
    /// cannot trigger a second operation or even the first operation when the connection is
    /// carrying multiple requests.
    pub fn serve_once(
        mut self,
        expected_peer: &PeerExpectation,
        expected_nonce: &OperationNonce,
        deadline: Deadline,
        handler: impl FnOnce(HelperRequestV1) -> HelperResponseV1,
    ) -> Result<(), ServerError> {
        if self.consumed {
            return Err(ServerError::SecondClient);
        }
        self.consumed = true;
        let peer = self
            .transport
            .peer_identity()
            .map_err(ServerError::Transport)?;
        expected_peer
            .verify(&peer)
            .map_err(ServerError::PeerRejected)?;
        if deadline.expired() {
            return Err(ServerError::Frame(FrameError::Timeout));
        }
        let frame = self
            .transport
            .read_frame(&deadline)
            .map_err(ServerError::Frame)?;
        let request: HelperRequestV1 = decode_frame(&frame).map_err(ServerError::Frame)?;
        if self
            .transport
            .try_read_frame()
            .map_err(ServerError::Frame)?
            .is_some()
        {
            return Err(ServerError::Protocol(RequestRejectCode::SecondFrame));
        }
        validate_request(&request, expected_nonce, SystemTime::now())
            .map_err(ServerError::Protocol)?;
        let response = handler(request.clone());
        if response.version != request.version || response.request_id != request.request_id {
            return Err(ServerError::Protocol(RequestRejectCode::ProtocolRejected));
        }
        let response_frame = encode_frame(&response).map_err(ServerError::Frame)?;
        self.transport
            .write_frame(&response_frame, &deadline)
            .map_err(ServerError::Frame)
    }
}

impl<T: PipeTransport> Drop for PipeServer<T> {
    fn drop(&mut self) {
        self.transport.close_once();
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ServerError {
    #[error("peer identity was rejected")]
    PeerRejected(PeerRejectCode),
    #[error("request was rejected by the protocol")]
    Protocol(RequestRejectCode),
    #[error("IPC frame failed")]
    Frame(FrameError),
    #[error("transport peer could not be inspected")]
    Transport(crate::TransportError),
    #[error("a second client was presented")]
    SecondClient,
}

/// A deadline from the caller's operation is preserved across request and response I/O.  This
/// helper is public for tests that need a deterministic near-expiry state.
#[must_use]
pub fn deadline_from_instant(end: Instant) -> Deadline {
    Deadline::at(end)
}
