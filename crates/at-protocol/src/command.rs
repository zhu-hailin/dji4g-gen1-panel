use std::fmt;

use crate::{Apn, PdpContextId, VerifiedUsbNetProfile};

#[derive(Clone, Eq, PartialEq)]
pub enum AtCommand {
    Attention,
    Identity,
    Manufacturer,
    Model,
    Revision,
    SimState,
    SignalQuality,
    Operator,
    ServingCellInfo,
    EpsRegistration,
    PacketAttach,
    PdpContexts,
    PdpActivation,
    PdpAddresses,
    UsbNetQuery,
    ExtendedError,
    /// Subscriber phone numbers (AT+CNUM). Read-only but subscriber identity; empty OK is a
    /// normal outcome, not a device fault.
    SubscriberNumber,
    /// SIM ICCID (AT+QCCID). Read-only, subscriber identity; drives `sim_epoch` detection.
    Iccid,
    /// SMS message format query (AT+CMGF?). Configuration read; 实机未验证.
    SmsMessageFormat,
    /// Switch the module to PDU mode (AT+CMGF=0; per 3GPP TS 27.005 §3.2.2, 0 is PDU and 1 is
    /// text). Session setting, never retried.
    SmsSetPduMode,
    /// SMS storage/capacity query (AT+CPMS?). 实机未验证.
    SmsStorageQuery,
    /// Supported SMS storage names (AT+CPMS=?).
    SmsStorageCapabilities,
    /// Select mem1 only using closed wire tokens. mem2/mem3 are omitted.
    SmsSelectStorage {
        storage: dji4g_domain::SmsReadStorage,
    },
    /// List stored messages in PDU mode (AT+CMGL=4). Reading may mark messages as read
    /// (research §6.2), so it is never retried.
    SmsList,
    /// Read one stored message (AT+CMGR=<index>). Reading may mark it as read (research §6.2).
    SmsRead {
        index: u32,
    },
    /// Delete one stored message (AT+CMGD=<index>).
    SmsDelete {
        index: u32,
    },
    /// `AT+CMGS=<tpdu_octets>`: submit one already-encoded SMS-SUBMIT TPDU. The PDU body follows
    /// the `> ` prompt, never this command line (research §6.3); never retried.
    SmsSend {
        tpdu_octets: usize,
    },
    /// Module temperature query (AT+QTEMP). Sensor layout is firmware-defined (research §7.5).
    Temperature,
    RestartModule,
    SetApn {
        cid: PdpContextId,
        apn: Apn,
    },
    SetUsbNetProfile(VerifiedUsbNetProfile),
}

impl fmt::Debug for AtCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attention => formatter.write_str("Attention"),
            Self::Identity => formatter.write_str("Identity"),
            Self::Manufacturer => formatter.write_str("Manufacturer"),
            Self::Model => formatter.write_str("Model"),
            Self::Revision => formatter.write_str("Revision"),
            Self::SimState => formatter.write_str("SimState"),
            Self::SignalQuality => formatter.write_str("SignalQuality"),
            Self::Operator => formatter.write_str("Operator"),
            Self::ServingCellInfo => formatter.write_str("ServingCellInfo"),
            Self::EpsRegistration => formatter.write_str("EpsRegistration"),
            Self::PacketAttach => formatter.write_str("PacketAttach"),
            Self::PdpContexts => formatter.write_str("PdpContexts"),
            Self::PdpActivation => formatter.write_str("PdpActivation"),
            Self::PdpAddresses => formatter.write_str("PdpAddresses"),
            Self::UsbNetQuery => formatter.write_str("UsbNetQuery"),
            Self::ExtendedError => formatter.write_str("ExtendedError"),
            Self::SubscriberNumber => formatter.write_str("SubscriberNumber"),
            Self::Iccid => formatter.write_str("Iccid"),
            Self::SmsMessageFormat => formatter.write_str("SmsMessageFormat"),
            Self::SmsSetPduMode => formatter.write_str("SmsSetPduMode"),
            Self::SmsStorageQuery => formatter.write_str("SmsStorageQuery"),
            Self::SmsStorageCapabilities => formatter.write_str("SmsStorageCapabilities"),
            Self::SmsSelectStorage { storage } => formatter
                .debug_struct("SmsSelectStorage")
                .field("storage", storage)
                .finish(),
            Self::SmsList => formatter.write_str("SmsList"),
            Self::SmsRead { index } => formatter
                .debug_struct("SmsRead")
                .field("index", index)
                .finish(),
            Self::SmsDelete { index } => formatter
                .debug_struct("SmsDelete")
                .field("index", index)
                .finish(),
            Self::SmsSend { tpdu_octets } => formatter
                .debug_struct("SmsSend")
                .field("tpdu_octets", tpdu_octets)
                .finish(),
            Self::Temperature => formatter.write_str("Temperature"),
            Self::RestartModule => formatter.write_str("RestartModule"),
            Self::SetApn { cid, .. } => formatter
                .debug_struct("SetApn")
                .field("cid", cid)
                .field("apn", &"[REDACTED_APN]")
                .finish(),
            Self::SetUsbNetProfile(profile) => formatter
                .debug_tuple("SetUsbNetProfile")
                .field(profile)
                .finish(),
        }
    }
}

impl AtCommand {
    #[must_use]
    pub fn encode(&self) -> EncodedAtCommand {
        EncodedAtCommand {
            command: self.clone(),
            bytes: self.wire_bytes(),
        }
    }

    #[must_use]
    pub const fn is_write(&self) -> bool {
        matches!(
            self,
            Self::RestartModule
                | Self::SetApn { .. }
                | Self::SetUsbNetProfile(_)
                | Self::SmsSetPduMode
                | Self::SmsSelectStorage { .. }
                | Self::SmsDelete { .. }
                | Self::SmsSend { .. }
        )
    }

    /// Side-effect class for authorization, retry and transaction policy (research §8.2).
    /// `PureRead` still carries a sensitivity: read-only does not make a value public.
    #[must_use]
    pub const fn effect(&self) -> Effect {
        match self {
            Self::Attention
            | Self::Identity
            | Self::Manufacturer
            | Self::Model
            | Self::Revision
            | Self::SimState
            | Self::SignalQuality
            | Self::Operator
            | Self::ServingCellInfo
            | Self::EpsRegistration
            | Self::PacketAttach
            | Self::PdpContexts
            | Self::PdpActivation
            | Self::PdpAddresses
            | Self::UsbNetQuery
            | Self::ExtendedError
            | Self::SubscriberNumber
            | Self::Iccid
            | Self::SmsMessageFormat
            | Self::SmsStorageQuery
            | Self::SmsStorageCapabilities
            | Self::SmsList
            | Self::Temperature => Effect::PureRead,
            Self::RestartModule | Self::SetUsbNetProfile(_) => Effect::ConnectivityChange,
            Self::SetApn { .. }
            | Self::SmsSetPduMode
            | Self::SmsSelectStorage { .. }
            | Self::SmsDelete { .. } => Effect::SessionSetting,
            Self::SmsRead { .. } => Effect::ReadMayMarkRead,
            Self::SmsSend { .. } => Effect::NetworkTransaction,
        }
    }

    /// Sensitivity class for logging/export redaction (research §8.2).
    #[must_use]
    pub const fn sensitivity(&self) -> Sensitivity {
        match self {
            Self::SubscriberNumber | Self::Iccid => Sensitivity::SubscriberIdentity,
            Self::SetApn { .. } => Sensitivity::Credential,
            // The response may carry a message body; even the mode switch stays message-level
            // so nothing from the transaction can reach a log (research §8.2).
            Self::SmsSetPduMode
            | Self::SmsList
            | Self::SmsRead { .. }
            | Self::SmsDelete { .. }
            | Self::SmsSend { .. } => Sensitivity::MessageContent,
            _ => Sensitivity::Public,
        }
    }

    #[must_use]
    pub const fn retry_policy(&self) -> RetryPolicy {
        match self {
            // Reads may mutate unread state and deletes are writes; repeated transactions are
            // never assumed harmless (research §6.2).
            Self::SmsSetPduMode
            | Self::SmsList
            | Self::SmsRead { .. }
            | Self::SmsDelete { .. }
            | Self::SmsSend { .. } => RetryPolicy::Never,
            _ => {
                if self.is_write() {
                    RetryPolicy::Never
                } else {
                    RetryPolicy::OnceAfterQuietPeriod
                }
            }
        }
    }

    pub(crate) fn wire_bytes(&self) -> Vec<u8> {
        let body = match self {
            Self::Attention => "AT".to_owned(),
            Self::Identity => "ATI".to_owned(),
            Self::Manufacturer => "AT+CGMI".to_owned(),
            Self::Model => "AT+CGMM".to_owned(),
            Self::Revision => "AT+CGMR".to_owned(),
            Self::SimState => "AT+CPIN?".to_owned(),
            Self::SignalQuality => "AT+CSQ".to_owned(),
            Self::Operator => "AT+COPS?".to_owned(),
            Self::ServingCellInfo => "AT+QENG=\"servingcell\"".to_owned(),
            Self::EpsRegistration => "AT+CEREG?".to_owned(),
            Self::PacketAttach => "AT+CGATT?".to_owned(),
            Self::PdpContexts => "AT+CGDCONT?".to_owned(),
            Self::PdpActivation => "AT+CGACT?".to_owned(),
            Self::PdpAddresses => "AT+CGPADDR".to_owned(),
            Self::UsbNetQuery => "AT+QCFG=\"usbnet\"".to_owned(),
            Self::ExtendedError => "AT+CEER".to_owned(),
            Self::SubscriberNumber => "AT+CNUM".to_owned(),
            Self::Iccid => "AT+QCCID".to_owned(),
            Self::SmsMessageFormat => "AT+CMGF?".to_owned(),
            Self::SmsSetPduMode => "AT+CMGF=0".to_owned(),
            Self::SmsStorageQuery => "AT+CPMS?".to_owned(),
            Self::SmsStorageCapabilities => "AT+CPMS=?".to_owned(),
            Self::SmsSelectStorage { storage } => format!("AT+CPMS=\"{}\"", storage.as_str()),
            Self::SmsList => "AT+CMGL=4".to_owned(),
            Self::SmsRead { index } => format!("AT+CMGR={index}"),
            Self::SmsDelete { index } => format!("AT+CMGD={index}"),
            Self::SmsSend { tpdu_octets } => format!("AT+CMGS={tpdu_octets}"),
            Self::Temperature => "AT+QTEMP".to_owned(),
            Self::RestartModule => "AT+CFUN=1,1".to_owned(),
            Self::SetApn { cid, apn } => {
                format!("AT+CGDCONT={},\"IP\",\"{}\"", cid.get(), apn.as_str())
            }
            Self::SetUsbNetProfile(profile) => {
                format!("AT+QCFG=\"usbnet\",{}", profile.raw_value())
            }
        };

        let mut bytes = body.into_bytes();
        bytes.push(b'\r');
        bytes
    }
}

/// Side-effect class of a command (research document §8.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    PureRead,
    SessionSetting,
    ReadMayMarkRead,
    NetworkTransaction,
    ConnectivityChange,
    SimSecurityChange,
}

/// Privacy sensitivity of a command's data (research document §8.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sensitivity {
    Public,
    SubscriberIdentity,
    MessageContent,
    Credential,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryPolicy {
    Never,
    OnceAfterQuietPeriod,
}

#[derive(Clone, Eq, PartialEq)]
pub struct EncodedAtCommand {
    command: AtCommand,
    bytes: Vec<u8>,
}

impl fmt::Debug for EncodedAtCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodedAtCommand")
            .field("command", &self.command)
            .field("byte_len", &self.bytes.len())
            .finish()
    }
}

impl EncodedAtCommand {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn command(&self) -> &AtCommand {
        &self.command
    }
}
