//! Long-message reassembly in the SMS store (research document §6.2): fragments of one
//! concatenated message are presented as a single entry, and only a complete, consistent set of
//! fragments may be shown as a received long message. Raw fragments stay stored as evidence.

use dji4g_application::{Controller, SmsStore};
use dji4g_domain::{
    FeatureStatus, SmsEncoding, SmsMessage, SmsMultipartInfo, SmsStatus, SmsStorageId,
};

const NOW: std::time::SystemTime = std::time::SystemTime::UNIX_EPOCH;

fn storage() -> SmsStorageId {
    SmsStorageId("SM".to_owned())
}

fn fragment(
    index: u32,
    sender: &str,
    body: &str,
    reference: u8,
    total: u8,
    sequence: u8,
) -> SmsMessage {
    let mut message = SmsMessage::new(
        index,
        storage(),
        1,
        0,
        sender,
        body,
        SmsEncoding::Gsm7,
        SmsStatus::Received,
    );
    message.multipart = Some(SmsMultipartInfo {
        reference: dji4g_domain::SmsConcatReference::EightBit(reference),
        total,
        sequence,
    });
    message
}

fn plain(index: u32, body: &str) -> SmsMessage {
    SmsMessage::new(
        index,
        storage(),
        1,
        0,
        "+8613800138000",
        body,
        SmsEncoding::Gsm7,
        SmsStatus::Received,
    )
}

#[test]
fn complete_two_fragment_message_is_presented_as_one_received_message() {
    let mut store = SmsStore::new();
    assert!(store.ingest(fragment(1, "+8613800138000", "你好", 7, 2, 1)));
    assert!(store.ingest(fragment(2, "+8613800138000", "世界", 7, 2, 2)));

    assert_eq!(
        store.messages().len(),
        2,
        "raw fragments stay stored as evidence"
    );
    let displayed = store.display_messages();
    assert_eq!(
        displayed.len(),
        1,
        "the two fragments present as one message"
    );
    let merged = &displayed[0];
    assert_eq!(merged.body(), "你好世界");
    assert_eq!(merged.status, SmsStatus::Received);
    assert_eq!(merged.index, 1, "the smallest fragment index is presented");
    assert_eq!(
        merged.multipart.map(|info| (info.sequence, info.total)),
        Some((2, 2)),
        "the merged view reports the collected count over the total"
    );

    let summary = store.summary(FeatureStatus::Supported, None);
    assert_eq!(summary.message_count, 1, "the summary counts presentations");
    assert!(!summary.has_incomplete);
}

#[test]
fn a_missing_fragment_is_presented_as_one_incomplete_message() {
    let mut store = SmsStore::new();
    assert!(store.ingest(fragment(1, "+86", "第一段", 9, 3, 1)));
    assert!(store.ingest(fragment(3, "+86", "第三段", 9, 3, 3)));

    let displayed = store.display_messages();
    assert_eq!(displayed.len(), 1);
    assert_eq!(
        displayed[0].body(),
        "第一段第三段",
        "received fragments concatenate by sequence"
    );
    assert_eq!(displayed[0].status, SmsStatus::Incomplete);

    let summary = store.summary(FeatureStatus::Supported, None);
    assert_eq!(summary.message_count, 1);
    assert!(summary.has_incomplete);
}

#[test]
fn out_of_order_fragments_are_joined_by_sequence() {
    let mut store = SmsStore::new();
    assert!(store.ingest(fragment(5, "+86", "后半", 3, 2, 2)));
    assert!(store.ingest(fragment(2, "+86", "前半", 3, 2, 1)));

    let displayed = store.display_messages();
    assert_eq!(displayed.len(), 1);
    assert_eq!(displayed[0].body(), "前半后半");
    assert_eq!(displayed[0].status, SmsStatus::Received);
    assert_eq!(
        displayed[0].index, 2,
        "the display index is not arrival order"
    );
}

#[test]
fn repeated_fragments_are_not_concatenated_twice() {
    let mut store = SmsStore::new();
    assert!(store.ingest(fragment(1, "+86", "前半", 4, 2, 1)));
    assert!(
        store.ingest(fragment(5, "+86", "前半", 4, 2, 1)),
        "a relisted fragment carrying a new index is a distinct stored copy"
    );
    assert!(store.ingest(fragment(2, "+86", "后半", 4, 2, 2)));

    assert_eq!(store.messages().len(), 3);
    let displayed = store.display_messages();
    assert_eq!(displayed.len(), 1);
    assert_eq!(
        displayed[0].body(),
        "前半后半",
        "the same sequence contributes its text once"
    );
    assert_eq!(
        displayed[0].status,
        SmsStatus::Received,
        "identical repeated content is a duplicate, not a conflict"
    );
}

#[test]
fn conflicting_fragments_with_one_sequence_mark_the_view_incomplete() {
    let mut store = SmsStore::new();
    assert!(store.ingest(fragment(1, "+86", "旧窗口甲", 4, 2, 1)));
    assert!(store.ingest(fragment(2, "+86", "旧窗口乙", 4, 2, 2)));
    assert!(
        store.ingest(fragment(3, "+86", "新窗口甲", 4, 2, 1)),
        "a reused reference with different content across time windows is stored"
    );

    assert_eq!(store.messages().len(), 3, "all conflicting fragments stay");
    let displayed = store.display_messages();
    assert_eq!(displayed.len(), 1);
    assert_eq!(
        displayed[0].status,
        SmsStatus::Incomplete,
        "a cross-window conflict can never pretend to be complete"
    );
    assert_eq!(
        displayed[0].body(),
        "旧窗口甲旧窗口乙",
        "the earliest fragment keeps the contested sequence slot"
    );
    assert_eq!(displayed[0].multipart.map(|info| info.sequence), Some(2));
}

#[test]
fn fragments_from_different_senders_never_merge() {
    let mut store = SmsStore::new();
    assert!(store.ingest(fragment(1, "+8613800138000", "甲", 4, 2, 1)));
    assert!(store.ingest(fragment(2, "+8613800138001", "乙", 4, 2, 2)));

    let displayed = store.display_messages();
    assert_eq!(displayed.len(), 2, "a sender is part of the merge identity");
    assert!(
        displayed
            .iter()
            .all(|message| message.status == SmsStatus::Incomplete)
    );
    assert!(store.summary(FeatureStatus::Supported, None).has_incomplete);
}

#[test]
fn fragments_from_different_sim_or_device_epochs_never_merge() {
    let mut store = SmsStore::new();
    let mut old_sim = fragment(1, "+86", "甲", 4, 2, 1);
    old_sim.sim_epoch = 1;
    let mut new_sim = fragment(2, "+86", "乙", 4, 2, 2);
    new_sim.sim_epoch = 2;
    assert!(store.ingest(old_sim));
    assert!(store.ingest(new_sim));

    let mut old_device = fragment(3, "+86", "丙", 5, 2, 1);
    old_device.device_epoch = 1;
    let mut new_device = fragment(4, "+86", "丁", 5, 2, 2);
    new_device.device_epoch = 2;
    assert!(store.ingest(old_device));
    assert!(store.ingest(new_device));

    let displayed = store.display_messages();
    assert_eq!(displayed.len(), 4, "epochs are part of the merge identity");
    assert!(
        displayed
            .iter()
            .all(|message| message.status == SmsStatus::Incomplete)
    );
}

#[test]
fn mismatched_totals_never_present_as_complete() {
    let mut store = SmsStore::new();
    assert!(store.ingest(fragment(1, "+86", "甲", 4, 2, 1)));
    assert!(store.ingest(fragment(2, "+86", "乙", 4, 3, 2)));

    let displayed = store.display_messages();
    assert_eq!(
        displayed.len(),
        1,
        "one reference groups into one presentation"
    );
    assert_eq!(displayed[0].status, SmsStatus::Incomplete);
    assert_eq!(displayed[0].body(), "甲乙");
    assert!(store.summary(FeatureStatus::Supported, None).has_incomplete);
}

#[test]
fn mixed_encodings_never_present_as_one_complete_message() {
    let mut store = SmsStore::new();
    assert!(store.ingest(fragment(1, "+86", "甲", 4, 2, 1)));
    let mut ucs2 = fragment(2, "+86", "乙", 4, 2, 2);
    ucs2.encoding = SmsEncoding::Ucs2;
    assert!(store.ingest(ucs2));

    let displayed = store.display_messages();
    assert_eq!(displayed.len(), 1);
    assert_eq!(displayed[0].status, SmsStatus::Incomplete);
}

#[test]
fn an_out_of_range_sequence_is_excluded_and_marks_the_view_incomplete() {
    let mut store = SmsStore::new();
    assert!(store.ingest(fragment(1, "+86", "甲", 4, 2, 1)));
    assert!(store.ingest(fragment(9, "+86", "越界", 4, 2, 4)));

    let displayed = store.display_messages();
    assert_eq!(displayed.len(), 1);
    assert_eq!(
        displayed[0].body(),
        "甲",
        "a sequence outside 1..=total is never concatenated"
    );
    assert_eq!(displayed[0].status, SmsStatus::Incomplete);
}

#[test]
fn single_messages_pass_through_unchanged() {
    let mut store = SmsStore::new();
    assert!(store.ingest(plain(1, "普通短信")));
    assert!(store.ingest(fragment(2, "+86", "单分片", 4, 1, 1)));

    let displayed = store.display_messages();
    assert_eq!(displayed.len(), 2);
    assert_eq!(displayed[0].body(), "普通短信");
    assert_eq!(displayed[0].status, SmsStatus::Received);
    assert_eq!(displayed[0].multipart, None);
    assert_eq!(displayed[1].body(), "单分片");
    assert_eq!(
        displayed[1]
            .multipart
            .map(|info| (info.sequence, info.total)),
        Some((1, 1)),
        "a one-fragment concatenation header is not a long message"
    );

    let summary = store.summary(FeatureStatus::Supported, None);
    assert_eq!(summary.message_count, 2);
    assert!(!summary.has_incomplete);
}

#[test]
fn a_joined_message_is_unread_until_every_fragment_is_read() {
    let mut store = SmsStore::new();
    let mut first = fragment(1, "+86", "甲", 4, 2, 1);
    first.read = Some(true);
    assert!(store.ingest(first));
    let mut second = fragment(2, "+86", "乙", 4, 2, 2);
    second.read = Some(false);
    assert!(store.ingest(second));

    assert_eq!(
        store.summary(FeatureStatus::Supported, None).unread_count,
        1,
        "one unread fragment keeps the joined message unread"
    );

    assert!(store.mark_read(2, &storage()));
    assert_eq!(
        store.summary(FeatureStatus::Supported, None).unread_count,
        0,
        "marking the remaining fragment read marks the joined message read"
    );
}

#[test]
fn marking_one_fragment_read_marks_the_whole_long_message() {
    let mut store = SmsStore::new();
    assert!(store.ingest(fragment(1, "+86", "甲", 4, 2, 1)));
    assert!(store.ingest(fragment(2, "+86", "乙", 4, 2, 2)));
    assert_eq!(
        store.summary(FeatureStatus::Supported, None).unread_count,
        1
    );

    assert!(store.mark_read(2, &storage()));
    assert_eq!(
        store.summary(FeatureStatus::Supported, None).unread_count,
        0,
        "the read command carries one fragment index; its whole group follows"
    );
    assert!(!store.mark_read(2, &storage()), "already read is a no-op");
}

#[test]
fn the_controller_snapshot_presents_the_joined_long_message() {
    let mut controller = Controller::for_test(NOW);
    assert!(controller.ingest_sms(fragment(1, "+86", "甲", 4, 2, 1)));
    assert!(controller.ingest_sms(fragment(2, "+86", "乙", 4, 2, 2)));

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.sms_inbox.message_count, 1);
    assert_eq!(snapshot.sms_messages.len(), 1);
    assert_eq!(snapshot.sms_messages[0].body(), "甲乙");
    assert_eq!(snapshot.sms_messages[0].status, SmsStatus::Received);
    assert_eq!(
        controller.state().sms_store().messages().len(),
        2,
        "the store keeps the raw fragments behind the presentation"
    );
}
