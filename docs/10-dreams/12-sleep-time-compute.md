# Sleep-Time Compute: The Economics of Offline Processing

> **Layer**: L0 Runtime (scheduling) + L1 Framework (model routing)
>
> **Synapse Traits**: `Router` (model selection per dream phase), `Policy` (compute budget allocation)
>
> **Crate**: `roko-dreams`, `roko-learn` (cascade_router)
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md), [13-scheduling-and-triggers.md](13-scheduling-and-triggers.md)


> **Implementation**: Scaffold

---

## The Sleep-Time Compute Thesis

Lin et al. (2025, arXiv:2504.13171, "Sleep-time Compute: Beyond Inference Scaling at Test-time") demonstrated that dedicating computation to offline processing during idle periods yields a **5× reduction in test-time compute requirements**. The key insight: agents can perform significant cognitive work (knowledge organization, strategy refinement, counterfactual exploration) during idle periods, making their waking performance dramatically more efficient.

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

## Sleep-Time Compute: Detailed Mechanism (Lin et al. 2025)

**Reference**: Lin, Packer, Wooders et al. (2025), "Sleep-time Compute: Beyond Inference Scaling at Test-time," arXiv:2504.13171 (Letta / UC Berkeley Sky Computing Lab).

### Core Mechanism

The model processes persistent context (codebases, documents, conversation history) offline before queries arrive. Given a persistent context c, the model runs a sleep-time computation phase S(c) → c', transforming c into an enhanced representation c'. At test time, the query is answered from c' rather than c, decoupling thinking cost from latency.

Implementation uses function calling: the model has access to `rethink_memory(new_string)` (replaces current context with new_string) and `finish_rethinking()` (terminates sleep-time process). The model calls `rethink_memory` up to 10 times, progressively rewriting and condensing context into dense summaries optimized for anticipated queries.

Results: reduces test-time compute by ~5× on Stateful GSM-Symbolic and AIME with equal accuracy; further scaling sleep-time compute boosts accuracy by up to 13% on GSM-Symbolic and 18% on AIME. On multi-query workloads: **2.5× reduction in average cost-per-query** when amortizing across 10 queries per context.

### Query Predictability Metric

The key predictor of sleep-time compute efficacy is **query predictability**, operationalized as log P(q | c) — the log-probability of query q given context c under a base model. Examples are binned into 5 quantiles by this score. Higher log P(q | c) → larger accuracy gain from sleep-time compute. Intuitively: if the context already makes the question predictable, pre-computing answers to likely questions is high-yield.

Budget allocation by model type:
- Non-reasoning models (GPT-4o): verbosity prompts (levels 0–4) controlling explanation depth
- Reasoning models (o1, o3-mini): API-level compute control parameters
- Amortization cost model: test-time tokens weighted **10×** the cost of sleep-time tokens

Datasets validated: Stateful GSM-Symbolic (5,000 + 2,500 examples), Stateful AIME (60 problems), Multi-Query GSM-Symbolic (12,043 questions across 1,095 contexts), SWE-Features (33 pull requests).

### Key Insight for Roko

Sleep-time compute is most effective for long-lived contexts with predictable, repeated queries. Agent episodes are exactly this kind of context — recurring task patterns, repeated failure modes, consistent tool chains. The dream cycle's NREM replay should produce dense pre-computed summaries that accelerate waking inference. The 10× cost weighting for test-time vs. sleep-time tokens makes dream-time processing extremely cost-efficient.

### Configuration

```rust
/// Sleep-time compute pre-computation configuration.
/// Based on Lin et al. (2025), arXiv:2504.13171.
pub struct SleepTimePrecompute {
    /// Whether to generate pre-computed summaries during NREM.
    pub enable_precompute: bool,           // default: true
    /// Maximum summary token count per pre-computed context chunk.
    pub max_summary_tokens: usize,         // default: 512, range: 128-2048
    /// Query predictability threshold: only pre-compute for queries
    /// with predictability score above this.
    pub predictability_threshold: f64,     // default: 0.6, range: 0.3-0.9
    /// Maximum pre-computed summaries to cache.
    pub max_cached_summaries: usize,       // default: 100, range: 20-500
    /// TTL for cached summaries (hours).
    pub cache_ttl_hours: u64,             // default: 24, range: 4-168
    /// Whether to measure and log test-time compute savings.
    pub measure_savings: bool,            // default: true
}

/// Pre-computed summary for a recurring query pattern.
pub struct PrecomputedSummary {
    pub id: String,
    pub query_pattern: String,
    pub summary_content: String,
    pub token_count: usize,
    pub predictability_score: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub times_used: usize,
    pub estimated_tokens_saved: usize,
}
```

### Pre-Computation Algorithm

```
SLEEP-TIME-PRECOMPUTE(episodes, existing_summaries, config):
  // Phase 1: Identify predictable query patterns
  patterns = extract_recurring_patterns(episodes)
  predictable = [p for p in patterns if p.predictability > config.predictability_threshold]

  // Phase 2: Generate dense summaries for predictable patterns
  FOR pattern in predictable:
    IF pattern.id NOT IN existing_summaries OR summary_expired(pattern.id):
      context = gather_relevant_episodes(pattern, episodes)
      summary = llm_summarize(context, max_tokens=config.max_summary_tokens)
      cache_summary(PrecomputedSummary {
        id: pattern.id,
        query_pattern: pattern.description,
        summary_content: summary,
        predictability_score: pattern.predictability,
        expires_at: now + config.cache_ttl_hours,
      })

  // Phase 3: Evict stale summaries
  evict_expired(existing_summaries)
  IF existing_summaries.len() > config.max_cached_summaries:
    evict_least_used(existing_summaries)
```

### Test Criteria

```
1. Predictability filtering: patterns below predictability_threshold do not get summaries.
2. Summary token limit: no generated summary exceeds max_summary_tokens.
3. Cache eviction: expired summaries are removed; least-used evicted when count exceeds max.
4. Token savings measurement: times_used * original_context_tokens - times_used * summary_tokens > 0.
5. TTL enforcement: summaries older than cache_ttl_hours are not served.
```

---

## Academic Citations

| Paper | How It Informs Sleep-Time Compute |
|-------|----------------------------------|
| Lin et al. (2025), arXiv:2504.13171 | 5× reduction in test-time compute via sleep-time processing |
| WSCL (2024), "Wake-Sleep Continual Learning" | 38% reduction in catastrophic forgetting |
| Sumers et al. (2023), arXiv:2309.02427, CoALA | Cognitive architecture with three operating frequencies |
| Tononi & Cirelli (2006), "Synaptic homeostasis hypothesis" | Sleep as global synaptic renormalization |
| Lin, Packer, Wooders et al. (2025), arXiv:2504.13171, "Sleep-time Compute: Beyond Inference Scaling at Test-time" | Pre-computed dense summaries reduce test-time compute by 5x; query predictability determines effectiveness |

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [01-three-phase-cycle.md](01-three-phase-cycle.md) | Phase structure that determines compute allocation |
| [13-scheduling-and-triggers.md](13-scheduling-and-triggers.md) | Scheduling logic that determines when dreams fire |
| [07-hypnagogia-engine.md](07-hypnagogia-engine.md) | Hypnagogia phase compute requirements |
