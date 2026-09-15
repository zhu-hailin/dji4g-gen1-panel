use dji4g_at_protocol::{Apn, AtCommand, PdpContextId, RetryPolicy, VerifiedUsbNetProfile};

fn cid(value: u8) -> PdpContextId {
    PdpContextId::try_from(value).expect("fixture CID is valid")
}

#[test]
fn every_modeled_command_has_exact_wire_bytes() {
    let apn = Apn::try_from("3gnet").expect("fixture APN is valid");
    let cases = [
        (AtCommand::Attention, b"AT\r".as_slice()),
        (AtCommand::Identity, b"ATI\r".as_slice()),
        (AtCommand::Manufacturer, b"AT+CGMI\r".as_slice()),
        (AtCommand::Model, b"AT+CGMM\r".as_slice()),
        (AtCommand::Revision, b"AT+CGMR\r".as_slice()),
        (AtCommand::SimState, b"AT+CPIN?\r".as_slice()),
        (AtCommand::SignalQuality, b"AT+CSQ\r".as_slice()),
        (AtCommand::Operator, b"AT+COPS?\r".as_slice()),
        (AtCommand::EpsRegistration, b"AT+CEREG?\r".as_slice()),
        (AtCommand::PacketAttach, b"AT+CGATT?\r".as_slice()),
        (AtCommand::PdpContexts, b"AT+CGDCONT?\r".as_slice()),
        (AtCommand::PdpActivation, b"AT+CGACT?\r".as_slice()),
        (AtCommand::PdpAddresses, b"AT+CGPADDR\r".as_slice()),
        (AtCommand::UsbNetQuery, b"AT+QCFG=\"usbnet\"\r".as_slice()),
        (AtCommand::ExtendedError, b"AT+CEER\r".as_slice()),
        (AtCommand::RestartModule, b"AT+CFUN=1,1\r".as_slice()),
        (
            AtCommand::SetApn { cid: cid(1), apn },
            b"AT+CGDCONT=1,\"IP\",\"3gnet\"\r".as_slice(),
        ),
        (
            AtCommand::SetUsbNetProfile(VerifiedUsbNetProfile::DjiNdis),
            b"AT+QCFG=\"usbnet\",0\r".as_slice(),
        ),
        (
            AtCommand::SetUsbNetProfile(VerifiedUsbNetProfile::Ecm),
            b"AT+QCFG=\"usbnet\",1\r".as_slice(),
        ),
    ];

    for (command, expected) in cases {
        let encoded = command.encode();
        assert_eq!(encoded.as_bytes(), expected, "command: {command:?}");
    }
}

#[test]
fn writes_are_identified_and_never_retried() {
    let apn = Apn::try_from("internet").expect("fixture APN is valid");
    let writes = [
        AtCommand::RestartModule,
        AtCommand::SetApn { cid: cid(3), apn },
        AtCommand::SetUsbNetProfile(VerifiedUsbNetProfile::DjiNdis),
        AtCommand::SetUsbNetProfile(VerifiedUsbNetProfile::Ecm),
    ];

    for command in writes {
        assert!(command.is_write(), "command: {command:?}");
        assert_eq!(command.retry_policy(), RetryPolicy::Never);
    }

    assert!(!AtCommand::SignalQuality.is_write());
    assert_eq!(
        AtCommand::SignalQuality.retry_policy(),
        RetryPolicy::OnceAfterQuietPeriod
    );
}

#[test]
fn apn_rejects_empty_non_ascii_delimiters_controls_and_oversize_input() {
    let invalid = [
        "", "bad\"apn", "bad,apn", "bad\rapn", "bad\napn", "bad\tapn", "bad\0apn", "bad;apn",
        "移动",
    ];

    for candidate in invalid {
        assert!(Apn::try_from(candidate).is_err(), "accepted {candidate:?}");
    }

    assert!(Apn::try_from("a".repeat(100)).is_ok());
    assert!(Apn::try_from("a".repeat(101)).is_err());
}

#[test]
fn apn_validation_errors_are_stable_nonlocalized_codes() {
    assert_eq!(
        Apn::try_from("").expect_err("empty APN").to_string(),
        "apn:empty"
    );
    assert_eq!(
        Apn::try_from("a".repeat(101))
            .expect_err("oversize APN")
            .to_string(),
        "apn:too_long"
    );
    assert_eq!(
        Apn::try_from("bad,apn")
            .expect_err("unsafe APN")
            .to_string(),
        "apn:unsafe_character"
    );
}

#[test]
fn pdp_context_id_accepts_only_the_v1_range() {
    assert!(PdpContextId::try_from(0).is_err());
    assert_eq!(PdpContextId::try_from(1).expect("lower bound").get(), 1);
    assert_eq!(PdpContextId::try_from(16).expect("upper bound").get(), 16);
    assert!(PdpContextId::try_from(17).is_err());
    assert!(PdpContextId::try_from(255).is_err());
}

#[test]
fn encoded_transactions_have_one_terminator_and_no_chaining_or_echo_configuration() {
    let apn = Apn::try_from("internet").expect("fixture APN is valid");
    let commands = [
        AtCommand::Attention,
        AtCommand::Identity,
        AtCommand::Manufacturer,
        AtCommand::Model,
        AtCommand::Revision,
        AtCommand::SimState,
        AtCommand::SignalQuality,
        AtCommand::Operator,
        AtCommand::EpsRegistration,
        AtCommand::PacketAttach,
        AtCommand::PdpContexts,
        AtCommand::PdpActivation,
        AtCommand::PdpAddresses,
        AtCommand::UsbNetQuery,
        AtCommand::ExtendedError,
        AtCommand::RestartModule,
        AtCommand::SetApn { cid: cid(1), apn },
        AtCommand::SetUsbNetProfile(VerifiedUsbNetProfile::DjiNdis),
        AtCommand::SetUsbNetProfile(VerifiedUsbNetProfile::Ecm),
    ];

    for command in commands {
        let encoded = command.encode();
        let bytes = encoded.as_bytes();
        assert_eq!(bytes.last(), Some(&b'\r'), "command: {command:?}");
        assert_eq!(bytes.iter().filter(|&&byte| byte == b'\r').count(), 1);
        assert!(!bytes.contains(&b'\n'));
        assert!(!bytes.contains(&b';'));
        assert!(!bytes.windows(4).any(|window| window == b"ATE0"));
    }
}
