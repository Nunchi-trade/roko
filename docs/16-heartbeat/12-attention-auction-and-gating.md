# Attention Auction, Context Governor, and CorticalState

> VCG truthful bidding for limited context budget, the shared perception surface, meta-cognition hooks, and the frequency scheduler.


> **Implementation**: Specified

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md), [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md), [10-active-inference-compute-allocation.md](./10-active-inference-compute-allocation.md)
**Key sources**: `refactoring-prd/09-innovations.md` §II, legacy `bardo-backup/prd/01-golem/18-cortical-state.md`, `implementation-plans/12a-cognitive-layer.md` §I5

---

## Abstract

An agent's context window is its most constrained resource. A T2 tick assembles ~32,000 tokens of context from competing subsystems: Neuro knowledge entries, Daimon affect state, iteration memory, code intelligence, playbook rules, research artifacts, task context, and oracle predictions. Which sections get included — and how many tokens each receives — determines decision quality.

Na??ve approaches (fixed priority ordering, round-robin allocation, first-come-first-served) waste this resource. A Vickrey-Clarke-Groves (VCG) auction (Vickrey 1961, Clarke 1971, Groves 1973) provides the optimal solution: each subsystem bids for token budget based on its expected contribution to the current tick's success, and the mechanism guarantees truthful bidding — no subsystem can gain by inflating its bid.

This document also specifies the CorticalState (the 32-signal atomic shared perception surface that enables zero-latency inter-subsystem communication), the meta-cognition hook ("Am I stuck? Am I thrashing?"), and the frequency scheduler that coordinates all three cognitive loops.

---

## The VCG Attention Auction

### Why VCG, Not Priority Ranking

Priority ranking is a fixed ordering: system prompt > task description > retrieved knowledge > iteration memory > conversation. This has three problems:

1. **Static**: The ranking doesn't adapt. On a tick where iteration memory is critical (repeated failures on the same task), it still gets low priority.
2. **Gameable**: If subsystems could adjust their priority, they'd all claim highest priority. No incentive for truthful self-assessment.
3. **Wasteful**: A subsystem with 5,000 tokens of high-value content and another with 500 tokens of moderate-value content both get the same fixed allocation, regardless of value.

VCG solves all three problems. The auction runs on every T1/T2 tick during the INTEGRATE step (Step 4 of the Synapse loop):

### Mechanism

```
For each context section candidate:
  bid = expected_value_of_inclusion × urgency × affect_weight

Sorted by bid. Top sections fill the token budget.
Each winner pays the second-highest bid (VCG truthfulness guarantee).
"Payment" is deducted from the subsystem's attention budget for the next tick.
```

The truthfulness guarantee is the key property: because each winner pays the second price (not their own bid), no subsystem benefits from overbidding. If a subsystem bids higher than its true value, it might win and "pay" more than the content is worth. If it bids lower, it might lose inclusion when it should have won. Bidding truthfully is the dominant strategy.

### Bid computation

Each subsystem computes `bid = expected_value * urgency * affect_weight`.

**`expected_value` estimation.** Each subsystem estimates the marginal utility of including its content in the context. The estimation method varies by subsystem:

| Subsystem | expected_value method |
|---|---|
| **Neuro** | PredictiveScorer salience (see [10-active-inference-compute-allocation.md](./10-active-inference-compute-allocation.md)). Combines relevance, confidence, and recency into a single EFE-approximate score. |
| **Daimon** | Affect magnitude: `sqrt(pleasure^2 + arousal^2 + dominance^2)`. Larger affect states carry more information for the LLM. Minimum value: 0.1 (always some baseline affect context). |
| **Iteration Memory** | `failure_count * recency_weight`. Each past failure on the current task gets a base value of 0.3, decayed by `0.9^(ticks_since_failure)`. More recent and more numerous failures produce higher bids. |
| **Code Intelligence** | Coverage ratio: `symbols_referenced_in_context / symbols_referenced_in_task`. If the current task references 10 symbols and code intelligence covers 8 of them, the value is 0.8. |
| **Playbook Rules** | `rule.confidence * condition_match_score`. The match score is 1.0 for exact matches, scaled by the fraction of predicates matched for partial matches. |
| **Research Artifacts** | Cosine similarity between artifact embedding and current task embedding. Falls back to keyword overlap if embeddings are unavailable. |
| **Task Context** | Fixed base value of 0.9. Task context is almost always valuable. The remaining 0.1 margin prevents it from monopolizing the budget. |
| **Oracle Predictions** | `prediction.confidence * relevance_to_task`. Predictions with low confidence (< 0.3) are filtered out before bidding. |

**`urgency` metric.** Urgency is a multiplier in [0.5, 2.0] that reflects time pressure:

```rust
/// Compute urgency multiplier for a subsystem.
pub fn compute_urgency(subsystem: &dyn ContextBidder, state: &TickState) -> f64 {
    let mut urgency = 1.0;

    // Task deadline pressure: increases urgency as deadline approaches
    if let Some(deadline_ticks) = state.ticks_until_deadline() {
        if deadline_ticks < 10 {
            urgency += 0.5;  // imminent deadline
        } else if deadline_ticks < 50 {
            urgency += 0.2;
        }
    }

    // Retry pressure: more retries = more urgent to get it right
    if state.current_task_retries() > 2 {
        urgency += 0.3;
    }

    // Safety pressure: safety-tagged subsystems get urgency boost
    // when arousal is high
    if subsystem.is_safety_relevant() && state.pad.arousal > 0.5 {
        urgency += 0.2;
    }

    urgency.clamp(0.5, 2.0)
}
```

### Bidding subsystems

Eight subsystems compete for context budget:

| Subsystem | What it bids | Bid basis |
|---|---|---|
| **Neuro** | Knowledge entries (insights, heuristics, warnings) | Relevance score x confidence x recency |
| **Daimon** | Affect state and behavioral context | Arousal level x affect magnitude |
| **Iteration Memory** | Past failures and fixes for current task | Failure count x relevance to current task |
| **Code Intelligence** | Symbol graphs, type signatures, dependency info | Coverage of referenced symbols |
| **Playbook Rules** | Applicable learned heuristics | Rule confidence x situation match score |
| **Research Artifacts** | Pre-computed analyses, literature reviews | Relevance to current domain/task |
| **Task Context** | PRD sections, plan details, requirements | Always high base bid (task-defining) |
| **Oracle Predictions** | Relevant predictions and calibration data | Prediction confidence x relevance |

### Affect modulation

The Daimon PAD vector biases bidding:

- **High arousal** -> urgency multiplier on safety-related sections. An anxious agent (low pleasure, high arousal) prioritizes warnings and risk assessments.
- **Low dominance** -> boost for exploratory/research sections. An uncertain agent (low dominance) seeks more diverse context.
- **Low pleasure** -> boost for iteration memory (past failure context). An agent experiencing failures prioritizes understanding what went wrong.

#### Affect weight derivation

The three weights (+0.5 for arousal/safety, +0.3 for (1-dominance)/exploration, +0.4 for (-pleasure)/iteration memory) are grounded in the PAD dimensional model (Mehrabian 1996) and prioritized by consequence severity:

| Weight | Formula | Derivation |
|---|---|---|
| **0.5** (arousal -> safety) | `pad.arousal.abs() * 0.5` | Arousal is the strongest driver of attention in the PAD model. High arousal (positive or negative) signals threat or opportunity detection. The 0.5 coefficient means maximum arousal produces a 50% bid boost for safety content. This is the largest weight because ignoring safety signals has the worst downside. The `.abs()` means both positive arousal (excitement/urgency) and negative arousal inversion (recovering from shock) trigger safety awareness. |
| **0.3** ((1-dominance) -> exploration) | `(1.0 - pad.dominance) * 0.3` | Low dominance maps to uncertainty and openness to new information. Minimum dominance (-1.0) gives factor `(1.0 - (-1.0)) * 0.3 = 0.6`, a 60% boost. Maximum dominance (+1.0) gives factor `0.0 * 0.3 = 0.0`, no boost. The coefficient is smaller than the safety weight because exploration is valuable but never urgent. |
| **0.4** ((-pleasure) -> iteration memory) | `(-pad.pleasure).max(0.0) * 0.4` | Negative pleasure (frustration, failure) maps to a need to review past errors. Maximum displeasure (-1.0) gives factor `1.0 * 0.4 = 0.4`, a 40% boost. The `.max(0.0)` ensures positive pleasure (success, satisfaction) has no effect on iteration memory bidding -- a satisfied agent does not need failure review. Ranked between safety and exploration because learning from failure is important but not as critical as immediate safety. |

These are **multiplicative on the base bid**, not additive to the bid score. A subsystem with a low base bid (low expected value) still gets a low final bid even with maximum affect modulation. Affect shifts the ranking among competitive bids; it does not promote irrelevant content.

```rust
/// Compute VCG bids for context sections.
///
/// Each subsystem provides candidate sections. The auction
/// allocates tokens optimally under the budget constraint.
///
/// Citation: VCG mechanism (Vickrey 1961, Clarke 1971, Groves 1973).
/// Applied to attention allocation following the attention economics
/// framework in context engineering (Karpathy 2025).
fn run_attention_auction(
    candidates: &[ContextCandidate],
    budget_tokens: usize,
    pad: &PadVector,
) -> Vec<ContextAllocation> {
    // Compute bids with affect modulation
    let mut bids: Vec<(usize, f64)> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let base_bid = c.expected_value * c.urgency;

            // Affect modulation (multiplicative on base bid)
            let affect_mult = match c.category {
                ContextCategory::Safety => {
                    1.0 + pad.arousal.abs() * 0.5  // High arousal -> safety priority
                }
                ContextCategory::Exploration => {
                    1.0 + (1.0 - pad.dominance) * 0.3  // Low dominance -> explore
                }
                ContextCategory::IterationMemory => {
                    1.0 + (-pad.pleasure).max(0.0) * 0.4  // Low pleasure -> review
                }
                _ => 1.0,
            };

            (i, base_bid * affect_mult)
        })
        .collect();

    // Sort by bid descending
    bids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    // Allocate tokens greedily until budget exhausted
    let mut remaining = budget_tokens;
    let mut allocations = Vec::new();

    for (idx, bid) in &bids {
        let candidate = &candidates[*idx];
        let tokens = candidate.token_count.min(remaining);
        if tokens == 0 {
            break;
        }

        // VCG payment: the second-highest bid that would have taken this slot
        let next_bid = bids.get(allocations.len() + 1)
            .map(|(_, b)| *b)
            .unwrap_or(0.0);

        allocations.push(ContextAllocation {
            candidate_idx: *idx,
            tokens_allocated: tokens,
            bid: *bid,
            payment: next_bid,  // VCG second-price payment
        });

        remaining -= tokens;
    }

    allocations
}
```

### Second-price payment mechanics

**Tie-breaking.** When two candidates have identical bids, ties are broken by:
1. Token efficiency: `bid / token_count` (higher is better -- more value per token).
2. Subsystem priority: Task Context > Safety > Iteration Memory > others (static tiebreaker of last resort).

```rust
/// Tiebreaker for equal bids: prefer higher value per token,
/// then fall back to subsystem priority.
fn tiebreak(a: &ContextCandidate, b: &ContextCandidate) -> std::cmp::Ordering {
    let a_efficiency = a.expected_value / a.token_count.max(1) as f64;
    let b_efficiency = b.expected_value / b.token_count.max(1) as f64;
    b_efficiency.partial_cmp(&a_efficiency)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| b.category.priority().cmp(&a.category.priority()))
}
```

**Payment calculation.** The VCG second-price payment is the bid of the first excluded candidate -- the minimum bid that would have won this slot. If no candidate was excluded (budget was not exhausted), the payment is 0.0. This means the last winning candidate always pays 0.0 when there is remaining budget.

### Attention budget carryover

The "payment" each winner makes is deducted from its subsystem's attention budget for the next tick. This creates a dynamic balancing effect:

- A subsystem that wins many auctions accumulates "debt" and bids lower on future ticks.
- A subsystem that loses auctions accumulates "credit" and bids higher, increasing its chances.
- Over time, attention is distributed proportionally to actual value contributed.

**Carryover mechanics.**

```rust
/// Per-subsystem attention budget tracker.
pub struct AttentionBudget {
    /// Current balance per subsystem. Positive = credit, negative = debt.
    balances: HashMap<SubsystemId, f64>,
    /// Maximum debt a subsystem can accumulate before being blocked.
    max_debt: f64,  // default: -5.0
    /// Decay factor applied each tick to shrink balances toward zero.
    decay: f64,     // default: 0.95
}

impl AttentionBudget {
    /// Initialize all subsystems with zero balance.
    pub fn new(subsystems: &[SubsystemId]) -> Self {
        Self {
            balances: subsystems.iter().map(|s| (*s, 0.0)).collect(),
            max_debt: -5.0,
            decay: 0.95,
        }
    }

    /// Apply auction results: winners pay, losers gain credit.
    pub fn apply_auction_results(&mut self, results: &[ContextAllocation], candidates: &[ContextCandidate]) {
        // Winners: deduct payment
        for alloc in results {
            let subsystem = candidates[alloc.candidate_idx].subsystem_id;
            *self.balances.entry(subsystem).or_insert(0.0) -= alloc.payment;
        }

        // Losers: gain a small credit (0.1) for being excluded
        let winner_subsystems: HashSet<SubsystemId> = results.iter()
            .map(|a| candidates[a.candidate_idx].subsystem_id)
            .collect();
        for (subsystem, balance) in &mut self.balances {
            if !winner_subsystems.contains(subsystem) {
                *balance += 0.1;
            }
        }

        // Decay all balances toward zero
        for balance in self.balances.values_mut() {
            *balance *= self.decay;
        }
    }

    /// Get the carryover multiplier for a subsystem's bid.
    /// Positive balance -> bid boost. Negative balance -> bid penalty.
    pub fn bid_multiplier(&self, subsystem: SubsystemId) -> f64 {
        let balance = self.balances.get(&subsystem).copied().unwrap_or(0.0);
        if balance >= 0.0 {
            1.0 + balance * 0.1  // credit: up to ~1.5x boost
        } else {
            (1.0 + balance * 0.2).max(0.1)  // debt: down to 0.1x penalty
        }
    }
}
```

**Initial budget.** All subsystems start with zero balance. The first auction has no carryover effects. This means the first tick's allocation is purely determined by base bids + affect modulation. Carryover effects stabilize within ~10 ticks.

**Debt cap.** A subsystem that reaches `max_debt` (-5.0) has its bid multiplier floored at 0.1x, making it very unlikely to win. This prevents a subsystem from being permanently excluded -- the 0.95 decay factor erodes debt over time, so even a deeply indebted subsystem recovers within ~50 ticks (0.95^50 * 5.0 = 0.36).

**Decay.** The 0.95 decay factor means old auction results fade. A payment made 20 ticks ago contributes only `0.95^20 = 0.36` of its original penalty. This ensures the carryover responds to recent performance, not ancient history.

---

## The Context Governor

The context governor manages the overall token budget for each tick tier:

| Tier | Token Budget | Context Strategy |
|---|---|---|
| T0 | 0 tokens | No context assembly (pure probe + playbook) |
| T1 | ~4,000 tokens | Focused: task + top-5 retrieved + critical warnings |
| T2 | ~32,000 tokens | Full: VCG auction allocates across all 8 subsystems |

#### Context budget derivation (4K / 32K)

**T1 = ~4,000 tokens.** This targets a Haiku-class model with a 200K token context window. The 4K budget is not a model limitation -- it is a cost/latency constraint. At Haiku pricing ($1/MTok input), 4K tokens costs $0.004. The budget is sized to fit: one system prompt (~1.2K), five retrieved entries (~1.5K), active task summary (~0.8K), and warnings (~0.5K). Expanding to 8K doubles cost with diminishing returns -- the T1 model's reasoning depth does not benefit from additional context beyond what is needed for triage.

**T2 = ~32,000 tokens.** This targets Sonnet/Opus-class models. The 32K budget is a quality/cost sweet spot: below 32K, Cognitive Workspace sections get truncated and decision quality drops. Above 32K, cost increases linearly but quality gains are marginal -- most additional context is low-relevance padding. The VCG auction ensures the 32K is filled with the highest-value content.

**Task complexity adjustment.** The base budgets adjust for task complexity:

```rust
impl ContextGovernor {
    /// Adjust budget based on task complexity.
    ///
    /// Simple tasks (< 100 lines of code, single-file changes) get
    /// smaller budgets. Complex tasks (multi-crate, novel algorithms)
    /// get larger budgets, up to the model's effective limit.
    fn adjusted_budget(&self, tier: InferenceTier, task: &TaskSpec) -> usize {
        let base = self.tier_budgets.get(&tier).copied().unwrap_or(0);
        let complexity = task.estimated_complexity(); // 0.0..1.0

        // Scale: 0.5x at minimum complexity, 1.5x at maximum
        let scale = 0.5 + complexity;
        let adjusted = (base as f64 * scale) as usize;

        // Cap at model context limit minus expected output tokens
        let model_limit = match tier {
            InferenceTier::T0 => 0,
            InferenceTier::T1 => 8_000,     // haiku can handle more if needed
            InferenceTier::T2 => 128_000,   // sonnet/opus context limit
        };

        adjusted.min(model_limit)
    }
}
```

The context governor enforces the budget by:
1. Setting the total budget based on tier, adjusted for task complexity.
2. Running the VCG auction to allocate across subsystems.
3. Calling `Composer.compose()` with the allocation to assemble the actual context.
4. Verifying the assembled context fits within the budget (truncating lowest-bid sections if needed).

```rust
/// The context governor: manages token budget and auction.
///
/// Maps to Composer.compose() in the Synapse Architecture (L2 Scaffold).
pub struct ContextGovernor {
    pub tier_budgets: HashMap<InferenceTier, usize>,
}

impl ContextGovernor {
    /// Assemble context for a given tier.
    pub fn assemble(
        &self,
        tier: InferenceTier,
        subsystems: &[Box<dyn ContextBidder>],
        pad: &PadVector,
        composer: &dyn Composer,
    ) -> Result<AssembledContext> {
        let budget = self.tier_budgets
            .get(&tier)
            .copied()
            .unwrap_or(0);

        if budget == 0 {
            return Ok(AssembledContext::empty());
        }

        // Collect candidates from all subsystems
        let candidates: Vec<ContextCandidate> = subsystems
            .iter()
            .flat_map(|s| s.generate_candidates())
            .collect();

        // Run VCG auction
        let allocations = run_attention_auction(&candidates, budget, pad);

        // Assemble via Composer
        let engrams: Vec<Engram> = allocations.iter()
            .map(|a| candidates[a.candidate_idx].to_engram(a.tokens_allocated))
            .collect();

        composer.compose(&engrams, &Context::with_budget(budget))
    }
}
```

---

## CorticalState: The Shared Perception Surface

The CorticalState is a 32-signal atomic struct (~192 bytes, 4 cache lines) that provides zero-latency inter-subsystem communication. Any subsystem can read any signal with a single atomic load — no locks, no waiting, no contention.

### The Struct

```rust
/// Zero-latency shared perception surface.
///
/// Every subsystem writes its own signals; every subsystem reads
/// everyone else's. ~192 bytes total. Cache-line aligned to avoid
/// false sharing between signal groups.
///
/// Convention: f32 values stored via f32::to_bits() / f32::from_bits()
/// because Rust stable has no floating-point atomics.
#[repr(C, align(64))]
pub struct CorticalState {
    // ═══ AFFECT — written by Daimon ═══
    pub(crate) pleasure: AtomicU32,        // f32 [-1.0, 1.0]
    pub(crate) arousal: AtomicU32,         // f32 [-1.0, 1.0]
    pub(crate) dominance: AtomicU32,       // f32 [-1.0, 1.0]
    pub(crate) primary_emotion: AtomicU8,  // Plutchik label (0-7)

    // ═══ PREDICTION — written by Oracle ═══
    pub(crate) aggregate_accuracy: AtomicU32,        // f32 [0.0, 1.0]
    pub(crate) accuracy_trend: AtomicI8,             // -1, 0, +1
    pub(crate) category_accuracies: [AtomicU32; 16], // f32 per category
    pub(crate) surprise_rate: AtomicU32,             // f32 [0.0, 1.0]

    // ═══ ATTENTION — written by Oracle/AttentionForager ═══
    pub(crate) universe_size: AtomicU32,     // total tracked items
    pub(crate) active_count: AtomicU16,      // ACTIVE tier items
    pub(crate) pending_predictions: AtomicU32,

    // ═══ CREATIVE — written by Dream engine ═══
    pub(crate) creative_mode: AtomicU8,               // bool as 0/1
    pub(crate) fragments_captured: AtomicU32,
    pub(crate) last_novel_prediction_tick: AtomicU32,
    pub(crate) last_novel_prediction_tick_hi: AtomicU32,

    // ═══ ENVIRONMENT — written by domain probes ═══
    pub(crate) regime: AtomicU8,     // 0=calm, 1=trending, 2=volatile, 3=crisis
    pub(crate) gas_gwei: AtomicU32,  // f32 (domain-specific, chain only)

    // ═══ RESOURCE — written by budget tracker ═══
    pub(crate) resource_health: AtomicU32,   // f32 [0.0, 1.0] budget remaining
    pub(crate) knowledge_health: AtomicU32,  // f32 [0.0, 1.0] knowledge quality
    pub(crate) performance_trend: AtomicU32, // f32 [-1.0, 1.0] improving/declining
    pub(crate) behavioral_state: AtomicU8,   // 0-5 (Engaged..Resting)

    // ═══ DERIVED — written by runtime per-tick ═══
    pub(crate) compounding_momentum: AtomicU32, // f32 [0.0, 1.0]
}
```

### Design Properties

**No locks.** Writes use `Ordering::Release`, reads use `Ordering::Acquire`. This ensures that when a reader observes a new value, all preceding writes by that writer are visible. A snapshot where `pleasure` is from tick N and `accuracy` is from tick N+1 is acceptable — CorticalState is eventually consistent, not transactionally consistent.

**Clear ownership.** Each signal group has exactly one writer:

| Signal Group | Writer | Frequency |
|---|---|---|
| Affect (4 signals) | Daimon | Every prediction resolution (gamma) |
| Prediction (20 signals) | Oracle/CalibrationTracker | Every prediction resolution (gamma) |
| Attention (3 signals) | Oracle/AttentionForager | Per gamma tick |
| Creative (4 signals) | Dream engine | Per dream cycle |
| Environment (2 signals) | Domain probes | Per gamma tick |
| Resource (4 signals) | Budget tracker / Theta | Per theta tick |
| Derived (1 signal) | Runtime | Per delta tick |

**No write contention.** No signal has two writers. This eliminates contention entirely.

### Reading

```rust
impl CorticalState {
    /// Read the full PAD vector.
    pub fn pad(&self) -> PadVector {
        PadVector {
            pleasure: f32::from_bits(self.pleasure.load(Ordering::Acquire)) as f64,
            arousal: f32::from_bits(self.arousal.load(Ordering::Acquire)) as f64,
            dominance: f32::from_bits(self.dominance.load(Ordering::Acquire)) as f64,
        }
    }

    /// Read aggregate prediction accuracy.
    pub fn prediction_accuracy(&self) -> f32 {
        f32::from_bits(self.aggregate_accuracy.load(Ordering::Acquire))
    }

    /// Read current behavioral state.
    pub fn behavioral_state(&self) -> BehavioralState {
        BehavioralState::from_u8(self.behavioral_state.load(Ordering::Acquire))
    }

    /// Full snapshot for context assembly or rendering.
    pub fn snapshot(&self) -> CorticalSnapshot {
        CorticalSnapshot {
            pad: self.pad(),
            accuracy: self.prediction_accuracy(),
            regime: Regime::from_u8(self.regime.load(Ordering::Acquire)),
            behavioral_state: self.behavioral_state(),
            resource_health: f32::from_bits(
                self.resource_health.load(Ordering::Acquire)
            ),
            // ... all other fields
        }
    }
}
```

### Initialization

All signals start at neutral values. The PAD vector initializes to the personality baseline from agent configuration:

| Personality Preset | Pleasure | Arousal | Dominance |
|---|---|---|---|
| Cautious | -0.1 | 0.1 | -0.2 |
| Balanced | 0.0 | 0.0 | 0.0 |
| Aggressive | 0.1 | 0.3 | 0.2 |

#### Learned vs configured signals

CorticalState signals fall into two categories:

**Configured (static at startup).** These are set from `roko.toml` and do not change during a session:
- `gas_gwei` (domain-specific, only relevant for chain agents)
- The personality preset that determines PAD initialization

**Learned (dynamic during runtime).** Every other signal is written by its owner subsystem at the specified frequency. The initial values are:

```rust
impl CorticalState {
    /// Initialize all signals to neutral defaults.
    pub fn new(personality: &PersonalityPreset) -> Self {
        Self {
            // Affect: from personality preset
            pleasure: AtomicU32::new(personality.pleasure.to_bits()),
            arousal: AtomicU32::new(personality.arousal.to_bits()),
            dominance: AtomicU32::new(personality.dominance.to_bits()),
            primary_emotion: AtomicU8::new(0), // Joy (neutral)

            // Prediction: no data yet
            aggregate_accuracy: AtomicU32::new(0.5f32.to_bits()), // 50% = no info
            accuracy_trend: AtomicI8::new(0),   // flat
            category_accuracies: std::array::from_fn(|_| AtomicU32::new(0.5f32.to_bits())),
            surprise_rate: AtomicU32::new(0.0f32.to_bits()),

            // Attention: empty
            universe_size: AtomicU32::new(0),
            active_count: AtomicU16::new(0),
            pending_predictions: AtomicU32::new(0),

            // Creative: inactive
            creative_mode: AtomicU8::new(0),
            fragments_captured: AtomicU32::new(0),
            last_novel_prediction_tick: AtomicU32::new(0),
            last_novel_prediction_tick_hi: AtomicU32::new(0),

            // Environment: calm
            regime: AtomicU8::new(0),  // Calm
            gas_gwei: AtomicU32::new(0.0f32.to_bits()),

            // Resource: full health
            resource_health: AtomicU32::new(1.0f32.to_bits()),
            knowledge_health: AtomicU32::new(0.5f32.to_bits()),
            performance_trend: AtomicU32::new(0.0f32.to_bits()),
            behavioral_state: AtomicU8::new(0), // Engaged

            // Derived
            compounding_momentum: AtomicU32::new(0.0f32.to_bits()),
        }
    }
}
```

#### PAD initialization mapping

The personality preset maps to PAD values via a lookup table. These values are chosen to produce reasonable default behavior without any learned history:

| Preset | Pleasure | Arousal | Dominance | Behavioral effect |
|---|---|---|---|---|
| Cautious | -0.1 | 0.1 | -0.2 | Lower gating threshold (more T1/T2 escalation), higher exploration boost. Good for new domains where the agent should learn before acting. |
| Balanced | 0.0 | 0.0 | 0.0 | No affect modulation at startup. Pure prediction-error-driven gating. The default for most agents. |
| Aggressive | 0.1 | 0.3 | 0.2 | Higher gating threshold (more T0 suppression), confidence boost. Good for well-understood domains where the agent should act on heuristics. |

Custom presets can be defined in `roko.toml`:

```toml
[agent.personality]
preset = "custom"
pleasure = -0.05
arousal = 0.2
dominance = 0.1
```

---

## Meta-Cognition Hook

The meta-cognition hook runs at the end of each theta tick and during delta consolidation. It answers the question: "Am I doing this well?"

```rust
/// Meta-cognition: the agent's self-assessment.
///
/// This is implementation item I5 from 12a-cognitive-layer.md.
/// It detects common failure modes and triggers interventions.
pub fn meta_cognize(
    state: &AgentState,
    cortical: &CorticalState,
) -> MetaCognitionResult {
    let mut issues = Vec::new();

    // Am I stuck? (>3 retries on same task)
    if state.current_task_retries() > 3 {
        issues.push(MetaIssue::Stuck {
            task: state.current_task_id(),
            retries: state.current_task_retries(),
            suggestion: "Escalate to T2 with different approach, or request human review",
        });
    }

    // Am I thrashing? (oscillating between approaches without progress)
    if state.approach_changes_last_n(5) > 3 {
        issues.push(MetaIssue::Thrashing {
            changes: state.approach_changes_last_n(5),
            suggestion: "Commit to one approach for at least 3 more attempts",
        });
    }

    // Should I escalate? (declining performance trend)
    let trend = f32::from_bits(
        cortical.performance_trend.load(Ordering::Acquire)
    );
    if trend < -0.3 {
        issues.push(MetaIssue::PerformanceDecline {
            trend,
            suggestion: "Switch to stronger model or request different task",
        });
    }

    // Am I coasting? (high success but declining engagement)
    let accuracy = cortical.prediction_accuracy();
    let arousal = cortical.pad().arousal as f32;
    if accuracy > 0.8 && arousal < -0.2 {
        issues.push(MetaIssue::Complacency {
            accuracy,
            arousal,
            suggestion: "Seek novel challenges or increase exploration rate",
        });
    }

    MetaCognitionResult { issues }
}
```

#### Retry counting scope

`state.current_task_retries()` counts retries **per task, per agent**. The scope:

- A "retry" is defined as: the agent receives the same task ID from the executor after a previous attempt on that task ID resulted in a gate failure.
- The counter resets when the task ID changes (agent moves to a different task) or when the agent is replaced (a different agent takes over the task).
- The counter persists across gamma ticks within the same task execution. It does NOT persist across sessions -- on resume, retry counts start at zero.
- Retries are stored in the `AgentState` struct, not in the plan DAG. The plan DAG tracks task status (pending/running/passed/failed), not per-agent attempt counts.

```rust
/// Retry tracking for meta-cognition.
pub struct RetryTracker {
    /// Current task being worked on.
    current_task_id: Option<TaskId>,
    /// Number of retries on the current task.
    retry_count: u32,
    /// History of approaches tried (for thrashing detection).
    approach_history: VecDeque<ApproachTag>,
    /// Maximum history length.
    max_history: usize,  // default: 10
}

impl RetryTracker {
    pub fn record_attempt(&mut self, task_id: TaskId, approach: ApproachTag) {
        if self.current_task_id.as_ref() != Some(&task_id) {
            // New task: reset counters
            self.current_task_id = Some(task_id);
            self.retry_count = 0;
            self.approach_history.clear();
        } else {
            self.retry_count += 1;
        }
        self.approach_history.push_back(approach);
        if self.approach_history.len() > self.max_history {
            self.approach_history.pop_front();
        }
    }

    pub fn current_task_retries(&self) -> u32 {
        self.retry_count
    }
}
```

#### Approach change definition

`state.approach_changes_last_n(5)` counts the number of **distinct consecutive approaches** in the last N attempts. An "approach" is a tagged strategy identified by the agent or the executor:

- Each gate failure produces an `ApproachTag` -- a string describing the strategy used (e.g., "direct-implementation", "test-first", "refactor-then-implement", "minimal-patch").
- An approach change occurs when `approach_history[i] != approach_history[i-1]`.
- If the last 5 entries are `[A, B, A, B, A]`, there are 4 approach changes -- strong evidence of thrashing.
- If the last 5 entries are `[A, A, A, A, B]`, there is 1 approach change -- the agent tried A four times, then switched to B. This is not thrashing.

The threshold of `> 3` changes in 5 attempts means the agent switched strategies on more than 60% of attempts. This is conservative -- it allows the agent to try 2-3 approaches before flagging.

```rust
impl RetryTracker {
    /// Count approach changes in the last N attempts.
    pub fn approach_changes_last_n(&self, n: usize) -> u32 {
        let recent: Vec<&ApproachTag> = self.approach_history.iter()
            .rev()
            .take(n)
            .collect();

        if recent.len() < 2 {
            return 0;
        }

        recent.windows(2)
            .filter(|w| w[0] != w[1])
            .count() as u32
    }
}
```

Meta-cognition produces `CognitiveSignal` events when issues are detected:
- Stuck -> `CognitiveSignal::Escalate` (switch to stronger model)
- Thrashing -> `CognitiveSignal::Cooldown` (reduce frequency, commit to approach)
- Performance decline -> `CognitiveSignal::Escalate` or intervention request
- Complacency -> `CognitiveSignal::Explore` (seek novel territory)

---

## The Frequency Scheduler

The frequency scheduler (implementation item I4 from 12a-cognitive-layer.md) coordinates the three cognitive loops by deciding which loop should run next and adjusting intervals dynamically:

```rust
/// The frequency scheduler coordinates gamma, theta, and delta.
///
/// It reads the CorticalState to make scheduling decisions and
/// emits CognitiveSignals to adjust loop behavior.
pub struct FrequencyScheduler {
    clock: AdaptiveClock,
    cortical: Arc<CorticalState>,
}

impl FrequencyScheduler {
    /// Main scheduling loop: adjust frequencies based on current state.
    pub async fn run(&self) {
        loop {
            let snapshot = self.cortical.snapshot();

            // Adjust gamma based on probe anomalies
            let gamma_interval = self.clock.compute_gamma_interval(
                &snapshot.recent_anomalies
            );

            // Adjust theta based on regime
            let theta_interval = self.clock.compute_theta_interval(
                snapshot.regime
            );

            // Check if delta should fire
            if self.clock.should_enter_delta(
                snapshot.idle_duration,
                snapshot.episodes_since_delta,
            ) {
                self.clock.emit_signal(CognitiveSignal::Resume);
            }

            // Apply budget throttling
            let budget_pct = snapshot.resource_health;
            let throttled_theta = apply_budget_throttle(
                theta_interval,
                budget_pct as f64,
                &self.clock.config,
            );

            // Update intervals
            self.clock.set_gamma_interval(gamma_interval);
            self.clock.set_theta_interval(throttled_theta);

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
```

#### The `compute_gamma_interval()` algorithm

The gamma interval determines how often the agent perceives and potentially acts. The algorithm adapts the interval based on recent anomaly density:

```rust
impl AdaptiveClock {
    /// Compute gamma interval from recent anomaly density.
    ///
    /// More anomalies -> shorter interval (faster ticking).
    /// Fewer anomalies -> longer interval (slower ticking, save resources).
    ///
    /// Algorithm: exponential mapping from anomaly rate to interval.
    pub fn compute_gamma_interval(
        &self,
        recent_anomalies: &AnomalyHistory,
    ) -> Duration {
        let base = self.config.gamma_base_interval;  // default: 10s
        let min_interval = self.config.gamma_min_interval;  // default: 2s
        let max_interval = self.config.gamma_max_interval;  // default: 60s

        // Anomaly rate: fraction of probes that were anomalous
        // over the last 10 ticks.
        let anomaly_rate = recent_anomalies.rate_last_n(10);  // [0.0, 1.0]

        // Exponential mapping:
        // rate = 0.0 -> interval = max_interval (nothing happening, slow down)
        // rate = 0.5 -> interval ~ base (normal activity)
        // rate = 1.0 -> interval = min_interval (everything is anomalous, speed up)
        let t = anomaly_rate.clamp(0.0, 1.0);
        let log_min = (min_interval.as_secs_f64()).ln();
        let log_max = (max_interval.as_secs_f64()).ln();

        // Linear interpolation in log space -> exponential in real space
        let log_interval = log_max + t * (log_min - log_max);
        let interval_secs = log_interval.exp();

        Duration::from_secs_f64(interval_secs.clamp(
            min_interval.as_secs_f64(),
            max_interval.as_secs_f64(),
        ))
    }
}
```

| Anomaly rate | Interval | Ticks/minute |
|---|---|---|
| 0.00 (calm) | 60s | 1 |
| 0.10 (quiet) | ~33s | ~2 |
| 0.25 (normal) | ~15s | ~4 |
| 0.50 (active) | ~7s | ~9 |
| 0.75 (volatile) | ~3.5s | ~17 |
| 1.00 (crisis) | 2s | 30 |

#### Blocking vs polling loop control

The frequency scheduler uses a **polling loop** with a 1-second sleep, not blocking I/O. The reasons:

1. **Cross-loop coordination.** The scheduler needs to read CorticalState (written by other loops) and adjust intervals (read by other loops). A blocking wait on a single event source would miss updates from other subsystems.
2. **Tokio compatibility.** The scheduler runs as a `tokio::spawn` task. Blocking the thread would stall other tasks on the same runtime. The 1-second poll is cheap (a timer wake-up, a few atomic reads, and a branch).
3. **Budget throttling.** The scheduler applies budget throttling every second regardless of other events. A blocking model would require a separate timer task.

The 1-second poll interval is a meta-frequency: the scheduler checks CorticalState once per second and adjusts the gamma/theta intervals. This is much slower than the fastest gamma interval (2s) and does not create overhead. If finer-grained scheduling is needed (sub-second adjustments), the poll interval can be reduced to 250ms with negligible CPU cost.

---

## Academic Foundations

- **Vickrey 1961** — "Counterspeculation, auctions, and competitive sealed tenders" (Journal of Finance 16(1)). Second-price auction mechanism.
- **Clarke 1971** — "Multipart pricing of public goods" (Public Choice 11). Generalization of Vickrey to multiple goods.
- **Groves 1973** — "Incentives in teams" (Econometrica 41(4)). Truthful incentive compatibility.
- **Karpathy 2025** — "Context Engineering" (blog post, June 2025). "The delicate art and science of filling the context window with just the right information."
- **Baddeley 2000** — "The episodic buffer" (Trends in Cognitive Sciences 4(11)). Working memory model underlying context assembly.
- **Damasio 1994** — "Descartes' Error" (Putnam). Somatic markers biasing attention allocation.
- **Bower 1981** — "Mood and memory" (American Psychologist 36(2)). Affect-modulated memory retrieval.
- **Barrett 2017** — "How Emotions Are Made" (Houghton Mifflin). Constructed emotion from prediction residuals.
- **Friston 2010** — "The Free-Energy Principle" (Nature Reviews Neuroscience 11(2)). Precision weighting for attention.

---

## The AuctionRound and BidResult Structs

```rust
/// A complete auction round: inputs, bids, allocations, and diagnostics.
///
/// Persisted to the episode log for post-hoc analysis and
/// attention budget tuning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuctionRound {
    /// Tick identifier.
    pub tick_id: u64,

    /// Timestamp of the auction.
    pub timestamp: chrono::DateTime<chrono::Utc>,

    /// The tier that triggered this auction (T1 or T2).
    pub tier: InferenceTier,

    /// Total token budget for this round.
    pub budget_tokens: usize,

    /// PAD vector at auction time.
    pub pad: PadVector,

    /// All bids submitted (including losers).
    pub bids: Vec<BidResult>,

    /// Number of candidates that were allocated tokens.
    pub winners: usize,

    /// Tokens allocated (sum of all winner allocations).
    pub tokens_used: usize,

    /// Tokens remaining after allocation.
    pub tokens_remaining: usize,

    /// Total number of candidates submitted across all subsystems.
    pub total_candidates: usize,
}

/// A single bid in an auction round.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BidResult {
    /// Which subsystem submitted this bid.
    pub subsystem_id: SubsystemId,

    /// The context category of the candidate.
    pub category: ContextCategory,

    /// Raw expected value before modulation.
    pub expected_value: f64,

    /// Urgency multiplier applied.
    pub urgency: f64,

    /// Affect multiplier applied.
    pub affect_multiplier: f64,

    /// Carryover budget multiplier applied.
    pub carryover_multiplier: f64,

    /// Final bid: expected_value * urgency * affect_multiplier * carryover_multiplier.
    pub final_bid: f64,

    /// VCG payment (second price). 0.0 if this bid lost.
    pub payment: f64,

    /// Tokens requested by this candidate.
    pub tokens_requested: usize,

    /// Tokens allocated. 0 if this bid lost.
    pub tokens_allocated: usize,

    /// Whether this bid won the auction.
    pub won: bool,

    /// Brief description of the content for debugging.
    pub content_summary: String,
}
```

### Configuration parameters

| Parameter | Default | Range | Where |
|---|---|---|---|
| `t1_token_budget` | 4,000 | [2,000, 16,000] | `roko.toml` `[heartbeat.context]` |
| `t2_token_budget` | 32,000 | [8,000, 128,000] | `roko.toml` `[heartbeat.context]` |
| `affect_weight_arousal_safety` | 0.5 | [0.1, 1.0] | `roko.toml` `[heartbeat.auction]` |
| `affect_weight_dominance_explore` | 0.3 | [0.1, 0.8] | `roko.toml` `[heartbeat.auction]` |
| `affect_weight_pleasure_iteration` | 0.4 | [0.1, 0.8] | `roko.toml` `[heartbeat.auction]` |
| `carryover_decay` | 0.95 | [0.80, 0.99] | `roko.toml` `[heartbeat.auction]` |
| `carryover_max_debt` | -5.0 | [-10.0, -1.0] | `roko.toml` `[heartbeat.auction]` |
| `carryover_loser_credit` | 0.1 | [0.01, 0.5] | `roko.toml` `[heartbeat.auction]` |
| `scheduler_poll_interval` | 1s | [250ms, 5s] | `roko.toml` `[heartbeat.scheduler]` |
| `gamma_base_interval` | 10s | [2s, 60s] | `roko.toml` `[heartbeat.scheduler]` |
| `gamma_min_interval` | 2s | [500ms, 10s] | `roko.toml` `[heartbeat.scheduler]` |
| `gamma_max_interval` | 60s | [10s, 300s] | `roko.toml` `[heartbeat.scheduler]` |
| `meta_stuck_threshold` | 3 | [1, 10] | `roko.toml` `[heartbeat.meta]` |
| `meta_thrash_threshold` | 3 | [2, 8] | `roko.toml` `[heartbeat.meta]` |
| `meta_thrash_window` | 5 | [3, 10] | `roko.toml` `[heartbeat.meta]` |
| `meta_performance_decline_threshold` | -0.3 | [-0.8, -0.1] | `roko.toml` `[heartbeat.meta]` |

### Error handling

| Failure mode | Behavior |
|---|---|
| No candidates from any subsystem | Return empty context. Log warning. T1/T2 call proceeds with system prompt only. |
| All bids are zero or negative | Allocate budget proportionally by token count (round-robin fallback). Log warning. |
| Budget exceeded after assembly (token estimation was wrong) | Truncate the lowest-bid winning section until budget is met. Log the truncation. |
| CorticalState read returns NaN (corrupted atomic) | Use neutral PAD (0.0, 0.0, 0.0) for affect modulation. Log error. |
| Attention budget tracker state corrupted | Reset all subsystem balances to 0.0. Log error. |
| Frequency scheduler tick drift (system clock jump) | Clamp interval adjustments to 2x the previous interval. Detect drift by comparing wall-clock delta to expected delta. |
| Meta-cognition detects stuck + thrashing simultaneously | Stuck takes priority (Escalate). Thrashing Cooldown is deferred to the next theta tick. |

### Integration wiring

**VCG Auction:**
1. `ContextGovernor::assemble()` is called from the gamma tick handler in `orchestrate.rs` after tier selection.
2. Each registered `ContextBidder` implementation (one per subsystem) generates candidates.
3. `run_attention_auction()` runs the VCG mechanism.
4. Results are passed to `Composer.compose()` for final context assembly.
5. `AuctionRound` is persisted to `.roko/episodes.jsonl`.

**CorticalState:**
1. Allocated as `Arc<CorticalState>` in the runtime initialization (`main.rs` or `orchestrate.rs`).
2. Shared via `Arc` clone to all subsystems.
3. Each subsystem's writer task updates its signals at the documented frequency.
4. The frequency scheduler reads CorticalState for scheduling decisions.

**Meta-Cognition:**
1. `meta_cognize()` is called at the end of each theta tick.
2. `MetaCognitionResult` is checked for issues.
3. Each issue produces a `CognitiveSignal` dispatched through the event bus (`bardo-runtime`).
4. The frequency scheduler and tier gating consume `CognitiveSignal` events.

**Frequency Scheduler:**
1. Spawned as a `tokio::spawn` task during runtime initialization.
2. Reads CorticalState every `scheduler_poll_interval`.
3. Writes gamma/theta intervals to `AdaptiveClock`.
4. The gamma and theta loops read their intervals from `AdaptiveClock` at the start of each tick.

### Test criteria

| Test | Assertion |
|---|---|
| Single candidate within budget | Wins with payment = 0.0 (no competing bid) |
| Two candidates, budget fits both | Both win. Each pays the other's bid. |
| Two candidates, budget fits one | Higher bid wins. Pays the lower bid. |
| Affect modulation: arousal=1.0 boosts Safety 1.5x | Safety candidate bid = base_bid * 1.5 |
| Affect modulation: pleasure=-1.0 boosts IterationMemory 1.4x | Iteration memory bid = base_bid * 1.4 |
| Carryover: winner accumulates debt over 5 rounds | bid_multiplier < 1.0 after 5 consecutive wins |
| Carryover: loser accumulates credit over 5 rounds | bid_multiplier > 1.0 after 5 consecutive losses |
| Carryover: decay reduces balance by 5%/tick | balance_after = balance_before * 0.95 |
| Context governor: T0 budget = 0 | `assemble(T0, ...)` returns empty context |
| Context governor: T1 budget = 4000 | Total allocated tokens <= 4000 |
| Meta-cognition: 4 retries triggers Stuck | `issues` contains `MetaIssue::Stuck` |
| Meta-cognition: 4 approach changes in 5 attempts triggers Thrashing | `issues` contains `MetaIssue::Thrashing` |
| Gamma interval: anomaly_rate=0.0 -> 60s | `compute_gamma_interval` returns 60s |
| Gamma interval: anomaly_rate=1.0 -> 2s | `compute_gamma_interval` returns 2s |
| AuctionRound serializes/deserializes | Round-trip through serde_json preserves all fields |
| CorticalState: PAD round-trips through atomic store/load | `pad()` returns the same values written via `set_pad()` |

---

## Current Status and Gaps

**What exists:**
- `Composer` trait in `roko-core` with budget-constrained composition.
- `SystemPromptBuilder` in `roko-compose` with 6-layer context assembly.
- `RoleSystemPromptSpec` for template-based context construction.
- Basic priority ordering in the current context assembly.

**What is missing:**
- VCG auction mechanism for context allocation.
- `ContextBidder` trait for subsystem participation.
- Affect-modulated bidding weights.
- Attention budget carryover between ticks.
- CorticalState shared perception surface.
- Meta-cognition hook (I5 from 12a-cognitive-layer.md).
- Frequency scheduler (I4 from 12a-cognitive-layer.md).
- `CognitiveSignal` dispatch between loops.

---

## Cross-References

- See [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md) for INTEGRATE step where the auction runs
- See [07-adaptive-clock.md](./07-adaptive-clock.md) for the clock the frequency scheduler adjusts
- See [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) for tier-based token budgets
- See [10-active-inference-compute-allocation.md](./10-active-inference-compute-allocation.md) for EFE-based allocation
- See topic [09-daimon](../09-daimon/INDEX.md) for the PAD vectors that modulate bidding
- See topic [03-composition](../03-composition/INDEX.md) for the Composer trait and context engineering
