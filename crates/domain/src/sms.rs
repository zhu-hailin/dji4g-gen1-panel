//! SMS message model (research document §6). Message bodies and sender addresses are sensitive:
//! `Debug` and `Serialize` never emit their plaintext — the UI reads them through explicit
//! accessors, and diagnostics exports exclude the whole message store.

use serde::{Deserialize, Serialize};

use crate::FeatureStatus;

/// Storage class a message lives in (3GPP TS 27.005 `CPMS`). Kept as an open string: the
/// firmware may report holders beyond `SM`/`ME` (e.g. `MT`), and none of them may be guessed at.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SmsStorageId(pub String);

/// Message encoding actually decoded by the PDU codec.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SmsEncoding {
    Gsm7,
    Ucs2,
    /// A DCS the codec does not model; the body stays unset and the UI says so.
    Other,
}

/// Long-message concatenation header (3GPP TS 23.040 §9.2.3.24).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SmsMultipartInfo {
    /// Concatenated reference number (may repeat across windows).
    pub reference: u8,
    /// Total number of fragments.
    pub total: u8,
    /// One-based fragment sequence.
    pub sequence: u8,
}

/// Lifecycle of a stored message from this application's point of view.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SmsStatus {
    /// Received and complete (single-fragment or fully reassembled).
    Received,
    /// Received but the long message is incomplete (missing/ordered fragments).
    Incomplete,
    Submitted,
    Failed,
    OutcomeUnknown,
}

/// Direction of one stored message relative to this application (research document §6.3).
///
/// Outgoing records are local bookkeeping of a user-confirmed submission: the module reports no
/// storage index for them, so their `index` is a locally assigned transaction identifier used only
/// to keep repeated sends distinct, and their `sender` slot carries the recipient.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SmsDirection {
    Incoming,
    Outgoing,
}

/// One SMS message as held by the application store.
///
/// The sender and body are private; `Debug` redacts both and `Serialize` skips them, so a
/// diagnostics export can never carry message content (research §6.2, §8.4).
#[derive(Clone, Eq, PartialEq)]
pub struct SmsMessage {
    /// Storage index as reported by the module (indices are reused; never a stable identity).
    pub index: u32,
    pub storage: SmsStorageId,
    /// Device epoch active when the message was read.
    pub device_epoch: u64,
    /// SIM epoch active when the message was read.
    pub sim_epoch: u64,
    sender: String,
    body: String,
    /// Service-centre timestamp as reported in the PDU (not system time).
    pub service_centre_timestamp: Option<String>,
    pub encoding: SmsEncoding,
    pub multipart: Option<SmsMultipartInfo>,
    /// `None` while the read state is unknown; reading may itself mark a message as read
    /// (research §6.2), so this is evidence, never a guarantee.
    pub read: Option<bool>,
    pub status: SmsStatus,
    /// Whether the module delivered this message to us or we submitted it.
    pub direction: SmsDirection,
}

impl SmsMessage {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index: u32,
        storage: SmsStorageId,
        device_epoch: u64,
        sim_epoch: u64,
        sender: impl Into<String>,
        body: impl Into<String>,
        encoding: SmsEncoding,
        status: SmsStatus,
    ) -> Self {
        Self {
            index,
            storage,
            device_epoch,
            sim_epoch,
            sender: sender.into(),
            body: body.into(),
            service_centre_timestamp: None,
            encoding,
            multipart: None,
            read: None,
            status,
            direction: SmsDirection::Incoming,
        }
    }

    /// Construct a locally recorded outgoing submission.
    ///
    /// `transaction_id` occupies the index slot: the module reports no storage index for a message
    /// we sent, and repeated sends of identical text must stay distinct records, so this locally
    /// assigned identifier is what the store deduplicates on. The recipient is stored in the
    /// sender slot and is only ever displayed masked. The platform's PDU codec picks the encoding
    /// internally and the send receipt does not report it, so callers pass [`SmsEncoding::Other`]
    /// unless they have better evidence.
    #[must_use]
    pub fn new_outgoing(
        transaction_id: u32,
        device_epoch: u64,
        sim_epoch: u64,
        recipient: impl Into<String>,
        body: impl Into<String>,
        encoding: SmsEncoding,
        status: SmsStatus,
    ) -> Self {
        Self {
            index: transaction_id,
            storage: SmsStorageId("LOCAL".to_owned()),
            device_epoch,
            sim_epoch,
            sender: recipient.into(),
            body: body.into(),
            service_centre_timestamp: None,
            encoding,
            multipart: None,
            // The unread flag is inbox vocabulary for messages delivered to us; a message this
            // application submitted is never unread mail.
            read: Some(true),
            status,
            direction: SmsDirection::Outgoing,
        }
    }

    /// Sender address for explicit user-facing display; never log or export this.
    #[must_use]
    pub fn sender(&self) -> &str {
        &self.sender
    }

    /// Message body for explicit user-facing display/copy; never log or export this.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Content digest used for deduplication together with the storage/epoch/direction/type/
    /// sender/time tuple (research §6.2: an index alone can be reused and is not a stable
    /// identity).
    #[must_use]
    pub fn content_digest(&self) -> [u8; 32] {
        let mut material = Vec::with_capacity(128);
        material.extend_from_slice(b"dji4g-sms-content-v1\0");
        material.extend_from_slice(&self.index.to_le_bytes());
        material.extend_from_slice(self.storage.0.as_bytes());
        material.push(match self.direction {
            SmsDirection::Incoming => 0,
            SmsDirection::Outgoing => 1,
        });
        material.extend_from_slice(&self.device_epoch.to_le_bytes());
        material.extend_from_slice(&self.sim_epoch.to_le_bytes());
        material.extend_from_slice(self.sender.as_bytes());
        material.push(0);
        material.extend_from_slice(self.body.as_bytes());
        if let Some(timestamp) = &self.service_centre_timestamp {
            material.extend_from_slice(timestamp.as_bytes());
        }
        crate::sha256(&material)
    }

    /// Masked sender for logs: keeps at most the last four characters.
    #[must_use]
    pub fn sender_masked(&self) -> String {
        let count = self.sender.chars().count();
        if count <= 4 {
            return "****".to_owned();
        }
        format!(
            "****{}",
            self.sender.chars().skip(count - 4).collect::<String>()
        )
    }
}

impl std::fmt::Debug for SmsMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SmsMessage")
            .field("index", &self.index)
            .field("storage", &self.storage)
            .field("device_epoch", &self.device_epoch)
            .field("sim_epoch", &self.sim_epoch)
            .field("sender", &"[REDACTED]")
            .field("body", &"[REDACTED]")
            .field("service_centre_timestamp", &self.service_centre_timestamp)
            .field("encoding", &self.encoding)
            .field("multipart", &self.multipart)
            .field("read", &self.read)
            .field("status", &self.status)
            .field("direction", &self.direction)
            .finish()
    }
}

impl Serialize for SmsMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("SmsMessage", 10)?;
        state.serialize_field("index", &self.index)?;
        state.serialize_field("storage", &self.storage)?;
        state.serialize_field("direction", &self.direction)?;
        state.serialize_field("device_epoch", &self.device_epoch)?;
        state.serialize_field("sim_epoch", &self.sim_epoch)?;
        // The whole message store is excluded from diagnostics exports; this redaction is the
        // second line of defence if a future caller serializes one message anyway.
        state.serialize_field("sender", "[REDACTED]")?;
        state.serialize_field("body", "[REDACTED]")?;
        state.serialize_field("service_centre_timestamp", &self.service_centre_timestamp)?;
        state.serialize_field("encoding", &self.encoding)?;
        state.serialize_field("multipart", &self.multipart)?;
        state.end()
    }
}

/// Aggregate inbox state published in snapshots (bodies never travel in the snapshot; the UI
/// reads them from the application store on demand).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SmsInboxSummary {
    /// Number of stored messages currently held for this SIM/device epoch.
    pub message_count: usize,
    pub unread_count: usize,
    /// `(used, total)` slots as reported by `CPMS`; either component may be absent.
    pub capacity: Option<(u32, u32)>,
    /// Whether the last refresh/list attempt could read the store.
    pub status: FeatureStatus,
    /// True when at least one long message is still missing fragments.
    pub has_incomplete: bool,
    /// Messages dropped by the application's local storage cap (the module copy may still
    /// exist); non-zero means the local view is incomplete and the UI must say so.
    #[serde(default)]
    pub evicted: u32,
}
