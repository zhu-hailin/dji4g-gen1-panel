//! Wire encoding and incremental response parsing for one device-tool transaction.
//!
//! One transaction is one text AT request and one text final response. The parser is separate from
//! [`crate::StreamingParser`] on purpose: the monitoring and SMS parsers must keep their current
//! strictness, and the tool path additionally has to treat an unexpected `>`/`CONNECT` as a hard
//! stop instead of a prompt to continue, and has to keep the module's final error code *inside*
//! the response rather than turning it into a transport error.

use std::fmt;

use crate::AtFinalCode;
use crate::parser::is_known_urc;
use crate::tool_command::{ToolReadId, ValidatedToolLine, typed_read};

/// Longest single response line, matching the monitoring parser's limit.
const MAX_LINE_BYTES: usize = 4096;
/// Largest total response the tool path will accumulate.
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
/// Largest number of response lines the tool path will accumulate.
const MAX_RESPONSE_LINES: usize = 600;

/// A request that is ready to put on the wire.
///
/// The wire form is the validated line plus exactly one carriage return. The terminal never
/// supplies its own terminator, so a user cannot smuggle a second command terminator into the
/// transaction through a "body" or a "terminator" argument.
#[derive(Clone)]
pub struct ToolWireRequest {
    line: ValidatedToolLine,
}

impl fmt::Debug for ToolWireRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ToolWireRequest([REDACTED_TOOL_COMMAND])")
    }
}

impl ToolWireRequest {
    /// Build the request for a whitelisted read.
    ///
    /// Infallible by construction: the whitelist is tied to the typed encoder, and
    /// `whitelist_matches_the_typed_encoder` checks every [`ToolReadId`] against its encoding.
    #[must_use]
    pub fn from_read(id: ToolReadId) -> Self {
        let encoded = typed_read(id).encode();
        let text = String::from_utf8_lossy(encoded.as_bytes())
            .trim_end_matches('\r')
            .to_owned();
        Self {
            line: ValidatedToolLine::parse(&text)
                .expect("every whitelisted read is a valid tool line"),
        }
    }

    /// Build the request for a line the expert terminal validated.
    #[must_use]
    pub fn from_expert(line: ValidatedToolLine) -> Self {
        Self { line }
    }

    /// The bytes to write: the validated line and exactly one `CR`.
    #[must_use]
    pub fn wire_bytes(&self) -> Vec<u8> {
        let mut bytes = self.line.expose_for_confirmation().as_bytes().to_vec();
        bytes.push(b'\r');
        bytes
    }

    /// The validated line, for callers that need to classify or confirm the request.
    #[must_use]
    pub fn line(&self) -> &ValidatedToolLine {
        &self.line
    }
}

/// A completed tool transaction: whatever the module sent before its final code.
///
/// Nothing here implements `Serialize`, and `Debug` prints no line and no error detail: a vendor
/// response is treated as sensitive in full, because no keyword list can cover every vendor's
/// format. The UI reads the lines through [`ToolResponse::lines`] for on-screen display only, and
/// the clipboard path is a separate, explicit user action.
#[derive(Clone, Eq, PartialEq)]
pub struct ToolResponse {
    /// Response lines, in arrival order, excluding the echo, the final code and known URCs.
    pub lines: Vec<String>,
    /// URCs that arrived during the transaction. They belong to the module, not to this command,
    /// and are therefore never evidence that the command succeeded.
    pub urc_lines: Vec<String>,
    /// How many of [`ToolResponse::lines`] could not be attributed to a known shape.
    pub unclassified_lines: usize,
    pub final_code: AtFinalCode,
}

impl fmt::Debug for ToolResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The final code is redacted as well: a `+CME ERROR:` detail is vendor text and can carry
        // an identifier or a phone number in its tail.
        formatter
            .debug_struct("ToolResponse")
            .field("lines", &"[REDACTED_TOOL_RESPONSE]")
            .field("urc_lines", &"[REDACTED_TOOL_RESPONSE]")
            .field("unclassified_lines", &self.unclassified_lines)
            .field("final_code", &"[REDACTED_TOOL_FINAL_CODE]")
            .finish()
    }
}

impl ToolResponse {
    /// Log-safe tag for the final code. This is the only form that may be written to a log or a
    /// diagnostic summary.
    #[must_use]
    pub const fn final_code_tag(&self) -> &'static str {
        match &self.final_code {
            AtFinalCode::Ok => "ok",
            AtFinalCode::Error => "error",
            AtFinalCode::CmeError(_) => "cme_error",
            AtFinalCode::CmsError(_) => "cms_error",
            AtFinalCode::NoCarrier => "no_carrier",
            AtFinalCode::NoAnswer => "no_answer",
            AtFinalCode::Busy => "busy",
            AtFinalCode::NoDialTone => "no_dialtone",
        }
    }

    /// Total number of collected text lines, for a log-safe size report.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.lines.len() + self.urc_lines.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolParseError {
    LineTooLong,
    ResponseTooLarge,
    TooManyLines,
    /// A `>` prompt or a `CONNECT` data state: the module expects an interaction this path does
    /// not drive.
    UnsupportedInteraction,
    /// Bytes arrived after the transaction had already completed.
    UnexpectedData,
}

impl ToolParseError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::LineTooLong => "tool_parse:line_too_long",
            Self::ResponseTooLarge => "tool_parse:response_too_large",
            Self::TooManyLines => "tool_parse:too_many_lines",
            Self::UnsupportedInteraction => "tool_parse:unsupported_interaction",
            Self::UnexpectedData => "tool_parse:unexpected_data",
        }
    }
}

impl fmt::Display for ToolParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ToolParseError {}

/// Incremental parser for one tool transaction.
#[derive(Clone)]
pub struct ToolResponseParser {
    /// The exact text the module echoes back for this request.
    echo: String,
    echo_seen: bool,
    /// The response prefix this command's answer normally carries, when there is one. A line
    /// starting with it is attributed to this command; anything else is collected but counted as
    /// unattributable rather than assumed to be the answer.
    expected_prefix: Option<&'static str>,
    pending: Vec<u8>,
    lines: Vec<String>,
    urc_lines: Vec<String>,
    unclassified: usize,
    total_bytes: usize,
    finished: bool,
}

impl fmt::Debug for ToolResponseParser {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolResponseParser")
            .field("echo", &"[REDACTED_TOOL_COMMAND]")
            .field("pending_bytes", &self.pending.len())
            .field("lines", &self.lines.len())
            .field("urc_lines", &self.urc_lines.len())
            .field("total_bytes", &self.total_bytes)
            .field("finished", &self.finished)
            .finish()
    }
}

impl ToolResponseParser {
    #[must_use]
    pub fn new(request: &ToolWireRequest) -> Self {
        Self {
            echo: request.line().expose_for_confirmation().to_owned(),
            echo_seen: false,
            expected_prefix: request.line().read_id().and_then(response_prefix),
            pending: Vec::new(),
            lines: Vec::new(),
            urc_lines: Vec::new(),
            unclassified: 0,
            total_bytes: 0,
            finished: false,
        }
    }

    /// Feed received bytes. Returns `Ok(Some(response))` once a final code has been seen,
    /// `Ok(None)` while the transaction is still open, and `Err` when the transaction must be
    /// abandoned.
    ///
    /// A final error code is *not* an error here: the response is returned so the caller can keep
    /// the module's own explanation and classify Rejected/Unsupported with evidence.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Option<ToolResponse>, ToolParseError> {
        if self.finished {
            return Err(ToolParseError::UnexpectedData);
        }
        self.total_bytes = self.total_bytes.saturating_add(bytes.len());
        if self.total_bytes > MAX_RESPONSE_BYTES {
            self.finished = true;
            return Err(ToolParseError::ResponseTooLarge);
        }
        for byte in bytes {
            if *byte == b'\n' {
                let line = std::mem::take(&mut self.pending);
                let line = strip_carriage_return(&line);
                if let Some(response) = self.process_line(line)? {
                    return Ok(Some(response));
                }
                continue;
            }
            self.pending.push(*byte);
            if self.pending.len() > MAX_LINE_BYTES {
                self.finished = true;
                return Err(ToolParseError::LineTooLong);
            }
        }
        // A `>` with nothing after it is the module waiting for message content. This path never
        // sends content, so the transaction ends here without writing anything else.
        if is_trailing_prompt(&self.pending) {
            self.finished = true;
            return Err(ToolParseError::UnsupportedInteraction);
        }
        Ok(None)
    }

    fn process_line(&mut self, raw: &[u8]) -> Result<Option<ToolResponse>, ToolParseError> {
        if raw.len() > MAX_LINE_BYTES {
            self.finished = true;
            return Err(ToolParseError::LineTooLong);
        }
        let Ok(line) = std::str::from_utf8(raw) else {
            // Vendor ANSI/BINARY noise is not a text response; end the transaction rather than
            // storing undecodable bytes.
            self.finished = true;
            return Err(ToolParseError::UnsupportedInteraction);
        };
        let line = line.trim_end_matches(['\r', ' ']);
        if line.is_empty() {
            return Ok(None);
        }
        if !self.echo_seen && line == self.echo {
            self.echo_seen = true;
            return Ok(None);
        }
        if is_prompt(line) {
            self.finished = true;
            return Err(ToolParseError::UnsupportedInteraction);
        }
        if is_connect(line) {
            self.finished = true;
            return Err(ToolParseError::UnsupportedInteraction);
        }
        if let Some(final_code) = parse_final_code(line) {
            self.finished = true;
            return Ok(Some(ToolResponse {
                lines: std::mem::take(&mut self.lines),
                urc_lines: std::mem::take(&mut self.urc_lines),
                unclassified_lines: self.unclassified,
                final_code,
            }));
        }
        if is_known_urc(line) {
            // Kept for the transcript, never counted as this command's answer.
            self.urc_lines.push(line.to_owned());
            return Ok(None);
        }
        if self.lines.len() + self.urc_lines.len() >= MAX_RESPONSE_LINES {
            self.finished = true;
            return Err(ToolParseError::TooManyLines);
        }
        // Text that does not carry this command's response prefix is still collected, but it is
        // counted as unattributable: the tool path cannot prove it belongs to this command, and it
        // must not pretend to be able to tell every vendor format apart.
        if !self
            .expected_prefix
            .is_some_and(|prefix| line.starts_with(prefix))
        {
            self.unclassified += 1;
        }
        self.lines.push(line.to_owned());
        Ok(None)
    }
}

/// The prefix a whitelisted read's answer normally carries. `Attention` has none: the module
/// answers `AT` with a free-form identification block, so every one of its lines stays
/// unattributable instead of being guessed at.
fn response_prefix(id: ToolReadId) -> Option<&'static str> {
    match id {
        ToolReadId::Attention => None,
        ToolReadId::Manufacturer => Some("+CGMI:"),
        ToolReadId::Model => Some("+CGMM:"),
        ToolReadId::Revision => Some("+CGMR:"),
        ToolReadId::SimState => Some("+CPIN:"),
        ToolReadId::SignalQuality => Some("+CSQ:"),
        ToolReadId::Operator => Some("+COPS:"),
        ToolReadId::EpsRegistration => Some("+CEREG:"),
        ToolReadId::PacketAttach => Some("+CGATT:"),
        ToolReadId::PdpContexts => Some("+CGDCONT:"),
        ToolReadId::PdpActivation => Some("+CGACT:"),
        ToolReadId::PdpAddresses => Some("+CGPADDR:"),
        ToolReadId::UsbNet => Some("+QCFG:"),
        ToolReadId::Temperature => Some("+QTEMP:"),
        ToolReadId::ServingCell => Some("+QENG:"),
        ToolReadId::SmsFormat => Some("+CMGF:"),
        ToolReadId::SmsStorage => Some("+CPMS:"),
    }
}

fn strip_carriage_return(line: &[u8]) -> &[u8] {
    match line.strip_suffix(b"\r") {
        Some(stripped) => stripped,
        None => line,
    }
}

/// `>` possibly followed by spaces, with nothing else.
fn is_prompt(line: &str) -> bool {
    line.strip_prefix('>')
        .is_some_and(|rest| rest.trim().is_empty())
}

/// A chunk that ends in a bare `>` before any newline: the module is waiting for content.
fn is_trailing_prompt(pending: &[u8]) -> bool {
    match pending.first() {
        Some(b'>') => pending[1..].iter().all(u8::is_ascii_whitespace),
        _ => false,
    }
}

fn is_connect(line: &str) -> bool {
    line.eq_ignore_ascii_case("CONNECT")
        || line
            .to_ascii_uppercase()
            .strip_prefix("CONNECT ")
            .is_some_and(|rest| {
                rest.trim()
                    .chars()
                    .all(|character| character.is_ascii_digit())
            })
}

/// The final codes, matched exactly as the monitoring parser does.
fn parse_final_code(line: &str) -> Option<AtFinalCode> {
    match line {
        "OK" => Some(AtFinalCode::Ok),
        "ERROR" => Some(AtFinalCode::Error),
        "NO CARRIER" => Some(AtFinalCode::NoCarrier),
        "NO ANSWER" => Some(AtFinalCode::NoAnswer),
        "BUSY" => Some(AtFinalCode::Busy),
        "NO DIALTONE" => Some(AtFinalCode::NoDialTone),
        _ => {
            // Matched exactly as the monitoring parser does, and the detail keeps the module's own
            // bytes (never upper-cased) so an error code is never transcribed into another one.
            if let Some(detail) = line.strip_prefix("+CME ERROR:") {
                return Some(AtFinalCode::CmeError(detail.trim().to_owned()));
            }
            if let Some(detail) = line.strip_prefix("+CMS ERROR:") {
                return Some(AtFinalCode::CmsError(detail.trim().to_owned()));
            }
            None
        }
    }
}
