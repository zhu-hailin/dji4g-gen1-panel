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
    FeatureStatus, SmsConcatReference, SmsDirection, SmsDisplayMessage, SmsFragmentKey,
    SmsInboxSummary, SmsMessage, SmsMultipartInfo, SmsStatus, SmsStorageId,
};

/// Identity of one long message: fragments belong together only when the SIM session, the device
/// epoch, the exact sender address, and the concatenation reference all agree (research §6.2).
/// The SIM epoch is what prevents equal references from two time windows from merging.
type LongMessageKey = (
    u64,
    u64,
    SmsStorageId,
    SmsDirection,
    String,
    SmsConcatReference,
);
fn long_message_key(message: &SmsMessage, reference: SmsConcatReference) -> LongMessageKey {
    (
        message.sim_epoch,
        message.device_epoch,
        message.storage.clone(),
        message.direction,
        message.sender().to_owned(),
        reference,
    )
}

/// One slot of the presentation order: a message shown as stored, or all fragments of one long
/// message that merge into a single presented entry.
enum PresentationSlot {
    Single(SmsMessage),
    LongMessage(Vec<SmsMessage>),
}

/// Bounded raw-record budget, including every multipart fragment. This is a software safety
/// ceiling, not a claim about any particular module's physical storage capacity.
pub const MAX_STORED: usize = 1000;

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

    /// Reconcile slot reuse using one complete, successful listing before ingesting that batch.
    /// The caller must stamp the listing with the current device/SIM context first. Only slots
    /// explicitly represented by incoming records are authoritative: an empty or partial decoded
    /// list cannot identify the queried holder or prove that unlisted slots were deleted.
    /// All conflicting payloads reported in this same batch remain evidence, so this cannot turn
    /// an ambiguous listing into a deletable row. Exact identities retain their existing read state.
    pub fn reconcile_listed_slots(&mut self, listed: &[SmsMessage]) -> bool {
        type Slot = (u64, u64, SmsStorageId, u32);
        let mut observed: HashMap<Slot, HashSet<[u8; 32]>> = HashMap::new();
        for message in listed {
            if message.direction != SmsDirection::Incoming || message.storage.0.trim().is_empty() {
                continue;
            }
            observed
                .entry((
                    message.device_epoch,
                    message.sim_epoch,
                    message.storage.clone(),
                    message.index,
                ))
                .or_default()
                .insert(message.payload_fingerprint());
        }
        let before = self.messages.len();
        self.messages.retain(|message| {
            message.direction != SmsDirection::Incoming
                || observed
                    .get(&(
                        message.device_epoch,
                        message.sim_epoch,
                        message.storage.clone(),
                        message.index,
                    ))
                    .is_none_or(|payloads| payloads.contains(&message.payload_fingerprint()))
        });
        if self.messages.len() == before {
            return false;
        }
        self.rebuild_digests();
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
            .map(|info| long_message_key(&self.messages[position], info.reference));
        let mut changed = false;
        for message in &mut self.messages {
            let same_entry = message.index == index && message.storage == *storage;
            let same_long_message = group.as_ref().is_some_and(|key| {
                message.multipart.is_some_and(|info| {
                    info.total > 1 && long_message_key(message, info.reference) == *key
                })
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

    /// Validate the captured identity without interpreting a reused index as the same message.
    #[must_use]
    pub fn contains_fragment(&self, key: &SmsFragmentKey) -> bool {
        self.messages.iter().any(|message| {
            message.direction == SmsDirection::Incoming && message.fragment_key() == *key
        })
    }

    /// Remove only the incoming evidence acknowledged for this exact fragment identity.
    pub fn remove_fragment(&mut self, key: &SmsFragmentKey) -> bool {
        let Some(position) = self.messages.iter().position(|message| {
            message.direction == SmsDirection::Incoming && message.fragment_key() == *key
        }) else {
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
    pub fn display_messages(&self) -> Vec<SmsDisplayMessage> {
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
            let key = long_message_key(message, info.reference);
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
                PresentationSlot::Single(message) => display_single(message),
                PresentationSlot::LongMessage(fragments) => merge_long_message(&fragments),
            })
            .map(|mut row| {
                // Two payloads in one physical slot cannot both be targeted safely, even when
                // different headers placed them into separate presentation groups.
                if row.fragments.iter().any(|key| {
                    self.messages.iter().any(|message| {
                        message.direction == SmsDirection::Incoming
                            && message.device_epoch == key.device_epoch
                            && message.sim_epoch == key.sim_epoch
                            && message.storage == key.storage
                            && message.index == key.index
                            && message.payload_fingerprint() != key.payload_fingerprint
                    })
                }) {
                    row.delete_allowed = false;
                }
                row
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
fn merge_long_message(fragments: &[SmsMessage]) -> SmsDisplayMessage {
    let first = &fragments[0];
    let Some(header) = first.multipart else {
        return display_single(first.clone());
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
    let mut physical_fragments = fragments
        .iter()
        .filter(|fragment| fragment.direction == SmsDirection::Incoming)
        .map(SmsMessage::fragment_key)
        .collect::<Vec<_>>();
    physical_fragments.sort_by(|a, b| {
        (
            a.device_epoch,
            a.sim_epoch,
            &a.storage.0,
            a.index,
            a.payload_fingerprint,
        )
            .cmp(&(
                b.device_epoch,
                b.sim_epoch,
                &b.storage.0,
                b.index,
                b.payload_fingerprint,
            ))
    });
    physical_fragments.dedup();
    SmsDisplayMessage {
        message: merged,
        delete_allowed: !physical_fragments.is_empty()
            && totals_agree
            && encodings_agree
            && !conflict
            && sequences_valid
            && header.total > 0,
        fragments: physical_fragments,
    }
}

fn display_single(message: SmsMessage) -> SmsDisplayMessage {
    let incoming = message.direction == SmsDirection::Incoming;
    let fragments = if incoming {
        vec![message.fragment_key()]
    } else {
        Vec::new()
    };
    SmsDisplayMessage {
        message,
        fragments,
        delete_allowed: incoming,
    }
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

#[cfg(test)]
mod identity_tests {
    use super::*;

    fn fragment(index: u32, sequence: u8) -> SmsMessage {
        let mut message = SmsMessage::new(
            index,
            SmsStorageId("SM".into()),
            1,
            1,
            "+12025550123",
            "same",
            dji4g_domain::SmsEncoding::Gsm7,
            SmsStatus::Received,
        );
        message.multipart = Some(SmsMultipartInfo {
            reference: SmsConcatReference::EightBit(42),
            total: 2,
            sequence,
        });
        message.read = Some(false);
        message
    }

    #[test]
    fn different_storage_or_direction_never_merge_or_share_read_state() {
        for direction_change in [false, true] {
            let mut store = SmsStore::new();
            let first = fragment(1, 1);
            let mut second = fragment(2, 2);
            if direction_change {
                second.direction = SmsDirection::Outgoing;
            } else {
                second.storage = SmsStorageId("ME".into());
            }
            store.ingest(first);
            store.ingest(second);
            assert_eq!(store.display_messages().len(), 2);
            store.mark_read(1, &SmsStorageId("SM".into()));
            assert_eq!(store.messages()[1].read, Some(false));
        }
    }

    #[test]
    fn equal_payload_with_different_concat_header_is_not_deduplicated() {
        let mut store = SmsStore::new();
        assert!(store.ingest(fragment(1, 1)));
        assert!(store.ingest(fragment(1, 2)));
    }

    #[test]
    fn full_reference_and_wire_width_separate_groups_and_read_state() {
        for (left, right) in [
            (
                SmsConcatReference::SixteenBit(0x1234),
                SmsConcatReference::SixteenBit(0x5634),
            ),
            (
                SmsConcatReference::EightBit(52),
                SmsConcatReference::SixteenBit(52),
            ),
        ] {
            let mut store = SmsStore::new();
            let mut first = fragment(1, 1);
            let mut second = fragment(2, 2);
            first.multipart.as_mut().unwrap().reference = left;
            second.multipart.as_mut().unwrap().reference = right;
            store.ingest(first);
            store.ingest(second);
            assert_eq!(store.display_messages().len(), 2);
            store.mark_read(1, &SmsStorageId("SM".into()));
            assert_eq!(store.messages()[1].read, Some(false));
        }
    }

    #[test]
    fn missing_fragments_can_be_deleted_but_conflicting_evidence_cannot() {
        let mut store = SmsStore::new();
        store.ingest(fragment(1, 1));
        assert_eq!(store.display_messages()[0].status, SmsStatus::Incomplete);
        assert!(store.display_messages()[0].delete_allowed);
        for conflict in 0..4 {
            let mut store = store.clone();
            let mut second = fragment(2, 2);
            match conflict {
                0 => second.multipart.as_mut().unwrap().total = 3,
                1 => second.encoding = dji4g_domain::SmsEncoding::Ucs2,
                2 => second.multipart.as_mut().unwrap().sequence = 3,
                _ => {
                    second = SmsMessage::new(
                        2,
                        SmsStorageId("SM".into()),
                        1,
                        1,
                        "+12025550123",
                        "different",
                        dji4g_domain::SmsEncoding::Gsm7,
                        SmsStatus::Received,
                    );
                    second.multipart = fragment(1, 1).multipart;
                }
            }
            store.ingest(second);
            assert!(!store.display_messages()[0].delete_allowed);
            assert_eq!(store.display_messages()[0].status, SmsStatus::Incomplete);
        }
    }

    #[test]
    fn single_incoming_has_one_fragment_and_stable_identity_ignores_arrival_order() {
        let mut single = fragment(1, 1);
        single.multipart = None;
        let mut store = SmsStore::new();
        store.ingest(single.clone());
        assert_eq!(
            store.display_messages()[0].fragments,
            vec![single.fragment_key()]
        );
        let mut forward = SmsStore::new();
        let mut reverse = SmsStore::new();
        forward.ingest(fragment(1, 1));
        forward.ingest(fragment(2, 2));
        reverse.ingest(fragment(2, 2));
        reverse.ingest(fragment(1, 1));
        assert_eq!(
            forward.display_messages()[0].stable_id(),
            reverse.display_messages()[0].stable_id()
        );
        let key = fragment(1, 1).fragment_key();
        for mismatch in 0..4 {
            let mut altered = key.clone();
            match mismatch {
                0 => altered.device_epoch += 1,
                1 => altered.sim_epoch += 1,
                2 => altered.storage = SmsStorageId("ME".into()),
                _ => altered.payload_fingerprint = [0; 32],
            }
            assert!(!forward.contains_fragment(&altered));
            assert!(!forward.remove_fragment(&altered));
        }
    }

    #[test]
    fn display_tracks_every_physical_fragment_and_precise_deletion() {
        let mut store = SmsStore::new();
        let first = fragment(4, 1);
        let second = fragment(7, 2);
        let duplicate = fragment(9, 2);
        for item in [duplicate, second, first.clone()] {
            store.ingest(item);
        }
        let row = store.display_messages().remove(0);
        assert_eq!(
            row.fragments
                .iter()
                .map(|key| key.index)
                .collect::<Vec<_>>(),
            vec![4, 7, 9]
        );
        assert!(row.delete_allowed);
        assert_eq!(row.body(), "samesame");
        assert!(store.contains_fragment(&first.fragment_key()));
        let id = row.stable_id();
        store.mark_read(4, &SmsStorageId("SM".into()));
        assert_eq!(id, store.display_messages()[0].stable_id());
        assert!(store.remove_fragment(&first.fragment_key()));
        assert!(!store.remove_fragment(&first.fragment_key()));
        assert_eq!(store.messages().len(), 2);
        let mut changed = first.clone();
        changed.multipart.as_mut().unwrap().sequence = 2;
        store.ingest(changed);
        assert!(!store.remove_fragment(&first.fragment_key()));
    }

    #[test]
    fn conflicts_disable_whole_message_deletion() {
        let mut store = SmsStore::new();
        store.ingest(fragment(1, 1));
        store.ingest(fragment(1, 2));
        let row = &store.display_messages()[0];
        assert!(!row.delete_allowed, "one slot cannot identify two payloads");
    }

    #[test]
    fn outgoing_display_never_has_physical_fragments() {
        let mut store = SmsStore::new();
        let outgoing = SmsMessage::new_outgoing(
            1,
            1,
            1,
            "+12025550123",
            "sent",
            dji4g_domain::SmsEncoding::Gsm7,
            SmsStatus::Submitted,
        );
        let id = outgoing.content_digest();
        let key = outgoing.fragment_key();
        store.ingest(outgoing);
        let row = &store.display_messages()[0];
        assert!(row.fragments.is_empty());
        assert!(!row.delete_allowed);
        assert_eq!(row.stable_id(), id);
        assert!(!store.contains_fragment(&key));
        assert!(!store.remove_fragment(&key));
    }
}
