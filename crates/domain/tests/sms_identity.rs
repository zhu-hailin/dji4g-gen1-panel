use dji4g_domain::{
    SmsConcatReference, SmsEncoding, SmsMessage, SmsMultipartInfo, SmsStatus, SmsStorageId,
};

fn message(body: &str) -> SmsMessage {
    SmsMessage::new(
        1,
        SmsStorageId("SM".into()),
        1,
        2,
        "+12025550123",
        body,
        SmsEncoding::Gsm7,
        SmsStatus::Received,
    )
}

#[test]
fn payload_identity_is_stable_under_read_status_location_and_epochs() {
    let original = message("body");
    let mut changed = original.clone();
    changed.read = Some(true);
    changed.status = SmsStatus::Incomplete;
    assert_eq!(original.content_digest(), changed.content_digest());
    changed.index = 9;
    changed.storage = SmsStorageId("ME".into());
    changed.device_epoch = 8;
    changed.sim_epoch = 10;
    assert_eq!(
        original.payload_fingerprint(),
        changed.payload_fingerprint()
    );
    assert_ne!(original.content_digest(), changed.content_digest());
}

#[test]
fn payload_identity_covers_encoding_timestamp_and_complete_typed_header() {
    let original = message("same");
    let mut variants = vec![original.clone(); 8];
    variants[0].encoding = SmsEncoding::Ucs2;
    variants[1].service_centre_timestamp = Some(String::new());
    for (variant, reference, total, sequence) in [
        (2, SmsConcatReference::EightBit(52), 2, 1),
        (3, SmsConcatReference::SixteenBit(52), 2, 1),
        (4, SmsConcatReference::SixteenBit(0x1234), 2, 1),
        (5, SmsConcatReference::SixteenBit(0x5634), 2, 1),
        (6, SmsConcatReference::EightBit(52), 3, 1),
        (7, SmsConcatReference::EightBit(52), 2, 2),
    ] {
        variants[variant].multipart = Some(SmsMultipartInfo {
            reference,
            total,
            sequence,
        });
    }
    let mut unique = std::collections::HashSet::new();
    unique.insert(original.payload_fingerprint());
    for variant in variants {
        assert!(unique.insert(variant.payload_fingerprint()));
    }
}

#[test]
fn variable_length_payload_fields_have_unambiguous_boundaries() {
    let mut first = message("ab");
    first.service_centre_timestamp = Some("c".into());
    let mut second = message("a");
    second.service_centre_timestamp = Some("bc".into());
    assert_ne!(first.payload_fingerprint(), second.payload_fingerprint());
    let debug = format!("{:?}", first.fragment_key());
    assert!(!debug.contains("12025550123"));
    assert!(debug.contains("[REDACTED]"));
}
