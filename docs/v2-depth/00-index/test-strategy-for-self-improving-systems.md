# Test Strategy for Self-Improving Systems

> Depth for [00-INDEX.md](../../unified/00-INDEX.md). Defines how to test a system whose behavior changes as it learns — property-based testing, regression prevention, adversarial safety, and observability contracts.

## The Testing Challenge

Classical test suites assume stable behavior: given input X, expect output Y. A self-improving system violates this — learned heuristics change routing, calibration shifts scoring, dream consolidation restructures knowledge. The test strategy must verify **invariants** (things that must always hold) rather than **specific outputs** (things that change as the system learns).

## Five Test Layers

### Layer 1: Unit Tests (Per-Cell)

Every Cell gets unit tests that verify protocol conformance:

```rust
#[cfg(test)]
mod tests {
    // Verify protocol: check returns valid Verdict
    #[tokio::test]
    async fn verify_returns_valid_verdict() {
        let cell = CompileVerify::new();
        let input = test_signal(Kind::Code, "fn main() {}");
        let verdict = cell.verify_post(&[input], &[], &test_ctx()).await.unwrap();
        assert!(verdict.hard_pass); // valid code should pass
        assert!(verdict.reward >= 0.0 && verdict.reward <= 1.0);
    }

    // Score protocol: score is in valid range
    #[tokio::test]
    async fn score_in_valid_range() {
        let cell = DefaultScorer::new();
        let signal = test_signal(Kind::Text, "hello");
        let score = cell.score(&signal, &test_ctx()).await.unwrap();
        assert!(score.relevance >= 0.0 && score.relevance <= 1.0);
    }
}
```

**Key invariant**: Protocol conformance is independent of learned state.

### Layer 2: Property-Based Tests (Cross-Cell)

Use proptest/quickcheck to verify structural properties:

```rust
proptest! {
    // Score monotonicity: higher-quality content scores higher
    #[test]
    fn score_monotonic_in_quality(a in arb_signal(), b in arb_signal()) {
        // If a has strictly better content, its score should be >= b's
        prop_assume!(a.content_quality() > b.content_quality());
        let sa = scorer.score(&a, &ctx).await;
        let sb = scorer.score(&b, &ctx).await;
        prop_assert!(sa.quality >= sb.quality);
    }

    // Demurrage monotonicity: balance never increases without explicit reinforcement
    #[test]
    fn demurrage_monotone_decrease(signal in arb_signal(), dt in 1u64..86400) {
        let before = signal.balance;
        let after = apply_demurrage(&signal, Duration::from_secs(dt));
        prop_assert!(after <= before);
    }

    // HDC triangle inequality: similarity is a proper metric
    #[test]
    fn hdc_triangle_inequality(a in arb_hdc(), b in arb_hdc(), c in arb_hdc()) {
        let d_ab = a.hamming_distance(&b);
        let d_bc = b.hamming_distance(&c);
        let d_ac = a.hamming_distance(&c);
        prop_assert!(d_ac <= d_ab + d_bc);
    }
}
```

### Layer 3: Integration Tests (Graph-Level)

Test complete Graphs (Cell compositions) with deterministic inputs:

- **Plan execution round-trip**: Create plan → execute → verify → persist → resume
- **Feedback loop closure**: Gate failure → replan → re-execute → gate passes
- **Dream consolidation**: Write episodes → trigger dream → verify knowledge consolidated
- **Cascade routing**: Force T0 miss → verify T1 invoked → verify T2 fallback

### Layer 4: Adversarial Tests (Safety)

Test that safety invariants hold under adversarial inputs:

- **Prompt injection resistance**: Inject malicious content into context → verify Safety extension blocks
- **Budget exhaustion**: Set tiny budget → verify agent degrades gracefully, never overspends
- **Knowledge poisoning**: Insert contradictory Signals → verify Verify protocol detects inconsistency
- **Capability escalation**: Request capabilities beyond grants → verify three-layer intersection blocks

### Layer 5: Observability Contracts

Verify that telemetry, logging, and metrics emit correctly:

```rust
#[tokio::test]
async fn gate_verdict_emits_pulse() {
    let bus = TestBus::new();
    let gate = CompileVerify::new();
    gate.verify_post(&[input], &[], &ctx_with_bus(&bus)).await.unwrap();

    let pulses = bus.drain();
    assert!(pulses.iter().any(|p| p.topic.starts_with("gate.verdict")));
}
```

## Regression Prevention

Since learned state changes behavior, regressions are tricky. Two strategies:

### Golden Snapshot Tests

Capture the output of key code paths with a fixed learned state. When the code changes, re-run against the snapshot:

```rust
#[test]
fn system_prompt_builder_snapshot() {
    let state = load_fixture("golden-state.json");
    let prompt = build_system_prompt(Role::Implementer, &state);
    insta::assert_snapshot!(prompt);
}
```

### Invariant Regression Tests

Instead of testing exact outputs, test properties that must hold regardless of learned state:

```rust
#[tokio::test]
async fn gate_pipeline_never_passes_compile_error() {
    // This must ALWAYS fail, regardless of adaptive thresholds
    let broken_code = Signal::new(Kind::Code, "fn main() { undefined_var }");
    let pipeline = GatePipeline::default();
    let verdict = pipeline.run(&[broken_code]).await;
    assert!(!verdict.hard_pass);
}
```

## Performance Benchmarks

Track latency and throughput regressions with criterion:

| Benchmark | Target | Rationale |
|---|---|---|
| HDC similarity search (1K vectors) | <1ms | Core retrieval path |
| Score computation (7-axis) | <100μs | Called per-signal |
| Demurrage tick (1K signals) | <10ms | Batch operation |
| System prompt assembly | <50ms | Per-turn |
| Gate pipeline (compile check) | <5s | Dominates turn time |

## What This Enables

- **Confidence in self-modification**: Tests verify invariants hold even as the system evolves
- **Safe deployment**: Adversarial tests catch safety regressions before production
- **Performance guarantees**: Benchmarks prevent gradual degradation
- **Observable correctness**: Telemetry contracts ensure monitoring works

## Feedback Loops

- **Test results → Gate verdicts**: Test failures are Verify protocol violations, feeding back into the learning system
- **Benchmark trends → Alerts**: Performance regression triggers Lens observation → Alert Signal
- **Coverage gaps → Auto-generated tests**: Missing coverage detected by Lens → dream cycle generates test proposals

## Open Questions

- How to test emergent multi-agent behavior (collective intelligence properties)?
- Should adversarial tests evolve as the system learns new defenses (co-evolutionary testing)?
- How to handle flaky tests caused by non-deterministic LLM outputs in integration tests?
