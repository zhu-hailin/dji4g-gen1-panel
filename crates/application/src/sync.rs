//! Small dependency-free equivalents for the two bounded, non-blocking channels needed by the
//! application boundary. The production workspace can replace these internals with Tokio without
//! changing `ControllerHandle` or the immutable snapshot contract.

use std::sync::{Arc, Mutex, mpsc};

pub mod watch {
    use super::{Arc, Mutex};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct WatchSendError;

    #[derive(Debug)]
    struct State<T> {
        value: T,
        revision: u64,
        sender_count: usize,
        closed: bool,
    }

    #[derive(Debug)]
    pub struct Sender<T> {
        state: Arc<Mutex<State<T>>>,
    }

    #[derive(Debug)]
    pub struct Receiver<T> {
        state: Arc<Mutex<State<T>>>,
        seen_revision: u64,
    }

    pub fn channel<T>(value: T) -> (Sender<T>, Receiver<T>) {
        let state = Arc::new(Mutex::new(State {
            value,
            revision: 0,
            sender_count: 1,
            closed: false,
        }));
        (
            Sender {
                state: Arc::clone(&state),
            },
            Receiver {
                state,
                seen_revision: 0,
            },
        )
    }

    impl<T> Clone for Sender<T> {
        fn clone(&self) -> Self {
            if let Ok(mut state) = self.state.lock() {
                state.sender_count = state.sender_count.saturating_add(1);
            }
            Self {
                state: Arc::clone(&self.state),
            }
        }
    }

    impl<T> Sender<T> {
        pub fn send(&self, value: T) -> Result<(), WatchSendError> {
            let mut state = self.state.lock().map_err(|_| WatchSendError)?;
            if state.closed {
                return Err(WatchSendError);
            }
            state.value = value;
            state.revision = state.revision.saturating_add(1);
            Ok(())
        }

        #[must_use]
        pub fn subscribe(&self) -> Receiver<T>
        where
            T: Clone,
        {
            let state = self.state.lock().expect("watch state lock");
            Receiver {
                state: Arc::clone(&self.state),
                seen_revision: state.revision,
            }
        }
    }

    impl<T> Receiver<T> {
        #[must_use]
        pub fn has_changed(&self) -> bool {
            self.state
                .lock()
                .is_ok_and(|state| state.revision != self.seen_revision)
        }

        #[must_use]
        pub fn borrow(&self) -> T
        where
            T: Clone,
        {
            self.state.lock().expect("watch state lock").value.clone()
        }

        #[must_use]
        pub fn borrow_and_update(&mut self) -> T
        where
            T: Clone,
        {
            let state = self.state.lock().expect("watch state lock");
            self.seen_revision = state.revision;
            state.value.clone()
        }

        #[must_use]
        pub fn is_closed(&self) -> bool {
            self.state.lock().is_ok_and(|state| state.closed)
        }
    }

    impl<T> Clone for Receiver<T>
    where
        T: Clone,
    {
        fn clone(&self) -> Self {
            Self {
                state: Arc::clone(&self.state),
                seen_revision: self.seen_revision,
            }
        }
    }

    impl<T> Drop for Sender<T> {
        fn drop(&mut self) {
            if let Ok(mut state) = self.state.lock() {
                state.sender_count = state.sender_count.saturating_sub(1);
                state.closed = state.sender_count == 0;
            }
        }
    }
}

pub type Sender<T> = mpsc::SyncSender<T>;
pub type Receiver<T> = mpsc::Receiver<T>;

pub fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    mpsc::sync_channel(capacity)
}
