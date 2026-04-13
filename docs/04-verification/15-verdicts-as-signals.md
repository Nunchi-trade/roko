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

---

## 11. Verdict Aggregation Across Time: Trend Detection

Individual verdicts are snapshots. Trends across verdicts reveal systemic changes —
a gate that was stable for weeks but now fails frequently, a model that suddenly
produces worse code, a plan category that consistently underperforms. Trend detection
transforms raw verdict streams into actionable intelligence.

> **Citation**: "Contrasting Test Selection, Prioritization, and Batch Testing at Scale"
> (Empirical Software Engineering, 2024) — ML-driven trend detection in CI pipelines.

### 11.1 Verdict Time Series

```rust
/// A verdict time series for a specific gate, tracking outcomes over time.
pub struct VerdictTimeSeries {
    /// Gate identifier.
    pub gate: String,
    /// Ordered observations (newest last).
    pub observations: VecDeque<VerdictObservation>,
    /// Maximum observations retained (sliding window).
    pub max_observations: usize,    // default: 500
    /// EMA of pass rate (same as AdaptiveThresholds.ema_pass_rate).
    pub ema_pass_rate: f64,
    /// EMA of verdict score (continuous, not just binary).
    pub ema_score: f64,
    /// CUSUM accumulators for shift detection (see §06 SPC).
    pub cusum_upper: f64,
    pub cusum_lower: f64,
    /// Computed trend classification.
    pub trend: VerdictTrend,
}

#[derive(Debug, Clone)]
pub struct VerdictObservation {
    pub timestamp_ms: u64,
    pub passed: bool,
    pub score: f32,
    pub plan_id: String,
    pub task_id: String,
    pub signature: Option<String>,
    pub model: Option<String>,
}

/// Trend classification for a verdict time series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictTrend {
    /// No significant change in behavior.
    Stable,
    /// Consistent improvement over the observation window.
    Improving,
    /// Consistent degradation over the observation window.
    Degrading,
    /// High variance, no clear direction.
    Volatile,
    /// Fundamental change in statistical properties (detected by BOCPD).
    RegimeShift,
}
```

### 11.2 Trend Classification Algorithm

```rust
impl VerdictTimeSeries {
    /// Classify the current trend from the observation window.
    ///
    /// Uses three signals:
    /// 1. EMA derivative (slope of the smoothed pass rate)
    /// 2. CUSUM shift detection (sustained drift)
    /// 3. BOCPD regime change (fundamental change)
    pub fn classify_trend(&self) -> VerdictTrend {
        // 1. Compute EMA slope over the last N observations
        let slope = self.ema_slope(20); // slope over last 20 observations

        // 2. Check CUSUM for sustained shift
        let cusum_signal = self.cusum_upper > CUSUM_H
            || self.cusum_lower > CUSUM_H;

        // 3. Check BOCPD for regime change
        let regime_change = self.bocpd_changepoint_prob > BOCPD_THRESHOLD;

        // Classification logic:
        if regime_change {
            return VerdictTrend::RegimeShift;
        }
        if cusum_signal && slope > SLOPE_IMPROVING {
            return VerdictTrend::Improving;
        }
        if cusum_signal && slope < SLOPE_DEGRADING {
            return VerdictTrend::Degrading;
        }

        // Volatility check: coefficient of variation
        let cv = self.score_std_dev() / self.ema_score.max(0.01);
        if cv > VOLATILITY_THRESHOLD {
            return VerdictTrend::Volatile;
        }

        VerdictTrend::Stable
    }
}

/// Trend detection constants.
const SLOPE_IMPROVING: f64 = 0.005;   // ~0.5% improvement per observation
const SLOPE_DEGRADING: f64 = -0.005;
const CUSUM_H: f64 = 4.0;             // CUSUM decision interval
const BOCPD_THRESHOLD: f64 = 0.5;     // P(changepoint) threshold
const VOLATILITY_THRESHOLD: f64 = 0.3; // CV above this = volatile
```

### 11.3 Multi-Gate Trend Dashboard

```
Verdict Trends (last 200 observations):
  Compile:        ███████████████ 97.2% STABLE        (slope: +0.001)
  Lint:           █████████████░░ 86.5% DEGRADING ↓   (slope: -0.012, CUSUM alert)
  Test:           ████████████░░░ 79.1% STABLE        (slope: +0.003)
  Symbol:         ███████████████ 98.0% IMPROVING ↑   (slope: +0.008)
  Generated:      ████████░░░░░░░ 54.2% VOLATILE ⚡    (CV: 0.42)
  Property:       █████████████░░ 88.0% REGIME SHIFT  (BOCPD: new baseline 88%)
```

---

## 12. Verdict Pattern Mining

Beyond per-gate trends, cross-gate and cross-task patterns reveal deeper structural
issues.

### 12.1 Co-Failure Patterns

```rust
/// Detect gates that tend to fail together.
///
/// If compile and lint failures correlate > threshold, they likely
/// share a root cause (e.g., syntax errors cause both).
pub struct CoFailureDetector {
    /// Co-occurrence matrix: entry (i,j) = count of times gate i and
    /// gate j both failed on the same task.
    pub co_failures: HashMap<(String, String), u64>,
    /// Total observations per gate.
    pub gate_counts: HashMap<String, u64>,
    /// Minimum co-occurrence for significance.
    pub min_co_occurrences: u64,     // default: 5
    /// Phi coefficient threshold for declaring correlation.
    pub correlation_threshold: f64,   // default: 0.3
}

impl CoFailureDetector {
    /// Record a set of gate verdicts from one task execution.
    pub fn observe(&mut self, verdicts: &[(&str, bool)]) {
        let failed: Vec<&str> = verdicts.iter()
            .filter(|(_, passed)| !passed)
            .map(|(gate, _)| *gate)
            .collect();

        for i in 0..failed.len() {
            for j in (i+1)..failed.len() {
                let key = if failed[i] < failed[j] {
                    (failed[i].to_string(), failed[j].to_string())
                } else {
                    (failed[j].to_string(), failed[i].to_string())
                };
                *self.co_failures.entry(key).or_insert(0) += 1;
            }
        }
        for (gate, _) in verdicts {
            *self.gate_counts.entry(gate.to_string()).or_insert(0) += 1;
        }
    }

    /// Return significantly correlated gate pairs.
    pub fn correlated_pairs(&self) -> Vec<CoFailurePair> {
        self.co_failures.iter()
            .filter(|(_, &count)| count >= self.min_co_occurrences)
            .filter_map(|((a, b), &count)| {
                let phi = self.phi_coefficient(a, b, count);
                if phi.abs() > self.correlation_threshold {
                    Some(CoFailurePair {
                        gate_a: a.clone(),
                        gate_b: b.clone(),
                        co_failure_count: count,
                        phi_coefficient: phi,
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}

pub struct CoFailurePair {
    pub gate_a: String,
    pub gate_b: String,
    pub co_failure_count: u64,
    pub phi_coefficient: f64,
}
```

### 12.2 Failure Signature Clustering

Group failures by their error signature to identify recurring issues:

```rust
/// Cluster verdict failures by error signature.
///
/// Repeated failures with the same signature suggest a systemic issue
/// that needs structural attention, not just retries.
pub struct SignatureCluster {
    /// The error signature (hashed diagnostic).
    pub signature: String,
    /// Gate that produced these failures.
    pub gate: String,
    /// Number of occurrences.
    pub count: u64,
    /// First and last occurrence timestamps.
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    /// Task IDs that experienced this failure.
    pub affected_tasks: Vec<String>,
    /// Plan IDs that experienced this failure.
    pub affected_plans: Vec<String>,
    /// Models that produced this failure.
    pub affected_models: Vec<String>,
    /// Whether this cluster is growing (more frequent over time).
    pub trend: VerdictTrend,
}

impl SignatureCluster {
    /// Compute severity: a composite of frequency, recency, and breadth.
    pub fn severity(&self) -> f64 {
        let frequency = (self.count as f64).ln().max(0.0) / 5.0; // log-scaled
        let recency = 1.0; // would use time decay in practice
        let breadth = self.affected_plans.len() as f64
            / 10.0_f64.max(self.affected_plans.len() as f64);
        (frequency + recency + breadth) / 3.0
    }
}
```

---

## 13. Verdict-Driven Replanning

When verdict patterns indicate that a plan or task is fundamentally broken, automatic
replanning modifies the execution strategy without human intervention.

### 13.1 Replanning Triggers

```rust
/// Conditions that trigger automatic replanning.
pub struct ReplanTriggers {
    /// Same error signature fails N times across attempts → replan the task.
    pub same_signature_threshold: u32,    // default: 3
    /// Progress score negative for N consecutive turns → replan approach.
    pub negative_progress_threshold: u32, // default: 3
    /// Promise score below this for N turns → abort and replan.
    pub low_promise_threshold: f64,       // default: 0.2
    pub low_promise_turns: u32,           // default: 2
    /// Plan-level: if > N tasks fail in a plan → re-plan remaining tasks.
    pub plan_failure_threshold: u32,      // default: 3
    /// Gate degradation trend detected → replan with additional gates.
    pub trend_degradation_trigger: bool,  // default: true
}

/// Replanning action to take.
#[derive(Debug, Clone)]
pub enum ReplanAction {
    /// Modify the task: add constraints, decompose into sub-tasks,
    /// change the approach described in the task spec.
    ModifyTask {
        task_id: String,
        reason: String,
        /// Suggested modifications based on failure analysis.
        modifications: Vec<TaskModification>,
    },
    /// Replace the task entirely with a new plan generated from
    /// the failure context.
    ReplaceTask {
        task_id: String,
        reason: String,
        /// Context from failures to inform the new plan.
        failure_context: FailureContext,
    },
    /// Decompose a single failing task into smaller sub-tasks that
    /// are individually more likely to pass gates.
    DecomposeTask {
        task_id: String,
        reason: String,
        /// Suggested decomposition points.
        split_points: Vec<String>,
    },
    /// Escalate: add stronger gates, use more capable model, or
    /// flag for human review.
    Escalate {
        task_id: String,
        reason: String,
        escalation: EscalationType,
    },
}

#[derive(Debug, Clone)]
pub enum TaskModification {
    /// Add a constraint to the task spec (e.g., "do not modify file X").
    AddConstraint(String),
    /// Remove a requirement that is causing failures.
    RelaxRequirement(String),
    /// Change the approach (e.g., "use trait objects instead of generics").
    ChangeApproach(String),
    /// Add a prerequisite task that must complete first.
    AddPrerequisite(String),
}

pub struct FailureContext {
    /// Error signatures from failed attempts.
    pub signatures: Vec<String>,
    /// Gates that consistently fail.
    pub failing_gates: Vec<String>,
    /// Files that were modified in failed attempts.
    pub modified_files: Vec<String>,
    /// Successful approaches for similar tasks (from skill library).
    pub similar_successes: Vec<String>,
}

pub enum EscalationType {
    /// Add more verification gates.
    AddGates(Vec<Box<dyn Gate>>),
    /// Use a more capable model.
    UpgradeModel(String),
    /// Flag for human review.
    HumanReview,
}
```

### 13.2 Replanning Decision Engine

```rust
/// Engine that monitors verdict patterns and triggers replanning.
pub struct ReplanEngine {
    pub triggers: ReplanTriggers,
    /// Per-task failure tracking.
    pub task_failures: HashMap<String, TaskFailureState>,
    /// Per-plan failure tracking.
    pub plan_failures: HashMap<String, PlanFailureState>,
}

pub struct TaskFailureState {
    /// Consecutive attempts with same error signature.
    pub same_signature_streak: u32,
    /// Last seen error signature.
    pub last_signature: Option<String>,
    /// Progress scores for recent turns.
    pub recent_progress: VecDeque<f64>,
    /// Promise scores for recent turns.
    pub recent_promise: VecDeque<f64>,
}

impl ReplanEngine {
    /// Process a new verdict and determine if replanning is needed.
    pub fn process_verdict(&mut self, verdict: &GateVerdict,
                           process_reward: &ProcessReward)
        -> Option<ReplanAction>
    {
        let state = self.task_failures
            .entry(verdict.task_id.clone())
            .or_default();

        // Track signature streaks
        if !verdict.passed {
            if verdict.signature.as_deref() == state.last_signature.as_deref() {
                state.same_signature_streak += 1;
            } else {
                state.same_signature_streak = 1;
                state.last_signature = verdict.signature.clone();
            }
        } else {
            state.same_signature_streak = 0;
        }

        // Track process rewards
        state.recent_progress.push_back(process_reward.progress);
        state.recent_promise.push_back(process_reward.promise);
        if state.recent_progress.len() > 10 {
            state.recent_progress.pop_front();
            state.recent_promise.pop_front();
        }

        // Check triggers
        if state.same_signature_streak >= self.triggers.same_signature_threshold {
            return Some(ReplanAction::ModifyTask {
                task_id: verdict.task_id.clone(),
                reason: format!(
                    "Same error signature '{}' failed {} times consecutively",
                    state.last_signature.as_deref().unwrap_or("unknown"),
                    state.same_signature_streak
                ),
                modifications: self.suggest_modifications(verdict, state),
            });
        }

        let negative_progress_count = state.recent_progress.iter()
            .rev()
            .take_while(|&&p| p < -0.1)
            .count() as u32;
        if negative_progress_count >= self.triggers.negative_progress_threshold {
            return Some(ReplanAction::DecomposeTask {
                task_id: verdict.task_id.clone(),
                reason: format!(
                    "Negative progress for {} consecutive turns",
                    negative_progress_count
                ),
                split_points: self.suggest_decomposition(verdict),
            });
        }

        let low_promise_count = state.recent_promise.iter()
            .rev()
            .take_while(|&&p| p < self.triggers.low_promise_threshold)
            .count() as u32;
        if low_promise_count >= self.triggers.low_promise_turns {
            return Some(ReplanAction::ReplaceTask {
                task_id: verdict.task_id.clone(),
                reason: format!(
                    "Promise below {:.1} for {} turns — approach is not viable",
                    self.triggers.low_promise_threshold,
                    low_promise_count
                ),
                failure_context: self.build_failure_context(verdict, state),
            });
        }

        None // No replanning needed
    }
}
```

### 13.3 Replanning Feedback Loop

```
Verdict stream
    │
    ├── Trend detection ──────► VerdictTrend per gate
    │                              │
    ├── Co-failure analysis ──► Correlated gate pairs
    │                              │
    ├── Signature clustering ─► Recurring failure patterns
    │                              │
    └── ReplanEngine ◄────────── All signals combined
         │
         ├── ReplanAction::ModifyTask ──► Orchestrator adjusts task spec
         │                                before next attempt
         ├── ReplanAction::DecomposeTask ──► DAG executor creates sub-tasks
         │                                   with dependency edges
         ├── ReplanAction::ReplaceTask ──► Plan generator creates new task
         │                                 from failure context
         └── ReplanAction::Escalate ──► Stronger model / more gates / human
```

---

## 14. Meta-Learning from Verdict Patterns

Verdicts are the richest learning signal in the system. Meta-learning uses verdict
history to improve future verification decisions.

> **Citation**: Finn et al., "Model-Agnostic Meta-Learning for Fast Adaptation"
> (MAML, arXiv:1703.03400, ICML 2017).

> **Citation**: Machalica et al., "Predictive Test Selection" (ICSE-SEIP 2019) —
> Facebook's ML-based test selection achieving 2x cost reduction.

### 14.1 Predictive Gate Selection

Instead of using static complexity bands to select gates, predict which gates will
fail based on the task's features and select those gates for thorough verification:

```rust
/// Predictive gate selector using verdict history.
///
/// Features per (task, gate) pair, trained on historical verdicts:
///   - task_category: categorical (compile fix, test fix, new feature, refactor)
///   - files_modified: set of file paths
///   - model_used: which LLM model
///   - recent_gate_history: last 10 verdicts for this gate
///   - task_complexity: token count of task description
///   - gate_pass_rate: current EMA pass rate for this gate
///   - co_failure_rate: correlation with other failing gates
pub struct PredictiveGateSelector {
    /// Per-gate failure prediction models.
    /// Maps gate name → trained predictor.
    pub predictors: HashMap<String, Box<dyn FailurePredictor>>,
    /// Minimum predicted failure probability to include gate.
    pub inclusion_threshold: f64,    // default: 0.1
    /// Maximum gates to include (prevents over-verification).
    pub max_gates: usize,            // default: 7
}

pub trait FailurePredictor: Send + Sync {
    /// Predict the probability of failure for this gate given task features.
    fn predict_failure(&self, features: &TaskFeatures) -> f64;
}

pub struct TaskFeatures {
    pub category: String,
    pub files_modified: Vec<String>,
    pub model: String,
    pub task_complexity: usize,
    pub recent_gate_results: Vec<bool>,
}

impl PredictiveGateSelector {
    /// Select gates to run based on predicted failure probabilities.
    ///
    /// Strategy: always include mandatory gates (compile, test).
    /// Include optional gates only if P(failure) > threshold.
    /// Rank by P(failure) descending and take top max_gates.
    pub fn select_gates(&self, features: &TaskFeatures) -> Vec<String> {
        let mut predictions: Vec<(String, f64)> = self.predictors.iter()
            .map(|(gate, predictor)| {
                (gate.clone(), predictor.predict_failure(features))
            })
            .collect();

        // Always include mandatory gates
        let mut selected = vec!["compile".to_string(), "test".to_string()];

        // Sort optional gates by predicted failure probability (desc)
        predictions.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        for (gate, prob) in predictions {
            if selected.contains(&gate) { continue; }
            if prob >= self.inclusion_threshold && selected.len() < self.max_gates {
                selected.push(gate);
            }
        }

        selected
    }
}
```

### 14.2 Few-Shot Gate Calibration from Verdict History

When a new task arrives, find similar past tasks and use their verdict patterns to
pre-calibrate gate thresholds:

```rust
/// Few-shot gate calibration from historical verdict patterns.
///
/// For a new task, find the K nearest historical tasks and use their
/// verdict patterns to set initial thresholds and retry budgets.
pub struct VerdictPatternMemory {
    /// Historical task → verdict pattern entries.
    pub patterns: Vec<TaskVerdictPattern>,
    /// Number of nearest neighbors to use for calibration.
    pub k: usize,                    // default: 5
    /// Similarity metric for task features.
    pub similarity: Box<dyn TaskSimilarity>,
}

pub struct TaskVerdictPattern {
    pub features: TaskFeatures,
    pub gate_verdicts: Vec<(String, bool, f32)>,  // gate, passed, score
    pub outcome: bool,                             // task succeeded?
    pub steps_to_completion: u32,
    pub retries_used: u32,
    pub model: String,
}

impl VerdictPatternMemory {
    /// Calibrate gate thresholds for a new task using K nearest neighbors.
    pub fn calibrate(&self, new_task: &TaskFeatures) -> GateCalibration {
        let similar = self.k_nearest(new_task, self.k);

        // Predict which gates will likely fail
        let mut gate_failure_rates: HashMap<String, (u64, u64)> = HashMap::new();
        for pattern in &similar {
            for (gate, passed, _) in &pattern.gate_verdicts {
                let (fails, total) = gate_failure_rates
                    .entry(gate.clone()).or_insert((0, 0));
                if !passed { *fails += 1; }
                *total += 1;
            }
        }

        // Predict optimal retry count from similar tasks
        let avg_retries = similar.iter()
            .filter(|p| p.outcome) // only from successful tasks
            .map(|p| p.retries_used as f64)
            .sum::<f64>() / similar.len().max(1) as f64;

        GateCalibration {
            per_gate_failure_prediction: gate_failure_rates.into_iter()
                .map(|(gate, (fails, total))| {
                    (gate, fails as f64 / total as f64)
                })
                .collect(),
            suggested_retries: avg_retries.ceil() as u32,
            similar_task_success_rate: similar.iter()
                .filter(|p| p.outcome).count() as f64 / similar.len() as f64,
        }
    }
}

pub struct GateCalibration {
    /// Predicted failure rate per gate [0, 1].
    pub per_gate_failure_prediction: HashMap<String, f64>,
    /// Suggested retry budget from similar tasks.
    pub suggested_retries: u32,
    /// Success rate of similar historical tasks.
    pub similar_task_success_rate: f64,
}
```

---

## 15. Verdict Signal Persistence

### 15.1 Verdict Aggregation Store

```
.roko/learn/
├── verdict-trends.json         # Per-gate trend data
│   {"compile": {"trend": "Stable", "ema": 0.97, "slope": 0.001},
│    "lint": {"trend": "Degrading", "ema": 0.86, "slope": -0.012}}
├── co-failures.json            # Co-failure matrix
│   {"compile+lint": {"count": 12, "phi": 0.45}}
├── signature-clusters.json     # Recurring failure signatures
│   [{"signature": "E0425", "gate": "compile", "count": 23,
│     "trend": "Stable", "severity": 0.6}]
├── replan-log.jsonl            # Replanning decisions and outcomes
│   {"ts": ..., "task": "T5", "action": "DecomposeTask",
│    "reason": "negative progress 3 turns", "outcome": "succeeded"}
└── pattern-memory.json         # Historical verdict patterns for k-NN
    [{"features": {...}, "verdicts": [...], "outcome": true}]
```

---

## 16. Integration with Other Verification Components

| Component | How Verdict Signals Feed It |
|---|---|
| **AdaptiveThresholds** (§06) | Verdict trends adjust SPC parameters; regime shifts trigger recalibration |
| **ProcessRewardModels** (§07) | Verdict patterns → step-level labels; co-failures inform reward shaping |
| **EvoSkills** (§11) | Verdict success patterns → skill extraction; failure patterns → skill retirement |
| **GatePipeline** (§03) | Predictive gate selection uses verdict history to choose which gates to run |
| **GateRatchet** (§05) | Verdict trends inform ratchet strictness — degrading trends tighten ratchet |
| **CascadeRouter** | Verdict-model correlations drive routing; consistent failures with Model X → avoid X |
| **SystemPromptBuilder** | Verdict patterns injected as "lessons learned" section in prompts |
| **Dreams** | Verdict clusters feed Delta consolidation; recurring patterns become knowledge entries |

---

## 17. Extended Test Criteria

| Test | Property |
|---|---|
| `trend_stable_on_constant_rate` | 100 obs at 85% pass rate → VerdictTrend::Stable |
| `trend_degrading_on_declining_rate` | 50 obs at 90% then 50 at 70% → Degrading |
| `trend_regime_shift_on_sudden_change` | 100 at 95% then 100 at 50% → RegimeShift |
| `trend_volatile_on_oscillation` | Alternating pass/fail → Volatile |
| `co_failure_detects_correlation` | Compile+lint fail together 10x → phi > threshold |
| `co_failure_ignores_independent` | Compile and test fail independently → phi < threshold |
| `signature_cluster_counts_correctly` | 5 failures with same signature → cluster.count == 5 |
| `replan_on_same_signature_3x` | Same error 3 times → ReplanAction::ModifyTask |
| `replan_on_negative_progress` | Progress < -0.1 for 3 turns → DecomposeTask |
| `replan_on_low_promise` | Promise < 0.2 for 2 turns → ReplaceTask |
| `predictive_selects_high_risk_gates` | Gate with 80% predicted failure → selected |
| `predictive_skips_low_risk_gates` | Gate with 2% predicted failure → not selected |
| `knn_calibration_from_similar_tasks` | 5 similar tasks with 3 avg retries → suggest 3 |
| `verdict_signal_round_trips_through_substrate` | Write → query → correct tags and lineage |
