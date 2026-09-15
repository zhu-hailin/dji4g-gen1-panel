//! SMS store with content-digest deduplication and long-message reassembly (research document
//! §6.2).
//!
//! Storage indices are reused by the module and long-message reference numbers may repeat, so an
//! index alone can never identify a message. Deduplication keys on the message's content digest,
//! which already covers storage/index/epoch/sender/time/content. A duplicate must never overwrite
//! the stored copy: reading a message may itself mark it read, so a stale re-list that reports
//! "unread" must not regress evidence the user already saw.
//!
//! Concatenated messages (3GPP TS 23.040 §9.2.3.24) are merged into one entry for presentation
//! only. [`SmsStore::messages`] keeps every raw fragment as evidence; [`SmsStore::display_messages`]
//! joins a fragment set only when all fragments agree on SIM session, device epoch, sender,
//! reference, total, and encoding, and the sequences cover `1..=total`. Anything short of that is
//! presented as one [`SmsStatus::Incomplete`] entry instead of a complete long message.

use std::collections::{BTreeMap, HashMap, HashSet};

use dji4g_domain::{
    FeatureStatus, SmsDirection, SmsInboxSummary, SmsMessage, SmsMultipartInfo, SmsStatus,
    SmsStorageId,
};

/// Identity of one long message: fragments belong together only when the SIM session, the device
/// epoch, the exact sender address, and the concatenation reference all agree (research §6.2).
/// The SIM epoch is what prevents equal references from two time windows from merging.
type LongMessageKey = (u64, u64, String, u8);

/// One slot of the presentation order: a message shown as stored, or all fragments of one long
/// message that merge into a single presented entry.
enum PresentationSlot {
    Single(SmsMessage),
    LongMessage(Vec<SmsMessage>),
}

/// Upper bound on the stored inbox. The module stores far fewer messages than this; the cap only
/// exists so a pathological list loop cannot grow the application's memory without bound.
pub const MAX_STORED: usize = 200;

/// In-memory inbox for one device/SIM epoch, deduplicated by content digest.
///
/// The store is deliberately passive: it never invents a read state and never clears itself. The
/// owning control layer calls [`Self::clear`] when the device or SIM epoch changes.
#[derive(Clone, Debug, Default)]
pub struct SmsStore {
    messages: Vec<SmsMessage>,
    digests: HashSet<[u8; 32]>,
    /// Messages dropped by the local [MAX_STORED] cap; the module copy may still exist, but
    /// the UI must be able to say that the local view is incomplete instead of silently losing
    /// older messages.
    evicted: u32,
}

impl SmsStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Store one listed message. Returns `true` when it was new; a duplicate digest returns
    /// `false` and leaves the stored message — including its `read` state — untouched.
    pub fn ingest(&mut self, message: SmsMessage) -> bool {
        if !self.digests.insert(message.content_digest()) {
            return false;
        }
        self.messages.push(message);
        while self.messages.len() > MAX_STORED {
            let evicted = self.messages.remove(0);
            self.digests.remove(&evicted.content_digest());
            self.evicted = self.evicted.saturating_add(1);
        }
        true
    }

    /// Mark the message with this `(index, storage)` as read. Returns whether a stored message
    /// actually changed; an already-read message is a no-op.
    ///
    /// A fragment of a long message carries only its own module index, but the presented entry is
    /// read only when every fragment is read (see [`Self::display_messages`]), so marking one
    /// fragment read marks its whole `(sim_epoch, device_epoch, sender, reference)` group.
    pub fn mark_read(&mut self, index: u32, storage: &SmsStorageId) -> bool {
        let Some(position) = self
            .messages
            .iter()
            .position(|message| message.index == index && message.storage == *storage)
        else {
            return false;
        };
        let group = self.messages[position]
            .multipart
            .filter(|info| info.total > 1)
            .map(|info| {
                (
                    self.messages[position].sim_epoch,
                    self.messages[position].device_epoch,
                    self.messages[position].sender().to_owned(),
                    info.reference,
                )
            });
        let mut changed = false;
        for message in &mut self.messages {
            let same_entry = message.index == index && message.storage == *storage;
            let same_long_message =
                group
                    .as_ref()
                    .is_some_and(|(sim, device, sender, reference)| {
                        message.sim_epoch == *sim
                            && message.device_epoch == *device
                            && message.sender() == sender.as_str()
                            && message
                                .multipart
                                .is_some_and(|info| info.total > 1 && info.reference == *reference)
                    });
            if (same_entry || same_long_message) && message.read != Some(true) {
                message.read = Some(true);
                changed = true;
            }
        }
        changed
    }

    /// Remove the message with this `(index, storage)`. Returns whether one was removed.
    pub fn remove(&mut self, index: u32, storage: &SmsStorageId) -> bool {
        let Some(position) = self
            .messages
            .iter()
            .position(|message| message.index == index && message.storage == *storage)
        else {
            return false;
        };
        let removed = self.messages.remove(position);
        self.digests.remove(&removed.content_digest());
        true
    }

    /// Remove every incoming message carrying this index, regardless of storage holder. The
    /// panel-side delete command knows only the module index; removal is best-effort bookkeeping
    /// while the module deletion is still dispatched, so matching every holder is the honest
    /// interpretation. Outgoing records are never touched: their index is a local transaction id
    /// and has nothing to do with the module's index space.
    pub fn remove_by_index(&mut self, index: u32) -> bool {
        let before = self.messages.len();
        self.messages.retain(|message| {
            !(message.direction == SmsDirection::Incoming && message.index == index)
        });
        if self.messages.len() == before {
            return false;
        }
        self.rebuild_digests();
        true
    }

    /// Drop every message and digest (device or SIM epoch changed).
    pub fn clear(&mut self) {
        self.messages.clear();
        self.digests.clear();
    }

    /// Raw stored messages in arrival order, including every fragment of every long message.
    /// Presentation code uses [`Self::display_messages`] instead.
    #[must_use]
    pub fn messages(&self) -> &[SmsMessage] {
        &self.messages
    }

    /// Presentation copy of the inbox: the fragments of one long message merge into a single
    /// entry. Raw fragments stay in [`Self::messages`] as evidence.
    ///
    /// A fragment set is complete only when every fragment agrees on `total` and `encoding`, every
    /// `sequence` lies in `1..=total`, no sequence appears with two different bodies, and the
    /// sequences cover `1..=total`. The body joins the first body seen for each covered sequence in
    /// ascending order, with no separator; duplicate fragments never contribute twice. Missing,
    /// conflicting, or malformed sets present as one [`SmsStatus::Incomplete`] entry. A message
    /// without a concatenation header (or with `total == 1`) passes through unchanged.
    #[must_use]
    pub fn display_messages(&self) -> Vec<SmsMessage> {
        let mut slots: Vec<PresentationSlot> = Vec::new();
        let mut open_long_messages: HashMap<LongMessageKey, usize> = HashMap::new();
        for message in &self.messages {
            let Some(info) = message.multipart else {
                slots.push(PresentationSlot::Single(message.clone()));
                continue;
            };
            if info.total == 1 {
                slots.push(PresentationSlot::Single(message.clone()));
                continue;
            }
            let key = (
                message.sim_epoch,
                message.device_epoch,
                message.sender().to_owned(),
                info.reference,
            );
            match open_long_messages.get(&key) {
                Some(&slot) => {
                    if let PresentationSlot::LongMessage(fragments) = &mut slots[slot] {
                        fragments.push(message.clone());
                    }
                }
                None => {
                    open_long_messages.insert(key, slots.len());
                    slots.push(PresentationSlot::LongMessage(vec![message.clone()]));
                }
            }
        }
        slots
            .into_iter()
            .map(|slot| match slot {
                PresentationSlot::Single(message) => message,
                PresentationSlot::LongMessage(fragments) => merge_long_message(&fragments),
            })
            .collect()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Aggregate inbox state for the snapshot, counted over the merged presentation
    /// ([`Self::display_messages`]): a complete long message is one message, an incomplete one is
    /// one incomplete message. A message counts as read only when `read` is explicitly `Some(true)`;
    /// `None` (unknown) and `Some(false)` both count as unread.
    #[must_use]
    pub fn summary(&self, status: FeatureStatus, capacity: Option<(u32, u32)>) -> SmsInboxSummary {
        let displayed = self.display_messages();
        let unread_count = displayed
            .iter()
            .filter(|message| message.read != Some(true))
            .count();
        let has_incomplete = displayed
            .iter()
            .any(|message| message.status == SmsStatus::Incomplete);
        SmsInboxSummary {
            message_count: displayed.len(),
            unread_count,
            capacity,
            status,
            has_incomplete,
            evicted: self.evicted,
        }
    }

    fn rebuild_digests(&mut self) {
        self.digests = self
            .messages
            .iter()
            .map(SmsMessage::content_digest)
            .collect();
    }
}

/// Merge the fragments of one long message into a single presented message.
///
/// The first fragment defines the reference, total, and encoding. The smallest fragment index
/// becomes the display index so the row's read/delete commands address a real module entry; the
/// read state is read only when every fragment is read. The presented concatenation count is the
/// number of covered sequences capped at the total.
fn merge_long_message(fragments: &[SmsMessage]) -> SmsMessage {
    let first = &fragments[0];
    let Some(header) = first.multipart else {
        return first.clone();
    };
    let encoding = first.encoding;
    let mut totals_agree = true;
    let mut encodings_agree = true;
    let mut conflict = false;
    let mut by_sequence: BTreeMap<u8, &SmsMessage> = BTreeMap::new();
    for fragment in fragments {
        if fragment.multipart.map(|info| info.total) != Some(header.total) {
            totals_agree = false;
        }
        if fragment.encoding != encoding {
            encodings_agree = false;
        }
        let Some(info) = fragment.multipart else {
            continue;
        };
        match by_sequence.get(&info.sequence) {
            Some(existing) => {
                if existing.body() != fragment.body() {
                    conflict = true;
                }
            }
            None => {
                by_sequence.insert(info.sequence, fragment);
            }
        }
    }
    let sequences_valid = by_sequence.values().all(|fragment| {
        fragment
            .multipart
            .is_some_and(|info| info.sequence >= 1 && info.sequence <= header.total)
    });
    let covered: Vec<&SmsMessage> = by_sequence
        .values()
        .copied()
        .filter(|fragment| {
            fragment
                .multipart
                .is_some_and(|info| info.sequence >= 1 && info.sequence <= header.total)
        })
        .collect();
    let complete = totals_agree
        && encodings_agree
        && !conflict
        && sequences_valid
        && by_sequence.len() == usize::from(header.total);

    let body = covered
        .iter()
        .map(|fragment| fragment.body())
        .collect::<String>();
    let template = fragments
        .iter()
        .min_by_key(|fragment| fragment.index)
        .unwrap_or(first);
    let mut merged = SmsMessage::new(
        template.index,
        template.storage.clone(),
        template.device_epoch,
        template.sim_epoch,
        template.sender(),
        body,
        encoding,
        if complete {
            SmsStatus::Received
        } else {
            SmsStatus::Incomplete
        },
    );
    merged.service_centre_timestamp = template.service_centre_timestamp.clone();
    merged.multipart = Some(SmsMultipartInfo {
        reference: header.reference,
        total: header.total,
        sequence: u8::try_from(covered.len())
            .unwrap_or(u8::MAX)
            .min(header.total),
    });
    merged.read = merged_read_state(fragments);
    merged.direction = template.direction;
    merged
}

/// Read state of a merged long message: `Some(true)` only when every fragment is read,
/// `Some(false)` when any fragment is explicitly unread, `None` while every fragment is unknown.
fn merged_read_state(fragments: &[SmsMessage]) -> Option<bool> {
    if fragments.iter().all(|fragment| fragment.read == Some(true)) {
        Some(true)
    } else if fragments
        .iter()
        .any(|fragment| fragment.read == Some(false))
    {
        Some(false)
    } else {
        None
    }
}
