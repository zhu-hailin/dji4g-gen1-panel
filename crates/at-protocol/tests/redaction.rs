use dji4g_at_protocol::{Apn, AtCommand, PdpContextId, redact_at_text, redact_at_transaction_line};

#[test]
fn redacts_apns_identifiers_and_pin_like_values() {
    let input = concat!(
        "AT+CGDCONT=1,\"IP\",\"3gnet\"\r\n",
        "+CGDCONT: 1,\"IP\",\"secret.apn\",\"10.0.0.1\"\r\n",
        "IMEI: 123456789012345\r\n",
        "PIN=1234\r\n",
        "PIN: 5678\r\n",
        "PUK= 87654321\r\n",
    );

    let redacted = redact_at_text(input);
    assert!(!redacted.contains("3gnet"));
    assert!(!redacted.contains("secret.apn"));
    assert!(!redacted.contains("123456789012345"));
    assert!(!redacted.contains("PIN=1234"));
    assert!(!redacted.contains("5678"));
    assert!(!redacted.contains("87654321"));
    assert!(redacted.contains("[REDACTED_APN]"));
    assert!(redacted.contains("[REDACTED_IDENTIFIER]"));
    assert!(redacted.contains("PIN=[REDACTED]"));
    assert!(redacted.contains("PIN: [REDACTED]"));
    assert!(redacted.contains("PUK= [REDACTED]"));
}

#[test]
fn command_debug_output_does_not_disclose_an_apn() {
    let command = AtCommand::SetApn {
        cid: PdpContextId::try_from(1).expect("fixture CID is valid"),
        apn: Apn::try_from("private.apn").expect("fixture APN is valid"),
    };

    let debug = format!("{command:?}");
    assert!(!debug.contains("private.apn"));
    assert!(debug.contains("[REDACTED_APN]"));

    let encoded_debug = format!("{:?}", command.encode());
    assert!(!encoded_debug.contains("private.apn"));
    assert!(encoded_debug.contains("[REDACTED_APN]"));
}

#[test]
fn sms_transaction_lines_are_redacted_as_a_whole() {
    let pdu = "00040B912120550521F300004210203040502305E8329BFD06";
    for command in [
        AtCommand::SmsList,
        AtCommand::SmsRead { index: 1 },
        AtCommand::SmsDelete { index: 1 },
        AtCommand::SmsSetPduMode,
    ] {
        assert_eq!(
            redact_at_transaction_line(&command, "+CMGL: 1,1,,24"),
            "[REDACTED]",
            "command: {command:?}"
        );
        assert_eq!(
            redact_at_transaction_line(&command, pdu),
            "[REDACTED]",
            "command: {command:?}"
        );
    }

    // Configuration/capability queries keep the generic rules, not whole-line redaction.
    assert_eq!(
        redact_at_transaction_line(&AtCommand::SmsMessageFormat, "+CMGF: 1"),
        "+CMGF: 1"
    );
    assert_eq!(
        redact_at_transaction_line(&AtCommand::SmsStorageQuery, "+CPMS: \"SM\",3,20"),
        "+CPMS: \"SM\",3,20"
    );
    assert_eq!(
        redact_at_transaction_line(&AtCommand::Temperature, "+QTEMP: \"modem\",41"),
        "+QTEMP: \"modem\",41"
    );
}

#[test]
fn sms_response_debug_never_discloses_pdu_hex_or_sender() {
    use dji4g_at_protocol::{AtEvent, AtFinalCode, AtResponse};
    use dji4g_domain::DeviceEpoch;

    let response = AtResponse {
        epoch: DeviceEpoch(1),
        command: AtCommand::SmsRead { index: 1 },
        lines: vec![
            "+CMGR: 1,,24".to_owned(),
            "00040B912120550521F300004210203040502305E8329BFD06".to_owned(),
        ],
        final_code: AtFinalCode::Ok,
    };
    let debug = format!("{:?}", AtEvent::Response(response));
    assert!(!debug.contains("E8329BFD06"), "PDU leaked: {debug}");
    assert!(!debug.contains("+CMGR"), "header leaked: {debug}");
    assert!(debug.contains("[REDACTED]"));
}

#[test]
fn subscriber_identity_response_debug_never_discloses_numbers_or_iccid() {
    use dji4g_at_protocol::{AtEvent, AtFinalCode, AtResponse};
    use dji4g_domain::DeviceEpoch;

    let response = AtResponse {
        epoch: DeviceEpoch(1),
        command: AtCommand::SubscriberNumber,
        lines: vec![
            "+CNUM: ,\"123456789\",129".to_owned(),
            "+CNUM: \"line,1\",\"+12025550123\",145".to_owned(),
        ],
        final_code: AtFinalCode::Ok,
    };
    let debug = format!("{response:?}");
    assert!(!debug.contains("123456789"), "short number leaked: {debug}");
    assert!(!debug.contains("12025550123"), "number leaked: {debug}");
    assert!(debug.contains("[REDACTED]"));

    let iccid = AtResponse {
        epoch: DeviceEpoch(1),
        command: AtCommand::Iccid,
        lines: vec!["+QCCID: \"89860123456789012345\"".to_owned()],
        final_code: AtFinalCode::Ok,
    };
    let debug = format!("{:?}", AtEvent::Response(iccid));
    assert!(
        !debug.contains("89860123456789012345"),
        "ICCID leaked: {debug}"
    );
    assert!(debug.contains("[REDACTED]"));
}
