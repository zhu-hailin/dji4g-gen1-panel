//! Closed, versioned values that may cross the panel/helper boundary.
//!
//! The wire contract deliberately does not reuse the domain action enum.  Domain actions contain
//! strings and collections intended for an in-process controller; the helper contract is a much
//! smaller allow-list with validated newtypes for every value that can affect a device operation.

use std::{
    fmt,
    net::IpAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::{MAX_CLOCK_SKEW, MAX_OPERATION_LIFETIME};

const NONCE_BYTES: usize = 32;
const REQUEST_ID_BYTES: usize = 16;
const HASH_BYTES: usize = 32;

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex<const N: usize>(value: &str) -> Result<[u8; N], String> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!("expected {N}-byte lowercase hexadecimal value"));
    }
    let mut bytes = [0_u8; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = pair[0] - if pair[0] >= b'a' { b'a' - 10 } else { b'0' };
        let low = pair[1] - if pair[1] >= b'a' { b'a' - 10 } else { b'0' };
        bytes[index] = (high << 4) | low;
    }
    Ok(bytes)
}

struct HexVisitor<const N: usize>;

impl<'de, const N: usize> de::Visitor<'de> for HexVisitor<N> {
    type Value = [u8; N];

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "a {N}-byte lowercase hexadecimal string")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        decode_hex(value).map_err(E::custom)
    }
}

/// The only protocol version currently accepted by the helper.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct ProtocolVersion(u16);

impl ProtocolVersion {
    pub const V1: Self = Self(1);

    #[must_use]
    pub const fn from_raw(value: u16) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    #[must_use]
    pub const fn is_supported(self) -> bool {
        self.0 == Self::V1.0
    }
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        Self::V1
    }
}

impl fmt::Debug for ProtocolVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProtocolVersion")
            .field(&self.0)
            .finish()
    }
}

impl Serialize for ProtocolVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u16(self.0)
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(u16::deserialize(deserializer)?))
    }
}

/// A 256-bit one-shot operation secret.  Its formatter is always redacted and its storage is
/// cleared on drop.  The command line and request each carry this value only for comparison.
pub struct OperationNonce([u8; NONCE_BYTES]);

impl OperationNonce {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; NONCE_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn random() -> Result<Self, getrandom::Error> {
        let mut bytes = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut bytes)?;
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn from_hex(value: &str) -> Option<Self> {
        decode_hex(value).ok().map(Self)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; NONCE_BYTES] {
        &self.0
    }

    #[must_use]
    pub const fn is_zero(&self) -> bool {
        let mut index = 0;
        while index < NONCE_BYTES {
            if self.0[index] != 0 {
                return false;
            }
            index += 1;
        }
        true
    }

    #[must_use]
    pub fn to_hex(&self) -> String {
        encode_hex(&self.0)
    }
}

impl Clone for OperationNonce {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

impl PartialEq for OperationNonce {
    fn eq(&self, other: &Self) -> bool {
        // The values are never used as an authentication oracle; keeping comparison constant-time
        // avoids making equality accidental timing telemetry in a future native transport.
        self.0
            .iter()
            .zip(other.0)
            .fold(0_u8, |acc, (&a, b)| acc | (a ^ b))
            == 0
    }
}

impl Eq for OperationNonce {}

impl fmt::Debug for OperationNonce {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OperationNonce([REDACTED])")
    }
}

impl Serialize for OperationNonce {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode_hex(&self.0))
    }
}

impl<'de> Deserialize<'de> for OperationNonce {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(
            deserializer.deserialize_str(HexVisitor::<NONCE_BYTES>)?,
        ))
    }
}

impl Drop for OperationNonce {
    fn drop(&mut self) {
        // Clearing is best effort and is not a substitute for process isolation or protected
        // memory.
        self.0.fill(0);
    }
}

/// A request correlation id.  Unlike [`OperationNonce`], this is not a secret, but it is still
/// fixed-width and cannot be supplied as an arbitrary string.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct RequestId([u8; REQUEST_ID_BYTES]);

impl RequestId {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; REQUEST_ID_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn random() -> Result<Self, getrandom::Error> {
        let mut bytes = [0_u8; REQUEST_ID_BYTES];
        getrandom::fill(&mut bytes)?;
        Ok(Self(bytes))
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        let mut index = 0;
        while index < REQUEST_ID_BYTES {
            if self.0[index] != 0 {
                return false;
            }
            index += 1;
        }
        true
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; REQUEST_ID_BYTES] {
        &self.0
    }
}

impl fmt::Debug for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RequestId([REDACTED])")
    }
}

impl Serialize for RequestId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode_hex(&self.0))
    }
}

impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(
            deserializer.deserialize_str(HexVisitor::<REQUEST_ID_BYTES>)?,
        ))
    }
}

/// Fixed-width state/identity digest.  Its formatter deliberately does not reveal the digest.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct Hash32([u8; HASH_BYTES]);

impl Hash32 {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; HASH_BYTES]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        let mut index = 0;
        while index < HASH_BYTES {
            if self.0[index] != 0 {
                return false;
            }
            index += 1;
        }
        true
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; HASH_BYTES] {
        &self.0
    }
}

impl fmt::Debug for Hash32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Hash32([REDACTED])")
    }
}

impl Serialize for Hash32 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode_hex(&self.0))
    }
}

impl<'de> Deserialize<'de> for Hash32 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(
            deserializer.deserialize_str(HexVisitor::<HASH_BYTES>)?,
        ))
    }
}

/// A pipe name is generated by the panel and accepted only when it matches our private grammar.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct PipeName(String);

impl PipeName {
    pub const PREFIX: &'static str = r"\\.\pipe\dji4g-panel-";

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let suffix = value.strip_prefix(Self::PREFIX)?;
        if suffix.len() != 32
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return None;
        }
        Some(Self(value.to_owned()))
    }

    pub fn random() -> Result<Self, getrandom::Error> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes)?;
        Ok(Self(format!("{}{}", Self::PREFIX, encode_hex(&bytes))))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PipeName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PipeName([REDACTED])")
    }
}

impl Serialize for PipeName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PipeName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).ok_or_else(|| de::Error::custom("invalid generated pipe name"))
    }
}

/// Milliseconds since Unix epoch used only for expiry proofs on the wire.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnixMillis(pub u64);

impl UnixMillis {
    #[must_use]
    pub fn from_system_time(time: SystemTime) -> Option<Self> {
        time.duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            .map(Self)
    }

    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportedProfileV1 {
    DjiGen1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdpContextIdV1(u8);

impl PdpContextIdV1 {
    pub fn new(value: u8) -> Result<Self, ProtocolValueError> {
        (1..=16)
            .contains(&value)
            .then_some(Self(value))
            .ok_or(ProtocolValueError::InvalidPdpContextId)
    }

    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl Serialize for PdpContextIdV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(self.0)
    }
}

impl<'de> Deserialize<'de> for PdpContextIdV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u8::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ValidatedApn(String);

pub type ValidatedApnV1 = ValidatedApn;

impl ValidatedApn {
    pub fn try_from(value: String) -> Result<Self, ProtocolValueError> {
        if value.is_empty() || value.len() > 100 || !value.is_ascii() {
            return Err(ProtocolValueError::InvalidApn);
        }
        if value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'"' || byte == b',' || byte == 0x7f)
        {
            return Err(ProtocolValueError::InvalidApn);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ValidatedApn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ValidatedApn([REDACTED])")
    }
}

impl Serialize for ValidatedApn {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ValidatedApn {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedDnsServers(Vec<IpAddr>);

impl TryFrom<Vec<IpAddr>> for BoundedDnsServers {
    type Error = ProtocolValueError;

    fn try_from(value: Vec<IpAddr>) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 3 {
            return Err(ProtocolValueError::InvalidDnsServers);
        }
        for (index, address) in value.iter().enumerate() {
            if address.is_unspecified()
                || address.is_loopback()
                || address.is_multicast()
                || is_link_local(*address)
            {
                return Err(ProtocolValueError::InvalidDnsServers);
            }
            if value[..index].contains(address) {
                return Err(ProtocolValueError::InvalidDnsServers);
            }
        }
        Ok(Self(value))
    }
}

impl BoundedDnsServers {
    #[must_use]
    pub fn as_slice(&self) -> &[IpAddr] {
        &self.0
    }
}

impl Serialize for BoundedDnsServers {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BoundedDnsServers {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(Vec::<IpAddr>::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

fn is_link_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.octets()[0] == 169 && address.octets()[1] == 254,
        IpAddr::V6(address) => (address.segments()[0] & 0xffc0) == 0xfe80,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "args",
    deny_unknown_fields,
    rename_all = "snake_case"
)]
pub enum DnsProfileV1 {
    Automatic,
    Static { servers: BoundedDnsServers },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsbNetProfileV1 {
    DjiNdis,
    Ecm,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "args",
    deny_unknown_fields,
    rename_all = "snake_case"
)]
pub enum HelperActionV1 {
    InspectTarget,
    RenewDhcp,
    ApplyDnsProfile {
        profile: DnsProfileV1,
    },
    RestartAdapter,
    ReenumerateDevice,
    RestartModule,
    EditApn {
        cid: PdpContextIdV1,
        apn: ValidatedApnV1,
    },
    SetUsbNetProfile {
        profile: UsbNetProfileV1,
    },
    ToggleHotspot {
        enabled: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetProofV1 {
    pub profile: SupportedProfileV1,
    pub epoch: u64,
    pub identity_hash: Hash32,
    pub before_state_hash: Hash32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperRequestV1 {
    pub version: ProtocolVersion,
    pub request_id: RequestId,
    pub nonce: OperationNonce,
    pub issued_at: UnixMillis,
    pub expires_at: UnixMillis,
    pub target: TargetProofV1,
    pub action: HelperActionV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestRejectCode {
    UnsupportedVersion,
    MalformedFrame,
    UnknownField,
    UnknownAction,
    FrameTooLarge,
    EmptyFrame,
    Expired,
    FutureIssuedAt,
    InvalidLifetime,
    NonceMismatch,
    RequestIdMismatch,
    SecondFrame,
    SecondClient,
    RemoteClient,
    PeerPidMismatch,
    PeerCreationChanged,
    UserMismatch,
    SessionMismatch,
    IntegrityMismatch,
    PeerImageMismatch,
    HelperUntrusted,
    UnsupportedDevice,
    TargetNotFound,
    TargetAmbiguous,
    TargetIdentityChanged,
    EpochChanged,
    BeforeStateChanged,
    PermissionDenied,
    UacCancelled,
    Timeout,
    PeerDisconnected,
    ProtocolRejected,
    Internal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerRejectCode {
    RemoteClient,
    PeerPidMismatch,
    PeerCreationChanged,
    UserMismatch,
    SessionMismatch,
    IntegrityMismatch,
    PeerImageMismatch,
    ServerPidMismatch,
    ServerImageMismatch,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationCode {
    InvalidActionArguments,
    UnsupportedDevice,
    TargetNotFound,
    TargetAmbiguous,
    TargetIdentityChanged,
    EpochChanged,
    BeforeStateChanged,
    AtPortUnavailable,
    PermissionDenied,
    OperationCancelled,
    Timeout,
    PeerDisconnected,
    VerificationFailed,
    RollbackFailed,
    ProtocolRejected,
    Internal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackResultV1 {
    NotRequired,
    Applied,
    Failed { code: OperationCode },
    NotAttempted,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationResultV1 {
    Applied {
        after_state_hash: Hash32,
        verified_at: UnixMillis,
    },
    Failed {
        code: OperationCode,
        rollback: RollbackResultV1,
    },
    OutcomeUnknown {
        code: OperationCode,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "data",
    deny_unknown_fields,
    rename_all = "snake_case"
)]
pub enum HelperResultV1 {
    Inspected { state_hash: Hash32 },
    Completed(OperationResultV1),
    Rejected { code: RequestRejectCode },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HelperResponseV1 {
    pub version: ProtocolVersion,
    pub request_id: RequestId,
    pub result: HelperResultV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolValueError {
    InvalidPdpContextId,
    InvalidApn,
    InvalidDnsServers,
}

impl fmt::Display for ProtocolValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPdpContextId => "invalid PDP context id",
            Self::InvalidApn => "invalid APN",
            Self::InvalidDnsServers => "invalid DNS server list",
        })
    }
}

impl std::error::Error for ProtocolValueError {}

/// Validates all request properties that can be proven without opening a device handle.
pub fn validate_request(
    request: &HelperRequestV1,
    command_nonce: &OperationNonce,
    now: SystemTime,
) -> Result<(), RequestRejectCode> {
    if !request.version.is_supported() {
        return Err(RequestRejectCode::UnsupportedVersion);
    }
    if request.nonce != *command_nonce {
        return Err(RequestRejectCode::NonceMismatch);
    }
    if request.nonce.is_zero() || command_nonce.is_zero() {
        return Err(RequestRejectCode::NonceMismatch);
    }
    if request.request_id.is_zero() || request.target.epoch == 0 {
        return Err(RequestRejectCode::RequestIdMismatch);
    }
    let now = UnixMillis::from_system_time(now)
        .ok_or(RequestRejectCode::Internal)?
        .0;
    if request.issued_at.0 > now.saturating_add(MAX_CLOCK_SKEW.as_millis() as u64) {
        return Err(RequestRejectCode::FutureIssuedAt);
    }
    if request.expires_at.0 < now {
        return Err(RequestRejectCode::Expired);
    }
    if request.expires_at.0 < request.issued_at.0
        || request.expires_at.0 - request.issued_at.0 > MAX_OPERATION_LIFETIME.as_millis() as u64
    {
        return Err(RequestRejectCode::InvalidLifetime);
    }
    Ok(())
}
