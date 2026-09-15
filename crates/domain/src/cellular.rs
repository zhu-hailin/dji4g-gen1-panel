use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SimState {
    Ready,
    Missing,
    PinRequired,
    PukRequired,
    Rejected,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RegistrationState {
    RegisteredHome,
    RegisteredRoaming,
    Searching,
    Denied,
    NotRegistered,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AttachState {
    Attached,
    Detached,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CellularBlock {
    SimRejected,
    RegistrationRejected,
}

/// Capability/availability status of one optional feature (research document §8.1).
///
/// `UnsupportedConfirmed` requires interpretable evidence, not "an ERROR appeared once";
/// `Empty` is a successful command with no current data (e.g. CNUM empty OK) and must never be
/// cached as "the device cannot do this". Module capability is scoped to the model/firmware/USB
/// combination; SIM/operator-dependent availability is recorded separately.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum FeatureStatus {
    #[default]
    NotProbed,
    Supported,
    Empty,
    UnsupportedConfirmed,
    TemporarilyUnavailable,
    FormatMismatch,
    TransportFailure,
}

/// One reported phone number (AT+CNUM). The plaintext number never leaves this type without an
/// explicit user action, and serialization always emits the masked form so diagnostics exports
/// can never carry the plaintext.
#[derive(Clone, Eq, PartialEq)]
pub struct PhoneNumber {
    number: String,
    /// Type-of-address as reported by the device (145 international, 129 unknown, 161 national
    /// per the vendor manual); preserved raw, never guessed.
    pub toa: u8,
}

impl PhoneNumber {
    /// Construct from a device-reported value. Validated by the AT parser; this constructor is
    /// for tests and domain-internal reuse only.
    #[must_use]
    pub fn new(number: impl Into<String>, toa: u8) -> Self {
        Self {
            number: number.into(),
            toa,
        }
    }

    #[must_use]
    pub fn masked(&self) -> String {
        let count = self.number.chars().count();
        if count <= 4 {
            return "****".to_owned();
        }
        format!(
            "****{}",
            self.number.chars().skip(count - 4).collect::<String>()
        )
    }

    /// The plaintext number, only for explicit user-driven display/copy after the SIM epoch has
    /// been re-confirmed. Never feed this to logs, diagnostics exports, or crash contexts.
    #[must_use]
    pub fn expose_after_user_action(&self) -> &str {
        &self.number
    }

    #[must_use]
    pub fn is_masked_only(&self) -> bool {
        self.number.starts_with("****")
    }
}

impl std::fmt::Debug for PhoneNumber {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PhoneNumber")
            .field("number", &"[REDACTED]")
            .field("toa", &self.toa)
            .finish()
    }
}

impl Serialize for PhoneNumber {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Exports carry the masked form only; the plaintext stays UI-local.
        serializer.serialize_str(&self.masked())
    }
}

impl<'de> Deserialize<'de> for PhoneNumber {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Deserialized values must be already-masked export data; a plaintext-looking value is
        // rejected rather than silently rehydrated (production never round-trips numbers).
        let masked = String::deserialize(deserializer)?;
        if !masked.starts_with("****") {
            return Err(serde::de::Error::custom(
                "plaintext phone numbers are not deserialized",
            ));
        }
        Ok(Self::new(masked, 0))
    }
}

/// Number lookup result: `Empty` is a successful command with no records — never a device fault.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NumberLookup {
    Empty,
    Reported(Vec<PhoneNumber>),
}

/// SIM identity used to detect card changes (research document §4.3 `sim_epoch`). Only the
/// masked ICCID and a derived fingerprint travel in snapshots; the plaintext ICCID never leaves
/// the AT boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimIdentity {
    /// Masked display form (e.g. `8986…0123`).
    pub iccid_masked: String,
    /// Stable fingerprint derived from the full ICCID, used for epoch comparison; not a secret,
    /// but not the plaintext either.
    pub fingerprint: [u8; 8],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellularSnapshot {
    pub sim: SimState,
    pub registration: RegistrationState,
    pub attached: AttachState,
    pub carrier: Option<String>,
    pub radio_access_technology: Option<String>,
    pub signal_rssi_dbm: Option<i16>,
    pub apn: Option<String>,
    pub pdp_address: Option<String>,
    /// PDP context activation state of the primary context.
    #[serde(default)]
    pub pdp_state: Option<String>,
    /// Module firmware identity from AT+CGMR.
    #[serde(default)]
    pub firmware: Option<String>,
    /// Serving-cell measurements from AT+QENG (LTE); None = not reported this cycle.
    #[serde(default)]
    pub serving_cell: Option<ServingCell>,
    /// SIM identity when readable (AT+QCCID); None while unreadable/unsupported. Drives the
    /// `sim_epoch` invalidation in the application layer.
    #[serde(default)]
    pub sim_identity: Option<SimIdentity>,
    /// Phone numbers reported by AT+CNUM; masked on export.
    #[serde(default)]
    pub numbers: Option<NumberLookup>,
    /// Module temperature in whole degrees Celsius from AT+QTEMP (candidate command: sensor
    /// names and thresholds are firmware-defined — never treated as case temperature).
    #[serde(default)]
    pub temperature_celsius: Option<i16>,
    /// Classification of the temperature probe (research §8.1).
    #[serde(default)]
    pub temperature_status: FeatureStatus,
}

/// One LTE serving-cell report (research document §2.2 / appendix A — the 18-field layout,
/// `cellid` at index 6, hex). All fields degrade to `None` when the module reports the cell as
/// absent (`-` or a state-only report); nothing here is ever fabricated. SINR is kept raw: its
/// scaling is firmware-profile dependent and unconfirmed for this device, so the UI must not
/// display it as a plain dB value yet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServingCell {
    /// Registration/search state (CONNECT / NOCONN / SEARCH / LIMSRV). `SEARCH`/`LIMSRV` are
    /// valid states, not malformed responses; `NOCONN` means registered but idle.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub duplex: Option<String>,
    pub rat: Option<String>,
    #[serde(default)]
    pub mcc: Option<String>,
    /// MNC kept as a string so leading zeros survive.
    #[serde(default)]
    pub mnc: Option<String>,
    /// Hexadecimal Cell ID, parsed from the reference layout index 6.
    #[serde(default)]
    pub cell_id: Option<u32>,
    pub pci: Option<u16>,
    pub earfcn: Option<u32>,
    pub band: Option<u32>,
    #[serde(default)]
    pub ul_mhz: Option<f32>,
    #[serde(default)]
    pub dl_mhz: Option<f32>,
    /// Hexadecimal TAC.
    #[serde(default)]
    pub tac: Option<u16>,
    pub rsrp_dbm: Option<i16>,
    pub rsrq_db: Option<i16>,
    #[serde(default)]
    pub rssi_dbm: Option<i16>,
    /// Raw SINR value; unit/scale unconfirmed for this firmware profile (research §5.2).
    #[serde(default, rename = "sinr_db")]
    pub sinr_raw: Option<i16>,
    #[serde(default)]
    pub srxlev_raw: Option<i16>,
}
