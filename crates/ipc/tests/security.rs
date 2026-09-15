use dji4g_ipc::{
    Deadline, Hash32, IntegrityLevel, PeerExpectation, PeerIdentity, PeerRejectCode, PipeName,
};
use std::time::Duration;

fn peer() -> PeerIdentity {
    PeerIdentity {
        pid: 7,
        creation_time: 11,
        user_sid_hash: Hash32::from_bytes([1; 32]),
        session_id: 3,
        integrity: IntegrityLevel::High,
        image_hash: Hash32::from_bytes([2; 32]),
        remote: false,
    }
}

fn expectation() -> PeerExpectation {
    PeerExpectation {
        pid: 7,
        creation_time: 11,
        user_sid_hash: Hash32::from_bytes([1; 32]),
        session_id: 3,
        minimum_integrity: IntegrityLevel::High,
        image_hash: Hash32::from_bytes([2; 32]),
    }
}

#[test]
fn peer_proof_rejects_each_changed_binding_field() {
    let mut changed = peer();
    changed.remote = true;
    assert_eq!(
        expectation().verify(&changed),
        Err(PeerRejectCode::RemoteClient)
    );

    let mut changed = peer();
    changed.pid += 1;
    assert_eq!(
        expectation().verify(&changed),
        Err(PeerRejectCode::PeerPidMismatch)
    );

    let mut changed = peer();
    changed.creation_time += 1;
    assert_eq!(
        expectation().verify(&changed),
        Err(PeerRejectCode::PeerCreationChanged)
    );

    let mut changed = peer();
    changed.user_sid_hash = Hash32::from_bytes([3; 32]);
    assert_eq!(
        expectation().verify(&changed),
        Err(PeerRejectCode::UserMismatch)
    );

    let mut changed = peer();
    changed.session_id += 1;
    assert_eq!(
        expectation().verify(&changed),
        Err(PeerRejectCode::SessionMismatch)
    );

    let mut changed = peer();
    changed.integrity = IntegrityLevel::Medium;
    assert_eq!(
        expectation().verify(&changed),
        Err(PeerRejectCode::IntegrityMismatch)
    );

    let mut changed = peer();
    changed.image_hash = Hash32::from_bytes([4; 32]);
    assert_eq!(
        expectation().verify(&changed),
        Err(PeerRejectCode::PeerImageMismatch)
    );
}

#[test]
fn generated_pipe_names_are_private_and_strictly_parsed() {
    let name = PipeName::parse(r"\\.\pipe\dji4g-panel-000102030405060708090a0b0c0d0e0f").unwrap();
    assert_eq!(
        name.as_str(),
        r"\\.\pipe\dji4g-panel-000102030405060708090a0b0c0d0e0f"
    );
    assert!(PipeName::parse(r"\\.\pipe\dji4g-panel-000102030405060708090a0b0c0d0e0F").is_none());
    assert!(PipeName::parse(r"\\.\pipe\dji4g-panel-000102030405060708090a0b0c0d0e0").is_none());
    assert!(!format!("{name:?}").contains("00010203"));
}

#[test]
fn deadlines_are_hard_bounded_and_expire_without_waiting() {
    let bounded = Deadline::from_now(Duration::from_secs(600));
    assert!(bounded.remaining() <= Duration::from_secs(60));
    let expired = Deadline::from_now(Duration::ZERO);
    assert!(expired.expired());
}
