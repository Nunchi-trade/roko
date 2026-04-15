# StateHub + DashboardSnapshot + EventBus Context Pack

This pack documents the existing streaming infrastructure in roko-core that
the TUI needs to subscribe to (instead of polling files from disk).

## StateHub (`crates/roko-core/src/state_hub.rs`)

```rust
pub struct StateHub {
    snapshot_tx: watch::Sender<DashboardSnapshot>,
    snapshot_rx: watch::Receiver<DashboardSnapshot>,
    event_bus: EventBus<DashboardEvent>,
}

impl StateHub {
    pub fn new(ring_capacity: usize) -> Self;
    pub fn default_capacity() -> Self; // ring_capacity = 1024

    // Event publishing
    pub fn publish(&self, event: DashboardEvent);
    pub fn publish_batch(&self, events: impl IntoIterator<Item = DashboardEvent>);

    // Consumer interfaces
    pub fn snapshot(&self) -> watch::Receiver<DashboardSnapshot>;  // For TUI: borrow at 60fps
    pub fn current_snapshot(&self) -> DashboardSnapshot;            // For API: clone on demand
    pub fn subscribe_events(&self) -> broadcast::Receiver<...>;     // For WebSocket/SSE

    // Replay
    pub fn replay_from(&self, after_seq: u64) -> Vec<...>;

    // Clone-safe sender handle
    pub fn sender(&self) -> StateHubSender;
}

pub struct StateHubSender { /* clone of watch::Sender */ }
pub type SharedStateHub = Arc<StateHub>;
```

### TUI consumption pattern

```rust
// In App struct:
snapshot_rx: Option<watch::Receiver<DashboardSnapshot>>,

// In main_loop:
if let Some(rx) = &mut self.snapshot_rx {
    if rx.has_changed().unwrap_or(false) {
        let snapshot = rx.borrow_and_update();
        self.tui_state.update_from_dashboard_snapshot(&snapshot);
    }
} else {
    // Fallback: poll from disk every tick
    self.data.refresh_sync()?;
    self.tui_state.update_from_snapshot(&self.data);
}
```

## DashboardSnapshot (`crates/roko-core/src/dashboard_snapshot.rs`)

### Event types (15 variants)

```rust
pub enum DashboardEvent {
    PlanStarted { plan_id: String },
    PlanCompleted { plan_id: String, success: bool },
    TaskStarted { plan_id: String, task_id: String, phase: String },
    TaskCompleted { plan_id: String, task_id: String, outcome: String },
    TaskPhaseChanged { plan_id: String, task_id: String, old_phase: String, new_phase: String },
    AgentSpawned { agent_id: String, role: String },
    AgentOutput { agent_id: String, content: String },
    GateResult { plan_id: String, task_id: String, gate: String, passed: bool },
    PhaseTransition { plan_id: String, from: String, to: String },
    EfficiencyEvent { plan_id: String, task_id: String, metric: String, value: f64 },
    Error { message: String },
}
```

### Snapshot nested types

```rust
pub struct DashboardSnapshot {
    pub plans: HashMap<String, PlanState>,
    pub tasks: HashMap<String, TaskState>,
    pub agents: HashMap<String, AgentState>,
    pub gate_verdicts: Vec<GateVerdict>,
    pub errors: Vec<ErrorEntry>,
    pub seq: u64,
}

pub struct PlanState { plan_id, phase, tasks_total, tasks_done, tasks_failed, active }
pub struct TaskState { task_id, plan_id, phase, outcome }
pub struct AgentState { agent_id, role, active, output_bytes }
pub struct GateVerdict { plan_id, task_id, gate, passed, ts_millis }
pub struct ErrorEntry { message, ts_millis }
```

## EventBus (`crates/roko-runtime/src/event_bus.rs`)

```rust
pub struct EventBus<T> {
    // broadcast channel + replay ring buffer
}

impl<T: Clone> EventBus<T> {
    pub fn new(capacity: usize) -> Self;
    pub fn publish(&self, event: T);
    pub fn subscribe(&self) -> broadcast::Receiver<T>;
    pub fn replay_from(&self, after_seq: u64) -> Vec<(u64, T)>;
}
```

## Orchestrator integration (`crates/roko-cli/src/orchestrate.rs`)

The orchestrator already has a `StateHubSender`:

```rust
// In PlanRunner or equivalent:
state_hub_sender: Option<roko_core::StateHubSender>,

// Publishing events:
fn set_state_hub(&mut self, sender: StateHubSender) { ... }
fn emit_server_event(&self, event: DashboardEvent) { ... }
fn server_event_to_dashboard(&self, event: ServerEvent) -> DashboardEvent { ... }
```

## What T1 needs to do

1. Add `snapshot_rx: Option<watch::Receiver<DashboardSnapshot>>` to `App`
2. Add `App::new_connected(root, state_hub: &SharedStateHub)` constructor
3. Add `TuiState::update_from_dashboard_snapshot(&mut self, snap: &DashboardSnapshot)`
4. In main_loop, use `borrow_and_update()` when receiver is present
5. Keep polling as fallback when `snapshot_rx` is `None`
6. Wire `cmd_dashboard` to accept an optional `SharedStateHub`
