# Dream Scheduling and Triggers

> **Layer**: L0 Runtime (scheduling), L4 Orchestration (idle detection)
>
> **Synapse Traits**: `Policy` (dream scheduling policy)
>
> **Crate**: `roko-dreams` — `runner.rs` (scheduling logic)
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md)


> **Implementation**: Scaffold

---

## Trigger Conditions

Dreams in Roko are triggered by two mechanisms. Both are idle-based — dreams fire when the agent has capacity, not when a clock runs down.

### 1. Idle-Time Trigger (Primary)

The primary trigger fires when three conditions are simultaneously met:

| Condition | Default | Configuration |
|-----------|---------|---------------|
| Agent has no active tasks | — | Detected by the L4 Orchestration layer |
| Agent has been idle for ≥ threshold | 15 minutes | `dreams.idle_threshold_mins` in `roko.toml` |
| Agent has ≥ minimum unprocessed episodes | 5 episodes | `dreams.min_episodes_for_dream` in `roko.toml` |

The idle detection logic from `DreamRunner::schedule()`:

```rust
pub fn schedule(&self) -> Option<Duration> {
    if !self.config.auto_dream {
        return None;
    }

    let episodes = load_episodes_since_last_dream();
    if episodes.len() < self.config.min_episodes_for_dream {
        return None;
    }

    let latest_episode = episodes.iter().map(|e| e.timestamp).max()?;
    let idle_threshold = Duration::from_secs(
        self.config.idle_threshold_mins * 60
    );
    let target_fire_at = latest_episode + idle_threshold;
    let now = Utc::now();

    if target_fire_at <= now {
        Some(Duration::ZERO)  // Dream now
    } else {
        (target_fire_at - now).to_std().ok()
    }
}
```

### 2. Scheduled Trigger (Secondary)

The scheduled trigger fires dreams at fixed intervals regardless of idle state. This ensures that busy agents with continuous task queues still consolidate periodically:

```toml
[dreams]
scheduled_interval_hours = 4  # Dream every 4 hours regardless of idle state
```

When the scheduled trigger fires during active task execution, the dream is queued and executed at the next available idle gap. Dreams never interrupt active tasks.

### 3. Manual Trigger

The CLI provides a manual dream trigger for testing and development:

```bash
roko dream run         # Fire a dream cycle now
roko dream report      # Show the latest dream report
roko dream history     # List all dream reports
```

---

## What Does NOT Trigger Dreams

These are mechanisms from the legacy Bardo architecture that are **removed** in Roko:

| Legacy Trigger | Legacy Description | Why Removed |
|----------------|-------------------|-------------|
| **Death clock proximity** | Dream frequency increased as stochastic death clocks approached zero | No death clocks in Roko. Dreams fire based on idle time and backlog, not mortality. |
| **Vitality score thresholds** | Dreams triggered when vitality dropped below thresholds (Conservation, Declining, Terminal phases) | No vitality phases in Roko. Budget exhaustion and knowledge plateau are continuous metrics, not dream triggers. |
| **Terminal phase frantic dreaming** | Every 67 ticks in Terminal phase | No Terminal phase dreaming. If the agent has a large unprocessed backlog, it dreams more frequently through the standard mechanism — not because of approaching termination. |

---

## Dream Frequency Adaptation

While dreams are not death-triggered, their frequency does adapt to the agent's operational state:

| State | Dream Frequency | Mechanism |
|-------|----------------|-----------|
| Low activity, few episodes | Infrequent (1/day or less) | `min_episodes_for_dream` threshold not met |
| Normal activity | 2–4 per day | Standard idle gaps between tasks |
| High activity, many episodes | 4–8 per day | More episodes accumulate faster, reaching the threshold sooner |
| Very high activity, no idle time | Scheduled only | Idle trigger never fires; scheduled trigger ensures periodic consolidation |
| Large backlog (>50 unprocessed) | Intensive mode | Multiple dream cycles fire in sequence until the backlog is reduced below threshold |

### Intensive Consolidation Mode

When the unprocessed episode count exceeds a high-water mark (default: 50 episodes), the dream scheduler enters intensive mode:

1. Dream cycles fire back-to-back until the backlog is reduced to the low-water mark (default: 10 episodes)
2. Each cycle processes a batch of episodes (default: 10 per cycle)
3. Intensive mode is logged as a separate category for monitoring

This replaces the legacy "frantic dreaming" concept: instead of dreaming intensely because death is approaching, the agent dreams intensely because it has a lot of material to process. The motivation is cognitive, not mortal.

---

## Interaction with the Orchestrator

The dream scheduler coordinates with the L4 Orchestration layer (plan executor) to find appropriate idle windows:

```
Plan Executor                    Dream Scheduler
    |                                  |
    |-- Task A starts ----------------->|  (blocked: active task)
    |                                  |
    |-- Task A completes -------------->|
    |                                  |
    |-- Check for idle gap ------------>|
    |                                  |-- idle_threshold not met yet
    |                                  |
    |-- Task B starts ----------------->|  (blocked: active task)
    |                                  |
    |-- Task B completes -------------->|
    |                                  |
    |-- Check for idle gap ------------>|
    |                                  |-- idle_threshold met, episodes ≥ min
    |                                  |-- DREAM CYCLE FIRES
    |                                  |
    |<- Dream complete, resume ---------|
    |                                  |
    |-- Task C starts ----------------->|
```

The orchestrator calls `dream_runner.schedule()` after each task completion. If the scheduler returns `Some(Duration::ZERO)`, the dream fires immediately. If it returns `Some(d)` where `d > 0`, the orchestrator sets a timer. If it returns `None`, no dream is needed.

---

## Configuration Reference

```toml
[dreams]
# Enable automatic idle-triggered dreaming
auto_dream = true

# Minutes of inactivity before a dream can fire
idle_threshold_mins = 15

# Minimum unprocessed episodes required before a dream can fire
min_episodes_for_dream = 5

# Fixed-interval scheduled dreaming (0 = disabled)
scheduled_interval_hours = 4

# Fraction of inference budget allocated to dreams
budget_fraction = 0.15

# Intensive mode high-water mark (triggers back-to-back dreams)
intensive_threshold = 50

# Intensive mode low-water mark (stops back-to-back dreams)
intensive_low_water = 10

# Episodes processed per dream cycle
batch_size = 10

[dreams.agent]
# Agent backend for dream consolidation
command = "claude"
model = "claude-haiku-4-5-20251001"
bare_mode = true
effort = "low"
timeout_ms = 120000
```

---

## Circadian-Inspired Scheduling

Biological sleep follows circadian rhythms — not purely reactive. Roko agents operating on long-running tasks benefit from a circadian-like scheduling pattern that ensures dream cycles happen at regular intervals even when idle gaps are plentiful.

The circadian scheduler layers on top of the idle-time and scheduled triggers described above. It does not replace them — it biases the timing of dreams toward preferred hours while still respecting the `min_episodes_for_dream` and `idle_threshold_mins` constraints. When `circadian_strength` is 0.0, the scheduler behaves identically to the standard idle-time trigger. When `circadian_strength` is 1.0, dreams are strictly gated to `preferred_hours`.

The `max_interval_mins` parameter acts as a safety net: even if a continuously busy agent never hits a preferred hour while idle, a dream cycle will fire within the maximum interval to prevent unbounded consolidation debt.

```rust
/// Circadian-inspired dream scheduling.
/// Ensures regular consolidation rhythm even when idle time is abundant.
pub struct CircadianScheduler {
    /// Preferred dream times (hours of day, 0-23). Agent dreams more willingly
    /// during these hours. Empty = no circadian preference.
    pub preferred_hours: Vec<u8>,         // default: [2, 6, 14, 22]
    /// Circadian strength: how strongly preferred hours bias scheduling.
    /// 0.0 = no bias, 1.0 = only dream during preferred hours.
    pub circadian_strength: f64,          // default: 0.3, range: 0.0-1.0
    /// Minimum interval between dream cycles (minutes).
    pub min_interval_mins: u64,           // default: 60, range: 30-480
    /// Maximum interval between dream cycles (minutes).
    /// Ensures consolidation even when the agent is continuously busy.
    pub max_interval_mins: u64,           // default: 360, range: 120-720
    /// Whether to align dream cycles with task completion boundaries.
    pub align_to_task_boundaries: bool,   // default: true
}
```

### Test Criteria

1. **Circadian preference**: with `circadian_strength=1.0`, dreams only fire during `preferred_hours`.
2. **Min interval**: no two dream cycles fire within `min_interval_mins` of each other.
3. **Max interval**: a dream cycle always fires within `max_interval_mins`, even without idle time.
4. **Task alignment**: with `align_to_task_boundaries=true`, dreams never interrupt a running task.
5. **Zero strength**: with `circadian_strength=0.0`, `preferred_hours` has no effect on scheduling.

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [01-three-phase-cycle.md](01-three-phase-cycle.md) | Dream cycle structure that scheduling triggers |
| [12-sleep-time-compute.md](12-sleep-time-compute.md) | Compute budget that constrains dream frequency |
| [00-vision-and-dream-as-death-reframe.md](00-vision-and-dream-as-death-reframe.md) | Why dreams are idle-triggered, not death-triggered |
