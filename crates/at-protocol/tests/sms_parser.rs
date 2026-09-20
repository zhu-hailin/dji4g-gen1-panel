use dji4g_at_protocol::{
    AtCommand, AtEvent, AtFinalCode, AtResponse, ProtocolErrorKind, SensorTemperature,
    StreamingParser, decode_deliver_pdu, parse_cmti_line, parse_qtemp_lines,
};
use dji4g_domain::{DeviceEpoch, SmsStorageId};

const EPOCH: DeviceEpoch = DeviceEpoch(23);

fn fixture(name: &str) -> Vec<u8> {
    let text = match name {
        "sms_cmgf" => include_str!("../../../tests/fixtures/at/sms_cmgf.txt"),
        "sms_cpms" => include_str!("../../../tests/fixtures/at/sms_cpms.txt"),
        "sms_cmgl_two_pdu" => include_str!("../../../tests/fixtures/at/sms_cmgl_two_pdu.txt"),
        "sms_cmgr_single_pdu" => include_str!("../../../tests/fixtures/at/sms_cmgr_single_pdu.txt"),
        "sms_cmti_interleaved" => {
            include_str!("../../../tests/fixtures/at/sms_cmti_interleaved.txt")
        }
        "sms_qtemp_multi" => include_str!("../../../tests/fixtures/at/sms_qtemp_multi.txt"),
        "qtemp_unnamed_multi" => {
            include_str!("../../../tests/fixtures/at/qtemp_unnamed_multi.txt")
        }
        "sms_cmgl_empty" => include_str!("../../../tests/fixtures/at/sms_cmgl_empty.txt"),
        "sms_cmgl_66_lines" => include_str!("../../../tests/fixtures/at/sms_cmgl_66_lines.txt"),
        other => panic!("unknown fixture {other}"),
    };
    text.replace('\n', "\r\n").into_bytes()
}

fn one_response(events: Vec<AtEvent>) -> AtResponse {
    let mut responses = events.into_iter().filter_map(|event| match event {
        AtEvent::Response(response) => Some(response),
        AtEvent::Urc(_) | AtEvent::Prompt => None,
    });
    let response = responses.next().expect("one final response");
    assert!(responses.next().is_none());
    response
}

#[test]
fn parses_cmgf_and_cpms_single_line_queries() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SmsMessageFormat);
    let response = one_response(
        parser
            .push(&fixture("sms_cmgf"))
            .expect("CMGF fixture is valid"),
    );
    assert_eq!(response.lines, ["+CMGF: 0"]);
    assert_eq!(response.final_code, AtFinalCode::Ok);

    let mut parser = StreamingParser::new(EPOCH, AtCommand::SmsStorageQuery);
    let response = one_response(
        parser
            .push(&fixture("sms_cpms"))
            .expect("CPMS fixture is valid"),
    );
    assert_eq!(
        response.lines,
        ["+CPMS: \"SM\",3,20,\"SM\",3,20,\"SM\",3,20"]
    );
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn parses_cmgl_two_pdu_fixture_and_decodes_both_messages() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SmsList);
    let response = one_response(
        parser
            .push(&fixture("sms_cmgl_two_pdu"))
            .expect("CMGL fixture is valid"),
    );
    assert_eq!(response.lines.len(), 4);
    assert!(response.lines[0].starts_with("+CMGL: 1,1,"));
    assert!(response.lines[2].starts_with("+CMGL: 2,0,"));

    let first = decode_deliver_pdu(&response.lines[1]).expect("first PDU decodes");
    assert_eq!(first.body, "hello");
    let second = decode_deliver_pdu(&response.lines[3]).expect("second PDU decodes");
    assert_eq!(second.body, "中");
}

#[test]
fn parses_cmgr_single_pdu_fixture() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SmsRead { index: 1 });
    let response = one_response(
        parser
            .push(&fixture("sms_cmgr_single_pdu"))
            .expect("CMGR fixture is valid"),
    );
    assert_eq!(response.lines.len(), 2);
    assert!(response.lines[0].starts_with("+CMGR: 1,"));
    assert_eq!(
        decode_deliver_pdu(&response.lines[1])
            .expect("PDU decodes")
            .body,
        "hello"
    );
}

#[test]
fn cmgl_empty_ok_is_a_valid_empty_list() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SmsList);
    let response = one_response(
        parser
            .push(&fixture("sms_cmgl_empty"))
            .expect("empty list is valid"),
    );
    assert!(response.lines.is_empty());
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn interleaved_cmti_stays_a_urc_during_sms_transactions() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SmsList);
    let events = parser
        .push(&fixture("sms_cmti_interleaved"))
        .expect("interleaved CMTI is valid");

    let urcs: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AtEvent::Urc(urc) => Some(urc.line.as_str()),
            AtEvent::Response(_) | AtEvent::Prompt => None,
        })
        .collect();
    assert_eq!(urcs, ["+CMTI: \"SM\",3"]);

    let response = one_response(events);
    assert_eq!(response.lines.len(), 2);
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn parses_qtemp_multiple_sensor_lines() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Temperature);
    let response = one_response(
        parser
            .push(&fixture("sms_qtemp_multi"))
            .expect("QTEMP fixture is valid"),
    );
    assert_eq!(response.lines.len(), 3);
    let lines: Vec<&str> = response.lines.iter().map(String::as_str).collect();
    assert_eq!(
        parse_qtemp_lines(&lines),
        [
            SensorTemperature {
                name: Some("modem".to_owned()),
                celsius: 41,
            },
            SensorTemperature {
                name: Some("pa".to_owned()),
                celsius: 39,
            },
            SensorTemperature {
                name: Some("wifi".to_owned()),
                celsius: -5,
            },
        ]
    );
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn parses_the_unnamed_positional_qtemp_layout() {
    // The DJI Gen-1 firmware answers `AT+QTEMP` with a bare positional list rather than the
    // `"name",value` pairs other modules use.  The values are read in report order and the
    // channels stay unnamed — the position is the only identity the device gave.
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Temperature);
    let response = one_response(
        parser
            .push(&fixture("qtemp_unnamed_multi"))
            .expect("the unnamed positional layout is a valid response"),
    );
    assert_eq!(response.lines, ["+QTEMP: 57,51,51"]);
    assert_eq!(response.final_code, AtFinalCode::Ok);
    let lines: Vec<&str> = response.lines.iter().map(String::as_str).collect();
    assert_eq!(
        parse_qtemp_lines(&lines),
        [
            SensorTemperature {
                name: None,
                celsius: 57,
            },
            SensorTemperature {
                name: None,
                celsius: 51,
            },
            SensorTemperature {
                name: None,
                celsius: 51,
            },
        ]
    );
}

#[test]
fn temperature_keeps_an_unmodelled_line_without_failing_the_transaction() {
    // A firmware-defined sensor read must not be decided by one line's shape: an extra unknown
    // line is collected (and shown to the user) while the readable `+QTEMP:` answer still lands.
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Temperature);
    let response = one_response(
        parser
            .push(b"+QTEMP: 57,51,51\r\n+QTEMP: \"modem\"\r\nsome vendor notice\r\nOK\r\n")
            .expect("an unmodelled line is tolerated, not fatal"),
    );
    assert_eq!(
        response.lines,
        [
            "+QTEMP: 57,51,51",
            "+QTEMP: \"modem\"",
            "some vendor notice"
        ]
    );
    let lines: Vec<&str> = response.lines.iter().map(String::as_str).collect();
    assert_eq!(
        parse_qtemp_lines(&lines),
        [
            SensorTemperature {
                name: None,
                celsius: 57,
            },
            SensorTemperature {
                name: None,
                celsius: 51,
            },
            SensorTemperature {
                name: None,
                celsius: 51,
            },
        ]
    );
}

#[test]
fn temperature_accepts_an_empty_and_a_long_sensor_list() {
    // Sensor count is firmware-defined: a plain OK is a clean no-data answer...
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Temperature);
    let response = one_response(
        parser
            .push(b"OK\r\n")
            .expect("an OK without sensor lines is a valid empty result"),
    );
    assert!(response.lines.is_empty());

    // ...and a long multi-channel report must not be cut off at an arbitrary count.
    let nine = "+QTEMP: \"s\",1\r\n".repeat(9) + "OK\r\n";
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Temperature);
    let response = one_response(
        parser
            .push(nine.as_bytes())
            .expect("nine sensor lines are a valid report"),
    );
    assert_eq!(response.lines.len(), 9);
}

#[test]
fn temperature_still_rejects_foreign_wire_data() {
    // Tolerance is about an unmodelled *answer*, never about the wrong port: binary noise and
    // NMEA sentences keep failing closed.
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Temperature);
    assert_eq!(
        parser
            .push(b"+QTEMP: 57,51,51\r\n\x01\x02\x03\r\nOK\r\n")
            .expect_err("binary noise must fail closed")
            .kind,
        ProtocolErrorKind::WrongPortData
    );

    let mut parser = StreamingParser::new(EPOCH, AtCommand::Temperature);
    assert_eq!(
        parser
            .push(b"$GPGGA,123519,4807.038,N\r\nOK\r\n")
            .expect_err("NMEA data must fail closed")
            .kind,
        ProtocolErrorKind::WrongPortData
    );
}

#[test]
fn sms_list_accepts_sixty_six_records_and_rejects_above_two_hundred() {
    // A full store commonly holds dozens of messages; 66 records must parse (the cap is 200).
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SmsList);
    let response = one_response(
        parser
            .push(&fixture("sms_cmgl_66_lines"))
            .expect("66 message records are within the list cap"),
    );
    assert_eq!(response.final_code, AtFinalCode::Ok);

    // Above the cap the list fails closed instead of silently truncating.
    let mut over = String::new();
    for index in 0..201 {
        over.push_str(&format!(
            "+CMGL: {index},1,,,\r\n0011000B912120550521F30008024E2D\r\n"
        ));
    }
    over.push_str("OK\r\n");
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SmsList);
    assert_eq!(
        parser
            .push(over.as_bytes())
            .expect_err("more than 200 records exceed the list cap")
            .kind,
        ProtocolErrorKind::WrongPortData
    );
}

#[test]
fn rejects_malformed_sms_response_shapes() {
    let cases = [
        (AtCommand::SmsMessageFormat, "+CMGF: 2"),
        (AtCommand::SmsStorageQuery, "+CPMS: \"SM\""),
        (AtCommand::SmsList, "+CMGL: 1,9,,24"),
        (AtCommand::SmsList, "+CMGL: 10000,1,,24"),
        (AtCommand::SmsList, "+CMGL: x,1,,24"),
        (AtCommand::SmsRead { index: 1 }, "+CMGR: 5,,24"),
    ];

    for (command, line) in cases {
        let mut parser = StreamingParser::new(EPOCH, command.clone());
        assert_eq!(
            parser
                .push(format!("{line}\r\nOK\r\n").as_bytes())
                .expect_err("malformed SMS shape must fail closed")
                .kind,
            ProtocolErrorKind::WrongPortData,
            "command: {command:?}, line: {line}"
        );
    }
}

#[test]
fn unmodelled_temperature_lines_complete_without_readings() {
    // An unreadable QTEMP line no longer aborts the transaction — the streaming layer only decides
    // attribution. Interpreting it yields no reading, which the probe reports as a format
    // mismatch and the UI shows together with the raw line.
    for line in ["+QTEMP: \"modem\"", "+QTEMP: modem,41", "+QTEMP: 57,x"] {
        let mut parser = StreamingParser::new(EPOCH, AtCommand::Temperature);
        let response = one_response(
            parser
                .push(format!("{line}\r\nOK\r\n").as_bytes())
                .expect("an unmodelled line is collected, not fatal"),
        );
        assert_eq!(response.lines, [line]);
        assert_eq!(response.final_code, AtFinalCode::Ok);
        let lines: Vec<&str> = response.lines.iter().map(String::as_str).collect();
        assert!(
            parse_qtemp_lines(&lines).is_empty(),
            "line: {line} carries no reading"
        );
    }
}

#[test]
fn pdu_continuation_after_a_header_must_be_hex() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::SmsList);
    assert_eq!(
        parser
            .push(b"+CMGL: 1,1,,24\r\nnot-hex\r\nOK\r\n")
            .expect_err("non-hex continuation must fail")
            .kind,
        ProtocolErrorKind::WrongPortData
    );

    let mut bare = StreamingParser::new(EPOCH, AtCommand::SmsList);
    assert_eq!(
        bare.push(b"00040B912120550521F300004210203040502305E8329BFD06\r\nOK\r\n")
            .expect_err("a PDU without a CMGL header must fail")
            .kind,
        ProtocolErrorKind::WrongPortData
    );
}

#[test]
fn parse_cmti_line_accepts_only_well_formed_storage_prompts() {
    assert_eq!(
        parse_cmti_line("+CMTI: \"SM\",3"),
        Some((SmsStorageId("SM".to_owned()), 3))
    );
    assert_eq!(
        parse_cmti_line("+CMTI: \"ME\",9999"),
        Some((SmsStorageId("ME".to_owned()), 9999))
    );

    for line in [
        "+CMTI:",
        "+CMTI: \"SM\"",
        "+CMTI: SM,3",
        "+CMTI: \"\",3",
        "+CMTI: \"SM\",10000",
        "+CMTI: \"SM\",x",
        "+CMGL: 1,1,,24",
    ] {
        assert_eq!(parse_cmti_line(line), None, "line: {line}");
    }
}

#[test]
fn parse_qtemp_lines_skips_malformed_lines_without_panicking() {
    let lines = [
        "+QTEMP: \"modem\",42",
        "garbage",
        "+QTEMP: \"bad\"",
        "+QTEMP: \"pa\",-10",
        "+QTEMP: modem,1",
        "+QTEMP: \"sensor with space\",0",
        "+QTEMP: 57,51,51",
        "+QTEMP: 57,x",
        "+QTEMP: 1,\"modem\",2",
    ];
    assert_eq!(
        parse_qtemp_lines(&lines),
        [
            SensorTemperature {
                name: Some("modem".to_owned()),
                celsius: 42,
            },
            SensorTemperature {
                name: Some("pa".to_owned()),
                celsius: -10,
            },
            SensorTemperature {
                name: Some("sensor with space".to_owned()),
                celsius: 0,
            },
            SensorTemperature {
                name: None,
                celsius: 57,
            },
            SensorTemperature {
                name: None,
                celsius: 51,
            },
            SensorTemperature {
                name: None,
                celsius: 51,
            },
        ]
    );
}

#[test]
fn prompt_is_emitted_then_parsing_continues_to_the_final_code() {
    let mut parser = StreamingParser::new_with_prompt(EPOCH, AtCommand::Identity);
    let events = parser
        .push(b"ATI\r\n>\r\n0001000B912120550521F30008024E2D\r\nOK\r\n")
        .expect("prompt sequence is valid");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], AtEvent::Prompt);
    let AtEvent::Response(response) = &events[1] else {
        panic!("expected the final response");
    };
    assert_eq!(response.lines, ["0001000B912120550521F30008024E2D"]);
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn prompt_is_emitted_at_a_chunk_boundary_without_a_terminator() {
    let mut parser = StreamingParser::new_with_prompt(EPOCH, AtCommand::Identity);
    assert!(parser.push(b"ATI\r\n").expect("echo is valid").is_empty());
    assert_eq!(
        parser.push(b">").expect("prompt chunk is valid"),
        [AtEvent::Prompt]
    );
    let response = one_response(
        parser
            .push(b"\r\n+CMGS: 7\r\nOK\r\n")
            .expect("the result after the body is valid"),
    );
    assert_eq!(response.lines, ["+CMGS: 7"]);
    assert_eq!(response.final_code, AtFinalCode::Ok);
}

#[test]
fn prompt_tolerates_trailing_whitespace_and_survives_a_timeout() {
    let mut parser = StreamingParser::new_with_prompt(EPOCH, AtCommand::Identity);
    let events = parser
        .push(b"ATI\r\n> \r\n+CMGF: 0\r\nOK\r\n")
        .expect("a trailing space after the prompt is valid");
    assert_eq!(events[0], AtEvent::Prompt);
    assert_eq!(one_response(events).lines, ["+CMGF: 0"]);

    let mut parser = StreamingParser::new_with_prompt(EPOCH, AtCommand::Identity);
    assert_eq!(
        parser.push(b"ATI\r\n>\r\n").expect("prompt is valid"),
        [AtEvent::Prompt]
    );
    assert_eq!(parser.finish_timeout().kind, ProtocolErrorKind::Timeout);

    let mut parser = StreamingParser::new_with_prompt(EPOCH, AtCommand::Identity);
    assert!(parser.push(b"ATI\r\nfree-form\r\nOK\r\n").is_ok());
    assert_eq!(
        parser
            .push(b">")
            .expect_err("data after completion stays an error")
            .kind,
        ProtocolErrorKind::UnexpectedData
    );
}

#[test]
fn a_prompt_line_without_prompt_mode_still_fails_closed() {
    let mut parser = StreamingParser::new(EPOCH, AtCommand::Attention);
    assert_eq!(
        parser
            .push(b"AT\r\n>\r\nOK\r\n")
            .expect_err("prompt line is not a response shape")
            .kind,
        ProtocolErrorKind::WrongPortData
    );
}
