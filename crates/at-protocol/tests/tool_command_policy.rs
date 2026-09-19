//! Behaviour tests for the device-tool command policy.
//!
//! These are the security-relevant decisions of the terminal: what may run without asking the
//! user again, what is refused outright, and what stays unrecognised so the expert path has to
//! confirm it. Every refusal is asserted on the exact error variant, so a future widening of the
//! policy fails here rather than at a module.

use dji4g_at_protocol::{
    AtCommand, PdpContextId, ToolInputError, ToolReadId, ToolWriteId, ValidatedToolLine,
    VerifiedUsbNetProfile, classify_known_write, classify_read, typed_read,
};

fn line(input: &str) -> ValidatedToolLine {
    ValidatedToolLine::parse(input).expect("expected a valid tool line")
}

fn error(input: &str) -> ToolInputError {
    ValidatedToolLine::parse(input).expect_err("expected the input to be refused")
}

#[test]
fn single_character_input_is_refused_without_panicking() {
    for input in ["A", "x", " A "] {
        assert_eq!(error(input), ToolInputError::InvalidPrefix);
    }
}

#[test]
fn whitelist_matches_the_typed_encoder() {
    for id in ToolReadId::ALL {
        let encoded = typed_read(id).encode();
        let text = String::from_utf8_lossy(encoded.as_bytes());
        let text = text.trim_end_matches('\r');
        let parsed = ValidatedToolLine::parse(text)
            .unwrap_or_else(|error| panic!("{id:?} encodes to an unparsable line: {error}"));
        assert_eq!(
            classify_read(&parsed),
            Some(id),
            "{id:?} encodes to {text:?} which is not whitelisted"
        );
    }
}

#[test]
fn every_whitelisted_read_round_trips_through_its_own_id() {
    for id in ToolReadId::ALL {
        let typed = format!("{:?}", typed_read(id));
        assert!(!typed.is_empty());
        let encoded = typed_read(id).encode();
        // The encoded form is what the actor writes; it must be exactly the whitelisted text.
        let text = String::from_utf8_lossy(encoded.as_bytes()).to_string();
        assert!(text.ends_with('\r'), "{id:?} is not CR terminated");
        assert_eq!(text.matches('\r').count(), 1, "{id:?} has more than one CR");
    }
}

#[test]
fn question_mark_does_not_grant_read_access() {
    let vendor = line("AT+VENDOR?");
    assert!(classify_read(&vendor).is_none());
    // A well-formed query without a whitelist entry is refused by the advanced path...
    assert_eq!(
        ValidatedToolLine::parse_read_only("AT+VENDOR?").unwrap_err(),
        ToolInputError::NotWhitelisted
    );
    // ...but the same text stays available to the expert path, which must confirm it explicitly.
    assert_eq!(
        ValidatedToolLine::parse_read_only("AT+CSQ").unwrap().1,
        ToolReadId::SignalQuality
    );
}

#[test]
fn reads_are_recognised_in_any_keyword_case_but_parameters_stay_exact() {
    for input in ["AT+CSQ", "at+csq", "At+Csq"] {
        assert_eq!(classify_read(&line(input)), Some(ToolReadId::SignalQuality));
    }
    // AT keywords are case-insensitive, the quoted parameter is not: the tool must not normalise
    // user or vendor text into a different request.
    for input in ["AT+QCFG=\"usbnet\"", "at+qcfg=\"usbnet\""] {
        assert_eq!(classify_read(&line(input)), Some(ToolReadId::UsbNet));
    }
    assert!(classify_read(&line("AT+QCFG=\"USBNET\"")).is_none());
    assert!(classify_read(&line("AT+QCFG=\"usbnet2\"")).is_none());
}

#[test]
fn parameterised_variants_of_a_whitelisted_read_are_not_reads() {
    for input in [
        "AT+CSQ=?",
        "AT+CGDCONT=1,\"IP\",\"example\"",
        "AT+QCFG=\"usbnet\",0",
        "AT+CPMS=\"SM\"",
        "AT+CGATT=1",
        "AT+QENG=\"neighbourcell\"",
    ] {
        let parsed = line(input);
        assert!(
            classify_read(&parsed).is_none(),
            "{input} must not be treated as a read"
        );
    }
}

#[test]
fn classifier_reads_the_command_word_not_a_string_prefix() {
    // The word boundary matters: a command whose name merely starts with a rejected word or a
    // whitelisted word must not inherit its treatment.
    let parsed = line("AT+CSQ2");
    assert!(classify_read(&parsed).is_none());
    let parsed = line("AT+CGDCONT2?");
    assert!(classify_read(&parsed).is_none());
    // `AT+CMGS` is interactive; `AT+CMGSS` is not that family.
    assert!(ValidatedToolLine::parse("AT+CMGSS=1").is_ok());
}

#[test]
fn rejects_injected_second_command() {
    for value in [
        "AT+CSQ\rAT+CFUN=1,1",
        "AT+CSQ\n",
        "AT;AT+CSQ",
        "AT+CSQ\r",
        "AT+CSQ\r\n",
    ] {
        let error = ValidatedToolLine::parse(value).expect_err(value);
        assert!(
            matches!(
                error,
                ToolInputError::ControlCharacter | ToolInputError::ChainedCommand
            ),
            "{value:?} produced {error:?}"
        );
    }
}

#[test]
fn rejects_control_characters_and_non_ascii_before_anything_else() {
    assert_eq!(error("AT+CSQ\u{0}"), ToolInputError::ControlCharacter);
    assert_eq!(error("AT+CSQ\u{1a}"), ToolInputError::ControlCharacter);
    assert_eq!(error("AT+CSQ\u{7f}"), ToolInputError::ControlCharacter);
    assert_eq!(error("AT+CSQ\u{4e2d}"), ToolInputError::NonAscii);
    // A non-ASCII byte is refused even when the line would otherwise be too long.
    let mut long = String::from("AT+CSQ");
    long.push_str(&"A".repeat(300));
    long.push('中');
    assert_eq!(error(&long), ToolInputError::NonAscii);
}

#[test]
fn rejects_empty_over_long_and_wrong_prefix() {
    assert_eq!(error(""), ToolInputError::Empty);
    assert_eq!(error("   "), ToolInputError::Empty);
    assert_eq!(error("+++"), ToolInputError::InvalidPrefix);
    let mut long = String::from("AT+CSQ");
    long.push_str(&"A".repeat(251));
    assert_eq!(long.len(), 257);
    assert_eq!(error(&long), ToolInputError::TooLong);
    // Exactly 256 bytes is still accepted.
    let mut exact = String::from("AT+CSQ");
    exact.push_str(&"A".repeat(250));
    assert_eq!(exact.len(), 256);
    assert!(ValidatedToolLine::parse(&exact).is_ok());
    assert_eq!(error("CSQ"), ToolInputError::InvalidPrefix);
    // Ordinary spaces around the line are trimmed before the prefix rule, so they are accepted.
    assert_eq!(
        classify_read(&line("  AT+CSQ  ")),
        Some(ToolReadId::SignalQuality)
    );
}

#[test]
fn attention_is_a_whitelisted_read() {
    assert_eq!(classify_read(&line("AT")), Some(ToolReadId::Attention));
    assert_eq!(classify_read(&line("at")), Some(ToolReadId::Attention));
    assert_eq!(classify_read(&line("  AT  ")), Some(ToolReadId::Attention));
}

#[test]
fn interactive_families_are_refused_on_both_paths() {
    for value in [
        "AT+CMGS=12",
        "AT+CMGS=\"13800138000\"",
        "AT+CMGW=12",
        "AT+CMGC=1",
        "at+cmgs=?",
        "ATD13800138000",
        "ATD+8613800138000",
        "ATO",
        "ATO0",
        "ATO1",
        "ATD",
        "ATDL",
        "ATDT1234",
        "ATA",
        "AT+QFUPL=\"file\",100",
        "AT+QFOPEN=\"file\",2",
        "AT+QFWRITE=1,10",
        "AT+QFDOWNLOAD=1",
    ] {
        let error = ValidatedToolLine::parse(value).expect_err(value);
        assert_eq!(
            error,
            ToolInputError::InteractiveCommand,
            "{value} should be refused as interactive"
        );
    }
}

#[test]
fn interactive_detection_leaves_unrelated_commands_alone() {
    // A command that merely begins with the same letter is not the dial family.
    for value in ["AT+CSQ", "AT+CGMI", "AT+CEREG?", "AT+QTEMP"] {
        assert!(
            ValidatedToolLine::parse(value).is_ok(),
            "{value} must not be refused"
        );
    }
}

#[test]
fn known_writes_are_recognised_with_their_real_parameters() {
    match classify_known_write(&line("AT+CFUN=1,1")) {
        Some(ToolWriteId::RestartModule) => {}
        other => panic!("expected a restart request, got {other:?}"),
    }
    match classify_known_write(&line("AT+QCFG=\"usbnet\",0")) {
        Some(ToolWriteId::SetUsbNetProfile(VerifiedUsbNetProfile::DjiNdis)) => {}
        other => panic!("expected the DJI/NDIS profile, got {other:?}"),
    }
    match classify_known_write(&line("AT+QCFG=\"usbnet\",1")) {
        Some(ToolWriteId::SetUsbNetProfile(VerifiedUsbNetProfile::Ecm)) => {}
        other => panic!("expected the ECM profile, got {other:?}"),
    }
    match classify_known_write(&line("AT+CGDCONT=1,\"IP\",\"MiXeD\"")) {
        Some(ToolWriteId::SetApn { cid, apn }) => {
            assert_eq!(cid, PdpContextId::try_from(1).unwrap());
            // The APN is user data: its letter case must survive the classifier untouched.
            assert_eq!(apn.as_str(), "MiXeD");
        }
        other => panic!("expected an APN write, got {other:?}"),
    }
    assert_eq!(classify_known_write(&line("AT+CSQ")), None);
}

#[test]
fn known_write_classification_is_as_strict_as_the_typed_requests() {
    // Out-of-range CID, unverified usbnet value, missing quotes and an empty APN are not known
    // writes; they fall through to the expert path's own confirmation.
    for value in [
        "AT+CGDCONT=0,\"IP\",\"example\"",
        "AT+CGDCONT=99,\"IP\",\"example\"",
        "AT+CGDCONT=1,\"IP\"",
        "AT+CGDCONT=1,IP,example",
        "AT+CGDCONT=1,\"IP\",\"exa,mple\"",
        "AT+QCFG=\"usbnet\",2",
        "AT+QCFG=\"usbnet\",9",
        "AT+CFUN=1",
        "AT+CFUN=0",
        "AT+CGDCONT=1,\"IP\",\"\"",
    ] {
        assert!(
            classify_known_write(&line(value)).is_none(),
            "{value} must not be classified as a known write"
        );
    }
}

#[test]
fn known_apn_write_matches_the_typed_command_encoding() {
    // The classifier must recognise exactly the text the typed command produces, so a repair that
    // the user confirmed runs the same command the confirmation described.
    let cid = PdpContextId::try_from(3).unwrap();
    let apn = dji4g_at_protocol::Apn::try_from("MiXeD").unwrap();
    let encoded = AtCommand::SetApn {
        cid,
        apn: apn.clone(),
    }
    .encode();
    let text = String::from_utf8_lossy(encoded.as_bytes())
        .trim_end_matches('\r')
        .to_owned();
    match classify_known_write(&line(&text)) {
        Some(ToolWriteId::SetApn {
            cid: cid2,
            apn: apn2,
        }) => {
            assert_eq!(cid, cid2);
            assert_eq!(apn.as_str(), apn2.as_str());
        }
        other => panic!("typed APN command {text} classified as {other:?}"),
    }
}

#[test]
fn known_parameter_types_are_normalised_but_not_invented() {
    // The PDP type is the module's vocabulary; the APN is not.
    for value in [
        "AT+CGDCONT=2,\"ip\",\"example.org\"",
        "AT+CGDCONT=2,\"IPv6\",\"example.org\"",
    ] {
        assert!(matches!(
            classify_known_write(&line(value)),
            Some(ToolWriteId::SetApn { .. })
        ));
    }
    assert!(classify_known_write(&line("AT+CGDCONT=2,\"PPP\",\"example.org\"")).is_none());
}

#[test]
fn a_validated_line_never_reveals_its_text_through_debug() {
    let parsed = line("AT+CGDCONT=1,\"IP\",\"secret.example\"");
    let formatted = format!("{parsed:?}");
    assert!(!formatted.contains("secret"), "{formatted}");
    assert!(formatted.contains("REDACTED"));
    // The confirmation path is the only reader.
    assert_eq!(
        parsed.expose_for_confirmation(),
        "AT+CGDCONT=1,\"IP\",\"secret.example\""
    );
}

#[test]
fn input_errors_carry_no_input_text() {
    let secret = "AT+CSQ;SECRET-SENTINEL";
    let error = ValidatedToolLine::parse(secret).unwrap_err();
    assert!(!error.code().contains("SECRET"));
    assert!(!format!("{error:?}").contains("SECRET"));
    assert!(!error.to_string().contains("SECRET"));
}
