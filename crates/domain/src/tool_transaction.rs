//! Cancellation and write-attempt tracking shared by every layer of a device-tool transaction.
//!
//! The same handle reaches the panel (which cancels it), the application runner (which budgets it)
//! and the serial actor (which checks it between reads). It lives in the domain crate because the
//! platform crate must not depend on the application crate, and it uses nothing but the standard
//! library so every layer can hold one.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// One transaction's control word. Cloning shares the same state: a clone cannot carry a copy of
/// the flags, so cancelling through any handle is visible to all of them.
#[derive(Clone)]
pub struct ToolTransactionControl {
    inner: Arc<Inner>,
}

struct Inner {
    cancelled: AtomicBool,
    /// Set *before* the first OS write is attempted. A partial write already changed the module's
    /// input, so the transaction's effect is unknown from that moment on — waiting for
    /// `write_all` to succeed would classify a half-written command as "nothing happened".
    write_attempted: AtomicBool,
    deadline: Instant,
}

impl ToolTransactionControl {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                cancelled: AtomicBool::new(false),
                write_attempted: AtomicBool::new(false),
                deadline: Instant::now() + timeout,
            }),
        }
    }

    /// Request cancellation. Idempotent, and safe to call from any thread.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Record that the command is about to be handed to the operating system.
    pub fn mark_write_attempted(&self) {
        self.inner.write_attempted.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn write_attempted(&self) -> bool {
        self.inner.write_attempted.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.inner.deadline
    }

    /// Time left before the absolute deadline; zero once it has passed.
    #[must_use]
    pub fn remaining(&self) -> Duration {
        self.inner
            .deadline
            .saturating_duration_since(Instant::now())
    }

    #[must_use]
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.inner.deadline
    }
}

impl fmt::Debug for ToolTransactionControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // No request text and no response text ever passes through this handle.
        formatter
            .debug_struct("ToolTransactionControl")
            .field("cancelled", &self.is_cancelled())
            .field("write_attempted", &self.write_attempted())
            .field("remaining", &self.remaining())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn a_clone_shares_cancellation_and_the_write_marker() {
        let control = ToolTransactionControl::new(Duration::from_secs(30));
        let clone = control.clone();
        assert!(!clone.is_cancelled());
        assert!(!clone.write_attempted());
        control.cancel();
        clone.mark_write_attempted();
        // Both handles observe both flags: nothing is copied by value.
        assert!(clone.is_cancelled());
        assert!(control.write_attempted());
    }

    #[test]
    fn cancellation_is_visible_from_another_thread() {
        let control = ToolTransactionControl::new(Duration::from_secs(30));
        let other = control.clone();
        let handle = thread::spawn(move || {
            while !other.is_cancelled() {
                thread::sleep(Duration::from_millis(1));
            }
            other.write_attempted()
        });
        control.mark_write_attempted();
        control.cancel();
        assert!(handle.join().expect("thread"));
    }

    #[test]
    fn remaining_shrinks_and_never_goes_negative() {
        let control = ToolTransactionControl::new(Duration::from_millis(40));
        let first = control.remaining();
        assert!(first <= Duration::from_millis(40));
        thread::sleep(Duration::from_millis(80));
        assert_eq!(control.remaining(), Duration::ZERO);
        assert!(control.is_expired());
    }
}
