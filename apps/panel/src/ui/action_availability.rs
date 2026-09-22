//! Shared presentation gate for every controlled repair entry point.
use crate::localization::{Language, LocalizedText, TextKey, failure_text};
use dji4g_application::{ActionReadinessKey, ControllerSnapshot};
use dji4g_domain::{Availability, Freshness};
use std::time::SystemTime;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionAvailability {
    pub enabled: bool,
    pub reason: Option<LocalizedText>,
}

pub fn repair_action_availability(
    snapshot: &ControllerSnapshot,
    key: ActionReadinessKey,
    now: SystemTime,
    language: Language,
) -> ActionAvailability {
    let app = &snapshot.app;
    let reason = if snapshot.serial_work_busy {
        Some(LocalizedText::new(language, TextKey::CommandFeedbackBusy))
    } else if app.availability == Availability::UnsupportedDevice {
        Some(LocalizedText::new(
            language,
            TextKey::AvailabilityUnsupportedReason,
        ))
    } else if app.device.is_none() || app.availability == Availability::NotDetected {
        Some(LocalizedText::new(
            language,
            TextKey::AvailabilityNotDetectedReason,
        ))
    } else if app.freshness != Freshness::Fresh
        || snapshot.diagnostics.iter().any(|check| {
            matches!(
                check.id,
                dji4g_application::DiagnosticCheckId::UsbDevice
                    | dji4g_application::DiagnosticCheckId::AtControl
            ) && check.freshness(now) == Freshness::Stale
        })
    {
        Some(LocalizedText::new(language, TextKey::ErrorEvidenceExpired))
    } else {
        match snapshot
            .action_readiness
            .iter()
            .find(|entry| entry.key == key)
        {
            Some(entry) => entry
                .ready
                .as_ref()
                .err()
                .map(|code| failure_text(code, language)),
            None => Some(LocalizedText::new(language, TextKey::ErrorEvidenceExpired)),
        }
    };
    ActionAvailability {
        enabled: reason.is_none(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn busy_is_the_same_reason_even_without_a_device() {
        let now = SystemTime::now();
        let mut snapshot = dji4g_application::ReducerState::new(now).snapshot();
        snapshot.serial_work_busy = true;
        for key in [
            ActionReadinessKey::EditApn,
            ActionReadinessKey::SetUsbNetworkProfile,
            ActionReadinessKey::RestartModule,
        ] {
            let value = repair_action_availability(&snapshot, key, now, Language::ZhCn);
            assert!(!value.enabled);
            assert_eq!(value.reason.unwrap().key, TextKey::CommandFeedbackBusy);
        }
    }
    #[test]
    fn ready_current_repair_expires_when_its_evidence_expires() {
        let now = SystemTime::now();
        let snapshot = dji4g_application::ReducerState::test_ready(now).snapshot();
        let key = ActionReadinessKey::RestartModule;
        let baseline = repair_action_availability(&snapshot, key, now, Language::ZhCn);
        assert!(baseline.enabled);
        let expired = repair_action_availability(
            &snapshot,
            key,
            now + std::time::Duration::from_secs(3600),
            Language::ZhCn,
        );
        assert!(!expired.enabled);
        assert_eq!(expired.reason.unwrap().key, TextKey::ErrorEvidenceExpired);
    }
}
