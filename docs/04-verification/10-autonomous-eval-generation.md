# 10 — Autonomous Evaluation Generation

> **Layer**: L3 Harness — Verification
> **Crates**: `roko-gate` (generated_test_gate, property_test_gate), `roko-agent`
> **Status**: Scaffold (gate implementations exist, generation pipeline designed)

---

## 1. Overview

Autonomous evaluation generation is the system's ability to create its own verification
criteria — test cases, property assertions, invariant checks — without human
intervention. Rather than relying solely on hand-written tests, the system generates
targeted tests for each task, creating verification that specifically exercises the
agent's output.

This closes a critical gap: hand-written tests verify what the human thought to test.
Generated tests verify what the *agent actually changed*. The two are complementary, and
the generated tests often catch issues that hand-written tests miss.

> **Citation**: bardo-backup/tmp/agent-chain/17-autonomous-eval-generation.md —
> Autonomous evaluation generation architecture.

> **Citation**: bardo-backup/tmp/mori-agents/20-verification-first-architecture.md —
> "Separate test generation from implementation, tests must fail before implementation."

---

## 2. The Verification Generation Pipeline

The pipeline has three stages, executed before the implementation agent starts:

### Stage 1: Test Generation

A dedicated test-generation agent (not the implementation agent) reads the task
specification and generates test cases. These tests are:
- Targeted at the specific code being changed
- Written to fail before the implementation (red-green-refactor)
- Stored as immutable artifacts in the ArtifactStore

### Stage 2: Test Validation

The generated tests are compiled and run against the *current* codebase (before the
agent's changes). Expected behavior:
- New tests for new functionality: should fail (the functionality doesn't exist yet)
- Tests for existing functionality being modified: should pass (baseline)
- Tests that don't compile: rejected

This validation step ensures the generated tests are meaningful: they test something
that doesn't exist yet (or something that should change).

### Stage 3: Test Registration

Validated tests are registered with the `GeneratedTestGate` (Rung 4). When the
implementation agent produces its code, Rung 4 runs these tests against the new code.
If the implementation is correct, the tests should now pass.

```
Before implementation:
  Generated tests → FAIL (expected: feature doesn't exist)

After implementation:
  Generated tests → PASS (expected: feature now works)
```

This is the classic test-driven development cycle, but automated: the system generates
the tests, the agent generates the implementation, and the gate verifies alignment.

---

## 3. Test Generation Strategies

### 3.1 Example-Based Test Generation

The test agent generates concrete example tests with specific inputs and expected
outputs:

```rust
#[test]
fn rate_limiter_allows_within_limit() {
    let limiter = RateLimiter::new(100, Duration::from_secs(60));
    assert!(limiter.check("client-1").is_ok());
}

#[test]
fn rate_limiter_rejects_over_limit() {
    let limiter = RateLimiter::new(1, Duration::from_secs(60));
    limiter.check("client-1").unwrap();
    assert!(limiter.check("client-1").is_err());
}
```

These are the most common generated tests. They are easy to generate, easy to
understand, and have clear pass/fail semantics.

### 3.2 Property-Based Test Generation

For tasks where properties are more important than specific examples, the system
generates property tests:

```rust
#[proptest]
fn rate_limiter_never_allows_over_limit(limit in 1..100u32, requests in 1..200u32) {
    let limiter = RateLimiter::new(limit, Duration::from_secs(60));
    let mut allowed = 0;
    for _ in 0..requests {
        if limiter.check("client").is_ok() {
            allowed += 1;
        }
    }
    prop_assert!(allowed <= limit);
}
```

Property tests exercise a wider input space than example tests. They are more expensive
to run but catch edge cases that example tests miss.

### 3.3 Invariant Generation

The system identifies invariants that should hold before and after the agent's changes:

```rust
// Pre-condition: these tests pass before the agent's changes
// Post-condition: these tests still pass after the agent's changes
// (i.e., the agent didn't break existing functionality)

#[test]
fn existing_auth_still_works() {
    let auth = AuthService::new();
    assert!(auth.validate_token("valid-token").is_ok());
}
```

Invariant tests protect against regressions — they verify that the agent's changes
don't break things that were previously working.

---

## 4. The Key Architectural Decision: Separation

**Test generation and implementation are performed by different agents.**

This separation is critical. If the implementation agent generates its own tests, it
will generate tests that pass for its implementation — not tests that verify
correctness. The implementation agent is incentivized to make tests easy to pass. The
test generation agent is incentivized to make tests hard to pass (thorough).

This adversarial relationship improves verification quality:
- Test agent: "Here are the hardest tests I can think of for this task"
- Implementation agent: "Here is code that passes all those tests"
- Gate: "Confirmed — the code passes the tests"

> **Citation**: bardo-backup/tmp/mori-agents/20-verification-first-architecture.md —
> "Separate test generation from implementation" as a key architectural decision.

---

## 5. The Generation-Verification Gap

Song et al. (ICLR 2025) formalize the **Generation-Verification Gap**: self-improvement
works only when verification capability exceeds generation capability. In the context
of autonomous eval generation:

- If the test generator produces tests that are easier than the implementation task,
  generated tests add no value (the implementation passes trivially)
- If the test generator produces tests that are harder than the implementation task,
  generated tests provide strong verification signal

The system needs to ensure that test generation is at least as sophisticated as
implementation. This is achieved by:
1. Using a capable model for test generation (not a cheap model)
2. Including edge cases, error conditions, and adversarial inputs in generated tests
3. Validating that generated tests actually fail before implementation

> **Citation**: Song et al. (ICLR 2025) — "Self-improvement works only when the
> verifier is strong, not when the generator is strong."

---

## 6. Cheap Model Convergence Loop

For simple generated tests, the system uses a convergence loop with a cheap model:

```
while not converged:
    cheap_model generates implementation attempt
    generated_tests run against attempt
    if all pass: converged = true
    else: feed errors back to cheap_model

if converged:
    submit to full gate pipeline
else (after N attempts):
    escalate to expensive model
```

This is a cost optimization: many tasks can be solved by a cheap model (Haiku-class)
with generated tests providing the feedback signal. Only tasks that the cheap model
can't solve get escalated to an expensive model (Opus-class).

The savings compound: if 60% of tasks are solvable by a cheap model, the overall cost
drops by ~50% while maintaining quality (because generated tests enforce the same
standard regardless of which model produced the code).

> **Citation**: bardo-backup/tmp/mori-agents/20-verification-first-architecture.md —
> "Cheap model convergence loop" architecture.

---

## 7. Immutable Verification Artifacts

Generated tests are stored as immutable artifacts in the ArtifactStore before
implementation begins. This ensures:

1. **No tampering**: The implementation agent cannot modify the tests to make them pass
2. **Reproducibility**: The exact tests used for verification can be retrieved later
3. **Forensic replay**: Any verification outcome can be replayed with the original tests

The content-addressed nature of the ArtifactStore (BLAKE3 hashes) makes this
immutability cryptographic, not just conventional.

> **Citation**: bardo-backup/tmp/mori-agents/20-verification-first-architecture.md —
> "Tests must fail before implementation, immutable verification artifacts."

---

## 8. Dependency Detection and Mock Generation

For tasks that interact with external systems (databases, APIs, file systems), the
test generation pipeline includes:

### 8.1 Dependency Detection

Analyze the task specification and the code being modified to identify external
dependencies:
- File system access (read/write)
- Network calls (HTTP, gRPC)
- Database queries
- System clock usage
- Random number generation

### 8.2 Mock Strategy Selection

For each detected dependency, select a mock strategy:
- **In-process mock**: Replace the dependency with a mock implementation
- **Test fixture**: Provide pre-built data files
- **Temporal mock**: Fix the system clock to a known time
- **Deterministic seed**: Fix random seeds for reproducibility

### 8.3 Sidecar Generation

For complex dependencies (databases, external services), generate sidecar test
infrastructure:
- Docker Compose files for test databases
- Mock servers for external APIs
- Test data fixtures with known states

> **Citation**: bardo-backup/tmp/death/16-autonomous-verification.md — "Autonomous test
> infrastructure generation, dependency detection, mock strategies, invariant generation,
> sidecar lifecycle management."

---

## 9. Gate Integration

Generated tests integrate with the gate pipeline through two gates:

### 9.1 GeneratedTestGate (Rung 4)

Runs example-based generated tests. Behaves like `TestGate` but operates on a different
test suite — the generated tests rather than the project's existing tests.

### 9.2 PropertyTestGate (Rung 5)

Runs property-based generated tests. Uses proptest or quickcheck frameworks with
generated property definitions.

Both gates return standard `Verdict` objects, so they compose naturally with the gate
pipeline, ratchet, and adaptive thresholds.

---

## 10. Evaluation Quality Metrics

The autonomous eval generation system tracks its own quality:

| Metric | What It Measures | Target |
|---|---|---|
| Test generation success rate | % of generated tests that compile | > 95% |
| Test discrimination | % of generated tests that fail before implementation | > 80% |
| False positive rate | % of generated tests that fail on correct implementations | < 5% |
| Coverage improvement | Additional code coverage from generated tests | > 20% over hand-written |
| Cost per test | Token cost to generate one test | < $0.01 |

These metrics feed into the meta-learning loop (Loop 14 in the evaluation lifecycle),
which adjusts the test generation strategy over time.

---

## 11. Summary

Autonomous evaluation generation transforms the gate pipeline from a static checker
into a dynamic verification system. By generating tests specific to each task, the
system:

1. Catches issues that hand-written tests miss (they test what changed, not what the
   human thought to test)
2. Enforces the test-driven development cycle automatically
3. Creates an adversarial relationship between test generation and implementation
4. Enables cheap-model convergence loops that reduce cost while maintaining quality

The separation of test generation from implementation is the key architectural insight:
the agent that writes the code should not be the agent that writes the tests.
