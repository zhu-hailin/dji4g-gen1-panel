use dji4g_at_protocol::{
    Apn, AtCommand, AtEvent, AtFinalCode, PdpContextId, ProtocolErrorKind, StreamingParser,
    VerifiedUsbNetProfile, parse_serving_cell_line,
};
use dji4g_domain::{DeviceEpoch, ErrorCode};

const EPOCH: DeviceEpoch = DeviceEpoch(17);

fn fixture(name: &str) -> Vec<u8> {
    let text = match name {
        "echo_on_csq" => include_str!("../../../tests/fixtures/at/echo_on_csq.txt"),
        "echo_off_identity" => {
            include_str!("../../../tests/fixtures/at/echo_off_identity.txt")
        }
        "interleaved_cereg_urcs" => {
            include_str!("../../../tests/fixtures/at/interleaved_cereg_urcs.txt")
        }
        "interleaved_cereg_same_prefix" => {
            include_str!("../../../tests/fixtures/at/interleaved_cereg_same_prefix.txt")
        }
        "cme_error" => include_str!("../../../tests/fixtures/at/cme_error.txt"),
        other => panic!("unknown fixture {other}"),
    };
    text.replace('\n', "\r\n").into_bytes()
}

fn one_response(events: Vec<AtEvent>) -> dji4g_at_protocol::AtResponse {
    let mut responses = events.into_iter().filter_map(|event| match event {
        AtEvent::Response(response) => Some(response),
        AtEvent::Urc(_) | AtEvent::Prompt => None,
    });
    let response = responses.next().expect("one final response");
    assert!(responses.next().is_none());
    response
}

#[test]
fn parses_echo_on_and_fragmented_crlf_byte_by_byte() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SignalQuality);
    let mut events = Vec::new();
    for byte in fixture("echo_on_csq") {
        events.extend(parser.push(&[byte]).expect("fragment is valid"));
    }

    let response = one_response(events);
    assert_eq!(response.epoch, EPOCH);
    assert_eq!(response.command, AtCommand::SignalQuality);
    assert_eq!(response.lines, ["+CSQ: 26,99"]);
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn parses_echo_off_multiline_response() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Identity);
    let response = one_response(
        parser
            .push(&fixture("echo_off_identity"))
            .expect("fixture is valid"),
    );

    assert_eq!(
        response.lines,
        ["Quectel", "EC25", "Revision: EC25EFAR06A06M4G"]
    );
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn accepts_cr_lf_and_crlf_line_endings() {
    for bytes in [b"AT\rOK\r".as_slice(), b"AT\nOK\n", b"AT\r\nOK\r\n"] {
        let mut parser = StreamingParser::new(EPOCH, AtCommand::Attention);
        let response = one_response(parser.push(bytes).expect("line ending is valid"));
        assert!(response.lines.is_empty());
        assert_eq!(response.final_code, AtFinalCode::Ok);
    }
}

#[test]
fn keeps_matching_cereg_query_line_in_response_and_emits_known_urcs() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::EpsRegistration);
    let events = parser
        .push(&fixture("interleaved_cereg_urcs"))
        .expect("fixture is valid");

    let urcs: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AtEvent::Urc(urc) => Some(urc.line.as_str()),
            AtEvent::Response(_) | AtEvent::Prompt => None,
        })
        .collect();
    assert_eq!(urcs, ["+CMTI: \"SM\",1", "+CGEV: NW PDN ACT 1"]);

    let response = one_response(events);
    assert_eq!(response.lines, ["+CEREG: 0,1"]);
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn parses_cme_error_as_a_final_result() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::PacketAttach);
    let response = one_response(
        parser
            .push(&fixture("cme_error"))
            .expect("fixture is valid"),
    );
    assert!(response.lines.is_empty());
    assert_eq!(response.final_code, AtFinalCode::CmeError("30".into()));
}

#[test]
fn timeout_discards_an_incomplete_final_line_and_uses_domain_error_code() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SignalQuality);
    let events = parser
        .push(b"+CSQ: 26,99\r\nO")
        .expect("partial response is valid so far");
    assert!(events.is_empty());

    let error = parser.finish_timeout();
    assert_eq!(error.code, ErrorCode::Timeout);
    assert_eq!(error.kind, ProtocolErrorKind::Timeout);
    assert_eq!(error.to_string(), "at_protocol:timeout");
    assert!(!format!("{error:?}").contains("26,99"));
}

#[test]
fn rejects_nmea_binary_and_lines_over_4096_bytes_as_wrong_port_data() {
    let mut nmea = StreamingParser::new(EPOCH, AtCommand::Attention);
    let nmea_bytes = include_bytes!("../../../tests/fixtures/at/nmea_wrong_port.txt");
    assert_eq!(
        nmea.push(nmea_bytes)
            .expect_err("NMEA must fail closed")
            .kind,
        ProtocolErrorKind::WrongPortData
    );

    let binary_hex = include_str!("../../../tests/fixtures/at/binary_wrong_port.txt");
    let binary: Vec<u8> = binary_hex
        .split_ascii_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("fixture is hex"))
        .collect();
    let mut binary_parser = StreamingParser::new(EPOCH, AtCommand::Attention);
    assert_eq!(
        binary_parser
            .push(&binary)
            .expect_err("binary must fail closed")
            .kind,
        ProtocolErrorKind::WrongPortData
    );

    let mut overlong = StreamingParser::new(EPOCH, AtCommand::Attention);
    assert_eq!(
        overlong
            .push(&vec![b'A'; 4097])
            .expect_err("overlong line must fail closed")
            .kind,
        ProtocolErrorKind::LineTooLong
    );
}

#[test]
fn recognizes_all_modeled_final_result_codes() {
    let cases = [
        ("OK", AtFinalCode::Ok),
        ("ERROR", AtFinalCode::Error),
        (
            "+CME ERROR: operation not allowed",
            AtFinalCode::CmeError("operation not allowed".into()),
        ),
        ("+CMS ERROR: 500", AtFinalCode::CmsError("500".into())),
        ("NO CARRIER", AtFinalCode::NoCarrier),
        ("NO ANSWER", AtFinalCode::NoAnswer),
        ("BUSY", AtFinalCode::Busy),
        ("NO DIALTONE", AtFinalCode::NoDialTone),
    ];

    for (line, expected) in cases {
        let mut parser = StreamingParser::new(EPOCH, AtCommand::Attention);
        let response = one_response(
            parser
                .push(format!("{line}\r\n").as_bytes())
                .expect("final code is valid"),
        );
        assert_eq!(response.final_code, expected, "line: {line}");
    }
}

#[test]
fn recognizes_registration_and_exact_known_urcs_for_other_commands() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Attention);
    let events = parser
        .push(b"+CEREG: 5\r\n+CGREG: 1\r\n+CREG: 2\r\nRDY\r\nSMS READY\r\nOK\r\n")
        .expect("known URCs are valid");
    let urcs: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AtEvent::Urc(urc) => Some(urc.line.as_str()),
            AtEvent::Response(_) | AtEvent::Prompt => None,
        })
        .collect();
    assert_eq!(
        urcs,
        ["+CEREG: 5", "+CGREG: 1", "+CREG: 2", "RDY", "SMS READY"]
    );
    assert!(one_response(events).lines.is_empty());
}

#[test]
fn rejects_sustained_unrecognized_printable_wrong_port_data() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Attention);
    let noise = "unrecognized printable stream\r\n".repeat(33);
    assert_eq!(
        parser
            .push(noise.as_bytes())
            .expect_err("sustained unrecognized data must fail closed")
            .kind,
        ProtocolErrorKind::WrongPortData
    );
}

#[test]
fn deterministic_fragmentation_sweep_has_the_same_events() {
    let bytes = fixture("interleaved_cereg_urcs");
    let mut whole = StreamingParser::new(EPOCH, AtCommand::EpsRegistration);
    let expected = whole.push(&bytes).expect("whole fixture is valid");

    for chunk_size in 1..=bytes.len() {
        let mut parser = StreamingParser::new(EPOCH, AtCommand::EpsRegistration);
        let mut actual = Vec::new();
        for chunk in bytes.chunks(chunk_size) {
            actual.extend(parser.push(chunk).expect("chunked fixture is valid"));
        }
        assert_eq!(actual, expected, "chunk size {chunk_size}");
    }
}

#[test]
fn deterministic_byte_fuzz_never_panics_or_accepts_data_after_failure() {
    for seed in 0_u32..256 {
        let mut state = seed.wrapping_add(1);
        let mut bytes = [0_u8; 257];
        for byte in &mut bytes {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *byte = (state >> 24) as u8;
        }

        let mut parser = StreamingParser::new(DeviceEpoch(u64::from(seed)), AtCommand::Identity);
        let mut offset = 0;
        let mut failed = false;
        while offset < bytes.len() {
            let chunk_len = usize::from(bytes[offset] % 17) + 1;
            let end = (offset + chunk_len).min(bytes.len());
            if parser.push(&bytes[offset..end]).is_err() {
                failed = true;
                break;
            }
            offset = end;
        }
        if failed {
            assert!(parser.push(b"OK\r\n").is_err(), "seed {seed}");
        }
    }
}

#[test]
fn separates_status_only_cereg_urc_from_mode_and_status_query_response() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::EpsRegistration);
    let events = parser
        .push(&fixture("interleaved_cereg_same_prefix"))
        .expect("fixture is valid");

    let urcs: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AtEvent::Urc(urc) => Some(urc.line.as_str()),
            AtEvent::Response(_) | AtEvent::Prompt => None,
        })
        .collect();
    assert_eq!(urcs, ["+CEREG: 5"]);
    assert_eq!(one_response(events).lines, ["+CEREG: 0,1"]);
}

#[test]
fn fixed_commands_reject_arbitrary_printable_response_lines() {
    let apn = Apn::try_from("internet").expect("fixture APN is valid");
    let cid = PdpContextId::try_from(1).expect("fixture CID is valid");
    let commands = [
        AtCommand::Attention,
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
        AtCommand::SetApn { cid, apn },
        AtCommand::SetUsbNetProfile(VerifiedUsbNetProfile::DjiNdis),
    ];

    for command in commands {
        let mut parser = StreamingParser::new(EPOCH, command.clone());
        assert_eq!(
            parser
                .push(b"arbitrary printable garbage\r\nOK\r\n")
                .expect_err("fixed command must reject unmodeled lines")
                .kind,
            ProtocolErrorKind::WrongPortData,
            "command: {command:?}"
        );
    }
}

#[test]
fn structured_commands_accept_only_their_modeled_response_shapes() {
    let malformed = [
        (AtCommand::SimState, "+CPIN:"),
        (AtCommand::SignalQuality, "+CSQ: 26"),
        (AtCommand::Operator, "+COPS:"),
        (AtCommand::EpsRegistration, "+CEREG: x,y"),
        (AtCommand::PacketAttach, "+CGATT: 2"),
        (AtCommand::PdpContexts, "+CGDCONT: garbage"),
        (AtCommand::PdpActivation, "+CGACT: 1"),
        (AtCommand::PdpAddresses, "+CGPADDR: nope"),
        (AtCommand::UsbNetQuery, "+QCFG: \"other\",0"),
        (AtCommand::ExtendedError, "+CEER:"),
        (AtCommand::ServingCellInfo, "+QENG: garbage"),
        (AtCommand::ServingCellInfo, "+QENG: \"LTE\",1,460,01,351"),
        (
            AtCommand::ServingCellInfo,
            "+QENG: \"servingcell\",1,\"WCDMA\",460,01,351",
        ),
    ];

    for (command, line) in malformed {
        let mut parser = StreamingParser::new(EPOCH, command.clone());
        assert_eq!(
            parser
                .push(format!("{line}\r\nOK\r\n").as_bytes())
                .expect_err("malformed structured response must fail closed")
                .kind,
            ProtocolErrorKind::WrongPortData,
            "command: {command:?}, line: {line}"
        );
    }
}

#[test]
fn accepts_modeled_structured_response_shapes() {
    let cases = [
        (AtCommand::SimState, "+CPIN: READY"),
        (AtCommand::SignalQuality, "+CSQ: 26,99"),
        (AtCommand::Operator, "+COPS: 0,0,\"CHN-UNICOM\",7"),
        (AtCommand::EpsRegistration, "+CEREG: 0,1"),
        (AtCommand::PacketAttach, "+CGATT: 1"),
        (
            AtCommand::PdpContexts,
            "+CGDCONT: 1,\"IP\",\"3gnet\",\"10.0.0.1\",0,0",
        ),
        (AtCommand::PdpActivation, "+CGACT: 1,1"),
        (AtCommand::PdpAddresses, "+CGPADDR: 1,\"10.0.0.1\""),
        (AtCommand::UsbNetQuery, "+QCFG: \"usbnet\",0"),
        (AtCommand::ExtendedError, "+CEER: No report"),
        (
            AtCommand::ServingCellInfo,
            "+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,01,1A2B3C4,123,1650,3,5,5,0ABC,-95,-10,-65,15,20",
        ),
        (AtCommand::ServingCellInfo, "+QENG: \"NOCELL\""),
    ];

    for (command, line) in cases {
        let mut parser = StreamingParser::new(EPOCH, command.clone());
        let response = one_response(
            parser
                .push(format!("{line}\r\nOK\r\n").as_bytes())
                .expect("modeled response is valid"),
        );
        assert_eq!(response.lines, [line], "command: {command:?}");
    }
}

#[test]
fn structured_query_requires_a_response_line_before_ok() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SignalQuality);
    assert_eq!(
        parser
            .push(b"OK\r\n")
            .expect_err("successful query without data must fail closed")
            .kind,
        ProtocolErrorKind::WrongPortData
    );
}

#[test]
fn only_identity_and_version_commands_accept_bounded_free_form_lines() {
    for command in [
        AtCommand::Identity,
        AtCommand::Manufacturer,
        AtCommand::Model,
        AtCommand::Revision,
    ] {
        let mut parser = StreamingParser::new(EPOCH, command.clone());
        let response = one_response(
            parser
                .push(b"printable firmware text\r\nOK\r\n")
                .expect("free-form identity response is valid"),
        );
        assert_eq!(response.lines, ["printable firmware text"]);
    }
}

#[test]
fn unsupported_multipart_sms_urcs_fail_closed_even_for_free_form_commands() {
    for header in ["+CMT: \"sender\"", "+CDS: 23"] {
        let mut parser = StreamingParser::new(EPOCH, AtCommand::Identity);
        assert_eq!(
            parser
                .push(format!("{header}\r\npayload\r\nOK\r\n").as_bytes())
                .expect_err("multipart URC is not modeled")
                .kind,
            ProtocolErrorKind::WrongPortData,
            "header: {header}"
        );
    }
}

#[test]
fn device_removal_discards_a_partial_line_and_makes_failure_sticky() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SignalQuality);
    assert!(parser.push(b"+CSQ: 26").expect("partial line").is_empty());

    let error = parser.finish_removed();
    assert_eq!(error.code, ErrorCode::DeviceRemoved);
    assert_eq!(error.kind, ProtocolErrorKind::DeviceRemoved);
    assert_eq!(error.to_string(), "at_protocol:device_removed");
    assert!(!format!("{error:?}").contains("+CSQ"));
    assert_eq!(
        parser
            .push(b",99\r\nOK\r\n")
            .expect_err("removed parser must retain its terminal cause"),
        error
    );
    assert_eq!(parser.finish_timeout(), error);
    assert_eq!(parser.finish_removed(), error);
}

#[test]
fn device_removal_discards_complete_lines_from_an_unfinished_transaction() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::PdpContexts);
    assert!(
        parser
            .push(b"+CGDCONT: 1,\"IP\",\"private.apn\"\r\n")
            .expect("partial transaction")
            .is_empty()
    );

    let error = parser.finish_removed();
    assert_eq!(error.code, ErrorCode::DeviceRemoved);
    assert_eq!(error.kind, ProtocolErrorKind::DeviceRemoved);
    assert!(!format!("{error:?}").contains("private.apn"));
    assert_eq!(parser.push(b"OK\r\n").expect_err("sticky removal"), error);
}

#[test]
fn timeout_remains_the_terminal_cause_across_later_calls() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SignalQuality);
    assert!(parser.push(b"+CSQ: 26").expect("partial line").is_empty());

    let error = parser.finish_timeout();
    assert_eq!(error.code, ErrorCode::Timeout);
    assert_eq!(error.kind, ProtocolErrorKind::Timeout);
    assert_eq!(parser.push(b",99\r\n").expect_err("sticky timeout"), error);
    assert_eq!(parser.finish_removed(), error);
    assert_eq!(parser.finish_timeout(), error);
}

#[test]
fn ordinary_protocol_failure_remains_fail_closed_with_its_original_cause() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Attention);
    let error = parser
        .push(b"arbitrary printable garbage\r\n")
        .expect_err("fixed command rejects garbage");
    assert_eq!(error.code, ErrorCode::VerificationFailed);
    assert_eq!(error.kind, ProtocolErrorKind::WrongPortData);
    assert_eq!(parser.push(b"OK\r\n").expect_err("sticky failure"), error);
    assert_eq!(parser.finish_timeout(), error);
    assert_eq!(parser.finish_removed(), error);
}

#[test]
fn cnum_empty_ok_is_a_valid_empty_result() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SubscriberNumber);
    let response = one_response(
        parser
            .push(b"OK\r\n")
            .expect("an empty CNUM result is valid"),
    );
    assert!(response.lines.is_empty());
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn cnum_accepts_multiple_lines_before_ok() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SubscriberNumber);
    let response = one_response(
        parser
            .push(
                b"+CNUM: ,\"+12025550123\",145\r\n\
                  +CNUM: ,\"+12025550124\",145\r\n\
                  +CNUM: ,\"+12025550125\",145\r\n\
                  OK\r\n",
            )
            .expect("multi-line CNUM is valid"),
    );
    assert_eq!(response.lines.len(), 3);
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn cnum_quoted_label_with_comma_stays_one_response_line() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SubscriberNumber);
    let line = "+CNUM: \"line,1\",\"+12025550123\",145";
    let response = one_response(
        parser
            .push(format!("{line}\r\nOK\r\n").as_bytes())
            .expect("quoted-label CNUM is valid"),
    );
    assert_eq!(response.lines, [line]);
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn cnum_interleaved_cmti_urc_does_not_break_the_transaction_boundary() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SubscriberNumber);
    let events = parser
        .push(
            b"+CNUM: ,\"+12025550123\",145\r\n\
              +CMTI: \"SM\",1\r\n\
              +CNUM: ,\"+12025550124\",145\r\n\
              OK\r\n",
        )
        .expect("interleaved URC is valid");

    let urcs: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AtEvent::Urc(urc) => Some(urc.line.as_str()),
            AtEvent::Response(_) | AtEvent::Prompt => None,
        })
        .collect();
    assert_eq!(urcs, ["+CMTI: \"SM\",1"]);

    let response = one_response(events);
    assert_eq!(response.lines.len(), 2);
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn iccid_accepts_a_single_line_before_ok() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Iccid);
    let line = "+QCCID: \"89860123456789012345\"";
    let response = one_response(
        parser
            .push(format!("{line}\r\nOK\r\n").as_bytes())
            .expect("ICCID response is valid"),
    );
    assert_eq!(response.lines, [line]);
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn qeng_state_and_missing_value_shapes_stream_and_parse() {
    let cases = [
        (
            "+QENG: \"servingcell\",\"SEARCH\"",
            Some("SEARCH"),
            None,
            None,
        ),
        (
            "+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,01,1A2B3C4,123,1650,3,5,5,0ABC,-95,-10,-65,15,20",
            Some("NOCONN"),
            Some(0x1A2B3C4_u32),
            Some(-95),
        ),
        (
            "+QENG: \"servingcell\",\"NOCONN\",\"LTE\",\"FDD\",460,01,1A2B3C4,123,1650,3,5,5,0ABC,-,-,-,-,-",
            Some("NOCONN"),
            Some(0x1A2B3C4_u32),
            None,
        ),
    ];

    for (line, state, cell_id, rsrp_dbm) in cases {
        let mut parser = StreamingParser::new(EPOCH, AtCommand::ServingCellInfo);
        let response = one_response(
            parser
                .push(format!("{line}\r\nOK\r\n").as_bytes())
                .expect("modeled QENG shape is valid"),
        );
        assert_eq!(response.lines, [line]);
        let cell = parse_serving_cell_line(&response.lines[0]).expect("streamed line parses");
        assert_eq!(cell.state.as_deref(), state);
        assert_eq!(cell.cell_id, cell_id);
        assert_eq!(cell.rat.as_deref(), cell_id.map(|_| "LTE"));
        assert_eq!(cell.pci, cell_id.map(|_| 123));
        assert_eq!(cell.rsrp_dbm, rsrp_dbm);
    }
}

#[test]
fn cnum_more_than_sixteen_lines_fails_at_the_final_ok_gate() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SubscriberNumber);
    let bytes = "+CNUM: ,\"+12025550123\",145\r\n".repeat(17) + "OK\r\n";
    assert_eq!(
        parser
            .push(bytes.as_bytes())
            .expect_err("17 CNUM lines exceed the 16-line cap")
            .kind,
        ProtocolErrorKind::WrongPortData
    );
}

#[test]
fn iccid_non_digit_line_fails_closed_at_the_streaming_boundary() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Iccid);
    assert_eq!(
        parser
            .push(b"+QCCID: 8986-0123\r\nOK\r\n")
            .expect_err("non-digit ICCID must fail closed")
            .kind,
        ProtocolErrorKind::WrongPortData
    );
}

#[test]
fn qeng_old_16_field_layout_fails_closed_at_the_streaming_boundary() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::ServingCellInfo);
    let old = "+QENG: \"servingcell\",1,\"LTE\",1,460,01,351,1300,3,20,20,9365,-95,-8,-63,13";
    assert_eq!(
        parser
            .push(format!("{old}\r\nOK\r\n").as_bytes())
            .expect_err("old 16-field layout must fail closed")
            .kind,
        ProtocolErrorKind::WrongPortData
    );
}
