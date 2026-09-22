use std::mem;

use dji4g_domain::DeviceEpoch;

use crate::{
    AtCommand, AtEvent, AtFinalCode, AtResponse, AtUrc, PdpContextId, ProtocolError,
    ProtocolErrorKind, SensorTemperature,
};

const MAX_LINE_BYTES: usize = 4096;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_LINES: usize = 600;
const MAX_UNRECOGNIZED_LINES: usize = 32;
/// Upper bound for a module-reported message index (`CMGL`/`CMTI`); larger values are rejected
/// rather than formatted into a transaction.
const SMS_MAX_INDEX: u32 = 9999;
/// Upper bound for one `AT+CMGL=4` listing; an empty listing is a valid result.
const MAX_SMS_LIST_RECORDS: usize = 1000;
/// Upper bound for one PDU hex line inside a transaction (140-octet user data plus headers).
const MAX_PDU_HEX_CHARS: usize = 1024;
/// Upper bound for the channels of one `+QTEMP:` positional list.  A longer run of bare numbers
/// is not a plausible sensor report and is skipped whole rather than read as temperatures.
const MAX_QTEMP_CHANNELS: usize = 8;

pub struct StreamingParser {
    epoch: DeviceEpoch,
    command: AtCommand,
    echo: Vec<u8>,
    current_line: Vec<u8>,
    response_lines: Vec<String>,
    response_bytes: usize,
    unrecognized_lines: usize,
    /// Prompt mode is enabled only for PDU send transactions; elsewhere `>` stays invalid.
    prompt_enabled: bool,
    /// A `+CMGL:`/`+CMGR:` header is waiting for its PDU continuation line.
    expecting_pdu: bool,
    /// Number of `+CMGL:`/`+CMGR:` message records seen (PDU lines not counted).
    sms_records: usize,
    completed: bool,
    terminal_error: Option<ProtocolError>,
}

impl StreamingParser {
    #[must_use]
    pub fn new(epoch: DeviceEpoch, command: AtCommand) -> Self {
        Self::with_prompt(epoch, command, false)
    }

    /// Parser for transactions that legitimately wait for the `>` prompt (PDU send). The prompt
    /// is emitted as [`AtEvent::Prompt`] and normal parsing continues afterwards.
    #[must_use]
    pub fn new_with_prompt(epoch: DeviceEpoch, command: AtCommand) -> Self {
        Self::with_prompt(epoch, command, true)
    }

    fn with_prompt(epoch: DeviceEpoch, command: AtCommand, prompt_enabled: bool) -> Self {
        let mut echo = command.wire_bytes();
        debug_assert_eq!(echo.last(), Some(&b'\r'));
        echo.pop();
        Self {
            epoch,
            command,
            echo,
            current_line: Vec::with_capacity(256),
            response_lines: Vec::new(),
            response_bytes: 0,
            unrecognized_lines: 0,
            prompt_enabled,
            expecting_pdu: false,
            sms_records: 0,
            completed: false,
            terminal_error: None,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<AtEvent>, ProtocolError> {
        if let Some(error) = &self.terminal_error {
            return Err(error.clone());
        }

        let mut events = Vec::new();
        for &byte in bytes {
            if self.completed {
                if matches!(byte, b'\r' | b'\n') {
                    continue;
                }
                return self.fail(ProtocolErrorKind::UnexpectedData);
            }

            if matches!(byte, b'\r' | b'\n') {
                if self.current_line.is_empty() {
                    continue;
                }
                let line_bytes = mem::take(&mut self.current_line);
                let line = match String::from_utf8(line_bytes) {
                    Ok(line) => line,
                    Err(_) => return self.fail(ProtocolErrorKind::WrongPortData),
                };
                self.process_line(line, &mut events)?;
                continue;
            }

            if !byte.is_ascii_graphic() && byte != b' ' {
                return self.fail(ProtocolErrorKind::WrongPortData);
            }
            if self.current_line.len() == MAX_LINE_BYTES {
                return self.fail(ProtocolErrorKind::LineTooLong);
            }
            self.current_line.push(byte);
        }

        // The modem's `>` prompt is not newline-terminated: `\r\n>` may be the last bytes of a
        // read chunk, so a prompt-shaped pending line is flushed as soon as the chunk ends.
        if self.prompt_enabled && is_prompt_bytes(&self.current_line) {
            self.current_line.clear();
            events.push(AtEvent::Prompt);
        }

        Ok(events)
    }

    /// Number of raw SMS records received so far; multipart merging is unrelated.
    pub fn sms_record_count(&self) -> usize {
        self.sms_records
    }

    pub fn finish_timeout(&mut self) -> ProtocolError {
        self.terminate(ProtocolError::timeout())
    }

    pub fn finish_removed(&mut self) -> ProtocolError {
        self.terminate(ProtocolError::device_removed())
    }

    fn terminate(&mut self, error: ProtocolError) -> ProtocolError {
        if let Some(terminal_error) = &self.terminal_error {
            return terminal_error.clone();
        }
        self.current_line.clear();
        self.response_lines.clear();
        self.response_bytes = 0;
        self.unrecognized_lines = 0;
        self.expecting_pdu = false;
        self.sms_records = 0;
        self.terminal_error = Some(error.clone());
        error
    }

    fn process_line(
        &mut self,
        line: String,
        events: &mut Vec<AtEvent>,
    ) -> Result<(), ProtocolError> {
        if line.as_bytes() == self.echo {
            return Ok(());
        }
        if is_nmea(&line) {
            return self.fail(ProtocolErrorKind::WrongPortData);
        }
        if is_unsupported_multipart_urc(&line) {
            return self.fail(ProtocolErrorKind::WrongPortData);
        }
        if self.prompt_enabled && is_prompt_line(&line) {
            events.push(AtEvent::Prompt);
            return Ok(());
        }
        if let Some(final_code) = parse_final_code(&line) {
            if final_code == AtFinalCode::Ok
                && !response_is_complete(&self.command, self.response_lines.len(), self.sms_records)
            {
                return self.fail(ProtocolErrorKind::WrongPortData);
            }
            events.push(AtEvent::Response(AtResponse {
                epoch: self.epoch,
                command: self.command.clone(),
                lines: mem::take(&mut self.response_lines),
                final_code,
            }));
            self.completed = true;
            return Ok(());
        }

        match classify_line(&self.command, &line, self.expecting_pdu) {
            LineClassification::Response => {
                self.expecting_pdu = line_starts_sms_record(&self.command, &line);
                if self.expecting_pdu {
                    self.sms_records += 1;
                    if self.sms_records > MAX_SMS_LIST_RECORDS {
                        return self.fail(ProtocolErrorKind::WrongPortData);
                    }
                }
            }
            LineClassification::Urc => {
                events.push(AtEvent::Urc(AtUrc {
                    epoch: self.epoch,
                    line,
                }));
                return Ok(());
            }
            LineClassification::FreeFormResponse => {
                self.unrecognized_lines += 1;
                if self.unrecognized_lines > MAX_UNRECOGNIZED_LINES {
                    return self.fail(ProtocolErrorKind::WrongPortData);
                }
            }
            LineClassification::Invalid => {
                return self.fail(ProtocolErrorKind::WrongPortData);
            }
        }

        if self.response_lines.len()
            == (if matches!(self.command, AtCommand::SmsList) {
                2 * MAX_SMS_LIST_RECORDS
            } else {
                MAX_RESPONSE_LINES
            })
            || self.response_bytes.saturating_add(line.len())
                > (if matches!(self.command, AtCommand::SmsList) {
                    1024 * 1024
                } else {
                    MAX_RESPONSE_BYTES
                })
        {
            return self.fail(ProtocolErrorKind::ResponseTooLarge);
        }
        self.response_bytes += line.len();
        self.response_lines.push(line);
        Ok(())
    }

    fn fail<T>(&mut self, kind: ProtocolErrorKind) -> Result<T, ProtocolError> {
        let error = self.terminate(ProtocolError::verification(kind));
        Err(error)
    }
}

fn parse_final_code(line: &str) -> Option<AtFinalCode> {
    match line {
        "OK" => Some(AtFinalCode::Ok),
        "ERROR" => Some(AtFinalCode::Error),
        "NO CARRIER" => Some(AtFinalCode::NoCarrier),
        "NO ANSWER" => Some(AtFinalCode::NoAnswer),
        "BUSY" => Some(AtFinalCode::Busy),
        "NO DIALTONE" => Some(AtFinalCode::NoDialTone),
        _ => line
            .strip_prefix("+CME ERROR:")
            .map(|detail| AtFinalCode::CmeError(detail.trim().to_owned()))
            .or_else(|| {
                line.strip_prefix("+CMS ERROR:")
                    .map(|detail| AtFinalCode::CmsError(detail.trim().to_owned()))
            }),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LineClassification {
    Response,
    FreeFormResponse,
    Urc,
    Invalid,
}

fn classify_line(command: &AtCommand, line: &str, expecting_pdu: bool) -> LineClassification {
    if matches!(command, AtCommand::Temperature) {
        // The QTEMP sensor layout is firmware-defined (research §7.5) and this module answers with
        // an unnamed positional list.  The streaming boundary therefore decides *attribution*
        // only: a `+QTEMP:` answer carrying at least one readable value is this command's
        // response, and any other text line is collected as an unattributable free-form line
        // (bounded by MAX_UNRECOGNIZED_LINES) instead of failing the whole optional probe.
        // Interpreting the layout stays `parse_qtemp_lines`'s job.  Wire-level faults (NMEA,
        // binary noise, oversized lines) are still rejected above this point.
        if is_known_urc(line) {
            return LineClassification::Urc;
        }
        return if is_qtemp_response(line) {
            LineClassification::Response
        } else {
            LineClassification::FreeFormResponse
        };
    }
    if matches!(command, AtCommand::ServingCellInfo) && line.starts_with("+QENG:") {
        // Unsolicited +QENG reports and other shapes stay Invalid for this command; only the
        // strict modeled response is trusted.
        return if is_serving_cell_response(line) {
            LineClassification::Response
        } else {
            LineClassification::Invalid
        };
    }
    if matches!(command, AtCommand::EpsRegistration) && line.starts_with("+CEREG:") {
        if is_registration_query_response(line, "+CEREG:") {
            return LineClassification::Response;
        }
        return if is_registration_urc(line, "+CEREG:") {
            LineClassification::Urc
        } else {
            LineClassification::Invalid
        };
    }

    if is_known_urc(line) {
        return LineClassification::Urc;
    }
    if looks_like_known_urc(line) {
        return LineClassification::Invalid;
    }
    if expecting_pdu && matches!(command, AtCommand::SmsList | AtCommand::SmsRead { .. }) {
        // The line after a `+CMGL:`/`+CMGR:` header is either the PDU continuation or the next
        // header; anything else fails closed instead of being stored as a free-form line.
        if is_pdu_hex_line(line) {
            return LineClassification::Response;
        }
        return match command {
            AtCommand::SmsList if is_cmgl_response(line) => LineClassification::Response,
            AtCommand::SmsRead { .. } if is_cmgr_response(line) => LineClassification::Response,
            _ => LineClassification::Invalid,
        };
    }

    match command {
        AtCommand::Identity | AtCommand::Manufacturer | AtCommand::Model | AtCommand::Revision => {
            LineClassification::FreeFormResponse
        }
        AtCommand::SimState if is_sim_state_response(line) => LineClassification::Response,
        AtCommand::SignalQuality if is_signal_quality_response(line) => {
            LineClassification::Response
        }
        AtCommand::Operator if is_operator_response(line) => LineClassification::Response,
        AtCommand::PacketAttach if is_packet_attach_response(line) => LineClassification::Response,
        AtCommand::PdpContexts if is_pdp_context_response(line) => LineClassification::Response,
        AtCommand::PdpActivation if is_pdp_activation_response(line) => {
            LineClassification::Response
        }
        AtCommand::PdpAddresses if is_pdp_address_response(line) => LineClassification::Response,
        AtCommand::UsbNetQuery if is_usbnet_response(line) => LineClassification::Response,
        AtCommand::ExtendedError if nonempty_payload(line, "+CEER:").is_some() => {
            LineClassification::Response
        }
        // CNUM may legally report zero to several lines (an empty OK is not an error); every
        // +CNUM: line is attributed to this transaction, with shape validated at parse time.
        AtCommand::SubscriberNumber if nonempty_payload(line, "+CNUM:").is_some() => {
            LineClassification::Response
        }
        // QCCID is shape-checked at the streaming boundary (digits only, 6–32 chars) so a
        // malformed identity line can never complete a transaction.
        AtCommand::Iccid if parse_iccid_line(line).is_some() => LineClassification::Response,
        AtCommand::SmsMessageFormat if is_cmgf_response(line) => LineClassification::Response,
        AtCommand::SmsStorageQuery
        | AtCommand::SmsStorageCapabilities
        | AtCommand::SmsSelectStorage { .. }
            if is_cpms_response(line) =>
        {
            LineClassification::Response
        }
        AtCommand::SmsList if is_cmgl_response(line) => LineClassification::Response,
        AtCommand::SmsRead { .. } if is_cmgr_response(line) => LineClassification::Response,
        // The message reference line of a submission; the PDU body itself already went out on
        // the prompt path and never appears here.
        AtCommand::SmsSend { .. } if nonempty_payload(line, "+CMGS:").is_some() => {
            LineClassification::Response
        }
        AtCommand::Temperature if is_qtemp_response(line) => LineClassification::Response,
        _ => LineClassification::Invalid,
    }
}

fn response_is_complete(command: &AtCommand, line_count: usize, sms_records: usize) -> bool {
    match command {
        AtCommand::Attention
        | AtCommand::RestartModule
        | AtCommand::SetApn { .. }
        | AtCommand::SetUsbNetProfile(_) => line_count == 0,
        AtCommand::Identity | AtCommand::Manufacturer | AtCommand::Model | AtCommand::Revision => {
            (1..=MAX_UNRECOGNIZED_LINES).contains(&line_count)
        }
        AtCommand::PdpContexts | AtCommand::PdpActivation | AtCommand::PdpAddresses => {
            (1..=usize::from(PdpContextId::MAX)).contains(&line_count)
        }
        AtCommand::SimState
        | AtCommand::SignalQuality
        | AtCommand::Operator
        | AtCommand::ServingCellInfo
        | AtCommand::EpsRegistration
        | AtCommand::PacketAttach
        | AtCommand::UsbNetQuery
        | AtCommand::ExtendedError
        | AtCommand::Iccid => line_count == 1,
        // CNUM reports zero to sixteen lines; the final OK gate still applies (an empty OK is
        // a valid Empty result, never a protocol failure).
        AtCommand::SubscriberNumber => line_count <= 16,
        AtCommand::SmsMessageFormat
        | AtCommand::SmsStorageQuery
        | AtCommand::SmsStorageCapabilities => line_count == 1,
        AtCommand::SmsSelectStorage { .. } => line_count <= 1,
        // One CMGR record: the header, optionally followed by its PDU continuation line.
        AtCommand::SmsRead { .. } => sms_records == 1,
        // An empty list is a valid OK; PDU continuation lines are not records.
        AtCommand::SmsList => sms_records <= MAX_SMS_LIST_RECORDS,
        AtCommand::SmsSetPduMode | AtCommand::SmsDelete { .. } => line_count == 0,
        // Submission: the `+CMGS: <reference>` line is optional; the final OK still gates.
        AtCommand::SmsSend { .. } => line_count <= 1,
        // Sensor count and channel layout are firmware-defined; a plain OK is a clean "no data"
        // result rather than a protocol fault, and the unattributable-line bound already caps how
        // much unrecognised text one transaction may collect.
        AtCommand::Temperature => line_count <= MAX_UNRECOGNIZED_LINES,
    }
}

pub(crate) fn is_known_urc(line: &str) -> bool {
    if ["+CEREG:", "+CGREG:", "+CREG:"]
        .iter()
        .any(|prefix| is_registration_urc(line, prefix))
    {
        return true;
    }

    const PREFIXES: &[&str] = &["+CMTI:", "+CGEV:", "+QENG:", "+QIURC:", "+QIND:", "+QUSIM:"];
    const EXACT: &[&str] = &["RING", "RDY", "SMS READY", "PB DONE", "POWERED DOWN"];

    PREFIXES
        .iter()
        .any(|prefix| nonempty_payload(line, prefix).is_some())
        || EXACT.contains(&line)
}

fn looks_like_known_urc(line: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "+CEREG:", "+CGREG:", "+CREG:", "+CMTI:", "+CGEV:", "+QENG:", "+QIURC:", "+QIND:",
        "+QUSIM:",
    ];
    PREFIXES.iter().any(|prefix| line.starts_with(prefix))
}

fn is_unsupported_multipart_urc(line: &str) -> bool {
    line.starts_with("+CMT:") || line.starts_with("+CDS:")
}

fn is_sim_state_response(line: &str) -> bool {
    const STATES: &[&str] = &[
        "READY",
        "SIM PIN",
        "SIM PUK",
        "SIM PIN2",
        "SIM PUK2",
        "PH-SIM PIN",
        "PH-FSIM PIN",
        "PH-FSIM PUK",
        "PH-NET PIN",
        "PH-NET PUK",
        "PH-NETSUB PIN",
        "PH-NETSUB PUK",
        "PH-SP PIN",
        "PH-SP PUK",
        "PH-CORP PIN",
        "PH-CORP PUK",
        "NOT READY",
    ];
    nonempty_payload(line, "+CPIN:").is_some_and(|payload| STATES.contains(&payload))
}

fn is_signal_quality_response(line: &str) -> bool {
    let Some(payload) = nonempty_payload(line, "+CSQ:") else {
        return false;
    };
    let Some([rssi, ber]) = exactly_two_fields(payload) else {
        return false;
    };
    parse_u8(rssi).is_some_and(|value| value <= 99)
        && parse_u8(ber).is_some_and(|value| value <= 99)
}

/// Strict `+QENG: "servingcell",...` (LTE, the reference 18-field layout) or the honest
/// state-only reports (`"SEARCH"`, `"LIMSRV"`, `"NOCELL"`).
///
/// Reference layout (research document §2.2 / appendix A): 0 report keyword, 1 state, 2 RAT,
/// 3 duplex, 4 mcc, 5 mnc, 6 cellid (hex), 7 pci, 8 earfcn, 9 band, 10 ul_bw, 11 dl_bw,
/// 12 tac (hex), 13 rsrp, 14 rsrq, 15 rssi, 16 sinr (raw), 17 srxlev. `-` means the field is
/// absent and degrades to `None`; a malformed value rejects the whole line (never a silently
/// shifted read).
///
/// 该 18 字段布局为候选布局，待实机确认（含 SINR 的原始刻度）后才作为最终契约。
pub fn parse_serving_cell_line(line: &str) -> Option<dji4g_domain::ServingCell> {
    let payload = nonempty_payload(line, "+QENG:")?;
    let fields = split_csv(payload)?;
    let state_only = |state: &str| {
        Some(dji4g_domain::ServingCell {
            state: Some(state.to_owned()),
            duplex: None,
            rat: None,
            mcc: None,
            mnc: None,
            cell_id: None,
            pci: None,
            earfcn: None,
            band: None,
            ul_mhz: None,
            dl_mhz: None,
            tac: None,
            rsrp_dbm: None,
            rsrq_db: None,
            rssi_dbm: None,
            sinr_raw: None,
            srxlev_raw: None,
        })
    };
    match *fields.first()? {
        "\"servingcell\"" => {
            // State-only form: "servingcell","SEARCH" / "LIMSRV" / "NOCELL" — the state lives
            // in the second field, not the keyword (research §2.2).
            if fields.len() == 2 {
                let state = unquote(fields[1]);
                if matches!(state, "SEARCH" | "LIMSRV" | "NOCELL") {
                    return state_only(state);
                }
                return None;
            }
        }
        "\"NOCELL\"" => return state_only("NOCELL"),
        _ => return None,
    }
    if fields.len() != 18
        || fields[2] != "\"LTE\""
        || (fields[3] != "\"FDD\"" && fields[3] != "\"TDD\"")
    {
        return None;
    }
    let state = unquote(fields[1]);
    if !matches!(state, "CONNECT" | "NOCONN" | "SEARCH" | "LIMSRV") {
        return None;
    }
    let pci = parse_missing(fields[7], |value| parse_u16(value).filter(|v| *v <= 503))?;
    let ul_mhz = parse_missing(fields[10], bandwidth)?;
    let dl_mhz = parse_missing(fields[11], bandwidth)?;
    let tac = parse_missing(fields[12], |value| parse_hex_u32(value, 0xffff))
        .map(|value| value.map(|v| v as u16))?;
    Some(dji4g_domain::ServingCell {
        state: Some(state.to_owned()),
        duplex: Some(unquote(fields[3]).to_owned()),
        rat: Some("LTE".to_owned()),
        mcc: parse_missing(fields[4], |value| {
            (value.len() == 3 && value.bytes().all(|b| b.is_ascii_digit()))
                .then(|| value.to_owned())
        })?,
        mnc: parse_missing(fields[5], |value| {
            ((2..=3).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit()))
                .then(|| value.to_owned())
        })?,
        cell_id: parse_missing(fields[6], |value| parse_hex_u32(value, 0x0fff_ffff))?,
        pci,
        earfcn: parse_missing(fields[8], parse_u32)?,
        band: parse_missing(fields[9], parse_u32)?,
        ul_mhz,
        dl_mhz,
        tac,
        rsrp_dbm: parse_missing(fields[13], parse_i16)?,
        rsrq_db: parse_missing(fields[14], parse_i16)?,
        rssi_dbm: parse_missing(fields[15], parse_i16)?,
        // SINR stays raw: its unit/scale is firmware-profile dependent and unconfirmed for this
        // device (research §5.2). The UI must not render it as a plain dB value.
        sinr_raw: parse_missing(fields[16], parse_i16)?,
        srxlev_raw: parse_missing(fields[17], parse_i16)?,
    })
}

/// CNUM field-parsing failure; each variant maps onto a distinct user-visible explanation
/// (FormatMismatch vs malformed data) rather than a blanket 未获取.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CnumParseError {
    TooLong,
    Csv,
    Shape,
    Number,
    Range,
}

/// Quote-aware AT comma field splitter (research appendix A): quoted fields keep their commas,
/// empty fields stay empty, `""` inside quotes is an escaped quote, and control characters are
/// rejected. Values are trimmed when unquoted.
pub fn at_csv(value: &str) -> Result<Vec<String>, CnumParseError> {
    if value.len() > 4096 {
        return Err(CnumParseError::TooLong);
    }
    let mut out = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut closed = false;
    let mut was_quoted = false;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_control() {
            return Err(CnumParseError::Csv);
        }
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(ch);
            }
        } else if ch == ',' {
            out.push(if was_quoted {
                std::mem::take(&mut field)
            } else {
                std::mem::take(&mut field).trim().to_owned()
            });
            if out.len() >= 32 {
                return Err(CnumParseError::TooLong);
            }
            closed = false;
            was_quoted = false;
        } else if closed {
            if !ch.is_whitespace() {
                return Err(CnumParseError::Csv);
            }
        } else if ch == '"' {
            if !field.trim().is_empty() {
                return Err(CnumParseError::Csv);
            }
            field.clear();
            quoted = true;
            was_quoted = true;
        } else {
            field.push(ch);
        }
    }
    if quoted {
        return Err(CnumParseError::Csv);
    }
    out.push(if was_quoted {
        field
    } else {
        field.trim().to_owned()
    });
    Ok(out)
}

/// Parse CNUM data lines into the number lookup result. `lines` must contain only lines already
/// attributed to this transaction; the caller guarantees the final code was OK (an empty OK is
/// `NumberLookup::Empty`, never a failure). TOA is preserved raw; `129` is not treated as a
/// confirmed country format. Numbers are deduplicated by (number, toa).
///
/// TOA 语义（129/145/161）为候选映射，待实机确认；本解析器只保留设备上报的原始值，不做任何推断。
pub fn parse_cnum_lines(lines: &[&str]) -> Result<dji4g_domain::NumberLookup, CnumParseError> {
    if lines.len() > 16 {
        return Err(CnumParseError::TooLong);
    }
    let mut numbers = Vec::new();
    for line in lines {
        let payload = nonempty_payload(line, "+CNUM:").ok_or(CnumParseError::Shape)?;
        let fields = at_csv(payload)?;
        if !(3..=6).contains(&fields.len()) {
            return Err(CnumParseError::Shape);
        }
        let toa = fields[2]
            .parse::<u8>()
            .map_err(|_| CnumParseError::Number)?;
        let number = fields[1].trim();
        if number.is_empty() {
            continue;
        }
        // Store only the reported value: never invent a +86 prefix, never interpret TOA 129.
        if number.len() > 40
            || !number
                .chars()
                .all(|ch| ch.is_ascii_digit() || matches!(ch, '+' | '*' | '#'))
        {
            return Err(CnumParseError::Number);
        }
        if !numbers.iter().any(|entry: &dji4g_domain::PhoneNumber| {
            entry.expose_after_user_action() == number && entry.toa == toa
        }) {
            numbers.push(dji4g_domain::PhoneNumber::new(number.to_owned(), toa));
        }
    }
    Ok(if numbers.is_empty() {
        dji4g_domain::NumberLookup::Empty
    } else {
        dji4g_domain::NumberLookup::Reported(numbers)
    })
}

/// Parse the raw ICCID from a `+QCCID:` response line (digits only, 6–32 characters). Masking
/// and fingerprinting happen at the domain boundary.
///
/// 长度与纯数字约束为候选形态，待实机确认；不符合即整体拒绝，绝不猜测。
pub fn parse_iccid_line(line: &str) -> Option<String> {
    let payload = nonempty_payload(line, "+QCCID:")?;
    let value = unquote(payload);
    if !(6..=32).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(value.to_owned())
}

fn is_serving_cell_response(line: &str) -> bool {
    parse_serving_cell_line(line).is_some()
}

fn is_operator_response(line: &str) -> bool {
    let Some(payload) = nonempty_payload(line, "+COPS:") else {
        return false;
    };
    let Some(fields) = split_csv(payload) else {
        return false;
    };
    if !(1..=4).contains(&fields.len()) {
        return false;
    }
    if !parse_u8(fields[0]).is_some_and(|mode| mode <= 4) {
        return false;
    }
    if fields.len() >= 2 && !parse_u8(fields[1]).is_some_and(|format| format <= 2) {
        return false;
    }
    if fields.len() >= 3 && !is_quoted(fields[2]) {
        return false;
    }
    fields.len() < 4 || parse_u8(fields[3]).is_some_and(|access_technology| access_technology <= 9)
}

fn is_registration_query_response(line: &str, prefix: &str) -> bool {
    let Some(payload) = nonempty_payload(line, prefix) else {
        return false;
    };
    let Some(fields) = split_csv(payload) else {
        return false;
    };
    fields.len() >= 2
        && parse_u8(fields[0]).is_some_and(|mode| mode <= 5)
        && parse_u8(fields[1]).is_some_and(|status| status <= 10)
        && fields.iter().all(|field| !field.is_empty())
}

fn is_registration_urc(line: &str, prefix: &str) -> bool {
    let Some(payload) = nonempty_payload(line, prefix) else {
        return false;
    };
    !payload.contains(',') && parse_u8(payload).is_some_and(|status| status <= 10)
}

fn is_packet_attach_response(line: &str) -> bool {
    nonempty_payload(line, "+CGATT:")
        .and_then(parse_u8)
        .is_some_and(|value| value <= 1)
}

fn is_pdp_context_response(line: &str) -> bool {
    let Some(payload) = nonempty_payload(line, "+CGDCONT:") else {
        return false;
    };
    let Some(fields) = split_csv(payload) else {
        return false;
    };
    fields.len() >= 3
        && parse_context_id(fields[0]).is_some()
        && is_nonempty_quoted(fields[1])
        && is_quoted(fields[2])
}

fn is_pdp_activation_response(line: &str) -> bool {
    let Some(payload) = nonempty_payload(line, "+CGACT:") else {
        return false;
    };
    let Some([cid, state]) = exactly_two_fields(payload) else {
        return false;
    };
    parse_context_id(cid).is_some() && parse_u8(state).is_some_and(|value| value <= 1)
}

fn is_pdp_address_response(line: &str) -> bool {
    let Some(payload) = nonempty_payload(line, "+CGPADDR:") else {
        return false;
    };
    let Some(fields) = split_csv(payload) else {
        return false;
    };
    fields.len() >= 2 && parse_context_id(fields[0]).is_some() && !fields[1].is_empty()
}

fn is_usbnet_response(line: &str) -> bool {
    let Some(payload) = nonempty_payload(line, "+QCFG:") else {
        return false;
    };
    let Some([name, value]) = exactly_two_fields(payload) else {
        return false;
    };
    name == "\"usbnet\"" && parse_u8(value).is_some()
}

fn is_cmgf_response(line: &str) -> bool {
    nonempty_payload(line, "+CMGF:")
        .and_then(parse_u8)
        .is_some_and(|value| value <= 1)
}

fn is_cpms_response(line: &str) -> bool {
    let Some(payload) = nonempty_payload(line, "+CPMS:") else {
        return false;
    };
    split_csv(payload).is_some_and(|fields| fields.len() >= 3)
}

/// PDU-mode `+CMGL: <index>,<stat>[,<alpha>],<length>`; text-mode lines fail on the numeric
/// index/stat shape.
fn is_cmgl_response(line: &str) -> bool {
    let Some(payload) = nonempty_payload(line, "+CMGL:") else {
        return false;
    };
    let Some(fields) = split_csv(payload) else {
        return false;
    };
    fields.len() >= 2
        && parse_u32(fields[0]).is_some_and(|index| index <= SMS_MAX_INDEX)
        && parse_u8(fields[1]).is_some_and(|stat| stat <= 4)
}

/// PDU-mode `+CMGR: <stat>[,<alpha>],<length>`; only the status field is validated here.
fn is_cmgr_response(line: &str) -> bool {
    let Some(payload) = nonempty_payload(line, "+CMGR:") else {
        return false;
    };
    let Some(fields) = split_csv(payload) else {
        return false;
    };
    !fields.is_empty() && parse_u8(fields[0]).is_some_and(|stat| stat <= 4)
}

/// Whether a line is attributable to an `AtCommand::Temperature` transaction.  Attribution asks
/// only whether this is a `+QTEMP:` answer carrying at least one readable value; the layout is
/// interpreted by [`parse_qtemp_line`], never guessed here.
fn is_qtemp_response(line: &str) -> bool {
    !parse_qtemp_line(line).is_empty()
}

/// The PDU hex line that follows a `+CMGL:`/`+CMGR:` header in PDU mode.
fn is_pdu_hex_line(line: &str) -> bool {
    line.len() >= 2
        && line.len() <= MAX_PDU_HEX_CHARS
        && line.len() % 2 == 0
        && line.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn line_starts_sms_record(command: &AtCommand, line: &str) -> bool {
    match command {
        AtCommand::SmsList => is_cmgl_response(line),
        AtCommand::SmsRead { .. } => is_cmgr_response(line),
        _ => false,
    }
}

/// The `>` input prompt of a PDU send transaction, optionally followed by whitespace only.
fn is_prompt_line(line: &str) -> bool {
    line.strip_prefix('>')
        .is_some_and(|rest| rest.chars().all(char::is_whitespace))
}

/// Prompt-shaped pending bytes (no line terminator required yet).
fn is_prompt_bytes(line: &[u8]) -> bool {
    line.first() == Some(&b'>') && line[1..].iter().all(|byte| *byte == b' ')
}

/// Parse one `+CMTI: "<storage>",<index>` new-message indication (research §6.2). The storage
/// name must be quoted and the index within the modeled range; anything else is `None`.
pub fn parse_cmti_line(line: &str) -> Option<(dji4g_domain::SmsStorageId, u32)> {
    let payload = nonempty_payload(line, "+CMTI:")?;
    let fields = split_csv(payload)?;
    let [storage, index] = fields.as_slice() else {
        return None;
    };
    if !is_nonempty_quoted(storage) {
        return None;
    }
    let index = parse_u32(index).filter(|value| *value <= SMS_MAX_INDEX)?;
    Some((
        dji4g_domain::SmsStorageId(unquote(storage).to_owned()),
        index,
    ))
}

/// Parse `+QTEMP` readings (research §7.5).  Malformed lines are skipped, never guessed; sensor
/// names and thresholds remain firmware-defined, and an unnamed channel keeps `None`.
pub fn parse_qtemp_lines(lines: &[&str]) -> Vec<SensorTemperature> {
    lines
        .iter()
        .flat_map(|line| parse_qtemp_line(line))
        .collect()
}

/// One `+QTEMP:` line's readings, in report order.  Two shapes are modelled:
///
/// * the named form `+QTEMP: "<name>",<degrees>` — exactly one name/value pair;
/// * the unnamed positional form `+QTEMP: <degrees>[,<degrees>...]`, which the DJI Gen-1 firmware
///   answers with (`+QTEMP: 57,51,51`).
///
/// Anything else yields no readings — the raw line is still shown to the user, but nothing is
/// derived from a shape this build has not verified.
fn parse_qtemp_line(line: &str) -> Vec<SensorTemperature> {
    let Some(payload) = nonempty_payload(line, "+QTEMP:") else {
        return Vec::new();
    };
    if let Some([name, value]) = exactly_two_fields(payload) {
        if is_nonempty_quoted(name) {
            return match parse_i16(value) {
                Some(celsius) => vec![SensorTemperature {
                    name: Some(unquote(name).to_owned()),
                    celsius,
                }],
                None => Vec::new(),
            };
        }
    }
    positional_qtemp_readings(payload)
}

/// The unnamed positional form: one to [`MAX_QTEMP_CHANNELS`] bare integers, no quoting.  Every
/// field must be a value — a partially numeric list is not a layout this build has seen, so it is
/// skipped whole rather than trimmed into a plausible-looking reading.
fn positional_qtemp_readings(payload: &str) -> Vec<SensorTemperature> {
    if payload.contains('"') {
        return Vec::new();
    }
    let Some(fields) = split_csv(payload) else {
        return Vec::new();
    };
    if fields.is_empty() || fields.len() > MAX_QTEMP_CHANNELS {
        return Vec::new();
    }
    let mut readings = Vec::with_capacity(fields.len());
    for field in fields {
        let Some(celsius) = parse_i16(field) else {
            return Vec::new();
        };
        readings.push(SensorTemperature {
            name: None,
            celsius,
        });
    }
    readings
}

fn nonempty_payload<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let payload = line.strip_prefix(prefix)?.trim();
    (!payload.is_empty()).then_some(payload)
}

fn exactly_two_fields(payload: &str) -> Option<[&str; 2]> {
    let fields = split_csv(payload)?;
    (fields.len() == 2).then(|| [fields[0], fields[1]])
}

fn split_csv(payload: &str) -> Option<Vec<&str>> {
    let mut fields = Vec::new();
    let mut quoted = false;
    let mut field_start = 0;
    for (index, byte) in payload.bytes().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                fields.push(payload.get(field_start..index)?.trim());
                field_start = index + 1;
            }
            _ => {}
        }
    }
    if quoted {
        return None;
    }
    fields.push(payload.get(field_start..)?.trim());
    Some(fields)
}

fn parse_u16(value: &str) -> Option<u16> {
    value.parse::<u16>().ok()
}

fn parse_u32(value: &str) -> Option<u32> {
    value.parse::<u32>().ok()
}

fn parse_i16(value: &str) -> Option<i16> {
    value.parse::<i16>().ok()
}

fn parse_u8(value: &str) -> Option<u8> {
    value.parse().ok()
}

/// A `-` or empty field means "absent" (`Some(None)`); a malformed value rejects the whole
/// line (`None`), so a shifted layout can never silently produce plausible-looking numbers.
fn parse_missing<T>(value: &str, parse: impl Fn(&str) -> Option<T>) -> Option<Option<T>> {
    let value = value.trim();
    if value.is_empty() || value == "-" {
        Some(None)
    } else {
        parse(value).map(Some)
    }
}

/// Hex field (Cell ID, TAC) with an upper bound; `-` handled by the caller.
fn parse_hex_u32(value: &str, max: u32) -> Option<u32> {
    u32::from_str_radix(value.trim(), 16)
        .ok()
        .filter(|parsed| *parsed <= max)
}

/// LTE bandwidth index → MHz (research appendix A: 0..=5 → 1.4/3/5/10/15/20).
fn bandwidth(value: &str) -> Option<f32> {
    match parse_u8(value)? {
        index @ 0..=5 => Some([1.4, 3.0, 5.0, 10.0, 15.0, 20.0][index as usize]),
        _ => None,
    }
}

fn unquote(value: &str) -> &str {
    value.trim().trim_matches('"')
}

fn parse_context_id(value: &str) -> Option<PdpContextId> {
    PdpContextId::try_from(parse_u8(value)?).ok()
}

fn is_quoted(value: &str) -> bool {
    value.len() >= 2 && value.starts_with('"') && value.ends_with('"')
}

fn is_nonempty_quoted(value: &str) -> bool {
    is_quoted(value) && value.len() > 2
}

fn is_nmea(line: &str) -> bool {
    ["$GP", "$GN", "$GL", "$GA", "$GB", "$BD"]
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use dji4g_domain::NumberLookup;

    use super::*;
    use crate::CnumParseError;

    #[test]
    fn at_csv_accepts_up_to_32_fields() {
        let fields = (0..32)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(at_csv(&fields).expect("32 fields are valid").len(), 32);
    }

    #[test]
    fn at_csv_rejects_more_than_32_fields() {
        let fields = (0..33)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(at_csv(&fields), Err(CnumParseError::TooLong));
    }

    #[test]
    fn at_csv_rejects_control_characters() {
        assert_eq!(at_csv("a\x07b"), Err(CnumParseError::Csv));
        assert_eq!(at_csv("\"a\x1bb\""), Err(CnumParseError::Csv));
    }

    #[test]
    fn at_csv_rejects_unclosed_quotes_and_garbage_after_a_closed_quote() {
        assert_eq!(at_csv("\"oops"), Err(CnumParseError::Csv));
        assert_eq!(at_csv("\"a\"x"), Err(CnumParseError::Csv));
        assert_eq!(at_csv("\"a\"x,1"), Err(CnumParseError::Csv));
    }

    #[test]
    fn at_csv_trims_unquoted_fields_and_keeps_quoted_ones() {
        assert_eq!(at_csv(" 1 , \"two\" ,3 ").unwrap(), ["1", "two", "3"]);
        assert_eq!(at_csv("\" two \"").unwrap(), [" two "]);
    }

    #[test]
    fn parse_cnum_lines_keeps_empty_label_and_preserves_raw_toa() {
        let lookup = parse_cnum_lines(&["+CNUM: ,\"123\",129"]).expect("empty label is valid");
        let NumberLookup::Reported(numbers) = lookup else {
            panic!("expected a reported number");
        };
        assert_eq!(numbers[0].expose_after_user_action(), "123");
        assert_eq!(numbers[0].toa, 129);
        assert_eq!(numbers[0].masked(), "****");
    }

    #[test]
    fn parse_cnum_lines_masks_short_numbers_entirely() {
        let lookup = parse_cnum_lines(&["+CNUM: ,\"1234\",145", "+CNUM: ,\"12345\",145"])
            .expect("short numbers are valid");
        let debug = format!("{lookup:?}");
        let NumberLookup::Reported(numbers) = lookup else {
            panic!("expected numbers");
        };
        assert_eq!(numbers[0].masked(), "****");
        assert_eq!(numbers[1].masked(), "****2345");
        assert!(!debug.contains("1234"));
        assert!(!debug.contains("12345"));
    }

    #[test]
    fn parse_cnum_lines_rejects_more_than_sixteen_lines() {
        let lines = vec!["+CNUM: ,\"123\",145"; 17];
        assert_eq!(parse_cnum_lines(&lines), Err(CnumParseError::TooLong));
    }
}
