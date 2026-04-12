# Sleep-Time Compute: The Economics of Offline Processing

> **Layer**: L0 Runtime (scheduling) + L1 Framework (model routing)
>
> **Synapse Traits**: `Router` (model selection per dream phase), `Policy` (compute budget allocation)
>
> **Crate**: `roko-dreams`, `roko-learn` (cascade_router)
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md), [13-scheduling-and-triggers.md](13-scheduling-and-triggers.md)

---

## The Sleep-Time Compute Thesis

Lin et al. (2025, arXiv:2504.11651, "Scaling LLM Test-Time Compute Optimally with Sleep-Time Compute") demonstrated that dedicating computation to offline processing during idle periods yields a **5× reduction in test-time compute requirements**. The key insight: agents can perform significant cognitive work (knowledge organization, strategy refinement, counterfactual exploration) during idle periods, making their waking performance dramatically more efficient.

WSCL (2024, "Wake-Sleep Continual Learning") showed a complementary result: interleaving wake and sleep processing phases produces a **38% reduction in catastrophic forgetting** compared to continuous waking-only learning. Sleep consolidation prevents new knowledge from overwriting old knowledge — the CLS architecture in action.

These results provide the economic justification for Roko's dream system: the compute spent on dreams is not wasted idle-time activity but an investment in waking performance.

---

## Compute Budget Allocation

Each dream cycle has a compute budget computed from the agent's overall inference allocation:

```
dream_budget_usd = inference_daily_usd × dream_fraction
```

Where `dream_fraction` is configurable (default: 0.15 — 15% of the daily inference budget is allocated to dreams). This reflects the biological ratio: humans spend ~20–25% of sleep time in REM, and Roko dreams consume roughly that proportion of the agent's cognitive resources.

### Per-Phase Budget Distribution

| Phase | Budget Share | Model Tier | Rationale |
|-------|-------------|-----------|-----------|
| Hypnagogia | 10% | Haiku + Sonnet | Thalamic Gate is free (HDC); Executive Loosener + Dali Interrupt use Sonnet-class; Observer uses Haiku |
| NREM Replay | 30% | Haiku-class (T0) | Pattern matching is cheap — fast, small model suffices |
| REM Imagination | 50% | Sonnet-class (T1) | Creative reasoning requires capable model — this is where the budget goes |
| Integration | 0% | None | Pure computation — no model calls |
| EVOLUTION | 10% | Sonnet-class (T1) | Fires infrequently but requires reasoning |

### Model Selection via CascadeRouter

The `CascadeRouter` (see `crates/roko-learn/src/cascade_router.rs`) manages model selection for dream phases:

```rust
fn dream_phase_model(phase: &DreamPhase, router: &CascadeRouter) -> ModelConfig {
    match phase {
        DreamPhase::NremReplay { .. } => router.select_model(InferenceTier::T0),
        DreamPhase::RemImagination { .. } => router.select_model(InferenceTier::T1),
        DreamPhase::Integration { .. } => ModelConfig::None,
    }
}
```

The cascade routing ensures that if a T1 model is unavailable or the budget is depleted, the system gracefully degrades:
- T1 → T0 fallback for REM (lower quality but still functional)
- T0 → skip for NREM (if even cheap models are too expensive, defer the dream)

---

## Sleepwalker Mode

During dreaming, the agent enters **Sleepwalker mode** — a reduced-capability state where it can still respond to urgent interrupts but does not process normal tasks. Sleepwalker mode is a 3-step variant of the CoALA cognitive architecture (Sumers et al. 2023, arXiv:2309.02427):

1. **Perceive**: Check for urgent signals (process supervisor events, critical errors)
2. **Decide**: If urgent signal detected, abort dream and wake. If not, continue dreaming.
3. **Act**: Either continue the current dream phase or transition to waking mode.

The Sleepwalker does not run the full 9-step Gamma loop. It runs a minimal perception-decision loop with a binary outcome: sleep or wake. This keeps the agent responsive to emergencies while preserving the cognitive isolation needed for effective dreaming.

Sleepwalker mode is signaled to the L0 Runtime via `SIGPAUSE`:

```rust
// Signal the runtime that the agent is entering dream mode
runtime.signal(Signal::Pause { reason: "dream_cycle" });

// Run the dream cycle
let report = dream_cycle.run().await?;

// Signal that dreaming is complete
runtime.signal(Signal::Resume { reason: "dream_complete" });
```

---

## Cost-Effectiveness Analysis

For a typical Roko agent running `roko plan run`:

| Scenario | Episodes/Day | Dreams/Day | Dream Cost/Day | Waking Improvement |
|----------|-------------|-----------|----------------|-------------------|
| Light usage | 10–20 | 1 | ~$0.03–0.08 | Marginal |
| Standard usage | 50–100 | 3–4 | ~$0.10–0.30 | Measurable (10–15% fewer retries) |
| Heavy usage | 200+ | 6–8 | ~$0.30–0.60 | Significant (20–30% fewer retries) |

The cost-effectiveness depends on the ratio of dream cost to the savings from improved waking performance. If dreams prevent even one unnecessary task retry (which costs ~$0.10–0.50 per retry in model inference), a single dream cycle that costs ~$0.05 pays for itself.

---

## Concurrent Execution with Waking Tasks

Dream cycles do not require exclusive access to the agent. In some configurations, dreams can run concurrently with low-priority waking tasks:

| Mode | Description | Use Case |
|------|-------------|----------|
| **Exclusive** | Agent fully pauses waking operations during dreams | Default for single-threaded agents |
| **Background** | Dreams run in a separate thread with reduced priority | For agents with continuous task queues |
| **Interleaved** | Dreams fire between task completions, during natural idle gaps | For orchestrated plan execution |

The `DreamRunner` implementation in `crates/roko-dreams/src/runner.rs` supports all three modes. The default is interleaved: the plan executor detects idle gaps between tasks and signals the dream scheduler.

---

## Privacy Considerations

Dream content contains the agent's most sensitive internal representations: failure analyses, strategy weaknesses, knowledge gaps. If the agent uses external LLM providers for dream processing, this content is transmitted to the provider.

Mitigation options:

| Option | Description | Trade-off |
|--------|-------------|-----------|
| **Local models** | Run NREM replay on local models (e.g., quantized small models) | Lower quality but zero data exposure |
| **Zero-retention providers** | Use providers with contractual zero-retention (e.g., Anthropic API with zero-retention mode) | Full quality, provider commitment |
| **HDC-only dreams** | Skip LLM calls entirely; use only HDC operations for consolidation | Very limited creative capability but zero exposure |

The configuration in `roko.toml` allows per-phase privacy settings:

```toml
[dreams.privacy]
nrem_provider = "local"        # Use local model for NREM
rem_provider = "api"           # Use API for REM (needs quality)
hypnagogia_provider = "api"    # Use API for hypnagogia
```

---

## Academic Citations

| Paper | How It Informs Sleep-Time Compute |
|-------|----------------------------------|
| Lin et al. (2025), arXiv:2504.11651 | 5× reduction in test-time compute via sleep-time processing |
| WSCL (2024), "Wake-Sleep Continual Learning" | 38% reduction in catastrophic forgetting |
| Sumers et al. (2023), arXiv:2309.02427, CoALA | Cognitive architecture with three operating frequencies |
| Tononi & Cirelli (2006), "Synaptic homeostasis hypothesis" | Sleep as global synaptic renormalization |

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [01-three-phase-cycle.md](01-three-phase-cycle.md) | Phase structure that determines compute allocation |
| [13-scheduling-and-triggers.md](13-scheduling-and-triggers.md) | Scheduling logic that determines when dreams fire |
| [07-hypnagogia-engine.md](07-hypnagogia-engine.md) | Hypnagogia phase compute requirements |
