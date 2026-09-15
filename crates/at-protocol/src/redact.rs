use crate::{AtCommand, Sensitivity};

const REDACTED_APN: &str = "[REDACTED_APN]";
const REDACTED_IDENTIFIER: &str = "[REDACTED_IDENTIFIER]";
const REDACTED_LINE: &str = "[REDACTED]";

/// Transaction-level redaction for Debug/log output of one response or command line.
///
/// Subscriber-identity commands (`AT+CNUM`, `AT+QCCID`) never disclose their lines in any form:
/// the whole line becomes `[REDACTED]`, so a short phone number that would survive the generic
/// long-digit-run rule can never reach a log. Message-content transactions (`AT+CMGL`,
/// `AT+CMGR`, `AT+CMGD`, `AT+CMGF=0`) redact the whole line too, because a PDU hex line is
/// opaque to the generic rules and may carry a sender address or a body. All other commands
/// keep the existing APN / PIN / long-digit-run redaction unchanged.
#[must_use]
pub fn redact_at_transaction_line(command: &AtCommand, line: &str) -> String {
    if matches!(
        command.sensitivity(),
        Sensitivity::SubscriberIdentity | Sensitivity::MessageContent
    ) {
        REDACTED_LINE.to_owned()
    } else {
        redact_at_text(line)
    }
}

#[must_use]
pub fn redact_at_text(input: &str) -> String {
    let mut output = redact_cgdcont_apns(input);
    output = redact_key_value(&output, "PIN");
    output = redact_key_value(&output, "PUK");
    redact_long_digit_runs(&output)
}

fn redact_cgdcont_apns(input: &str) -> String {
    let mut output = input.to_owned();
    let mut search_from = 0;

    while let Some(relative) = output[search_from..].find("CGDCONT") {
        let marker = search_from + relative;
        let line_end = output[marker..]
            .find(['\r', '\n'])
            .map_or(output.len(), |offset| marker + offset);
        let segment = &output[marker..line_end];
        let quote_offsets: Vec<_> = segment.match_indices('"').map(|(index, _)| index).collect();
        if quote_offsets.len() >= 4 {
            let value_start = marker + quote_offsets[2] + 1;
            let value_end = marker + quote_offsets[3];
            output.replace_range(value_start..value_end, REDACTED_APN);
            search_from = value_start + REDACTED_APN.len();
        } else {
            search_from = line_end.saturating_add(1);
        }
    }

    output
}

fn redact_key_value(input: &str, key: &str) -> String {
    let mut output = input.to_owned();
    let mut search_from = 0;

    loop {
        let uppercase = output[search_from..].to_ascii_uppercase();
        let Some(relative) = uppercase.find(key) else {
            break;
        };
        let key_start = search_from + relative;
        let after_key = key_start + key.len();
        let separator = after_key
            + output.as_bytes()[after_key..]
                .iter()
                .take_while(|&&byte| matches!(byte, b' ' | b'\t'))
                .count();
        let separator_byte = output.as_bytes().get(separator).copied();
        if !matches!(separator_byte, Some(b'=') | Some(b':')) {
            search_from = separator;
            continue;
        }
        let after_separator = separator + 1;
        let value_start = after_separator
            + output.as_bytes()[after_separator..]
                .iter()
                .take_while(|&&byte| matches!(byte, b' ' | b'\t'))
                .count();
        let value_end = output[value_start..]
            .find(|character: char| character.is_ascii_whitespace() || character == ',')
            .map_or(output.len(), |offset| value_start + offset);
        if value_start == value_end {
            search_from = value_start;
            continue;
        }
        output.replace_range(value_start..value_end, "[REDACTED]");
        search_from = value_start + "[REDACTED]".len();
    }

    output
}

fn redact_long_digit_runs(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut digits = String::new();

    for character in input.chars().chain(std::iter::once('\0')) {
        if character.is_ascii_digit() {
            digits.push(character);
            continue;
        }
        if digits.len() >= 10 {
            output.push_str(REDACTED_IDENTIFIER);
        } else {
            output.push_str(&digits);
        }
        digits.clear();
        if character != '\0' {
            output.push(character);
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscriber_identity_lines_are_redacted_as_a_whole() {
        assert_eq!(
            redact_at_transaction_line(
                &AtCommand::SubscriberNumber,
                "+CNUM: ,\"+12025550123\",145",
            ),
            "[REDACTED]"
        );
        assert_eq!(
            redact_at_transaction_line(&AtCommand::SubscriberNumber, "+CNUM: ,\"123456789\",129",),
            "[REDACTED]"
        );
        assert_eq!(
            redact_at_transaction_line(&AtCommand::Iccid, "+QCCID: \"89860123456789012345\""),
            "[REDACTED]"
        );
    }

    #[test]
    fn non_identity_lines_keep_the_existing_redaction() {
        let line = redact_at_transaction_line(
            &AtCommand::PdpContexts,
            "+CGDCONT: 1,\"IP\",\"secret.apn\",\"10.0.0.1\"",
        );
        assert!(!line.contains("secret.apn"));
        assert!(line.contains("[REDACTED_APN]"));

        let line = redact_at_transaction_line(&AtCommand::SimState, "PIN=1234");
        assert!(!line.contains("1234"));
        assert!(line.contains("PIN=[REDACTED]"));

        let line = redact_at_transaction_line(&AtCommand::SignalQuality, "+CSQ: 26,99");
        assert_eq!(line, "+CSQ: 26,99");
    }
}
