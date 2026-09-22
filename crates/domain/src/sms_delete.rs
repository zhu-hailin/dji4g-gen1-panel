//! Content-free cancellation and acknowledgement for one checked SMS fragment deletion.
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub const SMS_DELETE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SmsDeleteItemResult {
    Deleted,
    Failed,
    OutcomeUnknown,
    NotAttempted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SmsDeleteReceipt {
    pub result: SmsDeleteItemResult,
    pub code: Option<String>,
}

#[derive(Debug)]
struct State {
    deadline: Instant,
    cancelled: AtomicBool,
    attempted: AtomicBool,
    cleanup_pending: AtomicBool,
}

#[derive(Clone, Debug)]
pub struct SmsDeleteControl(Arc<State>);

impl SmsDeleteControl {
    pub fn new(timeout: Duration) -> Self {
        Self(Arc::new(State {
            deadline: Instant::now() + timeout,
            cancelled: AtomicBool::new(false),
            attempted: AtomicBool::new(false),
            cleanup_pending: AtomicBool::new(false),
        }))
    }
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.deadline()
    }
    pub fn remaining(&self) -> Duration {
        self.deadline().saturating_duration_since(Instant::now())
    }
    pub fn deadline(&self) -> Instant {
        self.0.deadline
    }
    /// Called immediately before the real CMGD write, including potentially partial writes.
    pub fn mark_delete_attempted(&self) {
        self.0.attempted.store(true, Ordering::Release);
    }
    pub fn delete_attempted(&self) -> bool {
        self.0.attempted.load(Ordering::Acquire)
    }
    pub fn cleanup_pending(&self) -> bool {
        self.0.cleanup_pending.load(Ordering::Acquire)
    }
    pub fn mark_cleanup_pending(&self) {
        self.0.cleanup_pending.store(true, Ordering::Release);
    }
    pub fn mark_cleanup_complete(&self) {
        self.0.cleanup_pending.store(false, Ordering::Release);
    }
}
