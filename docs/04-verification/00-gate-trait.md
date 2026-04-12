# 00 — The Gate Trait

> **Layer**: L3 Harness — Verification
> **Crate**: `roko-core` (`crates/roko-core/src/traits.rs`)
> **Status**: Stable, implemented

---

## 1. Overview

The `Gate` trait is one of six foundational traits in Roko's Synapse Architecture. It
occupies a unique position: while Substrate stores, Scorer evaluates, Router selects,
Composer assembles, and Policy governs — **Gate verifies**. It is the system's sole
mechanism for establishing ground truth about agent-produced artifacts.

Gates answer one question: *did the agent's output meet the verifiable criteria for
this verification step?* The answer is always a `Verdict` — never an error, never a
maybe, never a "could not determine." This design is deliberate and load-bearing.

> **Citation**: Synapse Architecture (refactoring-prd/01-synapse-architecture.md) — "1
> noun (Signal) + 6 verb traits" composing the universal loop.

---

## 2. The Trait Signature

```rust
// crates/roko-core/src/traits.rs, lines 102–108

pub trait Gate: Send + Sync {
    /// Verify the signal and return a verdict.
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict;

    /// Human-readable name (appears in verdicts).
    fn name(&self) -> &str;
}
```

Two methods. That's the entire surface area.

### 2.1 `verify()` Returns `Verdict`, Not `Result<Verdict>`

This is the single most important design decision in the Gate trait and it requires
emphasis because it diverges from the idiomatic Rust pattern of returning `Result` from
fallible operations.

**Gate failure is not an error — it is a verdict.**

When `cargo check` reports a compilation error, that is not an infrastructure failure.
It is meaningful information: the code does not compile. The gate's job is to encode
that outcome into a `Verdict::fail()` with a reason string and optional error digest.
The caller never has to handle two failure paths (infrastructure error vs. verification
failure); there is only one path: the verdict.

This means:
- A gate that cannot spawn its subprocess returns `Verdict::fail("spawn failed: ...")`
- A gate that times out returns `Verdict::fail("timed out after N ms")`
- A gate whose input signal has malformed JSON returns `Verdict::fail("signal body is not a GatePayload: ...")`
- A gate whose test suite has 3 failures returns `Verdict::fail("test foo::bar ... FAILED; ...")`

All four are verdicts. The pipeline, ratchet, adaptive thresholds, and feedback systems
never need to pattern-match on `Result::Err`. They receive a `Verdict` and act on it.

> **Citation**: refactoring-prd/01-synapse-architecture.md — Gate trait signature:
> `async fn verify(&self, engram: &Engram, ctx: &Context) -> Verdict`. The canonical
> source explicitly uses `-> Verdict`, not `-> Result<Verdict>`.

> **Citation**: crates/roko-core/src/traits.rs:102–108 — Active implementation
> confirms the `-> Verdict` return type.

### 2.2 Why `async`

Gates shell out to external tools: compilers, test runners, linters, static analyzers.
These are I/O-bound operations measured in seconds to minutes. Making `verify()` async
allows the gate pipeline to:

1. Run multiple independent gates concurrently (future: parallel gate groups)
2. Apply timeouts via `tokio::time::timeout` without blocking the executor
3. Compose with the rest of Roko's async runtime (agent dispatch, signal persistence)

Every concrete gate implementation uses `#[async_trait]` from the `async-trait` crate
to satisfy the trait's async method.

### 2.3 Why `Send + Sync`

Gates are composed into pipelines (`GatePipeline`) that may be shared across tasks in
the plan executor. The `Send + Sync` bounds ensure gates can be stored in
`Vec<Box<dyn Gate>>` and passed between threads safely.

---

## 3. The `name()` Method

```rust
fn name(&self) -> &str;
```

Every verdict carries the name of the gate that produced it. This serves three purposes:

1. **Traceability**: When a verdict flows into the feedback system, the agent (and the
   human operator) can see *which* gate failed. "compile:cargo" vs. "test:cargo" vs.
   "clippy:cargo" vs. "diff" vs. "symbol" — these are distinct verification steps with
   distinct remediation paths.

2. **Ratcheting**: The `GateRatchet` tracks the highest rung passed per plan. Gate
   names associate verdicts with rungs.

3. **Adaptive thresholds**: The `AdaptiveThresholds` system tracks per-rung pass rates.
   Gate names allow the threshold system to correlate verdicts with rungs for EMA
   updates.

Naming convention in the codebase follows `category:tool` format:
- `compile:cargo`, `compile:npm`, `compile:go`
- `test:cargo`, `test:npm`, `test:go`
- `clippy:cargo` (lint gates)
- `shell:true`, `shell:custom_script`
- `diff`, `symbol` (standalone names)

---

## 4. The `Verdict` Type

While `Verdict` is defined in `roko-core` (not in the gate crate), understanding it is
essential to understanding gates. A `Verdict` carries:

| Field | Type | Purpose |
|---|---|---|
| `passed` | `bool` | Did the gate pass? |
| `gate` | `String` | Name of the gate that produced this verdict |
| `reason` | `String` | Human-readable explanation (especially on failure) |
| `detail` | `Option<String>` | Full output (stdout + stderr) for debugging |
| `error_digest` | `Option<String>` | Machine-parseable error summary |
| `duration_ms` | `u64` | Wall-clock time the gate took |
| `test_count` | `Option<TestCount>` | Parsed test counts (passed/failed/ignored) |

Key construction patterns used across all gate implementations:

```rust
// Passing verdict
Verdict::pass(&self.name)
    .with_detail(combined_output)
    .with_duration(elapsed_ms)

// Failing verdict with reason
Verdict::fail(&self.name, reason_string)
    .with_detail(combined_output)
    .with_duration(elapsed_ms)

// Failing verdict with machine-parseable digest
Verdict::fail(&self.name, "3 symbol expectations unmet")
    .with_error_digest(digest)
    .with_detail(digest.clone())
    .with_duration(elapsed_ms)

// Test gate attaches parsed counts
verdict.with_test_count(TestCount::new(passed, failed, ignored))
```

> **Citation**: crates/roko-gate/src/compile.rs:113–122 — CompileGate demonstrates
> the standard pass/fail verdict construction pattern.

---

## 5. The Gate Contract

Every gate implementation must satisfy these invariants:

### 5.1 Total Function

`verify()` must always return a `Verdict`. It must not panic, it must not return `Err`,
and it must not hang indefinitely. Every gate in the codebase enforces a timeout
(typically via `tokio::time::timeout`) and converts timeout expiration into
`Verdict::fail()`.

Timeouts by gate:
- `CompileGate`: 10 minutes (600,000 ms)
- `TestGate`: 15 minutes (900,000 ms)
- `ClippyGate`: 5 minutes (300,000 ms)
- `ShellGate`: 5 minutes (300,000 ms)

### 5.2 Deterministic on Identical Inputs

Given the same signal body and filesystem state, a gate should produce the same verdict.
Gates do not use randomness. They do not consult LLMs (except the future
`LlmJudgeGate`, which has its own reproducibility constraints).

### 5.3 Side-Effect Free (on the source)

Gates read the filesystem and run subprocesses, but they must not modify the source code
being verified. A gate that rewrites the code to make it pass would defeat its purpose.
Gates may produce artifacts (build caches, test output files), but these go into
`CARGO_TARGET_DIR` or the `ArtifactStore`, not into the source tree.

### 5.4 Duration Tracking

Every verdict must carry `duration_ms`. This feeds the adaptive threshold system's retry
budget calculations and the efficiency event logger's per-gate timing data.

> **Citation**: crates/roko-gate/src/shell.rs:57–118 — ShellGate's `verify()`
> demonstrates all contract properties: timeout handling, duration tracking, infallible
> return.

---

## 6. How Gates Differ from Scorers

Both Gates and Scorers evaluate signals, but they serve different purposes:

| Dimension | Gate | Scorer |
|---|---|---|
| Output | `Verdict` (pass/fail + metadata) | `Score` (numeric, 0.0–1.0) |
| Truth source | External tool (compiler, test runner) | Internal heuristic or model |
| Determinism | Deterministic (same code → same result) | May be probabilistic |
| Cost | High (subprocess spawn, minutes) | Low (computation, milliseconds) |
| Role in loop | Verification (L3 Harness) | Evaluation (L2 Engine) |

A Scorer might estimate "this code looks 80% correct." A Gate runs the compiler and
says definitively "this code compiles" or "this code does not compile." The Scorer's
estimate is useful for routing decisions; the Gate's verdict is ground truth.

> **Citation**: refactoring-prd/02-five-layers.md — Layer 2 (Engine) vs. Layer 3
> (Harness): scoring vs. verification distinction.

---

## 7. Position in the Universal Loop

The universal loop in Roko is: **query → score → route → compose → act → verify →
write → react**. Gates occupy the **verify** step.

```
Agent produces output (act)
    ↓
Gate pipeline verifies output (verify)
    ↓
Verdict flows to:
    → Substrate (write: persist verdict as signal)
    → Scorer (react: update scoring model)
    → Router (react: update bandit arms)
    → Composer (react: adjust prompt sections)
    → Conductor (react: circuit breaker, watchers)
    → Ratchet (react: track highest rung passed)
    → Adaptive thresholds (react: update per-rung EMA)
    → Agent feedback (react: filter output for agent context)
```

The gate verdict is the system's primary feedback signal. It drives six different
learning and adaptation mechanisms. This is why the gate returning `Verdict` directly
(not `Result<Verdict>`) matters so much — every downstream consumer expects a definitive
answer, and wrapping it in `Result` would mean every consumer needs error-handling code
for a case that should never arise.

> **Citation**: refactoring-prd/01-synapse-architecture.md — Universal loop:
> query→score→route→compose→act→verify→write→react. Cybernetic feedback loops from
> Gate to Scorer, Router, Composer.

---

## 8. Relationship to the GVU Framework

The Generation-Verification-Update (GVU) framework from Song et al. (ICLR 2025) proves
a critical theorem: **self-improvement succeeds when the verifier is strong, not when
the generator is strong**. Specifically, the Variance Inequality shows that an oracle
verifier (verification noise σ_V ≈ 0) enables improvement despite arbitrarily high
generation noise.

Roko's gate pipeline is, in GVU terms, the verifier. Compilers and test suites are
oracle verifiers for their respective properties — they have zero false positive rate
for the properties they check. This is why Roko invests in a rich gate ecosystem (11
gates, 7 rungs) rather than relying solely on better prompts: **the returns to stronger
verification compound, while the returns to stronger generation plateau**.

> **Citation**: Song et al. "Self-Improving Diffusion Models with Synthetic Data" (ICLR
> 2025) — Generation-Verification-Update framework, Variance Inequality theorem.

> **Citation**: refactoring-prd/09-innovations.md — Innovation VIII: Process Reward
> Models informed by GVU framework's verification-first insight.

---

## 9. Future: The Gate Trait in Multi-Language Contexts

The current Gate trait signature accepts `Signal` (which the canonical architecture
names `Engram`). The signal's body carries a `GatePayload` with `BuildSystem` (Cargo,
Npm, Go, Make). This means the Gate trait itself is language-agnostic — only the
concrete implementations (`CompileGate`, `TestGate`, `ClippyGate`) know about specific
build systems.

Adding support for a new language requires:
1. Adding a `BuildSystem` variant (e.g., `BuildSystem::Gradle`)
2. Implementing `check_args()`, `test_args()`, `lint_args()` for the new variant
3. Optionally: a new `parse_*_test_counts()` function for the test runner's output format

No changes to the Gate trait are needed.

> **Citation**: crates/roko-gate/src/test_gate.rs:171–179 — `parse_test_counts()`
> dispatches by `BuildSystem`, demonstrating language-agnostic gate design.

---

## 10. Summary

The Gate trait is deliberately minimal: two methods, no error return type, no generic
parameters. This simplicity is a feature. It means any verification step — from a 5ms
regex check to a 15-minute integration test suite — can be expressed as a Gate, composed
into a pipeline, tracked by a ratchet, measured by adaptive thresholds, and fed back to
agents and learning systems through a uniform interface.

The single most important thing to remember about the Gate trait:

**`verify()` returns `Verdict`, not `Result<Verdict>`. Gate failure is a verdict, not an error.**
