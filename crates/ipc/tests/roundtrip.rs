use std::{
    io::Cursor,
    net::{IpAddr, Ipv4Addr},
    time::{Duration, SystemTime},
};

use dji4g_ipc::{
    BoundedDnsServers, DnsProfileV1, Hash32, HelperActionV1, HelperRequestV1, HelperResponseV1,
    HelperResultV1, MAX_FRAME_BYTES, OperationCode, OperationNonce, OperationResultV1,
    PdpContextIdV1, ProtocolVersion, RequestId, RollbackResultV1, SupportedProfileV1,
    TargetProofV1, UnixMillis, UsbNetProfileV1, ValidatedApnV1, decode_frame, encode_frame,
    read_frame_from, write_frame_to,
};

fn nonce(seed: u8) -> OperationNonce {
    OperationNonce::from_bytes([seed; 32])
}

fn id(seed: u8) -> RequestId {
    RequestId::from_bytes([seed; 16])
}

fn hash(seed: u8) -> Hash32 {
    Hash32::from_bytes([seed; 32])
}

fn request(action: HelperActionV1) -> HelperRequestV1 {
    HelperRequestV1 {
        version: ProtocolVersion::V1,
        request_id: id(7),
        nonce: nonce(9),
        issued_at: UnixMillis(1_700_000_000_000),
        expires_at: UnixMillis(1_700_000_030_000),
        target: TargetProofV1 {
            profile: SupportedProfileV1::DjiGen1,
            epoch: 4,
            identity_hash: hash(1),
            before_state_hash: hash(2),
        },
        action,
    }
}

#[test]
fn every_closed_action_roundtrips_inside_one_bounded_frame() {
    let dns = BoundedDnsServers::try_from(vec![
        IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
        IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
    ])
    .unwrap();
    let actions = vec![
        HelperActionV1::InspectTarget,
        HelperActionV1::RenewDhcp,
        HelperActionV1::ApplyDnsProfile {
            profile: DnsProfileV1::Automatic,
        },
        HelperActionV1::ApplyDnsProfile {
            profile: DnsProfileV1::Static { servers: dns },
        },
        HelperActionV1::RestartAdapter,
        HelperActionV1::ReenumerateDevice,
        HelperActionV1::RestartModule,
        HelperActionV1::EditApn {
            cid: PdpContextIdV1::new(1).unwrap(),
            apn: ValidatedApnV1::try_from("internet.example".to_owned()).unwrap(),
        },
        HelperActionV1::SetUsbNetProfile {
            profile: UsbNetProfileV1::DjiNdis,
        },
        HelperActionV1::ToggleHotspot { enabled: true },
    ];

    for action in actions {
        let bytes = encode_frame(&request(action.clone())).unwrap();
        assert!(bytes.len() <= MAX_FRAME_BYTES + 4);
        let decoded: HelperRequestV1 = decode_frame(&bytes).unwrap();
        assert_eq!(decoded.action, action);
        assert_eq!(decoded.request_id, id(7));
        assert_eq!(decoded.nonce, nonce(9));
    }
}

#[test]
fn response_roundtrips_without_nonce_or_sensitive_target_material() {
    let response = HelperResponseV1 {
        version: ProtocolVersion::V1,
        request_id: id(7),
        result: HelperResultV1::Completed(OperationResultV1::Failed {
            code: OperationCode::VerificationFailed,
            rollback: RollbackResultV1::Failed {
                code: OperationCode::RollbackFailed,
            },
        }),
    };
    let bytes = encode_frame(&response).unwrap();
    let text = String::from_utf8(bytes[4..].to_vec()).unwrap();
    assert!(!text.contains("nonce"));
    assert!(!text.contains("internet"));
    let decoded: HelperResponseV1 = decode_frame(&bytes).unwrap();
    assert_eq!(decoded, response);
}

#[test]
fn redacted_debug_does_not_expose_nonce_or_apn() {
    let request = request(HelperActionV1::EditApn {
        cid: PdpContextIdV1::new(1).unwrap(),
        apn: ValidatedApnV1::try_from("secret.apn.example".to_owned()).unwrap(),
    });
    let debug = format!("{request:?}");
    assert!(!debug.contains("secret.apn.example"));
    assert!(!debug.contains("09090909"));
}

#[test]
fn max_size_payload_is_accepted_but_prefix_is_not_payload() {
    let payload = vec![b'a'; MAX_FRAME_BYTES];
    let frame = dji4g_ipc::encode_payload(&payload).unwrap();
    assert_eq!(
        u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize,
        MAX_FRAME_BYTES
    );
    assert_eq!(&frame[4..], payload);
}

#[test]
fn dns_and_apn_boundaries_are_typed() {
    assert!(ValidatedApnV1::try_from("a".to_owned()).is_ok());
    assert!(ValidatedApnV1::try_from("a".repeat(100)).is_ok());
    assert!(ValidatedApnV1::try_from("a".repeat(101)).is_err());
    assert!(ValidatedApnV1::try_from("a,b".to_owned()).is_err());
    assert!(PdpContextIdV1::new(0).is_err());
    assert!(PdpContextIdV1::new(16).is_ok());
    assert!(PdpContextIdV1::new(17).is_err());
    assert!(BoundedDnsServers::try_from(vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)); 4]).is_err());
}

#[test]
fn request_uses_explicit_version_and_bounded_lifetime() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_millis(1_700_000_000_000);
    let req = request(HelperActionV1::InspectTarget);
    dji4g_ipc::validate_request(&req, &nonce(9), now).unwrap();
}

#[test]
fn blocking_frame_helpers_roundtrip_the_length_prefix() {
    let payload = br#"{"ok":true}"#;
    let mut wire = Cursor::new(Vec::new());
    write_frame_to(&mut wire, payload).unwrap();
    let frame = read_frame_from(&mut Cursor::new(wire.into_inner())).unwrap();
    assert_eq!(dji4g_ipc::decode_payload(&frame).unwrap(), payload);
}
