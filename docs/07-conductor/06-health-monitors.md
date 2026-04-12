# Health Monitors

> Four system-level checks that produce a HealthStatus snapshot.
> Not individual task health — system health. Is the infrastructure
> functioning? Are agents alive? Is coverage trending down?

---

## SystemSnapshot

The health monitor operates on a point-in-time snapshot of system state:

```rust
pub struct SystemSnapshot {
    pub active_agents: usize,
    pub expected_agents: usize,
    pub last_agent_heartbeat_ms: Option<u64>,
    pub chain_connected: bool,
    pub chain_expected: bool,
    pub spec_hash_expected: Option<String>,
    pub spec_hash_actual: Option<String>,
    pub coverage_history: Vec<f64>,
}
```

The snapshot captures infrastructure health, not task health. Task
health is the watcher ensemble's domain. Infrastructure health is
about whether the foundation the tasks run on is solid.

---

## HealthStatus

```rust
pub enum HealthStatus {
    Healthy,
    Degraded,
    Critical,
}
```

**Healthy**: All checks pass. The system is operating normally.

**Degraded**: One or more checks indicate a non-critical issue. The
system continues operating but with reduced capacity or reliability.
Operator attention is recommended.

**Critical**: A fundamental infrastructure problem. The system cannot
reliably continue. Operator intervention is required.

---

## The Four Checks

### 1. Terminal Liveness

**What it checks**: Is the agent process still responsive?

**How it works**: Compares the `last_agent_heartbeat_ms` against
a liveness threshold. If the most recent heartbeat is older than
the threshold, the terminal is considered unresponsive.

**Healthy**: Heartbeat within threshold (or no agents expected).
**Degraded**: Heartbeat exceeds threshold — agent may have stalled.
**Critical**: No heartbeat received and agents are expected.

**Why this matters**: Agent processes can become unresponsive without
crashing. The process is alive (PID exists, no exit code) but the
agent has stopped producing output or responding to input. Without
heartbeat monitoring, this condition is invisible — the orchestrator
thinks the agent is working when it has actually stalled.

**Connection to ProcessSupervisor**: In the full runtime stack
(`bardo-runtime`), the ProcessSupervisor tracks agent processes with
heartbeat monitoring, resource limits, and descendant tree tracking.
The terminal liveness check is the health monitor's view of the
same data.

### 2. Agent Status

**What it checks**: Are the expected number of agents running?

**How it works**: Compares `active_agents` against `expected_agents`.
If fewer agents are active than expected, something has failed.

**Healthy**: `active_agents >= expected_agents`.
**Degraded**: `active_agents < expected_agents` — some agents have
died or failed to start.
**Critical**: `active_agents == 0` and `expected_agents > 0` — all
agents are down.

**Why this matters**: In a batch run with 5 parallel plans, each
requiring one implementer agent, the expected count is 5. If only 3
agents are active, 2 plans are stalled waiting for agents. This check
detects the shortfall before the stalled plans' timeout fires.

**Self-healing trigger**: When agent status is Degraded, the
orchestrator can proactively respawn missing agents rather than waiting
for the affected plans' time-overrun watchers to fire. This is the
"anticipate, don't react" principle (Design Principle 11) applied to
agent lifecycle.

### 3. Spec Drift

**What it checks**: Has the implementation diverged from its specification?

**How it works**: Compares `spec_hash_expected` against `spec_hash_actual`.
If the hashes differ, the specification has changed since the plan
was generated, or the implementation has drifted from the spec.

**Healthy**: Hashes match (or no spec tracking configured).
**Degraded**: Hashes differ — spec drift detected.
**Critical**: (Not used for this check — drift is always Degraded.)

**Why this matters**: Spec drift at the system level means the
acceptance criteria may no longer match the implementation. This can
happen when:
- A PRD is updated while a plan is in progress
- Multiple plans modify the same crate's public API
- External dependencies change their interface

System-level spec drift is distinct from the spec-drift watcher
(which monitors individual task file scope). The health monitor's
spec drift check looks at the entire system specification, not
individual task boundaries.

### 4. Coverage Trend

**What it checks**: Is test coverage trending down over time?

**How it works**: Examines the `coverage_history` vector (a sequence
of coverage percentages over recent builds). If the trend is
downward (recent values lower than earlier values), the system is
losing test coverage.

**Healthy**: Coverage stable or increasing.
**Degraded**: Coverage declining — agents are adding code without
corresponding tests.
**Critical**: (Not used — coverage decline is always Degraded.)

**Why this matters**: Test coverage is a leading indicator of
agent quality degradation. When agents start skipping tests to meet
gate criteria faster, coverage drops. This is especially dangerous
because coverage drops compound — less-tested code is harder for
future agents to modify correctly, leading to more failures, leading
to more corner-cutting, leading to less coverage.

The coverage trend check implements Design Principle 12: "The agent
builds the world it operates in." Declining coverage means agents
are making the codebase worse for future agents.

**Trend computation**: The health monitor uses a simple regression
on the coverage history. If the slope is negative and the recent
average is below the earlier average by more than a threshold (e.g.,
2 percentage points), the status is Degraded.

---

## HealthMonitor API

```rust
pub struct HealthMonitor {
    // Configuration: thresholds for each check
}

impl HealthMonitor {
    pub fn check(&self, snapshot: &SystemSnapshot) -> HealthStatus {
        let liveness = self.terminal_liveness(snapshot);
        let agents = self.agent_status(snapshot);
        let drift = self.spec_drift(snapshot);
        let coverage = self.coverage_trend(snapshot);

        // Worst status wins
        [liveness, agents, drift, coverage]
            .into_iter()
            .max()
            .unwrap_or(HealthStatus::Healthy)
    }
}
```

Like the intervention policy, the health monitor uses worst-status-wins
aggregation. If any single check returns Critical, the overall status
is Critical.

---

## Health vs. Watcher Ensemble

The health monitor and watcher ensemble serve different purposes:

| Dimension | Health Monitor | Watcher Ensemble |
|-----------|--------------|-----------------|
| **Scope** | System infrastructure | Individual plan/task execution |
| **Input** | SystemSnapshot | Signal stream |
| **Output** | HealthStatus (Healthy/Degraded/Critical) | WatcherOutput (per-watcher severity) |
| **Frequency** | Periodic (every N seconds) | Every conductor evaluation |
| **Trigger** | Infrastructure problems | Execution anomalies |

A system can be Healthy (all infrastructure checks pass) while
individual plans are failing (watchers detect stuck agents). Conversely,
all plans can be proceeding normally while the system is Degraded (an
expected agent has died, reducing parallelism).

Both feed into the Conductor's overall assessment. The health monitor's
Critical status can override watcher-based decisions — if the
infrastructure is failing, task-level interventions are pointless.

---

## Snapshot Collection

The SystemSnapshot is assembled by the orchestrator from multiple
sources:

| Field | Source |
|-------|--------|
| `active_agents` | ProcessSupervisor agent count |
| `expected_agents` | Orchestrator plan state (plans in Implementing phase) |
| `last_agent_heartbeat_ms` | ProcessSupervisor heartbeat tracker |
| `chain_connected` | (Not used in current deployment — reserved for future chain integration) |
| `chain_expected` | Configuration flag |
| `spec_hash_expected` | Plan TOML frontmatter |
| `spec_hash_actual` | Computed from current codebase state |
| `coverage_history` | Gate results over recent builds |

The orchestrator constructs the snapshot periodically (every 10 seconds
in the default configuration) and passes it to the health monitor.
The snapshot is a read-only copy of live state — computing the health
check does not hold any locks or block the orchestrator's main loop.

---

## VSM Mapping

In Beer's Viable System Model (Beer, 1972), the health monitor maps to
**System 3*** (System Three-Star) — the audit channel:

| VSM Component | Roko Equivalent |
|--------------|----------------|
| System 1 | Individual agents executing tasks |
| System 2 | Conventions, templates, shared protocols |
| System 3 | Orchestrator (internal oversight, resource allocation) |
| **System 3*** | **Health monitor (sporadic audit, independent check)** |
| System 4 | Learning system (external adaptation) |
| System 5 | Configuration and policy |

System 3* is the audit function — it checks whether System 3's
(the orchestrator's) model of reality matches actual reality. The
health monitor does exactly this: it independently checks whether
the agents the orchestrator thinks are running are actually running,
whether the spec the orchestrator is working from is still current,
and whether the quality metrics the orchestrator relies on are
trending in the right direction.

---

## File Reference

| File | What |
|------|------|
| `crates/roko-conductor/src/health.rs` | HealthMonitor, SystemSnapshot, HealthStatus, 4 check methods |
