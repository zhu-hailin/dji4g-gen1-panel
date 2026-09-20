use std::fmt;

use dji4g_domain::{DeviceEpoch, ErrorCode};

use crate::{AtCommand, redact_at_text, redact_at_transaction_line};

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct Apn(String);

impl Apn {
    pub const MAX_LEN: usize = 100;

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for Apn {
    type Error = ApnError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        validate_apn(value)?;
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for Apn {
    type Error = ApnError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_apn(&value)?;
        Ok(Self(value))
    }
}

fn validate_apn(value: &str) -> Result<(), ApnError> {
    if value.is_empty() {
        return Err(ApnError::Empty);
    }
    if value.len() > Apn::MAX_LEN {
        return Err(ApnError::TooLong);
    }
    if !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b'"' | b',' | b';'))
    {
        return Err(ApnError::UnsafeCharacter);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApnError {
    Empty,
    TooLong,
    UnsafeCharacter,
}

impl fmt::Display for ApnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ApnError {}

impl ApnError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Empty => "apn:empty",
            Self::TooLong => "apn:too_long",
            Self::UnsafeCharacter => "apn:unsafe_character",
        }
    }
}

impl fmt::Debug for Apn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Apn([REDACTED_APN])")
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PdpContextId(u8);

impl PdpContextId {
    pub const MIN: u8 = 1;
    pub const MAX: u8 = 16;

    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for PdpContextId {
    type Error = PdpContextIdError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(PdpContextIdError)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdpContextIdError;

impl fmt::Display for PdpContextIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("pdp_context_id:out_of_range")
    }
}

impl std::error::Error for PdpContextIdError {}

/// The PDP types accepted by the controlled APN workflow.
///
/// The modem may report other vendor-specific values, but accepting an unknown type would make
/// the APN write contract ambiguous.  Such contexts are therefore rejected by the complete
/// parser instead of being represented as an opaque string.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PdpType {
    Ip,
    Ipv6,
    Ipv4v6,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdpContextState {
    Active,
    Inactive,
}

/// A fully parsed `+CGDCONT` entry with its activation state.
///
/// APN data remains redacted in diagnostics; callers that need to issue a typed write can use the
/// private value through [`Self::apn`], but must not log the returned string.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct PdpContext {
    cid: PdpContextId,
    pdp_type: PdpType,
    apn: Apn,
    state: PdpContextState,
}

impl PdpContext {
    #[must_use]
    pub const fn cid(&self) -> PdpContextId {
        self.cid
    }

    #[must_use]
    pub const fn pdp_type(&self) -> PdpType {
        self.pdp_type
    }

    #[must_use]
    pub fn apn(&self) -> &Apn {
        &self.apn
    }

    #[must_use]
    pub const fn state(&self) -> PdpContextState {
        self.state
    }

    pub(crate) fn with_state(mut self, state: PdpContextState) -> Self {
        self.state = state;
        self
    }

    pub(crate) fn new(
        cid: PdpContextId,
        pdp_type: PdpType,
        apn: Apn,
        state: PdpContextState,
    ) -> Self {
        Self {
            cid,
            pdp_type,
            apn,
            state,
        }
    }
}

impl fmt::Debug for PdpContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PdpContext")
            .field("cid", &self.cid)
            .field("pdp_type", &self.pdp_type)
            .field("apn", &"[REDACTED_APN]")
            .field("state", &self.state)
            .finish()
    }
}

/// One module temperature reading from `AT+QTEMP` (research §7.5).
///
/// The sensor layout is firmware-defined.  Some modules name every channel
/// (`+QTEMP: "modem",41`); the DJI Gen-1 module reports an unnamed positional list
/// (`+QTEMP: 57,51,51`).  An unnamed channel keeps `None` here — the position is the only identity
/// the device gave, so nothing is invented and the UI labels such a channel by index instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SensorTemperature {
    /// Firmware channel name, exactly when the module reports one.
    pub name: Option<String>,
    /// Reported degrees Celsius, exactly as sent.
    pub celsius: i16,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VerifiedUsbNetProfile {
    DjiNdis,
    Ecm,
}

impl VerifiedUsbNetProfile {
    pub(crate) const fn raw_value(self) -> u8 {
        match self {
            Self::DjiNdis => 0,
            Self::Ecm => 1,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AtFinalCode {
    Ok,
    Error,
    CmeError(String),
    CmsError(String),
    NoCarrier,
    NoAnswer,
    Busy,
    NoDialTone,
}

#[derive(Clone, Eq, PartialEq)]
pub struct AtResponse {
    pub epoch: DeviceEpoch,
    pub command: AtCommand,
    pub lines: Vec<String>,
    pub final_code: AtFinalCode,
}

impl fmt::Debug for AtResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted_lines: Vec<_> = self
            .lines
            .iter()
            .map(|line| redact_at_transaction_line(&self.command, line))
            .collect();
        formatter
            .debug_struct("AtResponse")
            .field("epoch", &self.epoch)
            .field("command", &self.command)
            .field("lines", &redacted_lines)
            .field("final_code", &self.final_code)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct AtUrc {
    pub epoch: DeviceEpoch,
    pub line: String,
}

impl fmt::Debug for AtUrc {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AtUrc")
            .field("epoch", &self.epoch)
            .field("line", &redact_at_text(&self.line))
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum AtEvent {
    Urc(AtUrc),
    Response(AtResponse),
    /// The `>` input prompt seen during a PDU send transaction (research §6.3). The parser
    /// still belongs to the device epoch that created it.
    Prompt,
}

impl fmt::Debug for AtEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Urc(urc) => formatter.debug_tuple("Urc").field(urc).finish(),
            Self::Response(response) => formatter.debug_tuple("Response").field(response).finish(),
            Self::Prompt => formatter.write_str("Prompt"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolErrorKind {
    WrongPortData,
    LineTooLong,
    ResponseTooLarge,
    Timeout,
    DeviceRemoved,
    UnexpectedData,
}

impl ProtocolErrorKind {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::WrongPortData => "at_protocol:wrong_port_data",
            Self::LineTooLong => "at_protocol:line_too_long",
            Self::ResponseTooLarge => "at_protocol:response_too_large",
            Self::Timeout => "at_protocol:timeout",
            Self::DeviceRemoved => "at_protocol:device_removed",
            Self::UnexpectedData => "at_protocol:unexpected_data",
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ProtocolError {
    pub code: ErrorCode,
    pub kind: ProtocolErrorKind,
}

impl ProtocolError {
    pub(crate) const fn verification(kind: ProtocolErrorKind) -> Self {
        Self {
            code: ErrorCode::VerificationFailed,
            kind,
        }
    }

    pub(crate) const fn timeout() -> Self {
        Self {
            code: ErrorCode::Timeout,
            kind: ProtocolErrorKind::Timeout,
        }
    }

    pub(crate) const fn device_removed() -> Self {
        Self {
            code: ErrorCode::DeviceRemoved,
            kind: ProtocolErrorKind::DeviceRemoved,
        }
    }
}

impl fmt::Debug for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtocolError")
            .field("code", &self.code)
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.code())
    }
}

impl std::error::Error for ProtocolError {}
