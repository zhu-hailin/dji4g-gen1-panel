//! SMS age uses completed storage reads, never the unrelated device-monitor timestamp.
use std::time::{Duration, SystemTime};

use dji4g_domain::SmsInboxSummary;
use eframe::egui::{RichText, Ui};

use crate::localization::{Language, LocalizedText, TextKey, format_positional};

use super::{StatusTone, sms::SmsComposeState};

const STALE_AFTER: Duration = Duration::from_secs(45);

struct SyncVm {
    last_read: Option<String>,
    status: TextKey,
    tone: StatusTone,
}

fn sync_vm(
    summary: &SmsInboxSummary,
    pending: bool,
    failed: bool,
    partial: bool,
    paused: bool,
    now: SystemTime,
    language: Language,
) -> SyncVm {
    let observed = summary
        .last_synced_at
        .zip(summary.last_synced_storage.as_ref())
        .and_then(|(at, storage)| now.duration_since(at).ok().map(|age| (age, storage)));
    let last_read = observed.as_ref().map(|(age, storage)| {
        format_positional(
            language,
            TextKey::SmsLastSynced,
            &[&storage.0, &super::format_age(*age, language).text],
        )
    });
    let (status, tone) = if pending {
        (TextKey::SmsSyncReadingCache, StatusTone::Progress)
    } else if failed {
        (TextKey::SmsSyncFailedCache, StatusTone::Caution)
    } else if partial {
        (TextKey::SmsSyncPartialCache, StatusTone::Caution)
    } else if paused {
        (TextKey::SmsSyncPausedCache, StatusTone::Neutral)
    } else if let Some((age, _)) = observed {
        if age >= STALE_AFTER {
            (TextKey::SmsSyncStaleCache, StatusTone::Caution)
        } else {
            (TextKey::SmsSyncStoredList, StatusTone::Neutral)
        }
    } else {
        (TextKey::SmsSyncNotCompleted, StatusTone::Caution)
    };
    SyncVm {
        last_read,
        status,
        tone,
    }
}

pub(crate) fn render(
    ui: &mut Ui,
    snapshot: &dji4g_application::ControllerSnapshot,
    language: Language,
    state: &SmsComposeState,
) {
    let vm = sync_vm(
        &snapshot.sms_inbox,
        snapshot.sms_refresh_pending,
        snapshot.sms_inbox_failure.is_some() || state.refresh_error.is_some(),
        snapshot
            .sms_read_report
            .as_ref()
            .is_some_and(|report| !report.complete),
        state.auto_refresh_paused
            || !state.search_shortcuts_allowed()
            || snapshot.serial_work_busy
            || state.module_read_only
            || snapshot.app.device.is_none(),
        SystemTime::now(),
        language,
    );
    ui.horizontal_wrapped(|ui| {
        if let Some(last_read) = vm.last_read {
            ui.label(super::meta_text(last_read));
        }
        ui.label(
            RichText::new(LocalizedText::new(language, vm.status).text)
                .size(12.0)
                .color(vm.tone.color()),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use dji4g_domain::SmsStorageId;

    fn summary(at: SystemTime) -> SmsInboxSummary {
        SmsInboxSummary {
            last_synced_at: Some(at),
            last_synced_storage: Some(SmsStorageId("SM".into())),
            ..Default::default()
        }
    }

    #[test]
    fn sms_age_expires_without_a_new_device_snapshot() {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let summary = summary(at);
        for language in crate::localization::available_languages() {
            let recent = sync_vm(
                &summary,
                false,
                false,
                false,
                false,
                at + Duration::from_secs(44),
                *language,
            );
            assert_eq!(recent.status, TextKey::SmsSyncStoredList);
            assert!(recent.last_read.as_ref().unwrap().contains("SM"));
            let stale = sync_vm(
                &summary,
                false,
                false,
                false,
                false,
                at + STALE_AFTER,
                *language,
            );
            assert_eq!(stale.status, TextKey::SmsSyncStaleCache);
        }
    }

    #[test]
    fn failure_and_pending_do_not_relabel_cache_as_current() {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let summary = summary(at);
        let failed = sync_vm(&summary, false, true, false, false, at, Language::ZhCn);
        assert_eq!(failed.status, TextKey::SmsSyncFailedCache);
        let pending = sync_vm(&summary, true, true, false, false, at, Language::ZhCn);
        assert_eq!(pending.status, TextKey::SmsSyncReadingCache);
        assert_eq!(failed.last_read, pending.last_read);
        let paused = sync_vm(&summary, false, false, false, true, at, Language::ZhCn);
        assert_eq!(paused.status, TextKey::SmsSyncPausedCache);
        let partial = sync_vm(&summary, false, false, true, false, at, Language::ZhCn);
        assert_eq!(partial.status, TextKey::SmsSyncPartialCache);
    }

    #[test]
    fn absent_future_or_unanchored_sync_is_not_fresh() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        for summary in [
            SmsInboxSummary::default(),
            summary(now + Duration::from_secs(1)),
            SmsInboxSummary {
                last_synced_at: Some(now),
                ..Default::default()
            },
        ] {
            let vm = sync_vm(&summary, false, false, false, false, now, Language::ZhCn);
            assert_eq!(vm.status, TextKey::SmsSyncNotCompleted);
            assert!(vm.last_read.is_none());
        }
    }
}
