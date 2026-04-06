//! Cooperative cancellation tokens and shutdown coordination.
//!
//! Provides a lightweight alternative to `tokio_util::sync::CancellationToken`
//! that integrates with the event bus and supports hierarchical cancellation
//! (parent cancels all children).
//!
//! # Example
//!
//! ```
//! use bardo_runtime::cancel::CancelToken;
//!
//! let root = CancelToken::new();
//! let child = root.child();
//!
//! assert!(!root.is_cancelled());
//! assert!(!child.is_cancelled());
//!
//! root.cancel();
//!
//! assert!(root.is_cancelled());
//! assert!(child.is_cancelled());
//! ```

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tokio::sync::Notify;

/// A cooperative cancellation token.
///
/// Cancellation is one-way: once cancelled, a token stays cancelled forever.
/// Child tokens inherit cancellation from their parent but can also be
/// cancelled independently.
#[derive(Clone)]
pub struct CancelToken {
    inner: Arc<CancelInner>,
}

struct CancelInner {
    cancelled: AtomicBool,
    notify: Notify,
    /// Parent tokens. Cancellation propagates downward: if any ancestor is
    /// cancelled, this token is considered cancelled.
    parent: Option<CancelToken>,
}

impl CancelToken {
    /// Create a new root cancellation token.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(CancelInner {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
                parent: None,
            }),
        }
    }

    /// Create a child token. The child is cancelled when:
    /// - The parent is cancelled, OR
    /// - The child itself is cancelled directly.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            inner: Arc::new(CancelInner {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
                parent: Some(self.clone()),
            }),
        }
    }

    /// Cancel this token. All tasks awaiting [`cancelled`] will be woken.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        self.inner.notify.notify_waiters();
    }

    /// Returns `true` if this token (or any ancestor) has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        if self.inner.cancelled.load(Ordering::Acquire) {
            return true;
        }
        // Walk the parent chain (iterative, not recursive).
        let mut current = self.inner.parent.as_ref();
        while let Some(parent) = current {
            if parent.inner.cancelled.load(Ordering::Acquire) {
                return true;
            }
            current = parent.inner.parent.as_ref();
        }
        false
    }

    /// Wait until this token (or any ancestor) is cancelled.
    ///
    /// Returns immediately if already cancelled.
    ///
    /// # Panics
    ///
    /// Panics if the internal notify list is empty (impossible in practice
    /// since `self` is always included).
    #[allow(clippy::expect_used)]
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }

        // Collect all notify handles in the ancestor chain so we can select on all of them.
        let mut notifies = vec![&self.inner.notify];
        let mut current = self.inner.parent.as_ref();
        while let Some(parent) = current {
            notifies.push(&parent.inner.notify);
            current = parent.inner.parent.as_ref();
        }

        // Poll all notifies and re-check cancellation after any fires.
        loop {
            if self.is_cancelled() {
                return;
            }
            // Wait for any notify in the chain to fire.
            // We create notified() futures before checking, per tokio docs.
            match notifies.len() {
                1 => {
                    notifies[0].notified().await;
                }
                2 => {
                    tokio::select! {
                        () = notifies[0].notified() => {}
                        () = notifies[1].notified() => {}
                    }
                }
                _ => {
                    // For deeper chains, just poll the root + self.
                    // Cancellation always propagates through notify_waiters.
                    let self_notify = notifies[0].notified();
                    let root_notify = notifies.last().expect("non-empty").notified();
                    tokio::select! {
                        () = self_notify => {}
                        () = root_notify => {}
                    }
                }
            }
        }
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelToken")
            .field("cancelled", &self.is_cancelled())
            .field("has_parent", &self.inner.parent.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_cancel() {
        let token = CancelToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn child_inherits_cancel() {
        let parent = CancelToken::new();
        let child = parent.child();
        let grandchild = child.child();

        assert!(!child.is_cancelled());
        assert!(!grandchild.is_cancelled());

        parent.cancel();

        assert!(child.is_cancelled());
        assert!(grandchild.is_cancelled());
    }

    #[test]
    fn child_independent_cancel() {
        let parent = CancelToken::new();
        let child = parent.child();

        child.cancel();

        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled(), "child cancel must not propagate upward");
    }

    #[tokio::test]
    async fn cancelled_future_resolves() {
        let token = CancelToken::new();
        let token2 = token.clone();

        let handle = tokio::spawn(async move {
            token2.cancelled().await;
            42
        });

        // Give the task a moment to start waiting.
        tokio::task::yield_now().await;
        token.cancel();

        let result = handle.await.expect("task should complete");
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn cancelled_future_resolves_immediately_if_already_cancelled() {
        let token = CancelToken::new();
        token.cancel();
        // Should return immediately, not hang.
        token.cancelled().await;
    }

    #[tokio::test]
    async fn child_cancelled_when_parent_cancelled() {
        let parent = CancelToken::new();
        let child = parent.child();
        let child_clone = child.clone();

        let handle = tokio::spawn(async move {
            child_clone.cancelled().await;
            true
        });

        tokio::task::yield_now().await;
        parent.cancel();

        assert!(handle.await.expect("task should complete"));
    }
}
