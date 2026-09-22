use dji4g_application::SmsStore;
use dji4g_domain::{SmsEncoding, SmsMessage, SmsStatus, SmsStorageId};

fn incoming(index: u32, storage: &str, body: &str) -> SmsMessage {
    SmsMessage::new(
        index,
        SmsStorageId(storage.into()),
        1,
        2,
        "+12025550123",
        body,
        SmsEncoding::Gsm7,
        SmsStatus::Received,
    )
}

#[test]
fn successful_list_resolves_old_payload_after_external_slot_reuse() {
    let mut store = SmsStore::new();
    let old = incoming(4, "SM", "previous occupant");
    let current = incoming(4, "SM", "current occupant");
    store.ingest(old.clone());
    store.ingest(current.clone());
    assert!(
        store
            .display_messages()
            .iter()
            .all(|row| !row.delete_allowed)
    );
    // The next complete successful listing proves which payload currently occupies this slot.
    assert!(store.reconcile_listed_slots(std::slice::from_ref(&current)));
    store.ingest(current.clone());
    assert_eq!(
        store.messages().len(),
        1,
        "a successful refresh must remove stale slot evidence"
    );
    assert!(store.display_messages()[0].delete_allowed);
    assert!(store.contains_fragment(&current.fragment_key()));
    assert!(!store.contains_fragment(&old.fragment_key()));
}

#[test]
fn a_conflicting_complete_batch_stays_undeletable_and_single_ingest_stays_conservative() {
    let mut store = SmsStore::new();
    let left = incoming(4, "SM", "left");
    let right = incoming(4, "SM", "right");
    let batch = vec![left.clone(), right.clone()];
    store.ingest(left.clone());
    assert!(!store.reconcile_listed_slots(&batch));
    for message in &batch {
        store.ingest(message.clone());
    }
    assert_eq!(store.messages().len(), 2);
    assert!(
        store
            .display_messages()
            .iter()
            .all(|row| !row.delete_allowed)
    );
    assert!(!store.reconcile_listed_slots(&batch));
    assert_eq!(store.messages().len(), 2);
    assert!(!store.ingest(right));
    assert!(
        store
            .display_messages()
            .iter()
            .all(|row| !row.delete_allowed)
    );
}

#[test]
fn reconciliation_preserves_unlisted_contexts_outgoing_and_exact_read_evidence() {
    let mut store = SmsStore::new();
    let old = incoming(4, "SM", "old");
    let fresh = incoming(4, "SM", "fresh");
    let other_holder = incoming(4, "ME", "old");
    let other_index = incoming(5, "SM", "old");
    let mut other_sim = old.clone();
    other_sim.sim_epoch += 1;
    let mut other_device = old.clone();
    other_device.device_epoch += 1;
    let mut outgoing = old.clone();
    outgoing.direction = dji4g_domain::SmsDirection::Outgoing;
    let mut read = incoming(6, "SM", "keep read");
    read.read = Some(true);
    let mut relisted = read.clone();
    relisted.read = Some(false);
    for message in [
        &old,
        &other_holder,
        &other_index,
        &other_sim,
        &other_device,
        &outgoing,
        &read,
    ] {
        store.ingest(message.clone());
    }
    let batch = vec![fresh.clone(), relisted];
    assert!(store.reconcile_listed_slots(&batch));
    for message in batch {
        store.ingest(message);
    }
    assert_eq!(store.messages().len(), 7);
    assert!(!store.contains_fragment(&old.fragment_key()));
    for retained in [
        &fresh,
        &other_holder,
        &other_index,
        &other_sim,
        &other_device,
        &read,
    ] {
        assert!(store.contains_fragment(&retained.fragment_key()));
    }
    assert!(store.messages().contains(&outgoing));
    assert!(store.messages().contains(&read));
    assert!(!store.reconcile_listed_slots(&[]));
    assert_eq!(store.messages().len(), 7);
}
