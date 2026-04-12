# 03 — The Gate Pipeline

> **Layer**: L3 Harness — Verification
> **Crate**: `roko-gate` (`crates/roko-gate/src/gate_pipeline.rs`)
> **Status**: Implemented (593 lines), wired into orchestrate.rs


> **Implementation**: Shipping

---

## 1. Overview

The `GatePipeline` composes multiple gates into a single verification step. It accepts a
`Vec<Box<dyn Gate>>`, runs them sequentially, and produces an aggregated `Verdict`. It
implements the `Gate` trait itself, so a pipeline can be nested inside another pipeline
or used anywhere a single gate is expected.

This is the primary mechanism for turning the rung selector's output (a list of rungs)
into a concrete verification execution.

> **Citation**: crates/roko-gate/src/gate_pipeline.rs — Full implementation.

---

## 2. Structure

```rust
pub struct GatePipeline {
    gates: Vec<Box<dyn Gate>>,
    short_circuit: bool,
    name: String,
}
```

| Field | Purpose |
|---|---|
| `gates` | Ordered list of gates to execute |
| `short_circuit` | If true, stop on first failure |
| `name` | Display name for the pipeline's own verdict |

### Construction

```rust
GatePipeline::new(vec![
    Box::new(CompileGate::cargo()),
    Box::new(ClippyGate::cargo()),
    Box::new(TestGate::cargo()),
])
.with_short_circuit(true)
.with_name("rung-pipeline")
```

The builder pattern follows the same convention as individual gates: `with_*` methods
return `Self` for chaining.

---

## 3. Short-Circuit vs. Full Execution

### 3.1 Short-Circuit Mode (`short_circuit: true`)

The default and most common mode. The pipeline stops at the first gate that fails and
returns a failure verdict immediately. This is the correct behavior for the rung
pipeline: if compile fails, there is no point running lint or tests.

```
CompileGate → FAIL → stop → return Verdict::fail(...)
              (ClippyGate and TestGate never run)
```

**Why this matters**: In a 7-rung pipeline where integration tests take 30 minutes, a
compile failure caught in 3 seconds saves 30+ minutes of wasted compute. Short-circuit
mode is the mechanism that makes the verification-first architecture efficient.

### 3.2 Full Execution Mode (`short_circuit: false`)

All gates run regardless of individual outcomes. The final verdict is a failure if *any*
gate failed. This mode is useful when you want a comprehensive report of all issues
rather than just the first.

```
CompileGate → FAIL → continue
ClippyGate  → PASS → continue
TestGate    → FAIL → continue
→ return aggregated Verdict::fail(...)
  (detail includes output from all three gates)
```

---

## 4. Verdict Aggregation

The pipeline's `verify()` method aggregates individual verdicts into a single
pipeline-level verdict:

### 4.1 Pass Condition

The pipeline passes if and only if **every** gate passes. A single failure anywhere in
the chain makes the whole pipeline fail.

### 4.2 Detail Aggregation

Individual gate outputs are concatenated into the pipeline's `detail` field, separated
by headers:

```
--- [compile:cargo] ---
Compiling foo v0.1.0
Finished dev in 2.3s

--- [clippy:cargo] ---
warning: unused variable

--- [test:cargo] ---
test result: ok. 12 passed; 0 failed; 0 ignored
```

This gives the caller (and the agent) a complete view of what happened at each
verification step.

### 4.3 Reason Construction

On failure, the pipeline's `reason` field lists which gate(s) failed:

```
gate pipeline failed: compile:cargo (error: bad thing; error[E0425]: symbol not found)
```

In short-circuit mode, there is exactly one failed gate. In full-execution mode, there
may be multiple.

### 4.4 Duration

The pipeline's `duration_ms` is the sum of all individual gate durations. This tracks
total wall-clock time spent on verification.

### 4.5 Test Count Merging

If any gate in the pipeline produces `TestCount` (test gates do), the pipeline merges
them by summing passed, failed, and ignored counts across all gates. This is relevant
when a pipeline contains multiple test gates (e.g., unit tests + integration tests).

---

## 5. The Pipeline as a Gate

`GatePipeline` implements `Gate`:

```rust
#[async_trait]
impl Gate for GatePipeline {
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict {
        // ... iterate over self.gates, aggregate verdicts
    }

    fn name(&self) -> &str {
        &self.name
    }
}
```

This composability is intentional. It means:
- A pipeline can contain other pipelines (nesting)
- Any code that accepts `&dyn Gate` can accept a pipeline
- The adaptive threshold system, ratchet, and feedback systems all work with pipelines
  without special-casing

---

## 6. Execution Flow

```
Pipeline::verify(signal, ctx)
│
├─ gate[0].verify(signal, ctx) → verdict_0
│  ├─ if failed && short_circuit → return fail verdict
│  └─ collect detail, test counts
│
├─ gate[1].verify(signal, ctx) → verdict_1
│  ├─ if failed && short_circuit → return fail verdict
│  └─ collect detail, test counts
│
├─ ... (for each gate)
│
└─ aggregate:
   ├─ passed = all verdicts passed
   ├─ reason = join failure reasons
   ├─ detail = join all details with headers
   ├─ duration = sum of durations
   ├─ test_count = sum of test counts
   └─ return aggregated Verdict
```

---

## 7. How the Orchestrator Uses the Pipeline

In `crates/roko-cli/src/orchestrate.rs`, the orchestrator constructs a pipeline per task:

```
1. Determine plan complexity (Trivial/Simple/Standard/Complex)
2. Detect environment capabilities (which build tools exist)
3. select_rungs(complexity, caps, prior_failures)
4. Map each Rung to a concrete Box<dyn Gate>
5. GatePipeline::new(gates).with_short_circuit(true)
6. pipeline.verify(signal, ctx)
7. Feed verdict to ratchet, thresholds, feedback
```

The pipeline is constructed fresh for each task execution. This means:
- Different tasks can have different pipelines (based on complexity)
- Escalation adds gates to the pipeline on retry
- The pipeline is lightweight — no persistent state

---

## 8. Error Handling Within the Pipeline

Because the Gate trait returns `Verdict` (not `Result<Verdict>`), the pipeline never
has to handle gate errors. Every gate handles its own infrastructure failures internally.
The pipeline simply collects verdicts and aggregates them.

This is the practical benefit of the `-> Verdict` design decision: composition is
trivial. There are no error propagation paths to worry about, no `?` operators, no
`Result::map` chains. Just: run the gate, get a verdict, check if it passed.

> **Citation**: 00-gate-trait.md — "Gate failure is not an error — it is a verdict."

---

## 9. Pipeline Lifecycle in the Universal Loop

```
Universal loop: query → score → route → compose → act → VERIFY → write → react
                                                        ^^^^^^^^
                                                   GatePipeline lives here

Signal produced by agent (act step)
    ↓
GatePipeline.verify(signal, ctx)
    ↓
Verdict flows to:
    ├─ write: Verdict persisted as signal in Substrate
    ├─ react: GateRatchet.record_pass(plan_id, rung)
    ├─ react: AdaptiveThresholds.update(rung, passed)
    ├─ react: GateFeedback for agent context on retry
    ├─ react: EfficiencyEvent for learning
    └─ react: CascadeRouter.update_arm(model, reward)
```

The pipeline is the single point where all these downstream systems get their input.
This centralization means there's one place to add new feedback consumers, one place
to add instrumentation, and one place to add logging.

> **Citation**: refactoring-prd/01-synapse-architecture.md — Cybernetic feedback loops
> from Gate to Scorer, Router, Composer.

---

## 10. Testing the Pipeline

The pipeline has extensive tests in `gate_pipeline.rs`:

| Test | What It Verifies |
|---|---|
| `pipeline_empty_passes` | Empty pipeline returns pass verdict |
| `pipeline_single_pass` | Single passing gate → pass |
| `pipeline_single_fail` | Single failing gate → fail |
| `pipeline_short_circuits` | Stops at first failure |
| `pipeline_full_execution` | Runs all gates when short_circuit=false |
| `pipeline_aggregates_test_counts` | Merges test counts across gates |
| `pipeline_detail_headers` | Detail output has per-gate headers |
| `pipeline_duration_sums` | Total duration = sum of gate durations |

These tests use mock gates that return predetermined verdicts, avoiding the need for
actual subprocess spawning in unit tests.

---

## 11. Relationship to Other Components

| Component | Relationship |
|---|---|
| `RungSelector` | Determines which gates go into the pipeline |
| `GateRatchet` | Consumes pipeline verdicts to track regression |
| `AdaptiveThresholds` | Consumes pipeline verdicts for per-rung EMA |
| `GateFeedback` | Parses pipeline detail output for agent context |
| `ArtifactStore` | Future: stores pipeline artifacts content-addressed |
| Orchestrator | Constructs and executes the pipeline per task |

> **Citation**: crates/roko-gate/src/gate_pipeline.rs — Tests demonstrating pipeline
> behavior.

---

## 12. Design Rationale: Sequential, Not Parallel

The current pipeline executes gates sequentially. This is deliberate:

1. **Dependency ordering**: Rung N often depends on Rung N-1's success. Running tests
   on code that doesn't compile wastes time and produces confusing errors.
2. **Short-circuit value**: Sequential execution enables short-circuit, which is the
   pipeline's primary optimization.
3. **Simplicity**: Sequential execution has no synchronization concerns.

Future: A "gate group" concept where independent gates within the same rung run in
parallel. For example, if a rung has both a symbol gate and a format gate (both zero
cost, no dependency between them), they could run concurrently. This would be a new
composition primitive, not a change to the pipeline's sequential semantics.
