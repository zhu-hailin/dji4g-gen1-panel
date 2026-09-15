//! One-request client state machine.

use thiserror::Error;

use crate::{
    Deadline, FrameError, HelperRequestV1, HelperResponseV1, PipeTransport, decode_frame,
    encode_frame,
};

pub struct PipeClient<T: PipeTransport> {
    transport: T,
    used: bool,
}

impl<T: PipeTransport> PipeClient<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self {
            transport,
            used: false,
        }
    }

    pub fn verify_server(&self, expected: &crate::PeerExpectation) -> Result<(), ClientError> {
        let peer = self
            .transport
            .peer_identity()
            .map_err(ClientError::Transport)?;
        expected.verify(&peer).map_err(ClientError::PeerRejected)
    }

    pub fn request_once(
        &mut self,
        request: &HelperRequestV1,
        deadline: Deadline,
    ) -> Result<HelperResponseV1, ClientError> {
        if self.used {
            return Err(ClientError::AlreadyUsed);
        }
        self.used = true;
        if deadline.expired() {
            return Err(ClientError::Frame(FrameError::Timeout));
        }
        let frame = encode_frame(request).map_err(ClientError::Frame)?;
        self.transport
            .write_frame(&frame, &deadline)
            .map_err(ClientError::Frame)?;
        let response_frame = self
            .transport
            .read_frame(&deadline)
            .map_err(ClientError::Frame)?;
        let response: HelperResponseV1 =
            decode_frame(&response_frame).map_err(ClientError::Frame)?;
        if response.version != request.version || response.request_id != request.request_id {
            return Err(ClientError::RequestIdMismatch);
        }
        Ok(response)
    }
}

impl<T: PipeTransport> Drop for PipeClient<T> {
    fn drop(&mut self) {
        self.transport.close_once();
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ClientError {
    #[error("the client operation was already consumed")]
    AlreadyUsed,
    #[error("response request id or version did not match")]
    RequestIdMismatch,
    #[error("peer identity was rejected")]
    PeerRejected(crate::PeerRejectCode),
    #[error("IPC frame failed")]
    Frame(FrameError),
    #[error("transport peer could not be inspected")]
    Transport(crate::TransportError),
}
