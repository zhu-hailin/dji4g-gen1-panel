//! Behaviour tests for the tool transaction response parser.
//!
//! The parser is the boundary that must not turn "the module said ERROR" into "the tool failed to
//! talk to the module", must not count an unrelated URC as this command's answer, and must stop a
//! transaction that has turned into an interaction.

use dji4g_at_protocol::{
    AtFinalCode, ToolParseError, ToolReadId, ToolResponse, ToolResponseParser, ToolWireRequest,
    ValidatedToolLine, classify_read,
};

fn expert(text: &str) -> ToolWireRequest {
    ToolWireRequest::from_expert(ValidatedToolLine::parse(text).expect("valid line"))
}

fn push_all(parser: &mut ToolResponseParser, chunks: &[&[u8]]) -> Option<ToolResponse> {
    for chunk in chunks {
        if let Some(response) = parser.push(chunk).expect("parser accepted the bytes") {
            return Some(response);
        }
    }
    None
}

#[test]
fn a_whitelisted_read_carries_its_echo_and_its_answer() {
    let request = ToolWireRequest::from_read(ToolReadId::SignalQuality);
    let mut parser = ToolResponseParser::new(&request);
    let response = push_all(
        &mut parser,
        &[b"AT+CSQ\r\r\n", b"+CSQ: 21,0\r\n", b"\r\n", b"OK\r\n"],
    )
    .expect("a final code arrives");
    assert_eq!(response.final_code, AtFinalCode::Ok);
    assert_eq!(response.lines, vec!["+CSQ: 21,0".to_owned()]);
    assert!(response.urc_lines.is_empty());
    // The line carries the command's own response prefix, so it is attributed to this command.
    assert_eq!(response.unclassified_lines, 0);
}

#[test]
fn the_wire_form_is_the_line_plus_exactly_one_carriage_return() {
    let request = ToolWireRequest::from_read(ToolReadId::UsbNet);
    let bytes = request.wire_bytes();
    assert_eq!(bytes, b"AT+QCFG=\"usbnet\"\r");
    assert_eq!(bytes.iter().filter(|byte| **byte == b'\r').count(), 1);
    assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 0);
    // The expert path cannot append a terminator of its own: the same rule applies.
    let request = expert("AT+VENDOR?");
    assert_eq!(request.wire_bytes(), b"AT+VENDOR?\r");
}

#[test]
fn fragments_are_reassembled_across_chunks() {
    let request = ToolWireRequest::from_read(ToolReadId::Model);
    let mut parser = ToolResponseParser::new(&request);
    let response = push_all(
        &mut parser,
        &[
            b"AT+CGMM\r",
            b"\n+CGMM: Sim",
            b"ulated-Module\r",
            b"\nO",
            b"K\r\n",
        ],
    )
    .expect("a final code arrives");
    assert_eq!(response.final_code, AtFinalCode::Ok);
    assert_eq!(response.lines, vec!["+CGMM: Simulated-Module".to_owned()]);
}

#[test]
fn an_error_final_code_is_a_response_not_a_transport_failure() {
    for (line, expected) in [
        ("ERROR", AtFinalCode::Error),
        ("+CME ERROR: 10", AtFinalCode::CmeError("10".to_owned())),
        ("+CMS ERROR: 500", AtFinalCode::CmsError("500".to_owned())),
        ("NO CARRIER", AtFinalCode::NoCarrier),
    ] {
        let request = expert("AT+VENDOR?");
        let mut parser = ToolResponseParser::new(&request);
        let mut chunk = Vec::new();
        chunk.extend_from_slice(b"AT+VENDOR?\r\n");
        chunk.extend_from_slice(b"+VENDOR: 1\r\n");
        chunk.extend_from_slice(line.as_bytes());
        chunk.extend_from_slice(b"\r\n");
        let response = push_all(&mut parser, &[&chunk]).unwrap_or_else(|| panic!("{line}"));
        assert_eq!(response.final_code, expected, "{line}");
        // The module's own explanation is preserved for the caller instead of being dropped.
        assert_eq!(response.lines, vec!["+VENDOR: 1".to_owned()]);
    }
}

#[test]
fn an_ok_without_content_is_still_a_complete_response() {
    let request = ToolWireRequest::from_read(ToolReadId::Attention);
    let mut parser = ToolResponseParser::new(&request);
    let response = push_all(&mut parser, &[b"AT\r\n", b"\r\n", b"OK\r\n"]).expect("final code");
    assert_eq!(response.final_code, AtFinalCode::Ok);
    assert!(response.lines.is_empty());
}

#[test]
fn urcs_are_kept_apart_and_never_count_as_the_answer() {
    let request = ToolWireRequest::from_read(ToolReadId::SignalQuality);
    let mut parser = ToolResponseParser::new(&request);
    let response = push_all(
        &mut parser,
        &[
            b"AT+CSQ\r\r\n",
            b"+CMTI: \"SM\",4\r\n",
            b"+CSQ: 18,99\r\n",
            b"RING\r\n",
            b"OK\r\n",
        ],
    )
    .expect("final code");
    assert_eq!(response.lines, vec!["+CSQ: 18,99".to_owned()]);
    assert_eq!(
        response.urc_lines,
        vec!["+CMTI: \"SM\",4".to_owned(), "RING".to_owned()]
    );
    assert!(
        response
            .urc_lines
            .iter()
            .all(|line| !line.starts_with("+CSQ"))
    );
}

#[test]
fn text_without_the_expected_prefix_is_collected_but_marked_unattributable() {
    let request = ToolWireRequest::from_read(ToolReadId::Model);
    let mut parser = ToolResponseParser::new(&request);
    let response =
        push_all(&mut parser, &[b"AT+CGMM\r\n", b"Quectel\r\n", b"OK\r\n"]).expect("final code");
    assert_eq!(response.lines, vec!["Quectel".to_owned()]);
    // We cannot prove that line answers this command, and the parser says so instead of guessing.
    assert_eq!(response.unclassified_lines, 1);
    assert_eq!(response.final_code_tag(), "ok");
}

#[test]
fn a_prompt_stops_the_transaction_without_continuing_the_interaction() {
    let request = expert("AT+VENDOR=1");
    let mut parser = ToolResponseParser::new(&request);
    // The echo is not a prompt.
    assert_eq!(parser.push(b"AT+VENDOR=1\r\n").expect("echo"), None);
    let error = parser
        .push(b"> ")
        .expect_err("a prompt must end the transaction");
    assert_eq!(error, ToolParseError::UnsupportedInteraction);
    // Nothing further is accepted from this transaction: no body, no Ctrl-Z, no resume.
    assert_eq!(
        parser.push(b"payload").unwrap_err(),
        ToolParseError::UnexpectedData
    );
}

#[test]
fn a_prompt_splits_across_chunks_and_ends_the_transaction_once() {
    let request = expert("AT+VENDOR=1");
    let mut parser = ToolResponseParser::new(&request);
    assert_eq!(parser.push(b"AT+VENDOR=1\r\n").expect("echo"), None);
    // The `>` arrives at the end of a chunk with no newline: it is a prompt, not a data line.
    assert_eq!(
        parser.push(b"\r\n>").expect_err("prompt"),
        ToolParseError::UnsupportedInteraction
    );
    // The transaction is closed, so a later `OK` cannot be mistaken for this command's result.
    assert_eq!(
        parser.push(b"OK\r\n").unwrap_err(),
        ToolParseError::UnexpectedData
    );
}

#[test]
fn a_connect_data_state_stops_the_transaction() {
    let request = expert("AT+VENDOR=1");
    let mut parser = ToolResponseParser::new(&request);
    let error = parser
        .push(b"AT+VENDOR=1\r\nCONNECT 115200\r\n")
        .expect_err("data mode");
    assert_eq!(error, ToolParseError::UnsupportedInteraction);
}

#[test]
fn over_long_lines_and_over_sized_responses_are_refused() {
    let request = expert("AT+VENDOR?");
    let mut parser = ToolResponseParser::new(&request);
    let mut line = vec![b'X'; 5000];
    line.extend_from_slice(b"\r\n");
    assert_eq!(
        parser.push(&line).expect_err("over-long line"),
        ToolParseError::LineTooLong
    );

    // Many individually legal lines still hit the total cap, and the cap is reported as a failure
    // rather than being silently truncated into a successful small response.
    let request = expert("AT+VENDOR?");
    let mut parser = ToolResponseParser::new(&request);
    let mut chunk = vec![b'Y'; 4095];
    chunk.extend_from_slice(b"\r\n");
    let mut error = None;
    for _ in 0..20 {
        match parser.push(&chunk) {
            Ok(_) => {}
            Err(found) => {
                error = Some(found);
                break;
            }
        }
    }
    assert_eq!(error, Some(ToolParseError::ResponseTooLarge));
}

#[test]
fn too_many_lines_is_refused_rather_than_truncated_into_success() {
    let request = expert("AT+VENDOR?");
    let mut parser = ToolResponseParser::new(&request);
    let mut chunk = Vec::new();
    for index in 0..700 {
        chunk.extend_from_slice(format!("line{index}\r\n").as_bytes());
    }
    assert_eq!(
        parser.push(&chunk).expect_err("line flood"),
        ToolParseError::TooManyLines
    );
}

#[test]
fn neither_the_response_nor_the_parser_reveals_text_through_debug() {
    let request = expert("AT+CGDCONT=1,\"IP\",\"secret.example\"");
    let mut parser = ToolResponseParser::new(&request);
    // The echo is stripped, but the transcript still holds sensitive vendor text.
    let response = push_all(
        &mut parser,
        &[
            b"AT+CGDCONT=1,\"IP\",\"secret.example\"\r\n",
            b"+CGDCONT: 1,\"IP\",\"secret.example\"\r\n",
            b"+CME ERROR: 10\r\n",
        ],
    )
    .expect("final code");
    let debug = format!("{response:?}");
    assert!(!debug.contains("secret"), "{debug}");
    assert!(!debug.contains("+CME"), "{debug}");
    assert!(debug.contains("REDACTED"));
    let debug = format!("{parser:?}");
    assert!(!debug.contains("secret"), "{debug}");
    let debug = format!("{request:?}");
    assert!(!debug.contains("secret"), "{debug}");
    // The log-safe tag is the only classification that may leave the process.
    assert_eq!(response.final_code_tag(), "cme_error");
    // The bounded line count is available without exposing text.
    assert_eq!(response.line_count(), 1);
}

#[test]
fn retained_lines_are_only_reachable_through_the_explicit_accessor() {
    let request = expert("AT+VENDOR?");
    let mut parser = ToolResponseParser::new(&request);
    let response =
        push_all(&mut parser, &[b"AT+VENDOR?\r\n+sentinel-line\r\nOK\r\n"]).expect("final code");
    // The only way to read the text is the public field the UI renders; nothing derived prints it.
    assert_eq!(response.lines, vec!["+sentinel-line".to_owned()]);
    assert!(!format!("{response:?}").contains("sentinel"));
}

#[test]
fn the_whitelisted_request_lines_match_the_policy_classifier() {
    for id in ToolReadId::ALL {
        let request = ToolWireRequest::from_read(id);
        let line = request.line();
        assert_eq!(classify_read(line), Some(id), "{id:?}");
        assert_eq!(line.expose_for_confirmation(), {
            let bytes = request.wire_bytes();
            String::from_utf8_lossy(&bytes[..bytes.len() - 1]).to_string()
        });
    }
}
