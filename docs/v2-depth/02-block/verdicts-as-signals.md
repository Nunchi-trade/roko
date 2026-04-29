# Verdicts as Signals

> Depth for [02-CELL.md](../../unified/02-CELL.md). Verdicts as first-class Signals with scoring, lineage, demurrage, and forensic replay -- turning verification outcomes into cognitive data.

---

## Overview

In a standard CI pipeline, a gate verdict is an end state: pass or fail, logged and forgotten. In Roko, a Verdict is a Signal -- a first-class datum with a Kind, Score, Decay, lineage, and content hash. It enters the Store. Other Cells query it:

- The **Score Cell** appraises the Verdict (a compile error on a file the agent just modified scores higher than a pre-existing warning)
- The **Route Cell** uses Verdict history to select models (tasks that repeatedly fail compile get routed to stronger models)
- The **Compose Cell** injects recent Verdicts into agent prompts (the agent sees its own failures)
- **Dreams** replays Verdict patterns during consolidation (the system learns which patterns predict task failure)

The Verdict is not metadata about the pipeline. It is a data point in the agent's cognitive process.

---

## 1. Verdict-to-Signal Transformation

When a Verify Cell completes, the orchestrator transforms its output into a Signal:

```rust
fn verdict_to_signal(verdict: &GateVerdict, task_signal: &Signal) -> Signal {
    Signal::builder(Kind::GateVerdict)
        .body(Body::json(verdict))
        .decay(Decay::HalfLife { half_life_ms: 86_400_000 })  // 24h
        .lineage([task_signal.id])   // causal chain: verdict derives from task
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
| Kind | `Kind::GateVerdict` | Defined in `roko-core`, enables typed queries |
| Decay | HalfLife 86,400,000ms (24h) | Code changes invalidate Verdicts; yesterday's compile pass is stale |
| Lineage | Points to task Signal | Preserves causal chain for auditing and replay |
| Tags | gate name, passed, plan_id, task_id | Enable Store queries by gate type and outcome |

The two existing `GateVerdict` structs in the codebase:

```rust
// Episode logger version (learning-relevant fields):
pub struct GateVerdict {
    pub gate: String,       // "compile", "test", "lint"
    pub passed: bool,
    pub signature: Option<String>,  // Hashed diagnostic (never raw output)
}

// Dashboard version (adds plan/task context):
pub struct GateVerdict {
    pub plan_id: String,
    pub task_id: String,
    pub gate: String,
    pub passed: bool,
    pub ts_millis: u64,
}
```

---

## 2. Signal Pipeline Flow

Once emitted, the Verdict Signal enters the standard cognitive loop (see [04-EXECUTION.md](../../unified/04-EXECUTION.md)):

```
Verify Cell completes
    |
    v
Store.put(verdict_signal)         -- persisted to .roko/signals.jsonl
    |
    v
Score Cell rates verdict_signal   -- appraise relevance and urgency
    |
    v
Route Cell queries verdict history -- adjusts model selection
    |
    v
Compose Cell injects verdicts      -- recent verdicts in agent prompt
    |
    v
Dreams replays verdict patterns    -- extract heuristics during consolidation
```

---

## 3. Consumer Specifications

### 3.1 Score Cell: Verdict Appraisal

The Score Cell rates Verdict Signals along the standard 5 dimensions (see [02-CELL.md](../../unified/02-CELL.md) S2.2):

```
relevance:   1.0 if verdict is for the currently active task
             0.5 if for a task in the same plan
             0.1 if for a different plan
confidence:  1.0 (Verify Cells are deterministic)
urgency:     0.9 if failed (needs immediate attention)
             0.3 if passed (informational)
novelty:     1.0 if first verdict for this gate+task
             0.2 if repeated (re-run of same gate)
surprise:    1.0 if outcome contradicts the Route Cell's prediction
             0.0 if outcome matches prediction
```

### 3.2 Route Cell: Verdict-Informed Model Selection

The cascade router queries Verdict history when selecting a model:

```
For task T, query Store:
    Filter::kind(Kind::GateVerdict)
        .tag("task_id", T)
        .tag("passed", "false")

  0 prior failures  ->  standard routing
  1 prior failure   ->  escalate model tier by 1 (Haiku -> Sonnet)
  2+ prior failures ->  escalate to maximum tier (Opus)
  3+ same signature ->  flag for replanning
```

### 3.3 Compose Cell: Verdict Injection

The Compose Cell (SystemPromptBuilder) includes recent Verdicts as a high-priority section:

```
Section: "Recent Gate Results"
Priority: High
Max tokens: 500
Min tokens: 50

Content:
  ## Previous attempts on this task

  Attempt 1: FAIL (compile)
    Error: E0599 - no method named `foo` found for struct `Bar`
    Signature: a3f8c2

  Attempt 2: FAIL (test)
    Error: assertion failed in test_routing_basic
    Signature: 7d1e4b
```

This gives agents direct visibility into their own failure history, preventing repeated mistakes.

### 3.4 Dreams: Verdict Pattern Extraction

During NREM replay (consolidation), Dreams extracts patterns from Verdict sequences:

```
Input: all GateVerdict Signals from last consolidation window
Process:
    1. Group by (gate, signature) -- same error type
    2. For groups with >= 3 occurrences:
       a. Extract common context (file paths, error codes, task types)
       b. Generate Heuristic knowledge entry:
          "When working on [context], [gate] tends to fail with [signature]"
       c. Insert at Transient tier for validation
    3. For groups where failure was followed by success:
       a. Extract delta between failing and succeeding attempts
       b. Generate StrategyFragment:
          "To fix [signature], the successful approach was [delta]"
```

---

## 4. Verdict Decay and Lifecycle

| Stage | Timing | Action |
|---|---|---|
| **Emission** | Verify Cell completes | Signal written to Store |
| **Active use** | 0 - 4 hours | Compose Cell injects into prompts; Route Cell adjusts model |
| **Fading relevance** | 4 - 24 hours | Weight decays below 0.5; lower Compose priority |
| **Consolidation** | During Dreams Delta | Patterns extracted; individual Verdicts no longer needed |
| **Pruning** | Weight < threshold | Store.prune() removes the Verdict Signal |

The 24-hour HalfLife means a Verdict retains 50% weight after one day. Code changes within a day can invalidate any Verdict. After Dreams consolidation, the *patterns* survive in knowledge entries even after the raw Verdicts are pruned.

---

## 5. Lineage and the Causal DAG

Every Verdict Signal records its lineage -- the task Signal it derived from:

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

The `roko replay` command walks this DAG to reconstruct the full verification history for any plan or task. This is the foundation of forensic replay.

---

## 6. Forensic AI: Causal Replay

Forensic replay reconstructs, step by step, exactly what an agent did, why it did it, and what verification outcomes resulted -- with cryptographic proof that the reconstruction is accurate.

### 6.1 The Content-Addressed Chain

Every element in the replay chain is identified by its BLAKE3 hash:

```
TaskSpec (hash: 0xa3f...)
    -> SystemPrompt (hash: 0xb7c...)
    -> AgentTurn 1 (hash: 0xc1d...)
        ToolCall: Read "src/lib.rs" (hash: 0xd2e...)
        ToolCall: Edit "src/lib.rs" (hash: 0xf4a...)
    -> GateVerdict Rung 0 (hash: 0xc7d...)
        Detail: compile output (hash: 0xd8e...)
    -> AgentTurn 2 (hash: 0xe9f...)  [retry after gate failure]
    -> GateVerdict Rung 0 (hash: 0xab1...)  [pass]
    -> FinalOutcome (hash: 0xde4...)
```

Each node's hash incorporates its content. Modifying any byte changes the hash, making tampering detectable (same principle as Git commits and blockchain blocks).

### 6.2 Replay Algorithm

Given a task ID:

```
1. Query episode log for all turns with this task_id -> ordered turns
2. For each turn: retrieve system prompt, tool call I/O, agent response
3. Query signal log for all Verdicts with this task_id -> ordered Verdicts
4. For each Verdict: retrieve gate artifact from Store by hash
5. Build causal chain: TaskSpec -> Prompt -> Turns -> Verdicts -> Outcome
6. Verify chain integrity: recompute BLAKE3 hash per element, compare to stored
```

### 6.3 Causal Analysis Capabilities

**What-If**: Replay task with different model, compare Verdicts. Powers shadow testing (Loop 12).

**Root Cause**: Trace backward: which gate failed -> what was the failure -> which edit introduced it -> what was the agent's reasoning -> was the reasoning correct?

**Gap Analysis**: When a bug escapes all gates, identify which gate *should* have caught it. Feed back into eval generation to improve test templates.

### 6.4 Immutability Guarantees

| Mechanism | What It Guarantees |
|---|---|
| Content-addressed Signals | Changing any byte changes the hash |
| Append-only logs | Episodes and signals never modified during operation |
| Artifact Store (no delete/update) | Gate artifacts are permanent once stored |
| Hash chain (future) | Each Signal's hash incorporates parent's hash |

### 6.5 Performance and Storage

Per-execution overhead: < 5ms total (BLAKE3 hashing + logging).

For a typical plan (10 tasks, 3 attempts, 5 gates per attempt):
- Episodes: ~150 entries, ~500 KB
- Signals: ~150 entries, ~300 KB
- Artifacts: ~150, ~5 MB
- **Total: ~6 MB per plan**

A year of continuous operation: ~2 GB of forensic data.

### 6.6 Regulatory Compliance

| Regulation | Requirement | How Forensic Replay Satisfies |
|---|---|---|
| EU AI Act Art. 14 | Human oversight of high-risk AI | Complete action trace, Verdict checkpoints |
| SEC/CFTC | Algorithmic trading audit trail | Content-addressed chain from decision to execution |
| HIPAA | Health data access audit | Every file read/write timestamped |
| SOX | Financial system change controls | Immutable verification artifacts per code change |

---

## 7. Verdict Aggregation: Trend Detection

Individual Verdicts are snapshots. Trends across Verdicts reveal systemic changes.

### 7.1 Verdict Time Series

```rust
pub struct VerdictTimeSeries {
    pub gate: String,
    pub observations: VecDeque<VerdictObservation>,
    pub ema_pass_rate: f64,
    pub ema_score: f64,
    pub cusum_upper: f64,          // CUSUM for shift detection
    pub cusum_lower: f64,
    pub trend: VerdictTrend,
}

pub enum VerdictTrend {
    Stable,        // No significant change
    Improving,     // Consistent improvement
    Degrading,     // Consistent degradation
    Volatile,      // High variance, no direction
    RegimeShift,   // Fundamental change (BOCPD detected)
}
```

### 7.2 Trend Classification

Uses three signals in priority order:

1. **BOCPD regime change** (Bayesian Online Changepoint Detection): fundamental statistical shift -> `RegimeShift`
2. **CUSUM + EMA slope**: sustained drift upward -> `Improving`; downward -> `Degrading`
3. **Coefficient of variation**: CV > 0.3 -> `Volatile`
4. Otherwise: `Stable`

```
Constants:
  SLOPE_IMPROVING  =  0.005   (~0.5% improvement per observation)
  SLOPE_DEGRADING  = -0.005
  CUSUM_H          =  4.0     (decision interval)
  BOCPD_THRESHOLD  =  0.5     (changepoint probability)
  VOLATILITY_CV    =  0.3
```

### 7.3 Co-Failure Patterns

Gates that fail together likely share a root cause. The co-failure detector computes phi coefficients between gate pairs:

```rust
pub struct CoFailureDetector {
    pub co_failures: HashMap<(String, String), u64>,
    pub gate_counts: HashMap<String, u64>,
    pub correlation_threshold: f64,   // default: 0.3
}
```

Example: compile + lint fail together with phi > 0.3 -> likely same syntax error causes both.

### 7.4 Failure Signature Clustering

Group failures by error signature to identify recurring issues:

```rust
pub struct SignatureCluster {
    pub signature: String,
    pub gate: String,
    pub count: u64,
    pub affected_tasks: Vec<String>,
    pub affected_models: Vec<String>,
    pub trend: VerdictTrend,
}

impl SignatureCluster {
    pub fn severity(&self) -> f64 {
        // Composite of frequency (log-scaled), recency, breadth
        (frequency + recency + breadth) / 3.0
    }
}
```

---

## 8. Verdict-Driven Replanning

When Verdict patterns indicate structural failure, automatic replanning fires:

### 8.1 Triggers

| Trigger | Threshold | Action |
|---|---|---|
| Same error signature N times | 3 | `ModifyTask` (add constraints, change approach) |
| Negative progress N turns | 3 | `DecomposeTask` (split into sub-tasks) |
| Low promise N turns | 2 (threshold 0.2) | `ReplaceTask` (new task from failure context) |
| Plan-level failures | 3 tasks fail | Replan remaining tasks |
| Gate degradation trend | Detected | Add stronger gates |

### 8.2 Replanning Actions

```rust
pub enum ReplanAction {
    ModifyTask { modifications: Vec<TaskModification> },
    ReplaceTask { failure_context: FailureContext },
    DecomposeTask { split_points: Vec<String> },
    Escalate { escalation: EscalationType },
}

pub enum TaskModification {
    AddConstraint(String),
    RelaxRequirement(String),
    ChangeApproach(String),
    AddPrerequisite(String),
}
```

### 8.3 The Replanning Loop

```
Verdict stream
    |
    +-- Trend detection         -> VerdictTrend per gate
    +-- Co-failure analysis     -> Correlated gate pairs
    +-- Signature clustering    -> Recurring failure patterns
    |
    v
ReplanEngine (all signals combined)
    |
    +-- ModifyTask     -> Orchestrator adjusts task spec
    +-- DecomposeTask  -> DAG executor creates sub-tasks
    +-- ReplaceTask    -> Plan generator creates new task from failure context
    +-- Escalate       -> Stronger model / more gates / human review
```

---

## 9. Predictive Gate Selection

Instead of static complexity bands, predict which gates will fail based on task features and historical Verdicts:

```rust
pub struct PredictiveGateSelector {
    pub predictors: HashMap<String, Box<dyn FailurePredictor>>,
    pub inclusion_threshold: f64,    // default: 0.1
    pub max_gates: usize,            // default: 7
}
```

Strategy: always include mandatory gates (compile, test). Include optional gates only if P(failure) > threshold. Rank by P(failure) descending.

Few-shot calibration: for a new task, find K nearest historical tasks and use their Verdict patterns to set initial thresholds and retry budgets.

---

## 10. Configuration

| Parameter | Default | Range | Description |
|---|---|---|---|
| `verdict_decay_half_life_ms` | 86,400,000 (24h) | 3.6M - 604.8M | How fast Verdicts lose relevance |
| `verdict_max_prompt_tokens` | 500 | 50 - 2,000 | Max tokens for Verdict section in Compose Cell |
| `verdict_escalation_threshold` | 2 | 1 - 5 | Failures before model tier escalation |
| `verdict_replan_threshold` | 3 | 2 - 10 | Same-signature failures before replanning |
| `verdict_dreams_min_group_size` | 3 | 2 - 10 | Min occurrences for pattern extraction |

---

## 11. Persistence

```
.roko/learn/
  verdict-trends.json         # Per-gate trend data
  co-failures.json            # Co-failure matrix (phi coefficients)
  signature-clusters.json     # Recurring failure signatures
  replan-log.jsonl            # Replanning decisions and outcomes
  pattern-memory.json         # Historical verdict patterns for k-NN calibration
```

---

## What This Enables

1. **Verification as cognition**: Verdicts are not terminal events but cognitive data that flows through the entire system -- scored, routed, composed, dreamed.
2. **Self-correcting routing**: Failed Verdicts automatically escalate to more capable models. Persistent failure patterns trigger structural replanning.
3. **Forensic auditability**: Content-addressed chain from task to outcome, replayable and tamper-evident. Enables regulatory compliance for autonomous agents.
4. **Trend intelligence**: CUSUM/BOCPD trend detection surfaces systemic quality changes. Co-failure analysis finds shared root causes across gates.
5. **Predictive verification**: Historical Verdict patterns predict which gates will fail, enabling focused verification that reduces cost without reducing coverage.

## Feedback Loops

| Loop | Input | Output | Speed |
|---|---|---|---|
| Verdict -> Route Cell | Failed Verdict Signals | Model escalation | Per-attempt |
| Verdict -> Compose Cell | Recent Verdict history | Agent retry context | Per-turn |
| Verdict -> Dreams | Verdict patterns | Heuristic knowledge entries | Consolidation |
| Verdict trends | Time series observations | Trend classification (SPC) | Continuous |
| Verdict -> Replan | Failure streaks, trends | Task modification/decomposition | Per-task |
| Verdict -> Predictive selection | Historical patterns | Gate inclusion decisions | Per-task |
| Co-failure detection | Verdict pairs | Root cause candidates | Consolidation |

## Open Questions

1. **Verdict-to-Signal wiring gap**: Verdicts are currently logged to episodes but not emitted as first-class Signals into the Store. The transformation is ~120 LOC. Should this be wired before or after the Route Cell query path?
2. **Decay tuning**: The 24h half-life is appropriate for active development. For long-running projects with weekly deployments, should the half-life be configurable per workspace?
3. **Cross-plan Verdicts**: Should Verdicts from Plan A inform routing decisions for Plan B? Currently they're isolated by task_id. Broadening to plan-level or workspace-level could help but risks noise.
4. **Forensic data retention**: At ~6 MB per plan, long-running systems accumulate GB of forensic data. What is the right GC policy? Keep all data for N days, then prune to patterns only?
5. **Hash chain implementation**: The hash chain (each Signal incorporates parent's hash) is designed but not implemented. Priority relative to other wiring work?

---

## References

- [02-CELL.md](../../unified/02-CELL.md) -- Verify protocol, Verdict type, predict-publish-correct
- [01-SIGNAL.md](../../unified/01-SIGNAL.md) -- Signal structure, Kind system, demurrage, lineage
- [verify-as-universal-oracle.md](verify-as-universal-oracle.md) -- Verify's four simultaneous roles
- [gate-feedback-and-retry.md](gate-feedback-and-retry.md) -- Structured feedback classification
- `crates/roko-core/src/kind.rs` -- `Kind::GateVerdict` definition
- `crates/roko-learn/src/episode_logger.rs` -- GateVerdict struct
- `crates/roko-gate/src/feedback.rs` -- Feedback classification implementation
- Song et al. (ICLR 2025) -- Generation-Verification Gap
- Machalica et al. (ICSE-SEIP 2019) -- Predictive Test Selection
- Finn et al. (ICML 2017) -- MAML meta-learning
