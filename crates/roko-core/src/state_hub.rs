//! Unified state hub: single source of truth for dashboard consumers.
//!
//! The [`StateHub`] bridges the event bus to a materialized
//! [`DashboardSnapshot`] via a `tokio::sync::watch` channel. Three consumer
//! interfaces:
//!
//! - **TUI** — borrows the watch receiver at 60 fps.
//! - **WebSocket / SSE** — subscribes to the broadcast channel for live events.
//! - **REST API** — clones the current snapshot on demand.
//!
//! ```text
//! Orchestrator
//!     │ publish(DashboardEvent)
//!     ▼
//! StateHub
//!     ├── watch<DashboardSnapshot>  ← TUI reads (60fps, zero-copy borrow)
//!     ├── broadcast<DashboardEvent> ← WebSocket/SSE clients subscribe
//!     └── ring buffer (1024)        ← replay for late joiners
//! ```

use std::fmt;
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

use roko_runtime::event_bus::{self, EventBus};
use tokio::sync::watch;

use crate::dashboard_snapshot::{DashboardEvent, DashboardSnapshot};

/// Unified state hub driving all dashboard consumers from a single event
/// stream.
///
/// Events are published once via [`publish`]. Each call:
/// 1. Broadcasts the event to live subscribers (WebSocket, SSE).
/// 2. Records the event in the replay ring for late joiners.
/// 3. Applies the event to the materialized snapshot so the TUI can borrow it.
pub struct StateHub {
    snapshot_tx: watch::Sender<DashboardSnapshot>,
    snapshot_rx: watch::Receiver<DashboardSnapshot>,
    event_bus: EventBus<DashboardEvent>,
}

impl StateHub {
    /// Create a new hub with the given replay ring capacity.
    pub fn new(ring_capacity: usize) -> Self {
        let (snapshot_tx, snapshot_rx) = watch::channel(DashboardSnapshot::default());
        Self {
            snapshot_tx,
            snapshot_rx,
            event_bus: EventBus::new(ring_capacity),
        }
    }

    /// Create a new hub with the default ring capacity (1024).
    pub fn default_capacity() -> Self {
        Self::new(1024)
    }

    /// Publish an event: broadcast, record in ring, and apply to snapshot.
    pub fn publish(&self, event: DashboardEvent) {
        self.event_bus.emit(event.clone());
        self.snapshot_tx.send_modify(|snap| snap.apply(&event));
    }

    /// Publish a batch of events atomically (snapshot updates are visible
    /// together after the last event).
    pub fn publish_batch(&self, events: impl IntoIterator<Item = DashboardEvent>) {
        self.snapshot_tx.send_modify(|snap| {
            for event in events {
                self.event_bus.emit(event.clone());
                snap.apply(&event);
            }
        });
    }

    /// Replace the current materialized snapshot atomically.
    pub fn apply_snapshot(&self, snapshot: DashboardSnapshot) {
        let _ = self.snapshot_tx.send(snapshot);
    }

    /// Get a receiver for the materialized snapshot.
    ///
    /// The TUI calls `borrow()` or `borrow_and_update()` on this at render
    /// time for a zero-copy read of the current state.
    pub fn snapshot(&self) -> watch::Receiver<DashboardSnapshot> {
        self.snapshot_rx.clone()
    }

    /// Clone the current snapshot (for REST API responses).
    pub fn current_snapshot(&self) -> DashboardSnapshot {
        self.snapshot_rx.borrow().clone()
    }

    /// Subscribe to live events (for WebSocket / SSE streaming).
    pub fn subscribe_events(
        &self,
    ) -> tokio::sync::broadcast::Receiver<event_bus::Envelope<DashboardEvent>> {
        self.event_bus.subscribe()
    }

    /// Replay events from the ring buffer starting at `after_seq`.
    pub fn replay_from(&self, after_seq: u64) -> Vec<event_bus::Envelope<DashboardEvent>> {
        self.event_bus.replay_from(after_seq)
    }

    /// Get a clone-safe sender handle for subsystems that only produce events.
    pub fn sender(&self) -> StateHubSender {
        StateHubSender {
            snapshot_tx: self.snapshot_tx.clone(),
            bus_sender: self.event_bus.sender(),
        }
    }

    /// Total events ever published.
    pub fn total_published(&self) -> u64 {
        self.event_bus.total_emitted()
    }

    /// Current ring buffer length.
    pub fn ring_len(&self) -> usize {
        self.event_bus.ring_len()
    }
}

/// A clone-safe, send-safe handle for publishing events into a [`StateHub`].
///
/// Created via [`StateHub::sender`]. Safe to send across threads/tasks.
/// This is what gets passed to the orchestrator.
#[derive(Clone)]
pub struct StateHubSender {
    snapshot_tx: watch::Sender<DashboardSnapshot>,
    bus_sender: event_bus::BusSender<DashboardEvent>,
}

impl StateHubSender {
    /// Publish an event through the hub.
    pub fn publish(&self, event: DashboardEvent) {
        self.bus_sender.emit(event.clone());
        self.snapshot_tx.send_modify(|snap| snap.apply(&event));
    }
}

/// Shared reference-counted handle to a [`StateHub`].
#[derive(Clone)]
pub struct SharedStateHub(Arc<StateHub>);

impl SharedStateHub {
    /// Wrap an existing hub in a shared handle.
    pub fn new(state_hub: StateHub) -> Self {
        Self(Arc::new(state_hub))
    }

    /// Create a new in-process hub for standalone clients.
    pub fn new_in_process() -> Self {
        Self::new(StateHub::default_capacity())
    }

    /// Seed the materialized snapshot from a workspace root.
    ///
    /// Missing or unreadable sources are handled by loading an empty
    /// snapshot and warning, so standalone consumers keep running.
    pub fn bootstrap_from_workdir(&self, workdir: &Path) -> Result<(), std::io::Error> {
        match DashboardSnapshot::load_from_workdir(workdir) {
            Ok(snapshot) => {
                self.0.apply_snapshot(snapshot);
                Ok(())
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    workdir = %workdir.display(),
                    "failed to bootstrap dashboard snapshot from workdir; using empty snapshot"
                );
                self.0.apply_snapshot(DashboardSnapshot::default());
                Err(err)
            }
        }
    }
}

impl Deref for SharedStateHub {
    type Target = StateHub;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<StateHub> for SharedStateHub {
    fn as_ref(&self) -> &StateHub {
        &self.0
    }
}

impl fmt::Debug for SharedStateHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedStateHub").finish_non_exhaustive()
    }
}

impl From<StateHub> for SharedStateHub {
    fn from(state_hub: StateHub) -> Self {
        Self::new(state_hub)
    }
}

impl From<Arc<StateHub>> for SharedStateHub {
    fn from(state_hub: Arc<StateHub>) -> Self {
        Self(state_hub)
    }
}

/// Create a new shared state hub with the default capacity.
pub fn shared_state_hub() -> SharedStateHub {
    SharedStateHub::new_in_process()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_updates_snapshot_and_broadcasts() {
        let hub = StateHub::default_capacity();
        let mut rx = hub.snapshot();
        let mut event_rx = hub.subscribe_events();

        hub.publish(DashboardEvent::PlanStarted {
            plan_id: "p1".into(),
        });

        // Snapshot is updated synchronously.
        let snap = rx.borrow_and_update();
        assert_eq!(snap.stats.plans_active, 1);
        assert!(snap.plans.contains_key("p1"));
        drop(snap);

        // Broadcast also received the event.
        let envelope = event_rx.recv().await.unwrap();
        assert!(matches!(
            envelope.payload,
            DashboardEvent::PlanStarted { .. }
        ));
    }

    #[test]
    fn sender_handle_publishes() {
        let hub = StateHub::default_capacity();
        let sender = hub.sender();

        sender.publish(DashboardEvent::AgentSpawned {
            agent_id: "a1".into(),
            role: "coder".into(),
        });

        let snap = hub.current_snapshot();
        assert_eq!(snap.stats.agents_active, 1);
        assert!(snap.agents.contains_key("a1"));
    }

    #[test]
    fn replay_ring_works() {
        let hub = StateHub::new(16);
        hub.publish(DashboardEvent::PlanStarted {
            plan_id: "p1".into(),
        });
        hub.publish(DashboardEvent::PlanStarted {
            plan_id: "p2".into(),
        });

        let events = hub.replay_from(0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 0);
        assert_eq!(events[1].seq, 1);

        let partial = hub.replay_from(1);
        assert_eq!(partial.len(), 1);
    }

    #[test]
    fn batch_publish() {
        let hub = StateHub::default_capacity();
        hub.publish_batch(vec![
            DashboardEvent::PlanStarted {
                plan_id: "p1".into(),
            },
            DashboardEvent::TaskStarted {
                plan_id: "p1".into(),
                task_id: "t1".into(),
                phase: "compose".into(),
            },
        ]);

        let snap = hub.current_snapshot();
        assert_eq!(snap.stats.plans_active, 1);
        assert_eq!(snap.stats.tasks_active, 1);
    }

    #[test]
    fn in_process_bootstrap_populates_live_receiver() {
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let state_dir = tmpdir.path().join(".roko/state");
        std::fs::create_dir_all(&state_dir).expect("state dir");

        let executor_state = serde_json::json!({
            "plan_states": {
                "plan-a": {
                    "current_phase": { "kind": "implementing" },
                    "task_id": "task-a",
                    "assigned_agents": ["agent-a"],
                    "gate_results": [
                        {
                            "gate_name": "compile",
                            "passed": true,
                            "duration_ms": 42
                        }
                    ],
                    "last_error": "boom"
                }
            }
        });
        std::fs::write(
            state_dir.join("executor.json"),
            serde_json::to_vec(&executor_state).expect("executor json"),
        )
        .expect("write executor state");

        let hub = SharedStateHub::new_in_process();
        let rx = hub.snapshot();
        assert!(rx.borrow().plans.is_empty());

        hub.bootstrap_from_workdir(tmpdir.path())
            .expect("bootstrap workdir");

        let snapshot = rx.borrow();
        assert!(snapshot.plans.contains_key("plan-a"));
        assert!(snapshot.tasks.contains_key("plan-a/task-a"));
        assert!(snapshot.agents.contains_key("agent-a"));
        assert_eq!(snapshot.stats.gates_passed, 1);
        assert_eq!(snapshot.stats.errors_total, 1);
    }
}
