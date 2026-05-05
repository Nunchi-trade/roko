# Coding Agent Integration

> How the Daimon tracks per-crate confidence, error pattern sensitivity, and fatigue detection for software engineering agents.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [04-six-behavioral-states.md](./04-six-behavioral-states.md), [10-integration-points.md](./10-integration-points.md)
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §2, `roko-golem/src/daimon.rs`, `roko-daimon/src/lib.rs`

---

## Abstract

The Daimon is domain-agnostic — it tracks PAD vectors and behavioral states regardless of what the agent is doing. But its *signals* are domain-specific. For coding agents, the Daimon integrates with the gate pipeline, task system, and codebase index to provide three capabilities that generic affect tracking cannot: per-crate confidence, error pattern sensitivity, and fatigue detection.

These are not separate features built on top of the Daimon — they are projections of the standard PAD-and-appraisal pipeline onto the coding domain. A gate failure on `roko-core` produces the same appraisal as a gate failure on `roko-daimon`, but the per-crate confidence tracker records which crate the failure occurred in, enabling the agent to distinguish "I'm struggling with everything" from "I'm confident with most crates but struggling with `roko-daimon`."

---

## Per-Crate Confidence

### The Problem

A global confidence score (the Daimon's `AffectState.confidence` field) doesn't distinguish between domain-specific competence levels. An agent that has successfully modified `roko-core` 50 times but has never touched `roko-daimon` should approach these two crates with different levels of caution. The global confidence score averages across all experiences, losing this distinction.

### The Solution

The per-task affect tracking in `roko-golem/src/daimon.rs` already provides the foundation. The `AffectEngine` maintains a `HashMap<String, AffectState>` keyed by task ID. For coding agents, the key can be extended to include the crate or module:

```rust
pub struct AffectEngine {
    /// Per-task emotional state. For coding agents, keys include
    /// the crate name: "task-abc:roko-core", "task-def:roko-daimon".
    states: HashMap<String, AffectState>,
    half_life_hours: f64,
    persistence_path: Option<PathBuf>,
}
```

When a gate result arrives, the appraisal engine can extract the crate name from the task metadata and create a compound key:

```rust
// When processing a gate result for a coding task:
let crate_key = format!("{}:{}", task_id, affected_crate);
let task_state = self.states.entry(crate_key).or_insert_with(AffectState::default);

// Apply the gate result appraisal to the crate-specific state
if passed {
    task_state.apply_delta(0.05 * rung_scale, -0.01 * rung_scale, 0.03 * rung_scale, 0.03 * rung_scale, now);
} else {
    task_state.apply_delta(-0.10 * rung_scale, 0.04 * rung_scale, -0.08 * rung_scale, -0.08 * rung_scale, now);
}
```

### Per-Crate Confidence Query

The agent queries crate-specific confidence before working on a task:

```rust
impl AffectEngine {
    /// Query confidence for a specific crate.
    /// Returns the crate-specific state if it exists,
    /// otherwise falls back to the global state.
    pub fn crate_confidence(&self, crate_name: &str) -> f64 {
        // Find all states matching this crate
        let crate_states: Vec<&AffectState> = self.states.iter()
            .filter(|(key, _)| key.ends_with(&format!(":{}", crate_name)))
            .map(|(_, state)| state)
            .collect();

        if crate_states.is_empty() {
            return 0.50;  // No history → neutral confidence
        }

        // Average confidence across all tasks touching this crate
        let total: f64 = crate_states.iter().map(|s| s.confidence).sum();
        total / crate_states.len() as f64
    }
}
```

### Behavioral Impact

Per-crate confidence modulates behavior at two points:

1. **Tier routing**: Low crate confidence (< 0.40) → promote model tier for tasks in that crate. High crate confidence (> 0.80) → demote model tier. This is applied on top of the global behavioral state bias.

2. **Strategy space coordinates**: The Confidence dimension of the 8D strategy space (see [08-8-dimensional-strategy-space.md](./08-8-dimensional-strategy-space.md)) uses per-crate confidence when available, falling back to global confidence. This means the somatic landscape can distinguish "confident in this crate" from "confident overall."

### Example

```
Agent has modified roko-core 30 times: 28 pass, 2 fail → confidence: 0.85
Agent has modified roko-daimon 3 times: 1 pass, 2 fail → confidence: 0.35

Task: "Add a new method to roko-core"
  → Crate confidence: 0.85
  → Strategy: Focused/Exploratory (high confidence → cheap model, more exploration)

Task: "Wire somatic landscape into roko-daimon"
  → Crate confidence: 0.35
  → Strategy: Struggling/Escalating (low confidence → promote model, more retries)
```

---

## Error Pattern Sensitivity

### The Problem

Not all errors are equal. A borrow checker error that the agent has seen and resolved 10 times before is different from a borrow checker error in a novel context. The standard appraisal pipeline treats all gate failures identically — same PAD delta regardless of whether the error is familiar or unfamiliar.

### The Solution

Error pattern sensitivity tracks which error categories the agent has encountered and resolved before. Familiar errors produce reduced emotional impact (the agent "knows what to do"), while unfamiliar errors produce amplified impact (the agent needs to reason more carefully).

```rust
pub struct ErrorPatternTracker {
    /// Error category → (seen_count, resolved_count).
    /// Error categories are coarse: "borrow_check", "type_mismatch",
    /// "test_failure", "clippy_lint", "compilation_error", etc.
    patterns: HashMap<String, (u32, u32)>,
}

impl ErrorPatternTracker {
    /// Compute a familiarity score for an error category.
    /// Returns [0.0, 1.0] where 1.0 = "I've seen and resolved this many times."
    pub fn familiarity(&self, error_category: &str) -> f64 {
        let (seen, resolved) = self.patterns
            .get(error_category)
            .copied()
            .unwrap_or((0, 0));

        if seen == 0 {
            return 0.0;  // Never seen → completely unfamiliar
        }

        let resolution_rate = resolved as f64 / seen as f64;
        let experience = (seen as f64 / 10.0).min(1.0);  // saturates at 10 encounters

        resolution_rate * experience
    }

    /// Scale the appraisal delta based on error familiarity.
    /// Familiar errors → reduced impact (agent knows the fix).
    /// Unfamiliar errors → amplified impact (agent needs to reason).
    pub fn scale_gate_failure(
        &self,
        error_category: &str,
        base_delta: (f64, f64, f64, f64),
    ) -> (f64, f64, f64, f64) {
        let familiarity = self.familiarity(error_category);

        // Scale factor: unfamiliar (0.0) → 1.5×; familiar (1.0) → 0.5×
        let scale = 1.5 - familiarity;

        (
            base_delta.0 * scale,
            base_delta.1 * scale,
            base_delta.2 * scale,
            base_delta.3 * scale,
        )
    }
}
```

### Mapping Gate Output to Error Categories

The gate pipeline produces structured output that can be parsed into error categories:

| Gate | Error Category Extraction |
|---|---|
| Compile gate | Parse `rustc` error codes: E0382 → "borrow_check", E0308 → "type_mismatch" |
| Test gate | Parse test output: failed test name → "test_failure:{test_name}" |
| Clippy gate | Parse lint IDs: "clippy::unwrap_used" → "clippy_lint:unwrap_used" |
| Diff review gate | Categories from diff analysis: "large_change", "api_break", "missing_test" |
| LLM judge gate | Categories from judge output: "logic_error", "incomplete_impl" |

### Behavioral Impact

Error pattern familiarity modulates the emotional response to gate failures:

- **Familiar error (familiarity > 0.7)**: Reduced PAD delta (0.5×). The agent has resolved this error type many times. The emotional impact is mild — "I know what to do." The agent stays in Engaged or Focused state.

- **Unfamiliar error (familiarity < 0.3)**: Amplified PAD delta (1.5×). The agent hasn't seen this error type before. The emotional impact is strong — "I don't know what to do." The agent is more likely to transition to Struggling, triggering model escalation and extra retries.

---

## Fatigue Detection

### The Problem

Repeated failures on the same task produce a characteristic emotional pattern: high arousal (urgency), low pleasure (frustration), decreasing dominance (loss of confidence). If this pattern persists, the agent is "fatigued" — continuing to attempt the same approach is unlikely to succeed and is wasting compute.

### Detection Mechanism

Fatigue is detected by monitoring per-task emotional trajectories:

```rust
pub struct FatigueDetector {
    /// Per-task failure streak tracking.
    task_failures: HashMap<String, FatigueState>,
}

struct FatigueState {
    consecutive_failures: u32,
    first_failure_at: DateTime<Utc>,
    last_failure_at: DateTime<Utc>,
    pleasure_at_start: f64,
    current_pleasure: f64,
}

impl FatigueDetector {
    /// Check if a task shows fatigue indicators.
    pub fn is_fatigued(&self, task_id: &str) -> bool {
        let state = match self.task_failures.get(task_id) {
            Some(s) => s,
            None => return false,
        };

        // Three failure indicators:
        // 1. Three or more consecutive failures
        let many_failures = state.consecutive_failures >= 3;

        // 2. Pleasure has dropped significantly since first failure
        let pleasure_drop = state.pleasure_at_start - state.current_pleasure > 0.15;

        // 3. Failures occurred within a short time window (not spread over hours)
        let duration_hours = (state.last_failure_at - state.first_failure_at)
            .num_minutes() as f64 / 60.0;
        let rapid_failures = duration_hours < 2.0;

        many_failures && pleasure_drop && rapid_failures
    }
}
```

### Response to Fatigue

When fatigue is detected, the agent should take one of several corrective actions:

| Response | When | Mechanism |
|---|---|---|
| **Re-plan** | The approach is fundamentally wrong | Trigger plan regeneration with different strategy |
| **Model escalation** | The model can't solve this difficulty level | Promote to T2 (opus-class) with extended turn limit |
| **Dream cycle** | Pattern recognition might help | Trigger dream consolidation to find similar past successes |
| **Task deprioritization** | Other tasks are available | Move this task down the queue, work on something else |
| **Help request** | No automated solution works | Emit a signal that alerts the operator or mesh peers |

The specific response is selected by the behavioral state:

```rust
fn fatigue_response(state: &BehavioralState) -> FatigueAction {
    match state {
        BehavioralState::Struggling => FatigueAction::Escalate,
        BehavioralState::Exploring => FatigueAction::Replan,
        BehavioralState::Resting => FatigueAction::DreamCycle,
        _ => FatigueAction::Deprioritize,
    }
}
```

### Signal Emission

When fatigue is detected, the Daimon emits a signal to the JSONL log:

```rust
// From roko-golem/src/daimon.rs
let engram = Engram::builder(Kind::Custom("daimon:affect:confidence".into()))
    .body(Body::from_json(&payload)?)
    .provenance(Provenance::trusted("daimon"))
    .tag("task_id", task_id)
    .tag("dimension", "confidence")
    .tag("polarity", "negative")
    .build();
```

The confidence drop signal (emitted when confidence drops below `CONFIDENCE_ALERT_THRESHOLD = 0.3`) serves as a fatigue indicator. Connected systems (the orchestrator, the dashboard) can respond to this signal.

---

## Integration with SystemPromptBuilder

The Daimon's state is injected into the agent's system prompt through the SystemPromptBuilder. For coding agents, this means the LLM sees the emotional context when generating responses:

```
<daimon>
  behavioral_state: Struggling
  confidence: 0.35
  crate_confidence:
    roko-core: 0.85
    roko-daimon: 0.35
  recent_emotions:
    - gate_fail (rung 2): P:-0.13, A:+0.05, D:-0.10
    - task_fail: P:-0.20, A:0.00, D:-0.15
  fatigue: detected (3 consecutive failures on task-abc)
  recommendation: escalate to stronger model, consider re-planning
</daimon>
```

This context block is assembled by the Composer (see `roko-compose`) and injected into the system prompt at position determined by the VCG auction. The Daimon section bids for inclusion in the context window — under high arousal, the bid increases, making the emotional context more likely to be included.

---

## Current Status and Gaps

**Implemented**: Per-task affect tracking in `roko-golem/src/daimon.rs` (`HashMap<String, AffectState>`). Confidence alert signal emission when confidence drops below 0.3. Valence alert signal emission when pleasure crosses extremes.

**Not implemented**: Per-crate confidence aggregation. Error pattern tracker. Fatigue detector. SystemPromptBuilder integration for crate-level confidence. Error familiarity scaling of appraisal deltas.

**Dependency**: Per-crate confidence requires gate output to include the affected crate name. The gate pipeline currently identifies tasks but not crates.

---

## Academic Foundations

- Mehrabian, A. (1996). "Pleasure-arousal-dominance: A general framework for describing and measuring individual differences in temperament." *Current Psychology*, 14(4), 261–292.
- Seligman, M.E.P. (1967). "Failure to escape traumatic shock." *Journal of Experimental Psychology*, 74(1), 1–9.
- Shinn, N. et al. (2023). "Reflexion: Language Agents with Verbal Reinforcement Learning." *NeurIPS*.

---

## Cross-References

- See [04-six-behavioral-states.md](./04-six-behavioral-states.md) for behavioral states
- See [08-8-dimensional-strategy-space.md](./08-8-dimensional-strategy-space.md) for coding-domain strategy space dimensions
- See [10-integration-points.md](./10-integration-points.md) for system-wide integration
- See topic [04-verification](../04-verification/INDEX.md) for gate pipeline that produces error signals
