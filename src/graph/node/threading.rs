//! Cancellation support for computations running on worker threads.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Default, Debug)]
pub(crate) struct CancelSignal {
    cancelled: Mutex<bool>,
    condvar: Condvar,
}

impl CancelSignal {
    pub(crate) fn cancel(&self) {
        *self.cancelled.lock().unwrap() = true;
        self.condvar.notify_all();
    }

    pub(crate) fn wait_or_cancelled(&self, timeout: Duration) -> bool {
        let cancelled = self.cancelled.lock().unwrap();
        let (cancelled, _) = self
            .condvar
            .wait_timeout_while(cancelled, timeout, |cancelled| !*cancelled)
            .unwrap();

        *cancelled
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;
    use std::time::Instant;

    use super::*;

    const SHORT_TIMEOUT: Duration = Duration::from_millis(10);
    const LONG_TIMEOUT: Duration = Duration::from_secs(10);
    const CANCEL_DELAY: Duration = Duration::from_millis(50);
    const EARLY_WAKE_BOUND: Duration = Duration::from_secs(5);

    #[test]
    fn wait_or_cancelled_without_cancel_returns_false_after_timeout() {
        let signal = CancelSignal::default();

        let result = signal.wait_or_cancelled(SHORT_TIMEOUT);

        assert!(!result);
    }

    #[test]
    fn wait_or_cancelled_after_cancel_returns_true() {
        let signal = CancelSignal::default();
        signal.cancel();

        let result = signal.wait_or_cancelled(SHORT_TIMEOUT);

        assert!(result);
    }

    #[test]
    fn cancel_called_twice_keeps_signal_cancelled() {
        let signal = CancelSignal::default();

        signal.cancel();
        signal.cancel();

        assert!(signal.wait_or_cancelled(SHORT_TIMEOUT));
    }

    #[test]
    fn cancel_from_another_thread_wakes_waiter_before_timeout() {
        let signal = Arc::new(CancelSignal::default());
        let canceller = Arc::clone(&signal);
        let started = Instant::now();
        let handle = thread::spawn(move || {
            thread::sleep(CANCEL_DELAY);
            canceller.cancel();
        });

        let result = signal.wait_or_cancelled(LONG_TIMEOUT);

        handle.join().unwrap();
        assert!(result);
        assert!(started.elapsed() < EARLY_WAKE_BOUND);
    }
}
