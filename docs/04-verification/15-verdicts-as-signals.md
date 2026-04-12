# Verdicts as signals

> Layer 3 Harness -- Verification as Cognition
> Status: **Specification** -- gate verdict emission wired, signal re-entry planned
> Canonical source: `crates/roko-gate/`, `crates/roko-core/src/kind.rs` (Kind::GateVerdict)
> Cross-references: [00-gate-trait.md](00-gate-trait.md), [03-gate-pipeline.md](03-gate-pipeline.md)

---

## Purpose

Gate verdicts are not terminal events. They are signals -- first-class Engrams that re-enter the signal pipeline. A compile failure is knowledge. A test pass is evidence. A clippy warning is a heuristic.

This document specifies how gate verdicts become Engrams, how they flow through the universal cognitive loop, and how downstream consumers (Scorer, Router, Composer, Dreams) use them.

---

## 1. The core claim: verification is cognition

In a standard CI pipeline, a gate verdict is an end state: pass or fail, logged and forgotten. In Roko, a gate verdict is a beginning. When a compile gate fails, that failure is an Engram with a Kind, Score, Decay, and lineage. It enters the Substrate. Other components query it:

- The **Scorer** appraises the verdict (a compile error on a file the agent just modified scores higher than a pre-existing warning).
- The **Router** uses verdict history to select models (tasks that repeatedly fail compile get routed to stronger models).
- The **Composer** injects recent verdicts into agent prompts (the agent sees its own failures).
- **Dreams** replays verdict patterns during consolidation (the system learns which gate patterns predict task failure).

The verdict is not metadata about the pipeline. It is a data point in the agent's cognitive process.

---

## 2. Verdict-to-signal transformation

### 2.1 The GateVerdict struct

Two `GateVerdict` structs exist in the codebase. The episode logger's version carries the learning-relevant fields:

```rust
/// From crates/roko-learn/src/episode_logger.rs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateVerdict {
    /// Gate identifier ("compile", "test", "lint", ...).
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Optional short diagnostic (hashed, never raw output).
    pub signature: Option<String>,
}
```

The dashboard's version adds plan/task context:

```rust
/// From crates/roko-core/src/dashboard_snapshot.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateVerdict {
    pub plan_id: String,
    pub task_id: String,
    pub gate: String,
    pub passed: bool,
    pub ts_millis: u64,
}
```

### 2.2 Transformation to Signal

When a gate completes, the orchestrator transforms its verdict into a Signal:

```
fn verdict_to_signal(verdict: &GateVerdict, task_signal: &Signal) -> Signal {
    Signal::builder(Kind::GateVerdict)
        .body(Body::json(verdict))
        .decay(Decay::HalfLife { half_life_ms: 86_400_000 })  // 24h
        .lineage([task_signal.id])   // verdict derives from the task
        .tag("gate", &verdict.gate)
        .tag("passed", &verdict.passed.to_string())
        .tag("plan_id", &verdict.plan_id)
        .tag("task_id", &verdict.task_id)
        .build()
}
```

Key properties:

| Property | Value | Rationale |
|---|---|---|
| Kind | `Kind::GateVerdict` | Already defined in `roko-core` |
| Decay | `HalfLife { 86_400_000 }` (24h) | Code changes invalidate verdicts; yesterday's compile pass is stale |
| Lineage | Points to the task Signal | Preserves causal chain for auditing |
| Tags | gate name, passed, plan_id, task_id | Enable filtering by gate type and outcome |

---

## 3. Signal pipeline flow

Once emitted, the verdict Signal enters the standard cognitive loop:

```
Gate verdict emitted
    |
    v
Substrate.write(verdict_signal)     -- persisted to .roko/signals.jsonl
    |
    v
Scorer.score(verdict_signal)        -- appraise relevance and urgency
    |
    v
Router.select(candidates)           -- verdict history influences model routing
    |
    v
Composer.compose(context)           -- recent verdicts injected into prompts
    |
    v
Dreams.replay(episodes)             -- verdict patterns extracted during consolidation
```

---

## 4. Consumer specifications

### 4.1 Scorer: verdict appraisal

The Scorer assigns a Score to the verdict Signal based on:

```
Scoring dimensions for GateVerdict signals:

  relevance:   1.0 if verdict is for the currently active task
               0.5 if verdict is for a task in the same plan
               0.1 if verdict is for a different plan
  confidence:  1.0 (gate verdicts are deterministic)
  urgency:     0.9 if failed (failure needs immediate attention)
               0.3 if passed (success is informational)
  novelty:     1.0 if this is the first verdict for this gate+task
               0.2 if this is a repeated verdict (re-run of same gate)
  salience:    scaled by recency -- fresher verdicts score higher
  coherence:   1.0 (verdicts are self-consistent by construction)
  surprise:    1.0 if outcome contradicts the model's prediction
               0.0 if outcome matches prediction
```

### 4.2 Router: verdict-informed model selection

The cascade router queries verdict history when selecting a model for a task:

```
Routing adjustment from verdict history:

  For task T, query Substrate for GateVerdict signals where task_id == T:
    - If 0 prior failures: standard routing (no adjustment)
    - If 1 prior failure:  escalate model tier by 1 (e.g., Haiku -> Sonnet)
    - If 2+ prior failures: escalate to maximum tier (Opus)
    - If 3+ prior failures with same gate signature: flag for replanning

  Implementation path:
    CascadeRouter::select() calls Substrate::query(
        Filter::kind(Kind::GateVerdict)
            .tag("task_id", task_id)
            .tag("passed", "false")
    )
```

### 4.3 Composer: verdict injection

The SystemPromptBuilder includes recent verdicts in the agent's prompt:

```
Section: "Recent Gate Results"
Priority: High (same as gate errors in the budget table)
Max tokens: 500
Min tokens: 50

Content format:
  ## Previous attempts on this task

  Attempt 1: FAIL (compile)
    Error: E0599 - no method named `foo` found for struct `Bar`
    Signature: a3f8c2

  Attempt 2: FAIL (test)
    Error: assertion failed in test_routing_basic
    Signature: 7d1e4b
```

This gives the agent direct visibility into its own failure history, preventing it from repeating the same mistake.

### 4.4 Dreams: verdict pattern extraction

During NREM replay, Dreams extracts patterns from verdict sequences:

```
Pattern extraction from verdict signals:

  Input: all GateVerdict signals from the last consolidation window
  Process:
    1. Group by (gate, signature) -- same error type
    2. For each group with >= 3 occurrences:
       a. Extract common context (file paths, error codes, task types)
       b. Generate a Heuristic knowledge entry:
          "When working on [context], [gate] tends to fail with [signature]"
       c. Insert at Transient tier for validation
    3. For groups where failure was followed by success:
       a. Extract the delta between failing and succeeding attempts
       b. Generate a StrategyFragment:
          "To fix [signature], the successful approach was [delta]"
```

---

## 5. Verdict decay and lifecycle

| Stage | Timing | Action |
|---|---|---|
| Emission | Gate completes | Signal written to Substrate |
| Active use | 0 - 4 hours | Composer injects into prompts; Router adjusts model selection |
| Fading relevance | 4 - 24 hours | Weight decays below 0.5; lower priority in Composer |
| Consolidation | During Dreams Delta | Patterns extracted; individual verdicts no longer needed |
| Pruning | Weight < threshold | Substrate.prune() removes the verdict Signal |

The 24-hour HalfLife means a verdict retains 50% weight after one day. This is appropriate because code changes within a day can invalidate any verdict. After Dreams consolidation, the patterns survive in knowledge entries even after the raw verdicts are pruned.

---

## 6. Lineage and auditing

Every verdict Signal records its lineage -- the task Signal it derived from:

```
verdict.lineage = [task_signal.id]
```

This creates a DAG:

```
Plan Signal
  |
  +-- Task Signal (T1)
  |     |
  |     +-- GateVerdict (compile: pass)
  |     +-- GateVerdict (test: fail)
  |     +-- GateVerdict (test: pass, attempt 2)
  |
  +-- Task Signal (T2)
        |
        +-- GateVerdict (compile: fail)
```

The `roko replay` command walks this DAG to reconstruct the full verification history for any plan or task.

---

## 7. Configuration parameters

| Parameter | Default | Range | Description |
|---|---|---|---|
| `verdict_decay_half_life_ms` | 86,400,000 (24h) | 3,600,000 - 604,800,000 | How fast verdicts lose relevance |
| `verdict_max_prompt_tokens` | 500 | 50 - 2,000 | Max tokens for verdict section in prompts |
| `verdict_escalation_threshold` | 2 | 1 - 5 | Failures before model tier escalation |
| `verdict_replan_threshold` | 3 | 2 - 10 | Same-signature failures before replanning |
| `verdict_dreams_min_group_size` | 3 | 2 - 10 | Min occurrences for pattern extraction |

---

## 8. Error handling

| Condition | Response |
|---|---|
| Verdict body fails JSON serialization | Log error, emit verdict with `Body::text(gate + ":" + passed)` fallback |
| Substrate write fails (disk full, I/O error) | Buffer in memory (up to 100 verdicts), retry on next tick |
| Verdict references a task Signal that was pruned | Verdict still valid; lineage points to a hash that may not resolve |
| Duplicate verdict (same gate + task + attempt) | Deduplicate by content hash; the second write is a no-op |

---

## 9. Implementation wiring

Current state:

| Component | Status |
|---|---|
| `Kind::GateVerdict` in roko-core | **Implemented** |
| GateVerdict struct in episode_logger | **Implemented** |
| GateVerdict struct in dashboard_snapshot | **Implemented** |
| Gate verdict emission in orchestrate.rs | **Wired** (verdicts logged to episodes) |
| Verdict-to-Signal transformation | **Not yet** (verdicts logged but not emitted as Signals) |
| Scorer appraisal of verdict Signals | **Not yet** |
| Router verdict-based escalation | **Not yet** (escalation uses iteration count, not verdict Signals) |
| Composer verdict injection | **Partially** (gate errors injected, but not as Signal queries) |
| Dreams verdict pattern extraction | **Not yet** |

The wiring path:

1. In `orchestrate.rs`, after each gate run, call `verdict_to_signal()` and write to Substrate.
2. In `CascadeRouter::select()`, query Substrate for prior verdict Signals on the current task.
3. In `SystemPromptBuilder`, query Substrate for recent verdict Signals instead of passing gate errors directly.
4. In `DreamsEngine::consolidate()`, include verdict Signals in the replay set.

Estimated LOC: ~120 for transformation + Substrate writes, ~60 for Router query, ~40 for Composer query, ~80 for Dreams pattern extraction. Total: ~300 LOC.

---

## 10. Test criteria

1. Gate verdict produces a Signal with `Kind::GateVerdict`, correct lineage, and 24h HalfLife.
2. Verdict Signal round-trips through serde without loss.
3. Substrate query by `tag("gate", "compile")` returns only compile verdicts.
4. Router escalates model tier after 2 consecutive failures on the same task.
5. Composer includes verdict section with correct token budget and priority ordering.
6. Dreams extracts a Heuristic from 3+ same-signature failures.
7. Verdict weight reaches 0.5 at exactly 24 hours.
8. Duplicate verdicts (same content hash) are deduplicated on write.

---

## Cross-references

- [00-gate-trait.md](00-gate-trait.md) -- Gate trait and Verdict type
- [03-gate-pipeline.md](03-gate-pipeline.md) -- 6-rung gate pipeline
- [06-adaptive-thresholds.md](06-adaptive-thresholds.md) -- Threshold adjustment from verdict history
- [08-agent-feedback-from-gates.md](08-agent-feedback-from-gates.md) -- How agents receive gate feedback
- [../00-architecture/09-universal-cognitive-loop.md](../00-architecture/09-universal-cognitive-loop.md) -- The loop verdicts re-enter
- [../05-learning/00-episode-logger.md](../05-learning/00-episode-logger.md) -- Where verdicts are currently logged
- `crates/roko-core/src/kind.rs` -- Kind::GateVerdict definition
- `crates/roko-learn/src/episode_logger.rs` -- GateVerdict struct
