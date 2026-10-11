use dji4g_application::SmsStore;
use dji4g_domain::{SmsEncoding, SmsMessage, SmsReadReport, SmsStatus, SmsStorageId};

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

fn complete_report(storage: &str, count: usize) -> SmsReadReport {
    SmsReadReport {
        storage: Some(SmsStorageId(storage.into())),
        capacity: Some((count as u32, 30)),
        raw_records: count,
        decoded_records: count,
        supported_storages: vec![
            SmsStorageId("SM".into()),
            SmsStorageId("ME".into()),
            SmsStorageId("MT".into()),
        ],
        ..Default::default()
    }
}

#[test]
fn complete_holder_inventory_removes_unlisted_and_reused_slots_only_in_its_scope() {
    let mut store = SmsStore::new();
    let old = incoming(4, "SM", "old");
    let fresh = incoming(4, "SM", "fresh");
    let absent = incoming(5, "SM", "removed externally");
    let other_holder = incoming(5, "ME", "same index, other holder");
    let combined_holder = incoming(5, "MT", "combined holder index");
    let mut other_sim = absent.clone();
    other_sim.sim_epoch += 1;
    let mut other_device = absent.clone();
    other_device.device_epoch += 1;
    let mut outgoing = absent.clone();
    outgoing.direction = dji4g_domain::SmsDirection::Outgoing;
    let mut read = incoming(6, "SM", "keep read evidence");
    read.read = Some(true);
    let mut relisted = read.clone();
    relisted.read = Some(false);
    for message in [
        &old,
        &absent,
        &other_holder,
        &combined_holder,
        &other_sim,
        &other_device,
        &outgoing,
        &read,
    ] {
        store.ingest(message.clone());
    }
    let batch = vec![fresh.clone(), relisted];
    let report = complete_report("SM", batch.len());
    assert!(SmsStore::is_complete_listing(&batch, &report, 1, 2));
    assert!(store.reconcile_complete_listing(&batch, &report, 1, 2));
    for message in batch {
        store.ingest(message);
    }
    assert!(!store.contains_fragment(&old.fragment_key()));
    assert!(!store.contains_fragment(&absent.fragment_key()));
    for retained in [
        &fresh,
        &other_holder,
        &combined_holder,
        &other_sim,
        &other_device,
        &read,
    ] {
        assert!(store.contains_fragment(&retained.fragment_key()));
    }
    assert!(store.messages().contains(&outgoing));
    assert!(store.messages().contains(&read));
    assert_eq!(store.messages().len(), 7);
}

#[test]
fn an_empty_confirmed_holder_clears_only_its_incoming_cache_including_mt() {
    for holder in ["SM", "ME", "MT"] {
        let mut store = SmsStore::new();
        for storage in ["SM", "ME", "MT"] {
            store.ingest(incoming(4, storage, "holder-specific message"));
        }
        let outgoing = SmsMessage::new_outgoing(
            4,
            1,
            2,
            "+12025550123",
            "local send record",
            SmsEncoding::Ucs2,
            SmsStatus::Submitted,
        );
        store.ingest(outgoing.clone());
        let report = complete_report(holder, 0);
        assert!(SmsStore::is_complete_listing(&[], &report, 1, 2));
        assert!(store.reconcile_complete_listing(&[], &report, 1, 2));
        assert_eq!(store.messages().len(), 3);
        assert!(store.messages().contains(&outgoing));
        for storage in ["SM", "ME", "MT"] {
            assert_eq!(
                store.contains_fragment(
                    &incoming(4, storage, "holder-specific message").fragment_key()
                ),
                storage != holder
            );
        }
    }
}

#[test]
fn skipped_truncated_unknown_or_mismatched_inventory_never_removes_cached_records() {
    let fresh = incoming(4, "SM", "fresh");
    let complete = complete_report("SM", 1);
    let mut reports = Vec::new();
    let mut missing_holder = complete.clone();
    missing_holder.storage = None;
    reports.push(missing_holder);
    let mut missing_capacity = complete.clone();
    missing_capacity.capacity = None;
    reports.push(missing_capacity);
    let mut unconfirmed_holder = complete.clone();
    unconfirmed_holder.supported_storages.clear();
    reports.push(unconfirmed_holder);
    let mut skipped = complete.clone();
    skipped.raw_records = 2;
    skipped.skipped_records = 1;
    skipped.capacity = Some((2, 30));
    reports.push(skipped);
    let mut truncated = complete.clone();
    truncated.capacity = Some((2, 30));
    reports.push(truncated);
    let mut decoded_mismatch = complete.clone();
    decoded_mismatch.decoded_records = 2;
    reports.push(decoded_mismatch);
    let mut restoration_unknown = complete.clone();
    restoration_unknown.restoration = dji4g_domain::SmsStorageRestoration::Unknown;
    reports.push(restoration_unknown);
    for report in reports {
        assert_unconfirmed_keeps_records(std::slice::from_ref(&fresh), &report);
    }
    let mut wrong_holder = fresh.clone();
    wrong_holder.storage = SmsStorageId("ME".into());
    assert_unconfirmed_keeps_records(&[wrong_holder], &complete);
    let mut wrong_device = fresh.clone();
    wrong_device.device_epoch += 1;
    assert_unconfirmed_keeps_records(&[wrong_device], &complete);
    let mut wrong_sim = fresh.clone();
    wrong_sim.sim_epoch += 1;
    assert_unconfirmed_keeps_records(&[wrong_sim], &complete);
    assert_unconfirmed_keeps_records(&[fresh.clone(), fresh], &complete_report("SM", 2));
}

fn assert_unconfirmed_keeps_records(listed: &[SmsMessage], report: &SmsReadReport) {
    let mut store = SmsStore::new();
    store.ingest(incoming(4, "SM", "previous slot occupant"));
    store.ingest(incoming(5, "SM", "absent from this list"));
    let before = store.messages().to_vec();
    assert!(!SmsStore::is_complete_listing(listed, report, 1, 2));
    assert!(!store.reconcile_complete_listing(listed, report, 1, 2));
    assert_eq!(store.messages(), before.as_slice());
}

#[test]
fn complete_inventory_advances_exact_read_evidence_without_regressing_or_overwriting_unknown() {
    let mut store = SmsStore::new();
    let mut unread = incoming(1, "SM", "unread becomes read");
    unread.read = Some(false);
    let unknown = incoming(2, "SM", "unknown becomes read");
    let mut already_read = incoming(3, "SM", "must remain read");
    already_read.read = Some(true);
    let still_unknown = incoming(4, "SM", "unknown remains unknown");
    let other_holder = incoming(1, "ME", "other holder remains unknown");
    for message in [
        &unread,
        &unknown,
        &already_read,
        &still_unknown,
        &other_holder,
    ] {
        store.ingest(message.clone());
    }
    let mut confirmed_read = unread;
    confirmed_read.read = Some(true);
    let mut confirmed_unknown_read = unknown;
    confirmed_unknown_read.read = Some(true);
    let mut stale_unread = already_read;
    stale_unread.read = Some(false);
    let mut observed_unread = still_unknown;
    observed_unread.read = Some(false);
    let batch = vec![
        confirmed_read,
        confirmed_unknown_read,
        stale_unread,
        observed_unread,
    ];
    let report = complete_report("SM", batch.len());
    assert!(store.reconcile_complete_listing(&batch, &report, 1, 2));
    assert_eq!(store.messages()[0].read, Some(true));
    assert_eq!(store.messages()[1].read, Some(true));
    assert_eq!(store.messages()[2].read, Some(true));
    assert_eq!(store.messages()[3].read, None);
    assert_eq!(store.messages()[4].read, None);
    assert!(!store.reconcile_complete_listing(&batch, &report, 1, 2));
    assert_eq!(
        store
            .summary(dji4g_domain::FeatureStatus::Supported, None)
            .unread_count,
        2
    );
}

#[test]
fn sync_time_advances_only_for_complete_lists_and_disappears_with_the_device_context() {
    use dji4g_application::{Controller, DeviceEpoch, EpochInvalidationReason, PortError};
    use dji4g_domain::{ErrorCode, FeatureStatus};
    use std::time::{Duration, SystemTime};
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    let mut controller = Controller::for_test(now);
    let mut stored = incoming(1, "SM", "verified stored message");
    stored.device_epoch = controller.state().epoch().0;
    stored.sim_epoch = controller.state().sim_epoch();
    controller.ingest_sms(stored.clone());
    assert_eq!(controller.snapshot().sms_inbox.last_synced_at, None);
    let report = complete_report("SM", 1);
    assert!(controller.reconcile_sms_complete_listing(std::slice::from_ref(&stored), &report));
    let synced = controller.snapshot().sms_inbox;
    assert_eq!(synced.last_synced_at, Some(now));
    assert_eq!(synced.last_synced_storage, Some(SmsStorageId("SM".into())));

    controller.advance_time(Duration::from_secs(10));
    controller.mark_sms_read(&stored);
    controller.ingest_sms(SmsMessage::new_outgoing(
        7,
        stored.device_epoch,
        stored.sim_epoch,
        "+12025550123",
        "failed send",
        SmsEncoding::Ucs2,
        SmsStatus::Failed,
    ));
    controller.set_sms_inbox_failure(Some(PortError::new(ErrorCode::Timeout, "sms:timeout")));
    controller.record_sms_probe(FeatureStatus::TransportFailure, Some((1, 30)));
    assert_eq!(
        controller.snapshot().sms_inbox.last_synced_at,
        synced.last_synced_at
    );
    let mut partial = report.clone();
    partial.raw_records = 2;
    partial.skipped_records = 1;
    partial.capacity = Some((2, 30));
    assert!(!controller.reconcile_sms_complete_listing(std::slice::from_ref(&stored), &partial));
    assert_eq!(
        controller.snapshot().sms_inbox.last_synced_at,
        synced.last_synced_at
    );
    assert_eq!(
        controller.snapshot().sms_inbox.last_synced_storage,
        synced.last_synced_storage
    );

    controller.advance_time(Duration::from_secs(10));
    assert!(controller.reconcile_sms_complete_listing(&[], &complete_report("ME", 0)));
    let refreshed = controller.snapshot().sms_inbox;
    assert_eq!(
        refreshed.last_synced_at,
        Some(now + Duration::from_secs(20))
    );
    assert_eq!(
        refreshed.last_synced_storage,
        Some(SmsStorageId("ME".into()))
    );
    controller.invalidate_epoch(
        DeviceEpoch(stored.device_epoch + 1),
        EpochInvalidationReason::PhysicalRemoval,
    );
    assert_eq!(controller.snapshot().sms_inbox.last_synced_at, None);
    assert_eq!(controller.snapshot().sms_inbox.last_synced_storage, None);
}

#[test]
fn partial_controller_listing_reconciles_observed_slots_without_clearing_absent_cache_or_syncing() {
    use dji4g_application::Controller;
    use std::time::{Duration, SystemTime};
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    let mut controller = Controller::for_test(now);
    let old = incoming(4, "SM", "old slot occupant");
    let absent = incoming(5, "SM", "unconfirmed absent slot");
    controller.ingest_sms(old.clone());
    controller.ingest_sms(absent.clone());
    controller.reconcile_sms_complete_listing(&[old, absent], &complete_report("SM", 2));
    assert_eq!(controller.snapshot().sms_inbox.last_synced_at, Some(now));
    controller.advance_time(Duration::from_secs(10));
    let fresh = incoming(4, "SM", "fresh slot occupant");
    let report = SmsReadReport {
        raw_records: 1,
        decoded_records: 1,
        ..Default::default()
    };
    controller.reconcile_sms_complete_listing(std::slice::from_ref(&fresh), &report);
    controller.ingest_sms(fresh);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.sms_inbox.last_synced_at, Some(now));
    assert_eq!(snapshot.sms_messages.len(), 2);
    assert!(
        snapshot
            .sms_messages
            .iter()
            .any(|row| row.message.body() == "fresh slot occupant")
    );
    assert!(
        snapshot
            .sms_messages
            .iter()
            .any(|row| row.message.body() == "unconfirmed absent slot")
    );
    assert!(
        !snapshot
            .sms_messages
            .iter()
            .any(|row| row.message.body() == "old slot occupant")
    );
}

#[test]
fn partial_listing_advances_observed_read_evidence_without_syncing_or_removing_absent_rows() {
    use dji4g_application::Controller;
    use std::time::{Duration, SystemTime};
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    let mut controller = Controller::for_test(now);
    let mut unread = incoming(1, "SM", "observed unread");
    unread.read = Some(false);
    let unknown = incoming(2, "SM", "observed unknown");
    let mut already_read = incoming(3, "SM", "keep confirmed read");
    already_read.read = Some(true);
    let mut absent = incoming(4, "SM", "decode skipped this time");
    absent.read = Some(false);
    let initial = [
        unread.clone(),
        unknown.clone(),
        already_read.clone(),
        absent,
    ];
    for message in &initial {
        controller.ingest_sms(message.clone());
    }
    controller.reconcile_sms_complete_listing(&initial, &complete_report("SM", 4));
    controller.advance_time(Duration::from_secs(10));
    unread.read = Some(true);
    let mut confirmed_unknown_read = unknown;
    confirmed_unknown_read.read = Some(true);
    already_read.read = Some(false);
    let observed = [unread, confirmed_unknown_read, already_read];
    let mut partial = complete_report("SM", 4);
    partial.decoded_records = 3;
    partial.skipped_records = 1;
    assert!(!controller.reconcile_sms_complete_listing(&observed, &partial));
    for message in observed {
        controller.ingest_sms(message);
    }
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.sms_inbox.last_synced_at, Some(now));
    assert_eq!(
        snapshot.sms_inbox.last_synced_storage,
        Some(SmsStorageId("SM".into()))
    );
    assert_eq!(snapshot.sms_messages.len(), 4);
    assert_eq!(snapshot.sms_inbox.unread_count, 1);
    for row in &snapshot.sms_messages {
        assert_eq!(row.message.read, Some(row.message.index != 4));
    }
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
