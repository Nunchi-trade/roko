//! Dispatcher timeout enforcement (§36.40).
//!
//! Wraps `handler.execute(call, ctx)` in `tokio::time::timeout(…)` and
//! maps the elapsed future to [`ToolError::Timeout`] carrying the actual
//! wall-time consumed before cancellation.
//!
//! This module is intentionally small and self-contained: the dispatcher
//! calls [`with_timeout`] from inside its `tokio::select!` against the
//! cancellation watcher, so the timeout logic is independently
//! unit-testable.

use std::time::{Duration, Instant};

use roko_core::tool::{ToolError, ToolResult};

/// Run a future with a timeout, converting elapsed to
/// [`ToolError::Timeout`].
///
/// On timeout, the returned [`ToolError::Timeout::after_ms`] records how
/// many milliseconds actually elapsed before cancellation (not the
/// configured timeout). Uses [`u64::try_from`] defensively so exotic
/// platforms with 128-bit `Duration::as_millis` results can't panic.
pub async fn with_timeout<F>(timeout: Duration, fut: F) -> ToolResult
where
    F: std::future::Future<Output = ToolResult> + Send,
{
    let started = Instant::now();
    match tokio::time::timeout(timeout, fut).await {
        Ok(res) => res,
        Err(_elapsed) => {
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            ToolResult::err(ToolError::Timeout {
                after_ms: elapsed_ms,
            })
        }
    }
}

/// Build a [`ToolError::Timeout`] recording how many ms elapsed.
#[must_use]
pub const fn timeout_error(after_ms: u64) -> ToolError {
    ToolError::Timeout { after_ms }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::ToolResult;

    #[tokio::test(start_paused = true)]
    async fn with_timeout_expires_when_future_slow() {
        let slow = async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            ToolResult::text("done")
        };
        let out = tokio::spawn(with_timeout(Duration::from_millis(50), slow));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(50)).await;
        let out = out.await.expect("timeout task should join");
        match out {
            ToolResult::Err(ToolError::Timeout { after_ms }) => {
                // Elapsed should be close to 50 ms (the cap).
                assert!(
                    after_ms < 150,
                    "after_ms={after_ms} should be near the 50ms cap, not the 200ms sleep"
                );
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn with_timeout_passes_through_when_fast() {
        let fast = async { ToolResult::text("hello") };
        let out = with_timeout(Duration::from_millis(500), fast).await;
        match out {
            ToolResult::Ok { content, .. } => assert_eq!(content, "hello"),
            other => panic!("expected Ok, got {other:?}"),
        }
    }
}
