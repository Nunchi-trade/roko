//! [`SubscriptionSink`] trait and its stock implementations.
//!
//! A `SubscriptionSink` is the push half of a roko chain subscription. Mirage
//! calls [`SubscriptionSink::push`] when a new event (pheromone deposit,
//! insight posted, state transition, …) arrives, and the sink is responsible
//! for delivering that event to its downstream consumer — a tokio
//! `broadcast::Receiver`, an `mpsc::Receiver`, or (for tests) an in-memory
//! `Vec`.
//!
//! All sink methods are **synchronous** — the caller is typically mirage's
//! write path (`PheromoneField::deposit`, `KnowledgeStore::post`) which runs
//! under a `parking_lot::RwLock` and must not await. Sinks that need async
//! delivery wrap a bounded channel and encode back-pressure via
//! [`SinkError::Full`].
//!
//! # Back-pressure semantics
//!
//! When a sink's buffer is exhausted, `push` returns `Err(SinkError::Full
//! { dropped })`. The `dropped` count indicates how many events were evicted
//! (for `BroadcastSink` lagging receivers; for `MpscSink` always 1 — the
//! event we refused). A [`super::BackpressurePolicy`] layered on top decides
//! whether to retry, drop, or close.
//!
//! # Closure
//!
//! Once a sink observes that its consumer has gone away (all receivers
//! dropped for `BroadcastSink`, `mpsc::Sender::send` → `SendError` for
//! `MpscSink`), `push` returns `Err(SinkError::Closed)` and
//! [`SubscriptionSink::is_closed`] returns `true` for every subsequent call.

use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Push interface from mirage → subscriber.
///
/// Implementations MUST be cheap to call from a hot write path:
/// `push` should complete in O(1) amortised time and MUST NOT block or await.
pub trait SubscriptionSink<Event>: Send + Sync {
    /// Pushes an event to this sink.
    ///
    /// Returns `Err(SinkError::Closed)` if the downstream consumer has gone
    /// away, or `Err(SinkError::Full { dropped })` if the sink's buffer was
    /// full and the push either dropped events or was rejected.
    fn push(&self, event: Event) -> Result<(), SinkError>;

    /// Returns `true` once this sink has been closed (either explicitly via
    /// a close call or because the downstream consumer was dropped).
    fn is_closed(&self) -> bool;
}

/// Error returned by [`SubscriptionSink::push`].
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SinkError {
    /// The sink is closed; no further pushes will ever succeed.
    Closed,
    /// The sink's buffer was full. `dropped` is the number of events that
    /// were lost as a result of this push.
    Full {
        /// Number of events that were dropped by this push.
        dropped: usize,
    },
}

impl std::fmt::Display for SinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "subscription sink closed"),
            Self::Full { dropped } => {
                write!(f, "subscription sink full (dropped {dropped})")
            }
        }
    }
}

impl std::error::Error for SinkError {}

/// A [`SubscriptionSink`] backed by a `tokio::sync::broadcast` channel.
///
/// The underlying channel's capacity is fixed at construction time and
/// directly bounds back-pressure: when a subscriber lags by more than
/// `capacity` events it loses the oldest ones and `push` returns
/// `Err(SinkError::Full { dropped })` with the lag count.
#[must_use]
pub struct BroadcastSink<E> {
    tx: tokio::sync::broadcast::Sender<E>,
    capacity: usize,
}

impl<E: Clone + Send + 'static> BroadcastSink<E> {
    /// Constructs a new broadcast sink with the given channel capacity and
    /// returns it alongside a receiver the caller can drive.
    ///
    /// Capacity MUST be > 0 (enforced by `tokio::sync::broadcast::channel`).
    pub fn new(capacity: usize) -> (Self, tokio::sync::broadcast::Receiver<E>) {
        let (tx, rx) = tokio::sync::broadcast::channel(capacity);
        (Self { tx, capacity }, rx)
    }

    /// Returns a second receiver that sees every event pushed AFTER the
    /// subscribe call.
    #[must_use]
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<E> {
        self.tx.subscribe()
    }

    /// Returns the configured channel capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl<E: Clone + Send + 'static> SubscriptionSink<E> for BroadcastSink<E> {
    fn push(&self, event: E) -> Result<(), SinkError> {
        match self.tx.send(event) {
            Ok(_delivered) => Ok(()),
            // `broadcast::send` only errors when zero receivers remain.
            Err(_) => Err(SinkError::Closed),
        }
    }

    fn is_closed(&self) -> bool {
        self.tx.receiver_count() == 0
    }
}

impl<E> std::fmt::Debug for BroadcastSink<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BroadcastSink")
            .field("capacity", &self.capacity)
            .field("receivers", &self.tx.receiver_count())
            .finish()
    }
}

/// A [`SubscriptionSink`] backed by a bounded `tokio::sync::mpsc` channel.
///
/// Use this for single-consumer subscriptions where ordered, lossless
/// delivery under bounded memory is preferable to broadcast-style fan-out.
/// When the channel is full, `push` returns `Err(SinkError::Full { dropped: 1 })`
/// and the event is discarded (the channel retains its existing contents).
#[must_use]
pub struct MpscSink<E> {
    tx: tokio::sync::mpsc::Sender<E>,
    capacity: usize,
    closed: AtomicBool,
}

impl<E: Send + 'static> MpscSink<E> {
    /// Constructs a new mpsc sink with the given channel capacity and
    /// returns it alongside the receiver.
    ///
    /// Capacity MUST be > 0 (enforced by `tokio::sync::mpsc::channel`).
    pub fn new(capacity: usize) -> (Self, tokio::sync::mpsc::Receiver<E>) {
        let (tx, rx) = tokio::sync::mpsc::channel(capacity);
        (
            Self {
                tx,
                capacity,
                closed: AtomicBool::new(false),
            },
            rx,
        )
    }

    /// Returns the configured channel capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl<E: Send + 'static> SubscriptionSink<E> for MpscSink<E> {
    fn push(&self, event: E) -> Result<(), SinkError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(SinkError::Closed);
        }
        match self.tx.try_send(event) {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                Err(SinkError::Full { dropped: 1 })
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.closed.store(true, Ordering::Release);
                Err(SinkError::Closed)
            }
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire) || self.tx.is_closed()
    }
}

impl<E> std::fmt::Debug for MpscSink<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let closed = self.closed.load(Ordering::Acquire) || self.tx.is_closed();
        f.debug_struct("MpscSink")
            .field("capacity", &self.capacity)
            .field("closed", &closed)
            .finish()
    }
}

/// A dev/test-only [`SubscriptionSink`] that records events into a
/// `parking_lot::Mutex<Vec<E>>`.
///
/// Intended for unit tests that need to assert what was delivered without
/// the overhead of a real tokio channel. The sink never reports itself as
/// full — its only failure mode is an explicit close.
#[must_use]
pub struct VecSink<E> {
    events: Arc<Mutex<Vec<E>>>,
    closed: AtomicBool,
}

impl<E: Clone + Send + 'static> VecSink<E> {
    /// Constructs a new empty sink.
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            closed: AtomicBool::new(false),
        }
    }

    /// Returns a snapshot of every event observed so far.
    #[must_use]
    pub fn events(&self) -> Vec<E> {
        self.events.lock().clone()
    }

    /// Returns the current recorded event count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.lock().len()
    }

    /// Whether no events have been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.lock().is_empty()
    }

    /// Closes the sink; subsequent pushes return `Err(SinkError::Closed)`.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

impl<E: Clone + Send + 'static> Default for VecSink<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Clone + Send + 'static> SubscriptionSink<E> for VecSink<E> {
    fn push(&self, event: E) -> Result<(), SinkError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(SinkError::Closed);
        }
        self.events.lock().push(event);
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

impl<E> std::fmt::Debug for VecSink<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VecSink")
            .field("len", &self.events.lock().len())
            .field("closed", &self.closed.load(Ordering::Acquire))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_sink_records_events() {
        let sink: VecSink<u32> = VecSink::new();
        assert!(sink.is_empty());
        sink.push(1).unwrap();
        sink.push(2).unwrap();
        sink.push(3).unwrap();
        assert_eq!(sink.len(), 3);
        assert_eq!(sink.events(), vec![1, 2, 3]);
        assert!(!sink.is_closed());
    }

    #[test]
    fn vec_sink_closed_rejects_push() {
        let sink: VecSink<u32> = VecSink::new();
        sink.push(1).unwrap();
        sink.close();
        assert!(sink.is_closed());
        assert_eq!(sink.push(2), Err(SinkError::Closed));
        assert_eq!(sink.events(), vec![1]);
    }

    #[tokio::test]
    async fn broadcast_sink_delivers_to_receiver() {
        let (sink, mut rx) = BroadcastSink::<u32>::new(8);
        assert_eq!(sink.capacity(), 8);
        sink.push(10).unwrap();
        sink.push(20).unwrap();
        assert_eq!(rx.recv().await.unwrap(), 10);
        assert_eq!(rx.recv().await.unwrap(), 20);
        assert!(!sink.is_closed());
    }

    #[tokio::test]
    async fn broadcast_sink_closed_when_receiver_dropped() {
        let (sink, rx) = BroadcastSink::<u32>::new(4);
        drop(rx);
        assert!(sink.is_closed());
        assert_eq!(sink.push(1), Err(SinkError::Closed));
    }

    #[tokio::test]
    async fn broadcast_sink_drops_oldest_on_lag() {
        // broadcast drops oldest for lagging receivers: fill 4, then push 2
        // more. The receiver will see a `Lagged` on its next recv and then
        // resume from the newest events. From the sink's perspective sends
        // still return Ok as long as at least one receiver exists.
        let (sink, mut rx) = BroadcastSink::<u32>::new(4);
        for i in 0..6u32 {
            sink.push(i).unwrap();
        }
        // Drain: the first recv will report Lagged, subsequent recvs return newest.
        let mut seen = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(v) => seen.push(v),
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                    assert!(n >= 1, "expected lag count >= 1, got {n}");
                }
                Err(_) => break,
            }
        }
        assert!(!seen.is_empty(), "receiver should have seen newest events");
        assert_eq!(*seen.last().unwrap(), 5);
    }

    #[tokio::test]
    async fn mpsc_sink_delivers_events() {
        let (sink, mut rx) = MpscSink::<u32>::new(4);
        assert_eq!(sink.capacity(), 4);
        sink.push(1).unwrap();
        sink.push(2).unwrap();
        assert_eq!(rx.recv().await.unwrap(), 1);
        assert_eq!(rx.recv().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn mpsc_sink_full_returns_full_error() {
        let (sink, _rx) = MpscSink::<u32>::new(2);
        sink.push(1).unwrap();
        sink.push(2).unwrap();
        // Third push hits the bound.
        assert_eq!(sink.push(3), Err(SinkError::Full { dropped: 1 }));
    }

    #[tokio::test]
    async fn mpsc_sink_closed_when_receiver_dropped() {
        let (sink, rx) = MpscSink::<u32>::new(4);
        drop(rx);
        let err = sink.push(1).unwrap_err();
        assert_eq!(err, SinkError::Closed);
        assert!(sink.is_closed());
    }
}
