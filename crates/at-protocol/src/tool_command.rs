//! Command policy for the device-tool terminal.
//!
//! The terminal accepts one single-line text AT request and returns one text final response. It is
//! not a shell, not a flashing tool and not a binary transport, so the policy here is deliberately
//! closed: a line is either on the read-only whitelist, a *known* write that the existing reviewed
//! repair flow can execute, or it is only reachable through the expert path with its own frozen
//! confirmation.
//!
//! The whitelist is not a string table invented next to the implementation: every entry is tied to
//! the `AtCommand` variant that already encodes that request, and the policy tests assert that
//! `typed_read(id)` lands back on the same id. A parameter version is registered explicitly; no
//! prefix matching, no "it contains a question mark so it must be a read".

use std::fmt;

use crate::{Apn, AtCommand, PdpContextId, VerifiedUsbNetProfile};

/// Longest accepted request, in ASCII bytes. Longer lines are rejected before anything else runs.
pub const MAX_TOOL_LINE_BYTES: usize = 256;

/// A syntactically valid single-line tool request.
///
/// The text is private and `Debug` never prints it: a request may carry an APN, a phone number or
/// a vendor string, and a request/response must not reach a log or a diagnostic export through a
/// derived formatter. The UI shows it only through [`ValidatedToolLine::expose_for_confirmation`],
/// which is an explicit, reviewable call site.
#[derive(Clone, Eq, PartialEq)]
pub struct ValidatedToolLine(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolInputError {
    /// Nothing but whitespace was supplied.
    Empty,
    /// More than [`MAX_TOOL_LINE_BYTES`] ASCII bytes.
    TooLong,
    /// Any non-ASCII byte.
    NonAscii,
    /// Any ASCII control character, including CR, LF, NUL and Ctrl-Z.
    ControlCharacter,
    /// A `;` that would chain a second command.
    ChainedCommand,
    /// The line does not start with `AT`.
    InvalidPrefix,
    /// A well-formed line that the read-only whitelist does not cover.
    NotWhitelisted,
    /// A command family that opens an interaction this tool path cannot drive.
    InteractiveCommand,
}

impl ToolInputError {
    /// Stable, log-safe identifier. Never contains the offending text.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Empty => "tool_input:empty",
            Self::TooLong => "tool_input:too_long",
            Self::NonAscii => "tool_input:non_ascii",
            Self::ControlCharacter => "tool_input:control_character",
            Self::ChainedCommand => "tool_input:chained_command",
            Self::InvalidPrefix => "tool_input:invalid_prefix",
            Self::NotWhitelisted => "tool_input:not_whitelisted",
            Self::InteractiveCommand => "tool_input:interactive_command",
        }
    }
}

impl fmt::Display for ToolInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ToolInputError {}

impl fmt::Debug for ValidatedToolLine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ValidatedToolLine([REDACTED_TOOL_COMMAND])")
    }
}

/// One query this tool path is allowed to run without a per-command confirmation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ToolReadId {
    Attention,
    Manufacturer,
    Model,
    Revision,
    SimState,
    SignalQuality,
    Operator,
    EpsRegistration,
    PacketAttach,
    PdpContexts,
    PdpActivation,
    PdpAddresses,
    UsbNet,
    Temperature,
    ServingCell,
    SmsFormat,
    SmsStorage,
}

impl ToolReadId {
    /// Every whitelisted read, in the order a capability sweep should run.
    pub const ALL: [Self; 17] = [
        Self::Attention,
        Self::Manufacturer,
        Self::Model,
        Self::Revision,
        Self::SimState,
        Self::SignalQuality,
        Self::Operator,
        Self::EpsRegistration,
        Self::PacketAttach,
        Self::PdpContexts,
        Self::PdpActivation,
        Self::PdpAddresses,
        Self::UsbNet,
        Self::Temperature,
        Self::ServingCell,
        Self::SmsFormat,
        Self::SmsStorage,
    ];

    /// Stable identifier for logs, snapshots and tests.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Attention => "attention",
            Self::Manufacturer => "manufacturer",
            Self::Model => "model",
            Self::Revision => "revision",
            Self::SimState => "sim_state",
            Self::SignalQuality => "signal_quality",
            Self::Operator => "operator",
            Self::EpsRegistration => "eps_registration",
            Self::PacketAttach => "packet_attach",
            Self::PdpContexts => "pdp_contexts",
            Self::PdpActivation => "pdp_activation",
            Self::PdpAddresses => "pdp_addresses",
            Self::UsbNet => "usb_net",
            Self::Temperature => "temperature",
            Self::ServingCell => "serving_cell",
            Self::SmsFormat => "sms_format",
            Self::SmsStorage => "sms_storage",
        }
    }
}

/// A write the reviewed repair flow already knows how to execute with its own confirmation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolWriteId {
    RestartModule,
    SetApn { cid: PdpContextId, apn: Apn },
    SetUsbNetProfile(VerifiedUsbNetProfile),
}

/// Errors a tool request may carry.
///
/// `Empty`, `TooLong`, `NonAscii`, `ControlCharacter`, `ChainedCommand` and `InvalidPrefix` are
/// format errors; `NotWhitelisted` and `InteractiveCommand` are policy refusals. None of them
/// retain the input.
impl ValidatedToolLine {
    /// Validate one line of terminal input.
    ///
    /// Validation order matters: an injected control byte or a chained `;` is refused as such
    /// rather than being smuggled through a prefix or length error.
    pub fn parse(input: &str) -> Result<Self, ToolInputError> {
        if !input.is_ascii() {
            return Err(ToolInputError::NonAscii);
        }
        if input
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == 0x7f)
        {
            return Err(ToolInputError::ControlCharacter);
        }
        if input.contains(';') {
            return Err(ToolInputError::ChainedCommand);
        }
        let trimmed = input.trim_matches(' ');
        if trimmed.is_empty() {
            return Err(ToolInputError::Empty);
        }
        if trimmed.len() > MAX_TOOL_LINE_BYTES {
            return Err(ToolInputError::TooLong);
        }
        if trimmed.len() < 2 || !trimmed[..2].eq_ignore_ascii_case("AT") {
            return Err(ToolInputError::InvalidPrefix);
        }
        if is_interactive_command(trimmed) {
            return Err(ToolInputError::InteractiveCommand);
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Parse and require the read-only whitelist. The advanced terminal uses only this entry
    /// point, so a line it accepts can always be run without asking the user again.
    pub fn parse_read_only(input: &str) -> Result<(Self, ToolReadId), ToolInputError> {
        let line = Self::parse(input)?;
        let id = classify_read(&line).ok_or(ToolInputError::NotWhitelisted)?;
        Ok((line, id))
    }

    /// The exact text this request will put on the wire, for the one place that *must* show it:
    /// the confirmation the user approves. Never use it for logging or export.
    #[must_use]
    pub fn expose_for_confirmation(&self) -> &str {
        &self.0
    }

    /// True when the request text is exactly a whitelisted read. Used by the UI to decide whether
    /// the advanced tab may run it without further confirmation.
    #[must_use]
    pub fn read_id(&self) -> Option<ToolReadId> {
        classify_read(self)
    }
}

/// The whitelist: `(id, keyword, parameter text)`.
///
/// `keyword` is compared case-insensitively (AT keywords are case-insensitive) while the parameter
/// text is compared byte for byte, so `AT+qcfg="usbnet"` is the same request as
/// `AT+QCFG="usbnet"` but a vendor parameter is never silently upper-cased.
const READ_WHITELIST: [(ToolReadId, &str, &str); 17] = [
    (ToolReadId::Attention, "AT", ""),
    (ToolReadId::Manufacturer, "AT+CGMI", ""),
    (ToolReadId::Model, "AT+CGMM", ""),
    (ToolReadId::Revision, "AT+CGMR", ""),
    (ToolReadId::SimState, "AT+CPIN", "?"),
    (ToolReadId::SignalQuality, "AT+CSQ", ""),
    (ToolReadId::Operator, "AT+COPS", "?"),
    (ToolReadId::EpsRegistration, "AT+CEREG", "?"),
    (ToolReadId::PacketAttach, "AT+CGATT", "?"),
    (ToolReadId::PdpContexts, "AT+CGDCONT", "?"),
    (ToolReadId::PdpActivation, "AT+CGACT", "?"),
    (ToolReadId::PdpAddresses, "AT+CGPADDR", ""),
    (ToolReadId::UsbNet, "AT+QCFG", "=\"usbnet\""),
    (ToolReadId::Temperature, "AT+QTEMP", ""),
    (ToolReadId::ServingCell, "AT+QENG", "=\"servingcell\""),
    (ToolReadId::SmsFormat, "AT+CMGF", "?"),
    (ToolReadId::SmsStorage, "AT+CPMS", "?"),
];

/// The `AtCommand` that carries a whitelisted read. Predicates and setters stay separate commands
/// so the whitelist can never be widened by a parameter.
#[must_use]
pub const fn typed_read(id: ToolReadId) -> AtCommand {
    match id {
        ToolReadId::Attention => AtCommand::Attention,
        ToolReadId::Manufacturer => AtCommand::Manufacturer,
        ToolReadId::Model => AtCommand::Model,
        ToolReadId::Revision => AtCommand::Revision,
        ToolReadId::SimState => AtCommand::SimState,
        ToolReadId::SignalQuality => AtCommand::SignalQuality,
        ToolReadId::Operator => AtCommand::Operator,
        ToolReadId::EpsRegistration => AtCommand::EpsRegistration,
        ToolReadId::PacketAttach => AtCommand::PacketAttach,
        ToolReadId::PdpContexts => AtCommand::PdpContexts,
        ToolReadId::PdpActivation => AtCommand::PdpActivation,
        ToolReadId::PdpAddresses => AtCommand::PdpAddresses,
        ToolReadId::UsbNet => AtCommand::UsbNetQuery,
        ToolReadId::Temperature => AtCommand::Temperature,
        ToolReadId::ServingCell => AtCommand::ServingCellInfo,
        ToolReadId::SmsFormat => AtCommand::SmsMessageFormat,
        ToolReadId::SmsStorage => AtCommand::SmsStorageQuery,
    }
}

/// The whitelisted read a line denotes, if any.
///
/// A line containing `?` is *not* automatically a read: only the exact registered predicates are.
#[must_use]
pub fn classify_read(line: &ValidatedToolLine) -> Option<ToolReadId> {
    let parts = split_command(line.expose_for_confirmation())?;
    READ_WHITELIST
        .iter()
        .find(|(_, keyword, parameters)| {
            parts.keyword.eq_ignore_ascii_case(keyword) && parts.remainder == *parameters
        })
        .map(|(id, _, _)| *id)
}

/// A write the reviewed repair flow already implements, recovered from a terminal line.
///
/// Only the shapes the existing typed requests accept are recognised; an out-of-range CID, an
/// unknown `usbnet` value or an unquoted `CGDCONT` stays unrecognised and therefore needs the
/// expert path's own confirmation instead of silently becoming a different write.
#[must_use]
pub fn classify_known_write(line: &ValidatedToolLine) -> Option<ToolWriteId> {
    let parts = split_command(line.expose_for_confirmation())?;
    if parts.keyword.eq_ignore_ascii_case("AT+CFUN") {
        return (parts.remainder == "=1,1").then_some(ToolWriteId::RestartModule);
    }
    if parts.keyword.eq_ignore_ascii_case("AT+CGDCONT") {
        return parse_cgdcont_write(parts.remainder);
    }
    if parts.keyword.eq_ignore_ascii_case("AT+QCFG") {
        let value = parts.remainder.strip_prefix("=\"usbnet\",")?;
        let value: u8 = value.parse().ok()?;
        return VerifiedUsbNetProfile::from_raw(value).map(ToolWriteId::SetUsbNetProfile);
    }
    None
}

fn parse_cgdcont_write(remainder: &str) -> Option<ToolWriteId> {
    let body = remainder.strip_prefix('=')?;
    let fields = split_quoted_fields(body)?;
    if fields.len() != 3 {
        return None;
    }
    let cid = PdpContextId::try_from(fields[0].0.parse::<u8>().ok()?).ok()?;
    // The canonical encoding quotes both string parameters; an unquoted form is a different line
    // and stays outside the registered set.
    if !fields[1].1 || !fields[2].1 {
        return None;
    }
    // The type is the module's vocabulary and is normalised; the APN is user data and is kept
    // exactly as typed.
    if !matches!(
        fields[1].0.to_ascii_uppercase().as_str(),
        "IP" | "IPV6" | "IPV4V6"
    ) {
        return None;
    }
    let apn = Apn::try_from(fields[2].0.as_str()).ok()?;
    Some(ToolWriteId::SetApn { cid, apn })
}

/// Split `1,"IP","apn"` into `(value, was_quoted)` fields, removing the surrounding quotes.
/// Returns `None` for unbalanced quoting so a half-understood line is never treated as a known
/// write.
fn split_quoted_fields(body: &str) -> Option<Vec<(String, bool)>> {
    let mut fields: Vec<(String, bool)> = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut was_quoted = false;
    let mut chars = body.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '"' => {
                // A doubled quote inside a quoted field is the usual escaping; anything else must
                // open or close at a field boundary.
                if quoted && chars.peek() == Some(&'"') {
                    chars.next();
                    current.push('"');
                    continue;
                }
                if !quoted && !current.is_empty() {
                    // A quote in the middle of a bare field is malformed.
                    return None;
                }
                quoted = !quoted;
                was_quoted = true;
            }
            ',' if !quoted => {
                fields.push((std::mem::take(&mut current), was_quoted));
                was_quoted = false;
            }
            _ => current.push(character),
        }
    }
    if quoted {
        return None;
    }
    fields.push((current, was_quoted));
    Some(fields)
}

struct CommandParts<'a> {
    /// The command keyword including `AT`, e.g. `AT+QCFG`, exactly as written.
    keyword: &'a str,
    /// Everything after the keyword: `?`, `=1,"IP","x"`, `="usbnet"`, or empty.
    remainder: &'a str,
}

/// Split a validated line into its keyword and the rest, without touching the parameter text.
fn split_command(line: &str) -> Option<CommandParts<'_>> {
    // `ValidatedToolLine::parse` guarantees an ASCII line starting with `AT`, so byte offsets are
    // character offsets here.
    let rest = line.get(2..)?;
    let after_plus = match rest.strip_prefix('+') {
        Some(rest) => rest,
        None => rest,
    };
    let keyword_len = after_plus
        .find(|character: char| !character.is_ascii_alphanumeric())
        .unwrap_or(after_plus.len());
    let keyword_end = line.len() - after_plus.len() + keyword_len;
    Some(CommandParts {
        keyword: &line[..keyword_end],
        remainder: &line[keyword_end..],
    })
}

/// Command families whose request/response is an interaction (a `>` prompt, a `CONNECT` data
/// state, or a firmware/flash transfer) rather than one text query.
///
/// Matching is on the command word, so `AT+CMGS=12` and `AT+CMGS=?` are refused while an unrelated
/// command that merely starts with the same characters is not.
const REJECTED_COMMAND_WORDS: [&str; 14] = [
    "+CMGS",
    "+CMGW",
    "+CMGC",
    "+CMSS",
    "+CSMP",
    "+QFUPL",
    "+QFDOWNLOAD",
    "+QFOPEN",
    "+QFWRITE",
    "+QFREAD",
    "+QFCLOSE",
    "+QFLDS",
    "+QFSAVE",
    "+QFOTADL",
];

/// True for the dial and data-mode families, which take the port out of command mode.
fn is_dial_or_data_command(line: &str) -> bool {
    let rest = &line[2..];
    if rest.is_empty() {
        return false;
    }
    // Basic D/O/A commands include optional parameters (e.g. ATDL, ATDT..., ATO0).
    // Reject the whole basic family; extended commands begin with '+' and are unaffected.
    matches!(rest.as_bytes()[0].to_ascii_uppercase(), b'D' | b'O' | b'A')
}

fn is_interactive_command(line: &str) -> bool {
    // A bare `+++` cannot reach here (it does not start with AT) but is refused all the same if a
    // future prefix rule ever lets it through.
    if line == "+++" {
        return true;
    }
    if is_dial_or_data_command(line) {
        return true;
    }
    let Some(parts) = split_command(line) else {
        return false;
    };
    let candidate = parts.keyword[2..].to_ascii_uppercase();
    REJECTED_COMMAND_WORDS.contains(&candidate.as_str())
}

impl VerifiedUsbNetProfile {
    /// The profile a `AT+QCFG="usbnet",<value>` line selects, if the value is one this build has
    /// verified. Unknown values stay unknown rather than being rounded to a nearby profile.
    #[must_use]
    pub const fn from_raw(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::DjiNdis),
            1 => Some(Self::Ecm),
            _ => None,
        }
    }
}
