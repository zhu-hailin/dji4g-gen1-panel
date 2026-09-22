use crate::{SmsDeleteControl, SmsStorageId};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

/// Closed wire tokens; Combined is permitted only for restoring a confirmed MT holder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmsReadStorage {
    Sim,
    Device,
    Combined,
}
impl SmsReadStorage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sim => "SM",
            Self::Device => "ME",
            Self::Combined => "MT",
        }
    }
    pub fn from_token(value: &str) -> Option<Self> {
        match value {
            "SM" => Some(Self::Sim),
            "ME" => Some(Self::Device),
            "MT" => Some(Self::Combined),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SmsReadPhase {
    #[default]
    Verifying,
    ReadingStorage,
    SwitchingStorage,
    Listing,
    Decoding,
    RestoringStorage,
    Complete,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SmsStorageRestoration {
    #[default]
    NotNeeded,
    Restored,
    Unknown,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SmsReadReport {
    pub storage: Option<SmsStorageId>,
    pub capacity: Option<(u32, u32)>,
    pub raw_records: usize,
    pub decoded_records: usize,
    pub skipped_records: usize,
    pub supported_storages: Vec<SmsStorageId>,
    pub restoration: SmsStorageRestoration,
}
#[derive(Debug)]
struct State {
    transport: SmsDeleteControl,
    phase: AtomicU8,
    progress: AtomicUsize,
    restoration: AtomicU8,
}
#[derive(Clone, Debug)]
pub struct SmsReadControl(Arc<State>);
impl SmsReadControl {
    pub fn new(timeout: Duration) -> Self {
        Self(Arc::new(State {
            transport: SmsDeleteControl::new(timeout),
            phase: AtomicU8::new(0),
            progress: AtomicUsize::new(0),
            restoration: AtomicU8::new(0),
        }))
    }
    /// Shares the exact native opener and lease-release acknowledgement used by checked deletion.
    pub fn transport_control(&self) -> &SmsDeleteControl {
        &self.0.transport
    }
    pub fn cancel(&self) {
        self.0.transport.cancel();
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.transport.is_cancelled()
    }
    pub fn is_expired(&self) -> bool {
        self.0.transport.is_expired()
    }
    pub fn remaining(&self) -> Duration {
        self.0.transport.remaining()
    }
    pub fn deadline(&self) -> Instant {
        self.0.transport.deadline()
    }
    pub fn cleanup_pending(&self) -> bool {
        self.0.transport.cleanup_pending()
    }
    pub fn mark_cleanup_pending(&self) {
        self.0.transport.mark_cleanup_pending();
    }
    pub fn mark_cleanup_complete(&self) {
        self.0.transport.mark_cleanup_complete();
    }
    pub fn set_phase(&self, phase: SmsReadPhase) {
        self.0.phase.store(phase as u8, Ordering::Release);
    }
    pub fn phase(&self) -> SmsReadPhase {
        match self.0.phase.load(Ordering::Acquire) {
            1 => SmsReadPhase::ReadingStorage,
            2 => SmsReadPhase::SwitchingStorage,
            3 => SmsReadPhase::Listing,
            4 => SmsReadPhase::Decoding,
            5 => SmsReadPhase::RestoringStorage,
            6 => SmsReadPhase::Complete,
            _ => SmsReadPhase::Verifying,
        }
    }
    pub fn progress(&self) -> usize {
        self.0.progress.load(Ordering::Acquire)
    }
    pub fn set_progress(&self, count: usize) {
        self.0.progress.store(count, Ordering::Release);
    }
    pub fn set_restoration(&self, value: SmsStorageRestoration) {
        self.0.restoration.store(value as u8, Ordering::Release);
    }
    pub fn restoration(&self) -> SmsStorageRestoration {
        match self.0.restoration.load(Ordering::Acquire) {
            1 => SmsStorageRestoration::Restored,
            2 => SmsStorageRestoration::Unknown,
            _ => SmsStorageRestoration::NotNeeded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_control_clones_share_cancellation_deadline_progress_and_cleanup() {
        let control = SmsReadControl::new(Duration::from_secs(60));
        let clone = control.clone();
        assert_eq!(control.deadline(), clone.deadline());
        clone.set_phase(SmsReadPhase::Listing);
        clone.set_progress(500);
        clone.mark_cleanup_pending();
        clone.cancel();
        assert_eq!(control.phase(), SmsReadPhase::Listing);
        assert_eq!(control.progress(), 500);
        assert!(control.is_cancelled());
        assert!(control.cleanup_pending());
        control.transport_control().mark_cleanup_complete();
        assert!(!clone.cleanup_pending());
        clone.set_restoration(SmsStorageRestoration::Unknown);
        assert_eq!(control.restoration(), SmsStorageRestoration::Unknown);
    }
    #[test]
    fn history_storage_wire_tokens_are_closed_and_deadline_is_absolute() {
        assert_eq!(
            SmsReadStorage::from_token("MT"),
            Some(SmsReadStorage::Combined)
        );
        assert_eq!(SmsReadStorage::from_token("SM\",\"ME"), None);
        let control = SmsReadControl::new(Duration::ZERO);
        assert!(control.is_expired());
        assert_eq!(control.remaining(), Duration::ZERO);
    }
}
