use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;

/// A latching shutdown signal shared by the server, the metrics ticker, and
/// every inbound listener task.
///
/// A bare `Notify` is not enough here: `notify_waiters()` only wakes futures
/// that have *already* registered, so a task spawned moments before `stop()`
/// would miss the wake-up entirely and run forever — leaving `stop()` blocked
/// on its `JoinHandle`. Latching the state in an atomic makes the signal
/// edge-independent: a task that starts waiting after `trigger()` returns
/// immediately.
#[derive(Debug, Default)]
pub struct Shutdown {
    triggered: AtomicBool,
    notify: Notify,
}

impl Shutdown {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Returns `true` once `trigger` has been called and `reset` has not.
    pub fn is_triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }

    /// Latches the signal and wakes every current waiter.
    pub fn trigger(&self) {
        self.triggered.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Clears the signal so the gateway can be started again after a stop.
    pub fn reset(&self) {
        self.triggered.store(false, Ordering::SeqCst);
    }

    /// Resolves as soon as the signal is (or already was) triggered.
    pub async fn wait(&self) {
        loop {
            // Register interest *before* checking the flag, so a `trigger()`
            // landing between the check and the await still wakes us.
            let mut notified = pin!(self.notify.notified());
            notified.as_mut().enable();

            if self.is_triggered() {
                return;
            }

            notified.await;

            if self.is_triggered() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn wait_returns_immediately_when_already_triggered() {
        let shutdown = Shutdown::new();
        shutdown.trigger();

        // The key regression: a waiter that arrives *after* the trigger must
        // not block. With a bare `Notify` this would hang.
        tokio::time::timeout(Duration::from_secs(1), shutdown.wait())
            .await
            .expect("wait() must not block once the signal is latched");
    }

    #[tokio::test]
    async fn wait_wakes_on_later_trigger() {
        let shutdown = Shutdown::new();
        let waiter = Arc::clone(&shutdown);
        let handle = tokio::spawn(async move { waiter.wait().await });

        tokio::task::yield_now().await;
        shutdown.trigger();

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("waiter should be woken")
            .expect("waiter task should not panic");
    }

    #[tokio::test]
    async fn reset_allows_reuse_after_stop() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        assert!(shutdown.is_triggered());

        shutdown.reset();
        assert!(!shutdown.is_triggered());

        // After a reset the signal must block again until re-triggered.
        assert!(
            tokio::time::timeout(Duration::from_millis(100), shutdown.wait())
                .await
                .is_err(),
            "wait() should block again after reset"
        );
    }
}
