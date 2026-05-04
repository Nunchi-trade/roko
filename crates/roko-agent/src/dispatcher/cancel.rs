//! Cancellation threading (§36.45).
//!
//! The dispatcher runs handlers inside a `tokio::select!` between two
//! futures: the timeout-wrapped handler, and this module's
//! [`wait_cancelled`] watcher. When either future resolves first, the
//! other is dropped.
//!
//! [`CancelToken`] now has an async [`CancelToken::cancelled`] method that
//! implementations can back with a real notification primitive (e.g.
//! [`AtomicCancel`] uses `tokio::sync::Notify` for instant wake-up).
//! Foreign impls that only provide `is_cancelled` automatically get a
//! 50 ms polling fallback via the trait's default implementation.

use roko_core::tool::CancelToken;

/// Await until the token fires.
///
/// Delegates entirely to [`CancelToken::cancelled`]. Tokens backed by
/// `tokio::sync::Notify` (e.g. [`AtomicCancel`]) wake instantly with zero
/// CPU overhead; others fall back to 50 ms polling via the trait default.
pub async fn wait_cancelled(token: &dyn CancelToken) {
    token.cancelled().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{AtomicCancel, CancelToken};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn wait_cancelled_returns_immediately_if_already_tripped() {
        let token = AtomicCancel::new();
        token.cancel();
        let started = Instant::now();
        wait_cancelled(&token as &dyn CancelToken).await;
        // Should complete in well under one poll interval because the
        // fast-path check doesn't sleep.
        assert!(
            started.elapsed() < Duration::from_millis(30),
            "pre-tripped token should return on fast path, took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn wait_cancelled_returns_after_trip() {
        let token = Arc::new(AtomicCancel::new());
        let tripper = token.clone();
        // Trip the token after ~80 ms from a background task.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            tripper.cancel();
        });
        let started = Instant::now();
        wait_cancelled(token.as_ref() as &dyn CancelToken).await;
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(50),
            "watcher returned too early: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "watcher returned too late: {elapsed:?}"
        );
    }
}
