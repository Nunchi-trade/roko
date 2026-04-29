# Active Inference Context Selection

> Depth for [02-CELL.md](../../unified/02-CELL.md). How the Compose protocol uses expected free energy to score, select, and stop-searching for context Signals — replacing hand-tuned priorities with learned, task-adaptive allocation.

---

## Overview

The Compose protocol's budget-constrained assembly (see [02-CELL.md](../../unified/02-CELL.md) S2.5) requires a scoring function that ranks candidate context Signals by their expected contribution to task success. This doc specifies that scoring function: active inference via expected free energy (EFE) minimization.

The core idea from Karl Friston (2006, 2010, 2022): all self-organizing systems minimize the gap between their internal model and reality. Applied to context selection, this means an agent automatically explores novel context when uncertain and exploits proven context when confident. No separate exploration/exploitation tradeoff is needed — the balance emerges from the mathematics.

Two mechanisms work together here:
- **Active inference** answers: "which Signals should the Compose Cell include?" (scoring)
- **Marginal Value Theorem** answers: "when should the system stop searching for more Signals?" (stopping rule)

Both are expressed as Cells operating within the Compose protocol's assembly Graph.

---

## 1. EFE as a Score Cell

The scoring function is a **Score Cell** (see [02-CELL.md](../../unified/02-CELL.md) S2.2) that rates each candidate Signal along the EFE decomposition:

```
G(signal) = pragmatic_value(signal) + epistemic_value(signal) - ambiguity(signal)
```

| Component | Definition | Measurement |
|---|---|---|
| **Pragmatic value** | "Will including this Signal help the agent succeed?" | `E[task_success | included] - E[task_success | excluded]` — estimated from historical gate Verdicts (Verify protocol) when this Signal kind was/was not included |
| **Epistemic value** | "Will including this Signal reduce the agent's uncertainty?" | `D_KL(posterior || prior)` — Bayesian surprise (Itti & Baldi, NeurIPS 2005). Approximated via HDC fingerprint novelty: `1.0 - max_hamming_similarity(signal, known_context)` |
| **Ambiguity** | "How unclear is this Signal's contribution?" | `Var[task_success | included]` — variance in gate outcomes when this Signal kind is included |

The selection policy uses a softmax with inverse temperature gamma:

```
P(include signal_i) = exp(gamma * G_i) / sum_j exp(gamma * G_j)
```

With gamma = 8.0 (from the canonical spec). Higher gamma makes selection more deterministic; lower gamma increases exploration.

### Behavior Under Uncertainty

When the agent is **uncertain** (few episodes in this domain):
- Epistemic value dominates the EFE score
- The Cell prioritizes Signals that fill knowledge gaps — architectural overviews, module interfaces, patterns
- Even Signals unrelated to the immediate task may be selected if they resolve uncertainty

When the agent is **confident** (many successful episodes):
- Pragmatic value dominates
- The Cell selects highest-proven Signals for immediate application — file content, type signatures, proven patterns
- Epistemic Signals are deprioritized because the agent already knows the domain

No hyperparameters control this balance. It emerges from the mathematics.

### Pseudocode: EFE Score Cell

```rust
/// Score Cell implementing EFE-based context ranking.
/// Conforms to Score protocol; used within a Compose Graph
/// at Stage 2 (ranking) of the 5-stage assembly pipeline.
struct EfeScoreCell {
    /// Per-(signal_kind, task_category) Beta posteriors
    /// tracking gate pass/fail when this kind was included.
    track_records: HashMap<(SignalKind, String), BetaPosterior>,
}

impl ScoreProtocol for EfeScoreCell {
    async fn score(&self, signal: &Signal, ctx: &ScoreContext) -> Result<Score> {
        let task_cat = ctx.query.as_deref().unwrap_or("unknown");

        // Pragmatic: conditional pass rate delta
        let posterior = self.track_records
            .get(&(signal.kind, task_cat.into()))
            .unwrap_or(&BetaPosterior { alpha: 1.0, beta: 1.0 });
        let pragmatic = posterior.mean(); // E[success | included]

        // Epistemic: HDC novelty relative to current attention
        let epistemic = match &ctx.attention_focus {
            Some(focus) => 1.0 - signal.hdc.hamming_similarity(focus) as f64,
            None => 0.5, // neutral when no focus available
        };

        // Ambiguity: posterior variance
        let ambiguity = posterior.variance();

        let efe = pragmatic + epistemic - ambiguity;

        // Map EFE to 5-axis Score (utility dimension carries EFE)
        Ok(Score {
            relevance: pragmatic as f32,
            quality: 0.5,       // neutral — not assessed here
            confidence: (1.0 - ambiguity) as f32,
            novelty: epistemic as f32,
            utility: efe as f32,
        })
    }
}
```

---

## 2. Marginal Value Theorem — When to Stop Searching

Eric Charnov (1976) proved the optimal foraging strategy: leave a patch when the marginal gain drops to the average gain rate. Applied to context search:

```
Stop when: relevance(last_result) / cost(last_search) <= total_gain / total_cost
```

The gain follows a diminishing returns curve:

```
g(k) = G_max * (1 - exp(-lambda * k))
g'(k) = G_max * lambda * exp(-lambda * k)
```

Where `k` is the number of search iterations, `G_max` is the asymptotic maximum relevance, and `lambda` is the saturation rate.

### MVT as a Route Cell

The stopping decision is a **Route Cell** (see [02-CELL.md](../../unified/02-CELL.md) S2.4) that decides whether to continue searching or emit the current candidate set:

```rust
/// Route Cell implementing MVT stopping rule.
/// Takes a batch of candidate Signals as input.
/// Routes to "continue searching" or "emit candidates" based on
/// marginal vs average gain ratio.
struct MvtRouteCell {
    /// Per-source gain curve parameters (G_max, lambda).
    source_params: HashMap<String, (f64, f64)>,
    /// Cumulative state for current search session.
    total_gain: f64,
    total_cost: f64,
    iteration: usize,
}

impl RouteProtocol for MvtRouteCell {
    async fn route(
        &self,
        candidates: Vec<Signal>,
        ctx: &RouteContext,
    ) -> Result<RouteDecision> {
        let batch_relevance: f64 = candidates.iter()
            .map(|s| s.scores.utility as f64)
            .sum::<f64>() / candidates.len().max(1) as f64;
        let batch_cost = 1.0; // normalized per-iteration

        let marginal_ratio = batch_relevance / batch_cost;
        let average_ratio = self.total_gain / self.total_cost.max(f64::EPSILON);

        if marginal_ratio <= average_ratio || self.iteration >= 10 {
            RouteDecision::Emit // stop searching, proceed to assembly
        } else {
            RouteDecision::Continue // fetch more candidates
        }
    }
}
```

### Multi-Source Foraging

The system queries four sources. Each has different gain characteristics:

| Source | G_max | lambda | Travel Cost | Typical Iterations |
|---|---|---|---|---|
| Store (neuro knowledge) | 0.9 | 0.25 | Low | 5-8 |
| Store (episodes) | 0.6 | 0.4 | Low | 3-5 |
| File context | 0.8 | 0.5 | Medium | 2-4 |
| Signal log | 0.4 | 0.6 | Low | 1-3 |

A `MultiPatchForager` Route Cell determines optimal visitation order: visit the source with highest expected initial gain (`G_max * lambda`) first. Sources where even the first result's expected gain falls below the environment's average rate are skipped entirely.

### Calibration Loop

MVT parameters are not static. A feedback Loop (see [00-INDEX.md](../../unified/00-INDEX.md) "Loop" pattern) calibrates them:

```
Task outcome -> recorded in episode Store ->
  calibrate (G_max, lambda) per task category ->
    next search uses updated parameters
```

Different task categories saturate differently:
- Simple rename: lambda ~ 0.8 (few results needed)
- Cross-crate integration: lambda ~ 0.15 (many results valuable)
- Bug fix: lambda ~ 0.4 (moderate)

### Sufficient Context Integration

A dual stopping criterion combines MVT with a sufficiency check (Harel-Canada et al., ICLR 2025):

```
stop = (marginal_ratio <= average_ratio) OR (keyword_coverage >= 0.85)
```

---

## 3. The 5-Stage Assembly Pipeline as a Compose Graph

The full context assembly is a Pipeline Graph (see [03-GRAPH.md](../../unified/03-GRAPH.md)) of five Cells:

```
[QueryCell] -> [EfeScoreCell] -> [DedupCell] -> [BudgetCell] -> [FormatCell]
   Stage 1         Stage 2          Stage 3        Stage 4        Stage 5
```

| Stage | Cell Type | Protocol | What It Does |
|---|---|---|---|
| 1. Query | Connect Cell | Connect | Retrieves candidates from 4 sources via HDC + keyword hybrid search (RRF fusion) |
| 2. Score | Score Cell | Score | Ranks by EFE (pragmatic + epistemic - ambiguity) |
| 3. Deduplicate | Verify Cell | Verify | Rejects near-duplicates via HDC Hamming distance < 0.15 |
| 4. Budget | Compose Cell | Compose | Greedy knapsack: include whole Signals in score order until budget exhausted |
| 5. Format | Compose Cell | Compose | U-shaped placement: highest-scored at start and end (Liu et al., "Lost in the Middle", 2023) |

The MVT Route Cell wraps Stage 1, controlling how many iterations the QueryCell runs before handing off to scoring.

### Cold Start

Active inference requires ~10 episodes per task category to calibrate. Before that:
- Stage 2 falls back to static priority scoring (hand-tuned weights on source type, recency, confidence, relevance)
- The transition is automatic: once the EFE Score Cell's posteriors have >= 10 observations per tracked Signal kind, it activates

This mirrors the `CompositionStrategy::auto_select()` pattern from the VCG auction (see [vcg-attention-auction.md](vcg-attention-auction.md)).

---

## 4. Convergence: EFE and VCG

Active inference (EFE scoring) and VCG auction (see [vcg-attention-auction.md](vcg-attention-auction.md)) solve the same problem — optimal allocation of scarce context — through different mechanisms:

| Aspect | Active Inference (EFE) | VCG Auction |
|---|---|---|
| Setting | Single scorer, centralized | Multiple bidders, decentralized |
| Scoring | `pragmatic + epistemic - ambiguity` | `expected_value * urgency * affect_weight` |
| Selection | Softmax over EFE scores | Combinatorial knapsack optimization |
| Exploration | Emerges from epistemic value | Emerges from Thompson sampling on Beta posteriors |
| Truthfulness | N/A (single scorer) | Guaranteed by VCG payment rule |

Both converge on the same allocation when:
- VCG bidders bid truthfully (guaranteed by mechanism)
- EFE scorer has accurate track_record estimates (requires calibration)

In practice: EFE is simpler and sufficient for single-agent prompt assembly. VCG is designed for the multi-agent case where multiple bidder subsystems compete and truthfulness matters.

The current codebase uses a **hybrid path**: EFE scoring selects within a source, while the VCG-style `LearningBidder` allocates across bidder subsystems (Neuro, Task, Research). See the [mori-diffs reality note](#6-mori-diffs-reality) below.

---

## 5. Social Foraging — Collective Context Discovery

In multi-agent plan execution (5-20 parallel agents), each agent forages independently. Social foraging leverages collective retrieval patterns:

When Agent A finds Signals X, Y, Z useful (gate pass on first attempt), that Signal becomes a "stigmergic retrieval pheromone" (Pulse on Bus) boosting those Signals' scores for Agent B working on a related task.

This is expressed as a **Functor** pattern (see [00-INDEX.md](../../unified/00-INDEX.md) "Functor"): an endofunctor `F: Signal -> Signal` that enriches retrieval scores with social evidence before the EFE Score Cell runs.

```rust
/// Functor Cell: enriches candidate Signals with social foraging evidence.
/// Subscribes to retrieval-success Pulses on Bus.
fn social_boost(signal: &mut Signal, recent_pulses: &[Pulse]) {
    let social_evidence: f64 = recent_pulses.iter()
        .filter(|p| p.payload.signal_id == signal.id)
        .filter(|p| p.payload.gate_passed)
        .map(|p| p.payload.relevance * decay(p.timestamp))
        .sum();
    // Capped at 0.3 to prevent over-reliance on social information
    let boost = (social_evidence * 0.1).min(0.3);
    signal.scores.utility += boost as f32;
}
```

Social information helps when knowledge is heterogeneously distributed (clustered by topic) and hurts when uniformly distributed. Research validates this at field scale (Science, 2025, doi:10.1126/science.ady1055).

---

## 6. Mori-Diffs Reality

Per [09-COMPOSITION-AUCTION.md](../../mori-diffs/09-COMPOSITION-AUCTION.md), the actual codebase state:

**VCG auction (`vcg_allocate()`) is built but never called at runtime.** The `PromptComposer::compose()` path uses a greedy value-density sort that is structurally identical to VCG's allocation step but without payment computation, externality tracking, or diagnostic reporting.

**Active inference EFE scoring is not yet implemented.** The current scorer uses hand-tuned static weights: `source_priority * 0.4 + relevance * 0.3 + track_record * 0.2 + recency * 0.1`.

**The `LearningBidder` (Thompson sampling on Beta posteriors) exists in `auction.rs` but does not incorporate cost.** It tracks `(was_included, gate_passed)` pairs but ignores token cost, creating no cost-attribution feedback loop.

The planned transition:
1. **Cold start (< 10 observations)**: WeightedSum greedy path (current behavior)
2. **Warm (>= 10 observations)**: VCG path via `vcg_allocate()` with cost-aware `LearningBidder`
3. **Hot (>= 50 observations)**: Full EFE scoring replaces static weights in the Score Cell

---

## What This Enables

- **Automatic exploration/exploitation balance** — uncertain agents get architectural context; confident agents get targeted code. No tuning needed.
- **Optimal search termination** — MVT stops searching when marginal gains drop, avoiding both insufficient and excessive context retrieval.
- **Social learning** — parallel agents benefit from each other's successful retrievals via stigmergic Pulses on Bus.
- **Cold-to-warm transition** — static priorities bootstrap the system; learned EFE scoring takes over after calibration.

## Feedback Loops

1. **EFE Calibration Loop**: `gate Verdict -> update BetaPosterior for included Signal kinds -> next EFE score uses updated posteriors` (Loop pattern)
2. **MVT Calibration Loop**: `task outcome -> fit (G_max, lambda) per task category -> next search uses updated parameters` (Loop pattern)
3. **Social Foraging Loop**: `Agent A gate pass + retrieval record -> Pulse on Bus -> Agent B retrieval boost` (React protocol)
4. **Cold-to-Warm Transition**: `observation_count crosses threshold -> auto_select switches strategy` (Trigger)

## Open Questions

1. **EFE gamma sensitivity**: The spec uses gamma = 8.0 for softmax temperature. How sensitive are outcomes to this value? Should it be learnable per task category?
2. **MVT vs. sufficient-context stopping**: The dual stopping rule (MVT OR keyword_coverage >= 0.85) may be too conservative — both criteria can fire simultaneously. Should one dominate?
3. **Social foraging scale**: With 20+ parallel agents, social Pulses may flood the Bus. What is the right decay half-life and cap per source?
4. **Cost-aware EFE**: The current EFE decomposition does not include token cost. The VCG mechanism handles cost via budget-constrained knapsack. Should EFE have a cost term (`G = pragmatic + epistemic - ambiguity - cost_pressure`)?
5. **Belief change approximation**: HDC fingerprint novelty is a proxy for true Bayesian surprise. How well does Hamming distance correlate with information gain in practice?

---

## References

- Friston (2006, 2010, 2022), The Free Energy Principle
- Friston et al. (2015), Active Inference and Epistemic Value
- Itti & Baldi (2005), Bayesian Surprise Attracts Human Attention, NeurIPS
- Charnov (1976), Optimal Foraging: The Marginal Value Theorem
- Pirolli & Card (1999), Information Foraging Theory
- Hills et al. (2012), Optimal Foraging in Semantic Memory
- Sumers et al. (2023), CoALA: Cognitive Architectures for Language Agents
- Lewis et al. (2020), RAG
- Liu et al. (2023), "Lost in the Middle", arXiv:2307.03172
- Harel-Canada et al. (2025), Sufficient Context, ICLR
- Lacosse et al. (2026), Emerging Human-like Strategies for Semantic Memory Foraging in LLMs, arXiv:2603.01822
- Mehrabian (1996), PAD Model
