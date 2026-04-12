# Attention Auction, Context Governor, and CorticalState

> VCG truthful bidding for limited context budget, the shared perception surface, meta-cognition hooks, and the frequency scheduler.

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

### Bidding Subsystems

Eight subsystems compete for context budget:

| Subsystem | What It Bids | Bid Basis |
|---|---|---|
| **Neuro** | Knowledge entries (insights, heuristics, warnings) | Relevance score × confidence × recency |
| **Daimon** | Affect state and behavioral context | Arousal level × affect magnitude |
| **Iteration Memory** | Past failures and fixes for current task | Failure count × relevance to current task |
| **Code Intelligence** | Symbol graphs, type signatures, dependency info | Coverage of referenced symbols |
| **Playbook Rules** | Applicable learned heuristics | Rule confidence × situation match score |
| **Research Artifacts** | Pre-computed analyses, literature reviews | Relevance to current domain/task |
| **Task Context** | PRD sections, plan details, requirements | Always high base bid (task-defining) |
| **Oracle Predictions** | Relevant predictions and calibration data | Prediction confidence × relevance |

### Affect Modulation

The Daimon PAD vector biases bidding:

- **High arousal** → urgency multiplier on safety-related sections. An anxious agent (low pleasure, high arousal) prioritizes warnings and risk assessments.
- **Low dominance** → boost for exploratory/research sections. An uncertain agent (low dominance) seeks more diverse context.
- **Low pleasure** → boost for iteration memory (past failure context). An agent experiencing failures prioritizes understanding what went wrong.

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

            // Affect modulation
            let affect_mult = match c.category {
                ContextCategory::Safety => {
                    1.0 + pad.arousal.abs() * 0.5  // High arousal → safety priority
                }
                ContextCategory::Exploration => {
                    1.0 + (1.0 - pad.dominance) * 0.3  // Low dominance → explore
                }
                ContextCategory::IterationMemory => {
                    1.0 + (-pad.pleasure).max(0.0) * 0.4  // Low pleasure → review failures
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

### Attention Budget Carryover

The "payment" each winner makes is deducted from its subsystem's attention budget for the next tick. This creates a dynamic balancing effect:

- A subsystem that wins many auctions accumulates "debt" and bids lower on future ticks.
- A subsystem that loses auctions accumulates "credit" and bids higher, increasing its chances.
- Over time, attention is distributed proportionally to actual value contributed.

---

## The Context Governor

The context governor manages the overall token budget for each tick tier:

| Tier | Token Budget | Context Strategy |
|---|---|---|
| T0 | 0 tokens | No context assembly (pure probe + playbook) |
| T1 | ~4,000 tokens | Focused: task + top-5 retrieved + critical warnings |
| T2 | ~32,000 tokens | Full: VCG auction allocates across all 8 subsystems |

The context governor enforces the budget by:
1. Setting the total budget based on tier.
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

Meta-cognition produces `CognitiveSignal` events when issues are detected:
- Stuck → `CognitiveSignal::Escalate` (switch to stronger model)
- Thrashing → `CognitiveSignal::Cooldown` (reduce frequency, commit to approach)
- Performance decline → `CognitiveSignal::Escalate` or intervention request
- Complacency → `CognitiveSignal::Explore` (seek novel territory)

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
