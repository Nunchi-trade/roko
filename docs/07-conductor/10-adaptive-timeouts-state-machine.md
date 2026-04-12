# Adaptive Timeouts and the State Machine

> Phase timeouts by complexity band. Hard timeouts that are law,
> not suggestions. PhaseTransition records for audit trails.


> **Implementation**: Built

---

## The State Machine

Each plan progresses through a well-defined set of phases. The state
machine defines which transitions are valid and enforces them:

```
Queued → Implementing → Gating → Reviewing → Done → Merging → Complete
                          │         │
                          ▼         ▼
                      AutoFixing  (re-implement)
                          │
                          ▼
                      (back to Gating)
```

Invalid transitions are structurally impossible. A plan cannot jump
from Queued to Reviewing. A plan cannot go from Complete back to
Implementing. The state machine is a DATA STRUCTURE, not code paths.

This is Hard Guarantee 1 from the failure prevention catalog:
"Explicit State Machine with Compile-Time Transition Validation."

---

## Phase Timeouts

Every phase has a hard wall-clock timeout. When the timeout fires,
the plan transitions to Failed. No exceptions.

```rust
// From crates/roko-conductor/src/state_machine.rs
pub fn phase_timeout(phase: PlanPhase, complexity: Complexity) -> Duration {
    match (phase, complexity) {
        // Implementation timeouts scale with complexity
        (Implementing, Complex)  => Duration::from_secs(600),   // 10 min
        (Implementing, Standard) => Duration::from_secs(300),   // 5 min
        (Implementing, Fast)     => Duration::from_secs(120),   // 2 min

        // Other phases have fixed timeouts
        (Gating, _)              => Duration::from_secs(300),   // 5 min
        (Reviewing, _)           => Duration::from_secs(300),   // 5 min
        (Merging, _)             => Duration::from_secs(60),    // 1 min
        // ...
    }
}
```

### Why Hard Timeouts

Soft timeouts (conductor detects timeout → decides whether to
intervene) do not work. Production experience:

- Conductor detects timeout → nudges agent → agent continues
  (ignoring nudge) → conductor detects timeout again → nudges
  again → 10 minutes wasted

Hard timeouts are enforced by the state machine:

```rust
// Timer check runs every 5 seconds
for plan in active_plans {
    let elapsed = plan.phase_entered_at.elapsed();
    let timeout = phase_timeout(plan.phase, plan.complexity);
    if elapsed > timeout {
        transition(plan, Failed(Timeout));  // HARD. No negotiation.
    }
}
```

This is Hard Guarantee 2: "Every Phase Has a Hard Timeout."

### Complexity-Based Scaling

Implementation timeouts scale with plan complexity because complex
plans legitimately need more time. A trivial plan (add a const, fix
a typo) should complete in 2 minutes. A complex plan (implement a
new subsystem, wire multiple crates) may need 10 minutes.

The complexity classification comes from the plan's TOML frontmatter
or from the cascade router's complexity classifier:

| Complexity | Typical Plans | Implementation Timeout |
|-----------|--------------|----------------------|
| Fast | Typo fixes, const additions, doc updates | 120s (2 min) |
| Standard | Function implementations, module additions | 300s (5 min) |
| Complex | Multi-crate features, architectural changes | 600s (10 min) |

Other phases (Gating, Reviewing, Merging) do not scale with
complexity because their duration depends on codebase size and
test suite speed, not on plan complexity.

---

## PhaseTransition Records

Every phase transition produces an audit record:

```rust
pub struct PhaseTransition {
    pub plan_id: String,
    pub from: PlanPhase,
    pub to: PlanPhase,
    pub timestamp: String,   // ISO 8601
    pub reason: String,      // why the transition occurred
}
```

These records provide a complete history of every plan's progression:

```
plan-42: Queued → Implementing    (2026-04-09T10:00:00Z, "dependencies met")
plan-42: Implementing → Gating    (2026-04-09T10:03:22Z, "all tasks complete")
plan-42: Gating → Implementing    (2026-04-09T10:04:15Z, "gate failed: 2 compile errors")
plan-42: Implementing → Gating    (2026-04-09T10:06:48Z, "all tasks complete")
plan-42: Gating → Reviewing       (2026-04-09T10:07:30Z, "all gates passed")
plan-42: Reviewing → Merging      (2026-04-09T10:08:45Z, "review approved")
plan-42: Merging → Complete       (2026-04-09T10:09:02Z, "merge successful")
```

This audit trail enables:

1. **Post-mortem analysis**: How long did each phase take? How many
   gate-fail-retry cycles occurred?
2. **Performance optimization**: Which phase is the bottleneck?
   If Gating consistently takes 4 minutes of a 7-minute plan, gate
   optimization has the highest impact.
3. **Anomaly detection**: Plans that transition through unusual
   sequences can be flagged for investigation.
4. **Learning system input**: Phase timing data feeds into the
   cascade router's complexity classifier and the adaptive gate
   threshold system.

---

## Adaptive Timeout Computation

The static timeouts are derived from production experience. An
adaptive system would compute timeouts from observed execution data:

### P95-Based Adaptive Timeout

From the production hardening plan (doc 16):

```rust
impl LatencyStats {
    /// Recommended timeout = 2x the observed p95 latency,
    /// clamped to [5s, 300s].
    pub fn adaptive_timeout_ms(&self) -> u64 {
        if self.observations < 10 { return 120_000; }  // Not enough data
        let p95 = self.p95_ms();
        let timeout = (p95 * 2.0) as u64;
        timeout.clamp(5_000, 300_000)
    }
}
```

This approach sets the timeout at 2x the observed 95th percentile
latency. With enough observations, the timeout automatically adjusts
to match actual execution patterns:

- If complex plans consistently finish in 4 minutes, the adaptive
  timeout settles at ~8 minutes (p95 ≈ 4 min × 2)
- If model upgrades make agents faster (finishing in 2 minutes),
  the timeout automatically tightens to ~4 minutes
- If codebase growth makes compilation slower, the timeout
  automatically widens

### Cold Start Behavior

With fewer than 10 observations, the system uses the static default
(120 seconds). This prevents the adaptive system from setting
unreasonable timeouts based on a small, potentially unrepresentative
sample.

After 10+ observations, the adaptive timeout takes over. The p95
calculation uses a sliding window of recent observations, so the
timeout reflects current performance rather than historical averages.

### Per-Phase Adaptive Timeouts

Different phases have different timeout distributions:

| Phase | What Determines Duration |
|-------|------------------------|
| Implementing | Agent reasoning speed, codebase complexity |
| Gating | Compile time, test suite size |
| Reviewing | Reviewer model speed, number of reviewers |
| Merging | Git merge speed, post-merge test time |

Each phase should have its own adaptive timeout, computed from its
own observation window. The infrastructure exists in the latency
registry (`roko-learn/src/latency.rs`).

---

## TTFT Timeout

Time-to-first-token (TTFT) timeout provides early detection of
stalled providers:

```rust
pub struct ProviderConfig {
    pub timeout_ms: Option<u64>,        // Hard per-request timeout (120s default)
    pub ttft_timeout_ms: Option<u64>,   // Time-to-first-token timeout (15s default)
    pub connect_timeout_ms: Option<u64>, // TCP connection timeout (5s default)
}
```

If a provider has not sent a single token in 15 seconds, something is
wrong — fail fast and try a fallback rather than waiting 2 minutes for
the hard timeout. This layered timeout approach detects problems
earlier:

```
Request sent
    │
    │ ← connect_timeout_ms (5s): TCP connection must be established
    │
    │ ← ttft_timeout_ms (15s): first token must arrive
    │
    │ ← timeout_ms (120s): complete response must arrive
    │
Response received
```

Each layer catches a different failure mode:
- Connection timeout → DNS failure, firewall, provider down
- TTFT timeout → provider overloaded, queue backed up
- Full timeout → response generation taking too long

---

## Graceful Shutdown Sequence

When a Shutdown cognitive signal is received or Ctrl+C is pressed,
the orchestrator executes a four-phase shutdown:

```rust
pub async fn run_with_shutdown(executor: PlanExecutor, snapshot_path: &Path) -> Result<()> {
    let shutdown = signal::ctrl_c();

    tokio::select! {
        result = executor.run() => result,
        _ = shutdown => {
            // Phase 1: Stop accepting new tasks
            executor.stop_accepting();

            // Phase 2: Drain with timeout (30s grace period)
            let drain = tokio::time::timeout(
                Duration::from_secs(30),
                executor.drain_in_flight(),
            ).await;

            if drain.is_err() {
                // Phase 2b: Kill remaining agents if drain times out
                executor.kill_all_agents().await;
            }

            // Phase 3: Checkpoint
            executor.save_snapshot(snapshot_path)?;

            // Phase 4: Flush logs
            executor.flush_logs().await;

            Ok(())
        }
    }
}
```

### Atomic Checkpoint Writes

The checkpoint write uses temp-file-then-rename to prevent
corruption from mid-write crashes:

```rust
fn save_snapshot_atomic(snapshot: &ExecutorSnapshot, path: &Path) -> Result<()> {
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(snapshot)?;
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, path)?;  // Atomic on POSIX
    Ok(())
}
```

A kill signal mid-write leaves the previous snapshot intact rather
than producing a corrupted file. If the state file is corrupted
(disk issue, OOM kill during rename on some filesystems), the
persistence manager falls back to reconstructing completed tasks
from the append-only event log.

---

## Relationship to Process Supervision

Phase timeouts in the Conductor complement process-level supervision
in `bardo-runtime`:

| Layer | Timeout Type | What It Catches |
|-------|-------------|----------------|
| Process (bardo-runtime) | Process timeout | Agent process hangs |
| Task (Conductor) | Phase timeout | Task takes too long in any phase |
| Plan (Conductor) | Wall-clock limit | Total plan execution exceeds limit |
| Batch (Orchestrator) | Budget limit | Total batch cost exceeds limit |

Each layer catches problems at a different granularity. A process
timeout catches an individual agent hang. A phase timeout catches a
task that cycles through multiple agent processes but never
completes. A plan wall-clock limit catches plans that make progress
but too slowly. A batch budget limit catches runaway cost across
all plans.

---

## File Reference

| File | What |
|------|------|
| `crates/roko-conductor/src/state_machine.rs` | Phase timeouts, PhaseTransition, complexity-based scaling |
| `crates/roko-learn/src/latency.rs` | LatencyStats, adaptive_timeout_ms(), percentile computation |
| `crates/roko-core/src/config/schema.rs` | ProviderConfig with timeout fields |
| `crates/roko-cli/src/orchestrate.rs` | Graceful shutdown, atomic checkpoints |
