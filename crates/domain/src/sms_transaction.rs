//! Content-free state shared by the SMS worker, serial actor and presentation layer.
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

pub const SMS_SEND_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SmsSendPhase {
    Queued,
    Preparing,
    Submitting,
    WaitingForResult,
    Finished,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SmsSendResult {
    Submitted,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SmsFailureDetail {
    pub stage: SmsSendPhase,
    pub code: String,
    pub cms_code: Option<u32>,
    pub cme_code: Option<u32>,
    pub os_code: Option<i32>,
    pub submission_possible: bool,
}

impl SmsFailureDetail {
    pub fn new(stage: SmsSendPhase, code: impl Into<String>, submission_possible: bool) -> Self {
        Self {
            stage,
            code: code.into(),
            cms_code: None,
            cme_code: None,
            os_code: None,
            submission_possible,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SmsSendSnapshot {
    pub request_id: u64,
    pub phase: SmsSendPhase,
    pub result: Option<SmsSendResult>,
    pub failure: Option<SmsFailureDetail>,
}

impl SmsSendSnapshot {
    pub fn is_active(&self) -> bool {
        self.phase != SmsSendPhase::Finished
    }
}

#[derive(Clone, Debug)]
pub struct SmsTransactionControl {
    request_id: u64,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    phase: Arc<AtomicU8>,
    submitted: Arc<AtomicBool>,
    cleanup_waiting: Arc<AtomicBool>,
}

impl SmsTransactionControl {
    pub fn new(timeout: Duration) -> Self {
        Self {
            request_id: 0,
            deadline: Instant::now() + timeout,
            cancelled: Arc::new(AtomicBool::new(false)),
            phase: Arc::new(AtomicU8::new(SmsSendPhase::Preparing as u8)),
            submitted: Arc::new(AtomicBool::new(false)),
            cleanup_waiting: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn with_request_id(mut self, id: u64) -> Self {
        self.request_id = id;
        self
    }
    pub fn request_id(&self) -> u64 {
        self.request_id
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn set_phase(&self, phase: SmsSendPhase) {
        self.phase.store(phase as u8, Ordering::Release);
    }
    pub fn phase(&self) -> SmsSendPhase {
        match self.phase.load(Ordering::Acquire) {
            0 => SmsSendPhase::Queued,
            1 => SmsSendPhase::Preparing,
            2 => SmsSendPhase::Submitting,
            3 => SmsSendPhase::WaitingForResult,
            _ => SmsSendPhase::Finished,
        }
    }
    pub fn mark_submission_possible(&self) {
        self.submitted.store(true, Ordering::Release);
    }
    pub fn submission_possible(&self) -> bool {
        self.submitted.load(Ordering::Acquire)
    }
    pub fn mark_cleanup_pending(&self) {
        self.cleanup_waiting.store(true, Ordering::Release);
    }
    pub fn cleanup_pending(&self) -> bool {
        self.cleanup_waiting.load(Ordering::Acquire)
    }
}
