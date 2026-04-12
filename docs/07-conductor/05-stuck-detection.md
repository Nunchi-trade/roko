# Stuck Detection and Meta-Cognition

> Six heuristics for detecting stuck agents. A MetaCognitionHook
> that wraps them into a periodic self-assessment: "Am I stuck?
> Am I thrashing? Should I escalate?"

---

## The Stuck Problem

An agent can be stuck in ways that no single watcher catches:

- **Output loop**: The agent produces output, but it is the same
  output every turn. Tools are called, files are read, but nothing
  changes.
- **No progress**: The agent is active (producing output, calling
  tools) but no files change and no tests pass. Activity without
  progress.
- **Gate loop**: The agent fixes one error, introduces another, fixes
  that, reintroduces the first. The gate failure count oscillates
  but never reaches zero.
- **Compile loop**: A variation of gate loop specific to compile
  errors. The agent toggles between two incompatible fixes.
- **Empty output**: The agent returns content, but it is
  acknowledgments, descriptions of intended actions, or questions
  to nobody. No tool calls, no file changes.
- **Excessive retries**: The agent retries the same operation
  repeatedly without changing its approach.

These modes are all variations on the same theme: the agent is
consuming resources (tokens, wall-clock time, API quota) without
making progress toward task completion.

---

## StuckKind Enum

```rust
pub enum StuckKind {
    OutputLoop,
    NoProgress,
    GateLoop,
    CompileLoop,
    EmptyOutput,
    ExcessiveRetries,
}
```

Each variant represents a distinct detection heuristic. Multiple
variants can be detected simultaneously (an agent can be both in
an output loop and showing no progress).

---

## StuckDetector

```rust
pub struct StuckDetector {
    thresholds: StuckThresholds,
}

pub struct StuckThresholds {
    pub output_loop: usize,         // default: 4
    pub no_progress_ms: u64,        // default: 300_000 (5 minutes)
    pub gate_loop: usize,           // default: 3
    pub compile_loop: usize,        // default: 3
    pub empty_output: usize,        // default: 3
    pub excessive_retry: usize,     // default: 6
}
```

### Detection Heuristics

#### OutputLoop (threshold: 4)

Computes a content hash of each agent turn's output. If four
consecutive turns produce the same hash, the agent is stuck in an
output loop.

**Why hash-based**: Exact string comparison would miss near-identical
outputs (same content with minor formatting differences). Hashing
normalizes the comparison. In practice, true output loops produce
byte-identical output because the agent is executing the same
reasoning chain.

**Why 4**: One repeated output is common (agent checks something
twice). Two repetitions may indicate deliberate verification. Three
is suspicious. Four consecutive identical outputs is definitively
a loop.

#### NoProgress (threshold: 300,000 ms / 5 minutes)

Checks elapsed time since the last file modification or test state
change. If 5 minutes pass with no measurable progress, the agent is
stuck.

**Why time-based**: Unlike the other heuristics which count events,
no-progress detection is time-based because the agent may not produce
any events to count. A truly stuck agent might be in a reasoning loop
with no tool calls at all — no output, no file changes, nothing to
count.

**Why 5 minutes**: Based on production timing data. Normal implementation
tasks show file changes every 30-120 seconds. A 5-minute gap is
5-10x the normal interval, indicating the agent has stalled.

#### GateLoop (threshold: 3)

Tracks gate failure patterns per plan. If the agent's gate results
oscillate (fail, different fail, original fail) without making net
progress toward passing, it is in a gate loop.

**Differs from iteration-loop watcher**: The iteration-loop watcher
counts consecutive gate failures. The gate loop detector looks for
oscillation patterns — the agent might "fix" one error only to
reintroduce a previous one. The failure count stays the same but the
failures cycle.

#### CompileLoop (threshold: 3)

A specialized gate loop detector for compile errors. Tracks compile
error fingerprints across iterations. If the same set of errors
reappears after the agent attempted a fix, it is in a compile loop.

**Differs from compile-fail-repeat watcher**: The compile-fail-repeat
watcher detects identical errors across consecutive gates. The compile
loop detector detects cycling errors — error A appears, agent fixes A
but introduces B, agent fixes B but reintroduces A. The watcher sees
A, then B, then A (not repeated) — but the loop detector recognizes
the cycle.

#### EmptyOutput (threshold: 3)

Counts consecutive turns where the agent produces no tool calls and
no file changes. Three such turns in a row indicate the agent is
producing text (acknowledgments, descriptions, questions) but not
taking action.

**Why this is different from ghost turns**: Ghost turns are turns
with zero output — the model returned immediately with nothing.
Empty output turns have content (potentially verbose content) but
no actions. The agent is "thinking out loud" without doing anything.

#### ExcessiveRetries (threshold: 6)

Counts retry attempts for the same operation. If an agent retries
the same tool call (e.g., `cargo check`) six times without changing
its approach, it is in a retry loop.

---

## MetaCognitionHook

The `MetaCognitionHook` wraps the `StuckDetector` into a periodic
self-assessment mechanism that operates at Theta frequency:

```rust
pub struct MetaCognitionHook {
    detector: StuckDetector,
    frequency: OperatingFrequency,  // Theta
}
```

### Operating Frequency

The hook operates at Theta frequency — medium-rate periodic
assessment. In Roko's operating frequency model:

| Frequency | Rate | Purpose |
|-----------|------|---------|
| Gamma | High (every turn) | Real-time tool dispatch, safety checks |
| Theta | Medium (periodic) | Self-assessment, meta-cognition |
| Delta | Low (between sessions) | Consolidation, pattern extraction |

Theta frequency means the meta-cognition check runs periodically —
not on every turn (too expensive) but often enough to catch stuck
agents before they burn significant budget.

### Assessment Output

```rust
pub enum MetaCognitionAction {
    Continue,
    AdjustStrategy,
    Escalate,
}

pub struct MetaCognitionAssessment {
    pub frequency: OperatingFrequency,
    pub action: MetaCognitionAction,
    pub reason: String,
    pub stuck_kinds: Vec<StuckKind>,
}
```

The assessment maps stuck kinds to meta-cognition actions:

| Stuck Kind | MetaCognition Action | Rationale |
|-----------|---------------------|-----------|
| OutputLoop | AdjustStrategy | Agent needs a different approach |
| NoProgress | AdjustStrategy | Agent is stalled; refocus |
| GateLoop | Escalate | Cycling indicates fundamental problem |
| CompileLoop | Escalate | Cycling indicates architectural mismatch |
| EmptyOutput | AdjustStrategy | Agent needs more directive prompting |
| ExcessiveRetries | AdjustStrategy | Different operation or tool needed |

`AdjustStrategy` maps to a Conductor Restart (fresh agent, different
context). `Escalate` maps to a Conductor Fail (the problem is beyond
what a single-agent retry can solve).

### Signal Serialization

The `MetaCognitionAssessment` is serializable and can be emitted as
a Signal:

```rust
impl MetaCognitionAssessment {
    pub fn to_signal(&self) -> Signal {
        Signal::builder(Kind::Custom("conductor.meta_cognition".into()))
            .body(Body::from_json(self).expect("serialize assessment"))
            .tag("frequency", self.frequency.as_str())
            .tag("action", self.action.as_str())
            .tag("reason", &self.reason)
            .build()
    }
}
```

These signals feed into the Conductor's signal stream, where other
watchers or the intervention policy can incorporate them into the
overall decision.

---

## The Self-Model Requirement

The meta-cognition hook implements a principle from the Good Regulator
Theorem (Conant & Ashby, 1970):

> "Every good regulator of a system must be a model of that system."

The stuck detector is Roko's self-model — its representation of what
"healthy execution" looks like. By defining six specific stuck kinds,
the system models six ways execution can deviate from health. The
meta-cognition hook asks: "Does my current behavior match my model
of healthy execution?"

This self-model is necessarily incomplete. There are stuck modes that
the six heuristics will not catch. But the model improves over time:
each production failure that is not caught by the existing heuristics
becomes a candidate for a new stuck kind. The detection system grows
as the system's self-knowledge grows.

Ashby's Law of Requisite Variety (Ashby, 1956) constrains this growth:
the detector must have at least as many distinguishable states as the
execution system has pathological states. With six heuristics, the
detector can distinguish six stuck modes. If the execution system
can be stuck in seven distinct ways, the detector has insufficient
variety and will miss one.

---

## Threshold Tuning

The default thresholds balance sensitivity against false positives:

| Threshold | Default | Too Low → | Too High → |
|-----------|---------|-----------|-----------|
| output_loop | 4 | False positives on verification loops | Late detection (tokens wasted) |
| no_progress_ms | 300,000 | Kills slow-but-progressing agents | Agent stalls for 10+ minutes |
| gate_loop | 3 | Normal retry cycles flagged | Agent oscillates for 5+ cycles |
| compile_loop | 3 | Normal fix attempts flagged | Agent toggles errors for 5+ cycles |
| empty_output | 3 | Kills agents that are thinking | Agent describes instead of acting |
| excessive_retry | 6 | Normal retries flagged | Agent retries 10+ times |

The `StuckThresholds` struct accepts custom values through the
constructor, enabling per-deployment tuning. The learning system's
efficiency data provides the signal for tuning: if a threshold
consistently triggers without leading to recovery (the restarted
agent gets stuck the same way), the threshold should be lower. If
a threshold triggers and the restart succeeds, the threshold is
correctly calibrated.

---

## Relationship to Watcher Ensemble

The stuck detector and the watcher ensemble have overlapping but
distinct responsibilities:

| Detection | Stuck Detector | Watcher Ensemble |
|-----------|---------------|-----------------|
| Identical compile errors | CompileLoop heuristic | compile-fail-repeat watcher |
| Zero output | EmptyOutput heuristic | ghost-turn watcher |
| No file changes | NoProgress heuristic | (not directly covered) |
| Identical actions | OutputLoop heuristic | stuck-pattern watcher |
| Gate failure cycling | GateLoop heuristic | iteration-loop watcher |
| Cost overrun | (not covered) | cost-overrun watcher |
| Context pressure | (not covered) | context-window-pressure watcher |
| Review cycling | (not covered) | review-loop watcher |
| Spec drift | (not covered) | spec-drift watcher |
| Time overrun | (not covered) | time-overrun watcher |

The overlaps are intentional — the stuck detector provides a
complementary detection mechanism with different thresholds and
detection logic. The watcher ensemble operates on the signal stream
(structured data). The stuck detector can operate on raw agent output
(unstructured data). Both feed into the Conductor's decision process.

---

## File Reference

| File | What |
|------|------|
| `crates/roko-conductor/src/stuck_detection.rs` | StuckDetector, StuckKind, StuckThresholds, MetaCognitionHook, MetaCognitionAssessment |
