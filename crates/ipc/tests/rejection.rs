use std::time::{Duration, SystemTime};

use dji4g_ipc::{
    FrameError, Hash32, HelperActionV1, HelperRequestV1, OperationNonce, ProtocolVersion,
    RequestId, RequestRejectCode, SupportedProfileV1, TargetProofV1, UnixMillis, decode_frame,
    encode_frame, frame_error_code, validate_request,
};

fn request() -> HelperRequestV1 {
    HelperRequestV1 {
        version: ProtocolVersion::V1,
        request_id: RequestId::from_bytes([1; 16]),
        nonce: OperationNonce::from_bytes([2; 32]),
        issued_at: UnixMillis(1_000_000),
        expires_at: UnixMillis(1_000_030),
        target: TargetProofV1 {
            profile: SupportedProfileV1::DjiGen1,
            epoch: 1,
            identity_hash: Hash32::from_bytes([3; 32]),
            before_state_hash: Hash32::from_bytes([4; 32]),
        },
        action: HelperActionV1::InspectTarget,
    }
}

#[test]
fn rejects_empty_oversized_truncated_and_trailing_frames() {
    assert!(matches!(
        decode_frame::<HelperRequestV1>(&[]),
        Err(FrameError::EmptyFrame)
    ));
    let oversized = (32 * 1024 + 1_u32).to_le_bytes().to_vec();
    assert!(matches!(
        decode_frame::<HelperRequestV1>(&oversized),
        Err(FrameError::FrameTooLarge { .. })
    ));
    assert!(matches!(
        decode_frame::<HelperRequestV1>(&[3, 0, 0, 0, b'{']),
        Err(FrameError::Truncated { .. })
    ));
    let mut trailing = encode_frame(&request()).unwrap();
    trailing.extend_from_slice(&[1, 2, 3]);
    assert!(matches!(
        decode_frame::<HelperRequestV1>(&trailing),
        Err(FrameError::SecondFrame)
    ));
}

#[test]
fn rejects_invalid_json_unknown_fields_and_invalid_utf8() {
    let mut unknown = br#"{"version":1,"request_id":"01010101010101010101010101010101","nonce":"0202020202020202020202020202020202020202020202020202020202020202","issued_at":1000000,"expires_at":1000030,"target":{"profile":"DjiGen1","epoch":1,"identity_hash":"0303030303030303030303030303030303030303030303030303030303030303","before_state_hash":"0404040404040404040404040404040404040404040404040404040404040404"},"action":{"kind":"InspectTarget","args":null},"extra":true}"#.to_vec();
    let frame = dji4g_ipc::encode_payload(&unknown).unwrap();
    assert!(matches!(
        decode_frame::<HelperRequestV1>(&frame),
        Err(FrameError::Malformed)
    ));
    unknown[0] = 0xff;
    let frame = dji4g_ipc::encode_payload(&unknown).unwrap();
    assert!(matches!(
        decode_frame::<HelperRequestV1>(&frame),
        Err(FrameError::InvalidUtf8)
    ));
}

#[test]
fn rejects_second_json_value_and_deep_json() {
    let frame = dji4g_ipc::encode_payload(br#"{} {}"#).unwrap();
    assert!(matches!(
        decode_frame::<HelperRequestV1>(&frame),
        Err(FrameError::Malformed)
    ));
    let deep = format!("{}0{}", "[".repeat(129), "]".repeat(129));
    let frame = dji4g_ipc::encode_payload(deep.as_bytes()).unwrap();
    assert!(matches!(
        decode_frame::<HelperRequestV1>(&frame),
        Err(FrameError::DepthExceeded)
    ));
}

#[test]
fn rejects_expired_future_invalid_lifetime_and_nonce_mismatch() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_millis(1_000_000);
    let mut req = request();
    assert_eq!(
        validate_request(&req, &OperationNonce::from_bytes([2; 32]), now),
        Ok(())
    );
    req.nonce = OperationNonce::from_bytes([9; 32]);
    assert_eq!(
        validate_request(&req, &OperationNonce::from_bytes([2; 32]), now),
        Err(RequestRejectCode::NonceMismatch)
    );
    req = request();
    req.issued_at = UnixMillis(900_000);
    req.expires_at = UnixMillis(999_999);
    assert_eq!(
        validate_request(&req, &OperationNonce::from_bytes([2; 32]), now),
        Err(RequestRejectCode::Expired)
    );
    req = request();
    req.issued_at = UnixMillis(1_000_000 + 6_000);
    assert_eq!(
        validate_request(&req, &OperationNonce::from_bytes([2; 32]), now),
        Err(RequestRejectCode::FutureIssuedAt)
    );
    req = request();
    req.expires_at = UnixMillis(1_000_000 + 60_001);
    assert_eq!(
        validate_request(&req, &OperationNonce::from_bytes([2; 32]), now),
        Err(RequestRejectCode::InvalidLifetime)
    );
}

#[test]
fn frame_error_codes_are_stable_and_dont_include_payload() {
    let error = FrameError::FrameTooLarge { length: 99_999 };
    assert_eq!(frame_error_code(&error), "ipc:frame_too_large");
    assert!(!format!("{error:?}").contains("99_999"));
}
