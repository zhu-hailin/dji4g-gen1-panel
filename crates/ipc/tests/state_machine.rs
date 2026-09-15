use std::{
    collections::VecDeque,
    time::{Duration, SystemTime},
};

use dji4g_ipc::{
    ClientError, Deadline, FrameError, Hash32, HelperActionV1, HelperRequestV1, HelperResponseV1,
    HelperResultV1, IntegrityLevel, OperationNonce, PeerExpectation, PeerIdentity, PipeClient,
    PipeServer, PipeTransport, ProtocolVersion, RequestId, SupportedProfileV1, TargetProofV1,
    TransportError, UnixMillis, encode_frame,
};

#[derive(Debug)]
struct FakeTransport {
    peer: Result<PeerIdentity, TransportError>,
    incoming: VecDeque<Result<Vec<u8>, FrameError>>,
    writes: Vec<Vec<u8>>,
    closed: usize,
    probe_second: Option<Vec<u8>>,
}

impl FakeTransport {
    fn with_peer(peer: PeerIdentity) -> Self {
        Self {
            peer: Ok(peer),
            incoming: VecDeque::new(),
            writes: Vec::new(),
            closed: 0,
            probe_second: None,
        }
    }
}

impl PipeTransport for FakeTransport {
    fn peer_identity(&self) -> Result<PeerIdentity, TransportError> {
        self.peer
    }

    fn read_frame(&mut self, _deadline: &Deadline) -> Result<Vec<u8>, FrameError> {
        self.incoming
            .pop_front()
            .unwrap_or(Err(FrameError::Timeout))
    }

    fn write_frame(&mut self, payload: &[u8], _deadline: &Deadline) -> Result<(), FrameError> {
        self.writes.push(payload.to_vec());
        Ok(())
    }

    fn try_read_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        Ok(self.probe_second.take())
    }

    fn close_once(&mut self) {
        self.closed += 1;
    }
}

fn peer(seed: u8) -> PeerIdentity {
    PeerIdentity {
        pid: 42,
        creation_time: 77,
        user_sid_hash: Hash32::from_bytes([seed; 32]),
        session_id: 2,
        integrity: IntegrityLevel::High,
        image_hash: Hash32::from_bytes([6; 32]),
        remote: false,
    }
}

fn expectation() -> PeerExpectation {
    PeerExpectation {
        pid: 42,
        creation_time: 77,
        user_sid_hash: Hash32::from_bytes([3; 32]),
        session_id: 2,
        minimum_integrity: IntegrityLevel::High,
        image_hash: Hash32::from_bytes([6; 32]),
    }
}

fn request() -> HelperRequestV1 {
    let now = UnixMillis::from_system_time(SystemTime::now()).unwrap().0;
    HelperRequestV1 {
        version: ProtocolVersion::V1,
        request_id: RequestId::from_bytes([8; 16]),
        nonce: OperationNonce::from_bytes([9; 32]),
        issued_at: UnixMillis(now),
        expires_at: UnixMillis(now + 30_000),
        target: TargetProofV1 {
            profile: SupportedProfileV1::DjiGen1,
            epoch: 1,
            identity_hash: Hash32::from_bytes([1; 32]),
            before_state_hash: Hash32::from_bytes([2; 32]),
        },
        action: HelperActionV1::InspectTarget,
    }
}

#[test]
fn server_rejects_second_frame_before_handler_and_closes_transport() {
    let req = request();
    let mut transport = FakeTransport::with_peer(peer(3));
    transport
        .incoming
        .push_back(Ok(encode_frame(&req).unwrap()));
    transport.probe_second = Some(encode_frame(&req).unwrap());
    let mut executed = false;
    let result = PipeServer::new(transport).serve_once(
        &expectation(),
        &OperationNonce::from_bytes([9; 32]),
        Deadline::from_now(Duration::from_secs(1)),
        |_| {
            executed = true;
            HelperResponseV1 {
                version: ProtocolVersion::V1,
                request_id: RequestId::from_bytes([8; 16]),
                result: HelperResultV1::Rejected {
                    code: dji4g_ipc::RequestRejectCode::SecondFrame,
                },
            }
        },
    );
    assert_eq!(
        result,
        Err(dji4g_ipc::ServerError::Protocol(
            dji4g_ipc::RequestRejectCode::SecondFrame
        ))
    );
    assert!(!executed);
}

#[test]
fn server_rejects_wrong_peer_without_read_or_handler() {
    let mut transport = FakeTransport::with_peer(peer(4));
    transport
        .incoming
        .push_back(Ok(encode_frame(&request()).unwrap()));
    let result = PipeServer::new(transport).serve_once(
        &expectation(),
        &OperationNonce::from_bytes([9; 32]),
        Deadline::from_now(Duration::from_secs(1)),
        |_| panic!("wrong peer must not reach handler"),
    );
    assert_eq!(
        result,
        Err(dji4g_ipc::ServerError::PeerRejected(
            dji4g_ipc::PeerRejectCode::UserMismatch
        ))
    );
}

#[test]
fn client_is_one_shot_and_rejects_response_correlation_mismatch() {
    let req = request();
    let mut transport = FakeTransport::with_peer(peer(3));
    let wrong = HelperResponseV1 {
        version: ProtocolVersion::V1,
        request_id: RequestId::from_bytes([1; 16]),
        result: HelperResultV1::Rejected {
            code: dji4g_ipc::RequestRejectCode::Internal,
        },
    };
    transport
        .incoming
        .push_back(Ok(encode_frame(&wrong).unwrap()));
    let mut client = PipeClient::new(transport);
    assert_eq!(
        client.request_once(&req, Deadline::from_now(Duration::from_secs(1))),
        Err(ClientError::RequestIdMismatch)
    );
    assert_eq!(
        client.request_once(&req, Deadline::from_now(Duration::from_secs(1))),
        Err(ClientError::AlreadyUsed)
    );
}
