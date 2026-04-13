# Refactoring PRD Additions

Source: `/Users/will/dev/nunchi/roko/refactoring-prd/` (11 files)

These docs add significant detail beyond what was captured in 08-DEEP-ARCHITECTURAL-GAPS.md.

---

## Implementation Ordering (from 07-implementation-priorities.md)

The refactoring PRDs specify a strict critical path:

```
Tier 0: Renaming & Dissolution (immediate)
    ↓
Tier 1: Model Routing & Hardening
    ├── 1A-1B: Provider registry + 8 agent creation site refactors
    ├── 1C-1E: LlmBackend HTTP impls + MCP wiring
    ├── 1D: Claude CLI adapter fixes
    ├── 1F: Auto plan generation on PRD promote
    ├── 1G: API + Learning endpoints
    ├── 1H: TUI dashboard (ratatui)
    ├── 1I: Skill library & playbook extraction
    ├── 1J: LinUCB bandit + context attribution
    └── 1M: 8 missing cybernetic feedback loops ← CRITICAL
    ↓
Tier 2: Cognitive Integration (DIFFERENTIATOR)
    ├── 2A-2C: NeuroStore + active inference context
    ├── 2D-2H: Daimon + Dreams + three-speed cognition
    └── 2M: C-Factor instrumentation
    ↓
Tier 3: Platform (event ingress, MCP, daemon)
Tier 4: Interfaces (TUI, web portal)
Tier 5: Agent Mesh & Chain
Tier 6: Korai Chain (deferred)
```

## 8 Missing Feedback Loops (Tier 1M)

These are the cybernetic wiring that makes the system self-improving:

1. **Health → Routing**: Filter unhealthy providers from CascadeRouter
2. **Conductor → Routing**: Penalize model that caused gate failure
3. **Section → Scaffold**: Track which prompt sections correlate with task success
4. **Failure → Replanning**: Re-plan on gate failures (not just retry)
5. **Skills → Prompts**: Inject task-relevant skills from SkillLibrary
6. **Cost → Routing**: Force cheaper tier on cost spikes
7. **Latency → Reward**: Use actual latency (not static estimate) for routing reward
8. **Experiments → Static**: Update default configs when experiments conclude

## New Policy Implementations Specified

The docs specify concrete Policy trait implementations not yet in code:

| Policy | Purpose | Status |
|--------|---------|--------|
| EpisodePolicy | Record agent turns | Wired |
| RetryPolicy | Retry with escalation | Scaffold |
| TimeoutPolicy | Adaptive per-task timeouts | Scaffold |
| FailureRatePolicy | Circuit breaker | Built |
| StuckLoopPolicy | Detect agent loops | Built |
| **DaimonPolicy** | Affect-modulated decisions | Built (routing) |
| **PredictionPolicy** | Falsifiable predictions as learning signal | NOT IMPLEMENTED |
| **CFactorPolicy** | Collective intelligence metrics | NOT IMPLEMENTED |

## New Scorer Implementations Specified

| Scorer | Purpose | Status |
|--------|---------|--------|
| RecencyScorer | Weight by freshness | Scaffold |
| ReputationScorer | Weight by source trust | Scaffold |
| CatalystScorer | Weight by downstream impact | NOT IMPLEMENTED |
| CompositeScorer | Combine scorers | Built (SumScorer/MulScorer) |
| **PredictiveScorer** | Weight by prediction accuracy | NOT IMPLEMENTED |

## New Router Implementations Specified

| Router | Purpose | Status |
|--------|---------|--------|
| TopKRouter | Return top K candidates | Built |
| CascadeRouter | Multi-stage confidence routing | Built |
| ThompsonBanditRouter | Thompson sampling | NOT IMPLEMENTED |
| LinUCBRouter | Contextual bandit | Built (not wired) |
| **ActiveInferenceRouter** | Expected Free Energy routing | NOT IMPLEMENTED |

## Composition Innovations

### Active Inference Context Selection
- EFE formula: `G = E_Q[ln Q(s') - ln P(s', o')]`
- Decomposes into pragmatic value (goal-seeking) + epistemic value (information-seeking)
- Automatically balances exploration/exploitation with zero hyperparameters

### Predictive Foraging (MVT Stopping Rule)
- Marginal Value Theorem: stop searching when marginal gain < average gain
- Applied to context assembly: stop adding context sections when diminishing returns

### VCG Attention Auction (8 bidders)
1. Neuro (knowledge entries)
2. Daimon (affect-relevant context)
3. Iteration memory (recent agent turns)
4. Code intelligence (symbols, imports)
5. Playbook rules (successful patterns)
6. Research (domain knowledge)
7. Task context (brief, prior outputs)
8. Oracles (predictions, warnings)

Each subsystem bids, winner pays second-highest bid (truthfulness guarantee).

## Trait Signature Refinements

The refactoring PRDs confirm exact trait signatures. Key observations:

- **Scorer is SYNC** (not async) — intentionally fast, no I/O
- **Router is SYNC** — selection is a CPU operation
- **Composer is SYNC** — assembly under budget is deterministic
- **Gate is ASYNC** — verification requires I/O (compile, test, network)
- **Substrate is ASYNC** — storage requires I/O
- **Policy is SYNC** — batch evaluation, returns new signals

This sync/async split is a design decision that enables the Gamma loop to run at high frequency — only Gate and Substrate calls block.

## Knowledge Type Reconciliation

The refactoring PRDs clarify the intended knowledge types:

| PRD Type | Code Type | Action |
|----------|-----------|--------|
| Insight | Insight | Keep |
| Heuristic | Heuristic | Keep |
| Warning | Warning | Added |
| CausalLink | CausalLink | Added |
| StrategyFragment | StrategyFragment | Added |
| AntiKnowledge | AntiKnowledge | Keep |
| — | Fact | RETIRED in code, preserved as serde alias to `Insight` |
| — | Procedure | RETIRED in code, preserved as serde alias to `Heuristic` |
| — | Playbook | RETIRED in code, preserved as serde alias to `StrategyFragment` |
| — | Constraint | RETIRED in code, preserved as serde alias to `Warning` |

## Frontier Innovations Specified

These are the "blue ocean" features that differentiate roko from other agent frameworks:

1. **T0 Layer** — 16 zero-cost probes suppress LLM ~80% of time
2. **VCG Attention Auction** — Game-theoretically optimal context allocation
3. **Somatic Landscape** — Emotional memory as k-d tree navigation
4. **Hypnagogia Engine** — Computational sleep-onset creativity (4 layers)
5. **Dream Engine** — Three-phase consolidation (NREM/REM/integration)
6. **Collective Calibration** — 31.6× faster learning via √N scaling
7. **Predictive Foraging** — Falsifiable predictions as learning signal
8. **x402 Micropayments** — Self-funding agents via knowledge economy
9. **Forensic AI / Causal Replay** — Cryptographically verifiable audit trail
10. **EvoSkills** — Co-evolutionary skill generation (three-tier hierarchy)
11. **ADAS** — Meta-agent architecture search (agents design agents)
12. **Cognitive Kernel Primitives** — OS-level abstractions for agents
