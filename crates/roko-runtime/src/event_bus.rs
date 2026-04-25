//! Typed, bounded broadcast event bus with replay support.
//!
//! This generalises the pattern used in `golem-core::event::EventFabric` and the
//! ad-hoc `mpsc::UnboundedSender<AgentEvent>` channels in `apps/mori/src/agent/`.
//!
//! # Architecture
//!
//! ```text
//!   Producer ──emit()──► EventBus ──broadcast──► Subscriber₁
//!                           │                    Subscriber₂
//!                           │                    Subscriber₃
//!                           ▼
//!                      ReplayRing (bounded deque)
//! ```
//!
//! The bus is parameterised over a single event type `E: Clone + Send + Sync + 'static`.
//! It combines:
//! - A `tokio::sync::broadcast` channel for live fan-out to all subscribers.
//! - A bounded `VecDeque` ring for durable replay (new subscribers can catch up).
//! - Monotonic sequence numbering for gap detection and ordered replay.
//!
//! # Example
//!
//! ```
//! use roko_runtime::event_bus::EventBus;
//!
//! #[derive(Debug, Clone)]
//! enum MyEvent {
//!     Tick(u64),
//!     Message(String),
//! }
//!
//! let bus = EventBus::<MyEvent>::new(1024);
//! let mut rx = bus.subscribe();
//!
//! bus.emit(MyEvent::Tick(1));
//! bus.emit(MyEvent::Message("hello".into()));
//!
//! // Replay all events from sequence 0
//! let events = bus.replay_from(0);
//! assert_eq!(events.len(), 2);
//! ```

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::sync::broadcast;
use tracing::trace;

use crate::heartbeat::{CognitiveSignal, HeartbeatTick, WakeupCondition};
use crate::lifecycle::LifecycleTransition;

/// A sequenced, timestamped envelope wrapping a user event.
#[derive(Debug, Clone)]
pub struct Envelope<E> {
    /// Monotonically increasing sequence number (bus-scoped).
    pub seq: u64,
    /// Unix timestamp in milliseconds when the event was emitted.
    pub ts_millis: u64,
    /// The wrapped event payload.
    pub payload: E,
}

/// A compact summary of a gate verdict included in plan-revision events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateVerdictSummary {
    /// The gate name that produced the verdict.
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Optional structured failure classification, such as `type_error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    /// Stable failure pattern ids associated with this verdict.
    #[serde(default)]
    pub failure_pattern_ids: Vec<String>,
    /// Blocking findings that make same-prompt retry insufficient.
    #[serde(default)]
    pub blocking_findings: Vec<String>,
    /// Optional free-form details from the gate or logger.
    pub details: Option<String>,
}

/// Why a plan revision event was emitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanRevisionReason {
    /// The task exhausted its gate-failure retry budget.
    GateFailureLimit {
        /// The number of gate attempts that were allowed before revision.
        attempts: u32,
    },
}

/// Why a PRD publish event was emitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublishOrigin {
    /// The publish happened through the CLI.
    Cli,
    /// The publish happened through HTTP.
    Http,
    /// The publish happened while importing content.
    Import,
}

/// Events shared across the runtime event bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RokoEvent {
    /// Emitted when repeated gate failures should trigger a plan revision.
    PlanRevision {
        /// Durable request id for the future planner agent.
        request_id: String,
        /// The plan being revised.
        plan_id: String,
        /// The task that exhausted its retries.
        task_id: String,
        /// The reason the plan revision was requested.
        reason: PlanRevisionReason,
        /// Structured next action that produced this event.
        required_next_action: String,
        /// De-duplicated failure pattern ids across the request.
        failure_pattern_ids: Vec<String>,
        /// Blocking findings the planner must address.
        blocking_findings: Vec<String>,
        /// Summaries of the failing gate verdicts.
        failing_verdicts: Vec<GateVerdictSummary>,
        /// Tail of the task log captured at the point of failure.
        log_tail: String,
        /// UTC timestamp for when the revision event was emitted.
        issued_at: chrono::DateTime<chrono::Utc>,
    },
    /// Emitted when a PRD is promoted into the published state.
    PrdPublished {
        /// The published PRD slug.
        slug: String,
        /// The published PRD path.
        path: std::path::PathBuf,
        /// UTC timestamp for when the PRD was published.
        published_at: chrono::DateTime<chrono::Utc>,
        /// Where the publish originated.
        origin: PublishOrigin,
    },
    /// Emitted when the runtime heartbeat policy publishes a cognitive tick.
    HeartbeatTick(HeartbeatTick),
    /// Emitted when an urgent condition bypasses normal heartbeat cadence.
    HeartbeatWakeup {
        /// Wakeup condition that triggered the early gamma tick.
        condition: WakeupCondition,
        /// UTC timestamp for when the wakeup was emitted.
        issued_at: chrono::DateTime<chrono::Utc>,
    },
    /// Emitted when heartbeat meta-cognition or scheduling requests loop control.
    CognitiveSignal {
        /// Control signal produced by heartbeat governance.
        signal: CognitiveSignal,
        /// UTC timestamp for when the signal was emitted.
        issued_at: chrono::DateTime<chrono::Utc>,
    },
    /// Emitted when an agent lifecycle state changes.
    AgentLifecycleTransition(LifecycleTransition),
    /// BEAT-05 BROADCAST step: published after PERSIST to notify downstream
    /// consumers (dashboard, watchers, other agents) of tick outcomes.
    TickBroadcast {
        /// Tick sequence number.
        tick_id: u64,
        /// Agent that produced the tick.
        agent_id: String,
        /// Selected inference tier.
        tier: roko_primitives::tier::InferenceTier,
        /// Whether the tick outcome passed verification.
        passed: Option<bool>,
        /// Total cost of this tick in USD.
        cost_usd: f64,
        /// UTC timestamp of the broadcast.
        broadcast_at: chrono::DateTime<chrono::Utc>,
    },
    /// BEAT-05 REACT step: emitted when the Policy.decide() react hook fires.
    ReactDecision {
        /// Tick that triggered the react.
        tick_id: u64,
        /// Decision made by the policy.
        decision: String,
        /// Cognitive signals generated by the react step.
        signals: Vec<CognitiveSignal>,
        /// UTC timestamp.
        decided_at: chrono::DateTime<chrono::Utc>,
    },
}

/// Shared interior state backing both [`EventBus`] and [`BusSender`].
struct Shared<E> {
    tx: broadcast::Sender<Envelope<E>>,
    ring: Mutex<VecDeque<Envelope<E>>>,
    seq: AtomicU64,
    capacity: usize,
}

impl<E: Clone + Send + Sync + 'static> Shared<E> {
    fn emit_inner(&self, event: E) {
        let envelope = Envelope {
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            ts_millis: current_ts_millis(),
            payload: event,
        };

        // Append to replay ring (short lock).
        {
            let mut ring = self.ring.lock();
            if ring.len() >= self.capacity {
                ring.pop_front();
            }
            ring.push_back(envelope.clone());
        }

        trace!(seq = envelope.seq, "event emitted");
        let _ = self.tx.send(envelope);
    }
}

/// A typed, bounded broadcast event bus with replay ring.
///
/// Generic over any `E: Clone + Send + Sync + 'static`. The bus never blocks
/// producers: if a subscriber falls behind, it will miss events on the live
/// broadcast channel (but can always catch up via [`replay_from`]).
pub struct EventBus<E: Clone + Send + Sync + 'static> {
    shared: Arc<Shared<E>>,
}

impl<E: Clone + Send + Sync + 'static> EventBus<E> {
    /// Creates a new event bus with the given capacity for both the broadcast
    /// channel and the replay ring.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            shared: Arc::new(Shared {
                tx,
                ring: Mutex::new(VecDeque::with_capacity(capacity)),
                seq: AtomicU64::new(0),
                capacity,
            }),
        }
    }

    /// Emits an event to all live subscribers and appends it to the replay ring.
    ///
    /// This never blocks. If the ring is full, the oldest event is evicted.
    /// If all subscribers have dropped, the event is still recorded in the ring.
    pub fn emit(&self, event: E) {
        self.shared.emit_inner(event);
    }

    /// Subscribes to live events. Returns a broadcast receiver.
    ///
    /// Events emitted before this call are not received on this channel —
    /// use [`replay_from`] to catch up.
    pub fn subscribe(&self) -> broadcast::Receiver<Envelope<E>> {
        self.shared.tx.subscribe()
    }

    /// Returns a snapshot of all events in the replay ring with `seq >= after_seq`.
    pub fn replay_from(&self, after_seq: u64) -> Vec<Envelope<E>> {
        self.shared
            .ring
            .lock()
            .iter()
            .filter(|e| e.seq >= after_seq)
            .cloned()
            .collect()
    }

    /// Returns the total number of events ever emitted (including evicted ones).
    pub fn total_emitted(&self) -> u64 {
        self.shared.seq.load(Ordering::Relaxed)
    }

    /// Returns the current number of events in the replay ring.
    pub fn ring_len(&self) -> usize {
        self.shared.ring.lock().len()
    }

    /// Returns the capacity of the event bus.
    pub fn capacity(&self) -> usize {
        self.shared.capacity
    }

    /// Returns a sender handle that can be shared across tasks/threads.
    ///
    /// The `BusSender` only supports `emit()` — it cannot subscribe or replay.
    /// This is useful for passing to subsystems that only produce events.
    pub fn sender(&self) -> BusSender<E> {
        BusSender {
            shared: Arc::clone(&self.shared),
        }
    }
}

/// A cloneable, send-safe handle for emitting events into an [`EventBus`].
///
/// Created via [`EventBus::sender`]. Only supports emitting — cannot subscribe
/// or replay. Safe to send across threads and tasks.
#[derive(Clone)]
pub struct BusSender<E: Clone + Send + Sync + 'static> {
    shared: Arc<Shared<E>>,
}

impl<E: Clone + Send + Sync + 'static> BusSender<E> {
    /// Emit an event. Same semantics as [`EventBus::emit`].
    pub fn emit(&self, event: E) {
        self.shared.emit_inner(event);
    }
}

static ROKO_EVENT_BUS: OnceLock<EventBus<RokoEvent>> = OnceLock::new();

/// Returns the process-local shared runtime event bus for `RokoEvent`.
pub fn global_event_bus() -> &'static EventBus<RokoEvent> {
    ROKO_EVENT_BUS.get_or_init(|| EventBus::new(1024))
}

#[allow(clippy::cast_possible_truncation)]
fn current_ts_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum TestEvent {
        Ping(u32),
        Pong(String),
    }

    #[test]
    fn emit_and_replay() {
        let bus = EventBus::new(16);
        bus.emit(TestEvent::Ping(1));
        bus.emit(TestEvent::Pong("hello".into()));
        bus.emit(TestEvent::Ping(2));

        let all = bus.replay_from(0);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].seq, 0);
        assert_eq!(all[1].seq, 1);
        assert_eq!(all[2].seq, 2);

        // Replay from midpoint.
        let partial = bus.replay_from(2);
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].payload, TestEvent::Ping(2));
    }

    #[test]
    fn ring_eviction() {
        let bus = EventBus::new(3);
        bus.emit(TestEvent::Ping(1));
        bus.emit(TestEvent::Ping(2));
        bus.emit(TestEvent::Ping(3));
        bus.emit(TestEvent::Ping(4)); // evicts Ping(1)

        assert_eq!(bus.ring_len(), 3);
        assert_eq!(bus.total_emitted(), 4);

        let all = bus.replay_from(0);
        // seq 0 (Ping(1)) was evicted, so only seq 1, 2, 3 remain.
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].seq, 1);
    }

    #[tokio::test]
    async fn subscribe_receives_live_events() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        bus.emit(TestEvent::Ping(42));

        let env = rx
            .recv()
            .await
            .expect("invariant: subscriber should receive the emitted live event");
        assert_eq!(env.payload, TestEvent::Ping(42));
        assert_eq!(env.seq, 0);
        assert!(env.ts_millis > 0);
    }

    #[test]
    fn sender_handle() {
        let bus = EventBus::new(16);
        let sender = bus.sender();
        sender.emit(TestEvent::Pong("from sender".into()));

        let all = bus.replay_from(0);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].payload, TestEvent::Pong("from sender".into()));
    }

    #[test]
    fn plan_revision_event_round_trips_through_json() {
        let event = RokoEvent::PlanRevision {
            request_id: "replan-abc".into(),
            plan_id: "plan-123".into(),
            task_id: "task-abc".into(),
            reason: PlanRevisionReason::GateFailureLimit { attempts: 3 },
            required_next_action: "needs_replan".into(),
            failure_pattern_ids: vec!["E0277::src/lib.rs".into()],
            blocking_findings: vec!["failure requires plan shape revision".into()],
            failing_verdicts: vec![
                GateVerdictSummary {
                    gate: "compile".into(),
                    passed: false,
                    classification: Some("type_error".into()),
                    failure_pattern_ids: vec!["E0277::src/lib.rs".into()],
                    blocking_findings: vec!["failure requires plan shape revision".into()],
                    details: Some("E0277".into()),
                },
                GateVerdictSummary {
                    gate: "clippy".into(),
                    passed: false,
                    classification: None,
                    failure_pattern_ids: Vec::new(),
                    blocking_findings: Vec::new(),
                    details: None,
                },
            ],
            log_tail: "line 1\nline 2".into(),
            issued_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&event)
            .expect("invariant: plan revision event should serialize to JSON");
        let decoded: RokoEvent = serde_json::from_str(&json)
            .expect("invariant: serialized plan revision event should deserialize");

        assert_eq!(decoded, event);
    }

    #[test]
    fn plan_revision_event_flows_through_bus() {
        let bus = EventBus::new(8);
        let event = RokoEvent::PlanRevision {
            request_id: "replan-def".into(),
            plan_id: "plan-123".into(),
            task_id: "task-abc".into(),
            reason: PlanRevisionReason::GateFailureLimit { attempts: 3 },
            required_next_action: "needs_replan".into(),
            failure_pattern_ids: vec!["test::panic".into()],
            blocking_findings: vec!["repeated test failure exhausted retry budget".into()],
            failing_verdicts: vec![GateVerdictSummary {
                gate: "test".into(),
                passed: false,
                classification: Some("test_expectation_failure".into()),
                failure_pattern_ids: vec!["test::panic".into()],
                blocking_findings: vec!["repeated test failure exhausted retry budget".into()],
                details: Some("tests failed".into()),
            }],
            log_tail: "tail".into(),
            issued_at: chrono::Utc::now(),
        };

        bus.emit(event.clone());

        let replayed = bus.replay_from(0);
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].payload, event);
    }

    #[test]
    fn prd_published_event_round_trips_through_json() {
        let event = RokoEvent::PrdPublished {
            slug: "demo".into(),
            path: std::path::PathBuf::from(".roko/prd/published/demo.md"),
            published_at: chrono::Utc::now(),
            origin: PublishOrigin::Cli,
        };

        let json = serde_json::to_string(&event)
            .expect("invariant: PRD published event should serialize to JSON");
        let decoded: RokoEvent = serde_json::from_str(&json)
            .expect("invariant: serialized PRD published event should deserialize");

        assert_eq!(decoded, event);
    }

    #[test]
    fn prd_published_event_flows_through_bus() {
        let bus = EventBus::new(8);
        let event = RokoEvent::PrdPublished {
            slug: "demo".into(),
            path: std::path::PathBuf::from(".roko/prd/published/demo.md"),
            published_at: chrono::Utc::now(),
            origin: PublishOrigin::Http,
        };

        bus.emit(event.clone());

        let replayed = bus.replay_from(0);
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].payload, event);
    }
}
