//! Back-pressure policies and per-subscription counters.
//!
//! When a [`super::SubscriptionSink`] reports that it is full (or closed), the
//! subscription applies a [`BackpressurePolicy`] to decide what to do next.
//! Three stock policies are provided:
//!
//! - [`BackpressurePolicy::DropOldest`] — the sink has already evicted the
//!   oldest event (broadcast semantics); we accept the drop and advance the
//!   `dropped_oldest` counter.
//! - [`BackpressurePolicy::DropNewest`] — the push itself is refused; the
//!   rejected event is discarded and the `dropped_newest` counter advances.
//!   Existing buffered events survive.
//! - [`BackpressurePolicy::CloseOnOverflow`] — the first overflow marks the
//!   subscription as closed. Future pushes return `SinkError::Closed`.
//!
//! Each policy is paired with a [`SubscriptionStats`] accumulator per
//! subscription, so operators can monitor loss and act on sustained overflow.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// How a [`super::PheromoneSubscription`] or [`super::InsightSubscription`]
/// reacts when its sink reports a full buffer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum BackpressurePolicy {
    /// Accept the drop of the oldest buffered event. The newest push is
    /// recorded as delivered and `dropped_oldest` is incremented by the
    /// number of evicted events.
    #[default]
    DropOldest,
    /// Refuse the newest push. The rejected event is discarded and
    /// `dropped_newest` is incremented by one. Existing buffered events
    /// remain intact.
    DropNewest,
    /// Close the subscription on the first overflow. All subsequent pushes
    /// fail with `SinkError::Closed` and `dropped_newest` is incremented.
    CloseOnOverflow,
}

/// Running counters for a single subscription.
///
/// Counters are monotonic; they only ever go up. Use [`Self::snapshot`] to
/// read a stable view.
#[derive(Debug)]
pub struct SubscriptionCounters {
    delivered: AtomicU64,
    dropped_oldest: AtomicU64,
    dropped_newest: AtomicU64,
    closed: AtomicBool,
}

impl SubscriptionCounters {
    /// Constructs fresh zeroed counters.
    #[must_use]
    pub fn new() -> Self {
        Self {
            delivered: AtomicU64::new(0),
            dropped_oldest: AtomicU64::new(0),
            dropped_newest: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        }
    }

    /// Records one delivered event.
    pub fn record_delivered(&self) {
        self.delivered.fetch_add(1, Ordering::Relaxed);
    }

    /// Records `n` events dropped due to oldest-drop policy.
    pub fn record_dropped_oldest(&self, n: u64) {
        self.dropped_oldest.fetch_add(n, Ordering::Relaxed);
    }

    /// Records `n` events dropped due to newest-drop policy.
    pub fn record_dropped_newest(&self, n: u64) {
        self.dropped_newest.fetch_add(n, Ordering::Relaxed);
    }

    /// Marks the subscription as closed.
    pub fn mark_closed(&self) {
        self.closed.store(true, Ordering::Release);
    }

    /// Returns whether this subscription is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Returns a stable snapshot of the current counter values.
    #[must_use]
    pub fn snapshot(&self) -> SubscriptionStats {
        SubscriptionStats {
            delivered: self.delivered.load(Ordering::Relaxed),
            dropped_oldest: self.dropped_oldest.load(Ordering::Relaxed),
            dropped_newest: self.dropped_newest.load(Ordering::Relaxed),
            closed: self.closed.load(Ordering::Acquire),
        }
    }
}

impl Default for SubscriptionCounters {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable snapshot of [`SubscriptionCounters`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubscriptionStats {
    /// Total number of events successfully pushed to the sink.
    pub delivered: u64,
    /// Total number of events dropped because they were the oldest in the
    /// sink buffer when an overflow occurred.
    pub dropped_oldest: u64,
    /// Total number of events dropped because they were refused at push
    /// time (newest-drop or close-on-overflow policy).
    pub dropped_newest: u64,
    /// Whether the subscription has been closed.
    pub closed: bool,
}

impl SubscriptionStats {
    /// Total drops (oldest + newest).
    #[must_use]
    pub fn total_dropped(&self) -> u64 {
        self.dropped_oldest + self.dropped_newest
    }

    /// Zero-valued snapshot.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            delivered: 0,
            dropped_oldest: 0,
            dropped_newest: 0,
            closed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_record_and_snapshot() {
        let c = SubscriptionCounters::new();
        c.record_delivered();
        c.record_delivered();
        c.record_dropped_oldest(3);
        c.record_dropped_newest(1);
        let snap = c.snapshot();
        assert_eq!(snap.delivered, 2);
        assert_eq!(snap.dropped_oldest, 3);
        assert_eq!(snap.dropped_newest, 1);
        assert_eq!(snap.total_dropped(), 4);
        assert!(!snap.closed);
    }

    #[test]
    fn mark_closed_is_observable() {
        let c = SubscriptionCounters::new();
        assert!(!c.is_closed());
        c.mark_closed();
        assert!(c.is_closed());
        assert!(c.snapshot().closed);
    }

    #[test]
    fn default_policy_is_drop_oldest() {
        assert_eq!(
            BackpressurePolicy::default(),
            BackpressurePolicy::DropOldest
        );
    }
}
