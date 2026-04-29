# 24. Attention Auction and CorticalState

> The VCG attention auction as a Compose Cell that allocates context budget across competing bidder subsystems. CorticalState as the 32-signal atomic shared perception surface. Section effects close the feedback loop.

See [02-CELL.md](../../unified/02-CELL.md) for Compose protocol, [05-AGENT.md](../../unified/05-AGENT.md) for Agent runtime, [01-SIGNAL.md](../../unified/01-SIGNAL.md) for Signal structure.

---

## 1. The Problem: Context is Scarce

An Agent's context window is its most constrained resource. A T2 tick assembles ~32,000 tokens from competing sources: knowledge store entries, playbook rules, somatic markers, conversation history, dream hypotheses, domain signals, iteration memory, and task context. Every token not included is information the model cannot reason about.

Naive allocation (fixed priority ordering, round-robin) wastes this resource. The Agent needs a mechanism that:
1. Allocates tokens to sections that contribute most to the current task's success.
2. Incentivizes truthful reporting of each section's value (no gaming).
3. Adapts over time based on observed outcomes.
4. Operates under strict budget constraints.

The Vickrey-Clarke-Groves (VCG) auction (Vickrey 1961, Clarke 1971, Groves 1973) provides exactly this: a truthful mechanism where each bidder reports its true value because misreporting can only hurt its allocation.

---

## 2. The Auction as a Compose Cell

The attention auction is a Cell implementing the Compose protocol. It takes bidder signals as input and produces an allocated context Signal as output:

```rust
/// VCG Attention Auction: Compose Cell that allocates context budget.
///
/// Each bidder submits: (content, bid_value, min_tokens, max_tokens).
/// The auction maximizes total value under the token budget constraint.
/// VCG pricing ensures truthful bidding.
///
/// Crate: `crates/roko-compose/src/attention_auction.rs`
pub struct AttentionAuction {
    budget_tokens: u32,          // Total token budget for this tick
    section_effects: SectionEffectTracker,  // Beta(a,b) per bidder
}

impl Compose for AttentionAuction {
    type Input = Vec<Bid>;
    type Output = AllocatedContext;

    fn compose(&self, bids: Vec<Bid>, budget: &Budget) -> AllocatedContext {
        let effective_budget = budget.tokens.min(self.budget_tokens);

        // 1. Sort bids by value density (bid / tokens_requested)
        let mut ranked: Vec<_> = bids.iter()
            .map(|b| (b, b.value / b.max_tokens as f64))
            .collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // 2. Greedy allocation under budget
        let mut allocated = Vec::new();
        let mut remaining = effective_budget;

        for (bid, _density) in &ranked {
            if remaining < bid.min_tokens {
                continue;
            }
            let tokens = bid.max_tokens.min(remaining);
            allocated.push(Allocation {
                bidder: bid.bidder.clone(),
                tokens,
                content: bid.content.truncate_to(tokens),
                vcg_price: 0.0, // computed below
            });
            remaining -= tokens;
        }

        // 3. Compute VCG prices (what others lost by this bidder's presence)
        for i in 0..allocated.len() {
            allocated[i].vcg_price = self.compute_vcg_price(
                &bids, &allocated, i, effective_budget,
            );
        }

        AllocatedContext {
            sections: allocated,
            total_tokens: effective_budget - remaining,
            budget_utilization: (effective_budget - remaining) as f64
                / effective_budget as f64,
        }
    }
}
```

---

## 3. Eight Bidder Subsystems

Each bidder is a Cell that computes its bid value based on the current task context. Bid values are modulated by CorticalState readings:

| Bidder | What It Provides | Typical Bid | When It Bids High |
|---|---|---|---|
| **Neuro** | Knowledge store entries (insights, heuristics, warnings) | 0.3-0.8 | High similarity to current task |
| **Playbook** | Machine-evolved rules from dream consolidation | 0.5-0.9 | Rule condition matches current situation |
| **Somatic** | Affect-tagged memories (prospect markers, PAD annotations) | 0.2-0.6 | High arousal or strong prior association |
| **Pheromone** | Mesh coordination signals from peer agents | 0.1-0.4 | Threat/opportunity signals active |
| **Dream** | Hypotheses from REM imagination (low confidence) | 0.1-0.3 | Hypothesis relevant to current phase |
| **Iteration** | Recent turn history within current task | 0.4-0.7 | Multi-turn task with cumulative context |
| **Conversation** | Human chat tail (if user is present) | 0.6-0.9 | User actively chatting |
| **Domain** | Domain-specific signals (prices, build status, etc.) | 0.3-0.7 | Regime is Volatile or Crisis |

### 3.1 Bid Computation

Each bidder implements a standard interface:

```rust
/// A context bidder: computes its bid for the current tick.
///
/// The bid value represents the expected marginal contribution
/// to gate-pass probability if this section is included.
pub trait AttentionBidder: Cell {
    /// Compute bid for the current tick context.
    fn bid(&self, task: &TaskContext, cortical: &CorticalState) -> Bid;
}

pub struct Bid {
    pub bidder: BidderId,
    pub content: Signal,       // The actual content to include
    pub value: f64,            // Expected contribution to success [0, 1]
    pub min_tokens: u32,       // Minimum useful allocation
    pub max_tokens: u32,       // Maximum useful allocation
}
```

### 3.2 Affect Modulation of Bids

The Daimon's behavioral state modulates bids via a Functor (see [18-affect-as-functor.md](18-affect-as-functor.md)):

| Behavioral State | Modulation |
|---|---|
| Struggling | Somatic bids +30%, Dream bids -20% (prefer safety over speculation) |
| Exploring | Dream bids +40%, Playbook bids -10% (prefer novelty) |
| Focused | Iteration bids +20%, Pheromone bids -30% (minimize distraction) |
| Coasting | Domain bids +20% (stay alert despite low arousal) |

---

## 4. CorticalState: The Shared Perception Surface

CorticalState is a lock-free atomic struct containing 32 typed signals. Any Cell can read it at any time with zero blocking. Probes write to it every gamma tick. The Daimon writes affect fields. The BudgetTracker writes resource fields.

```rust
/// CorticalState: 32-signal atomic shared perception surface.
///
/// Design: lock-free reads via atomics. Writers are well-defined
/// (one writer per field, multiple readers). No contention.
///
/// Crate: `crates/roko-core/src/cortical.rs`
#[repr(C, align(128))]  // cache-line aligned
pub struct CorticalState {
    // === Regime & Environment (written by RegimeDetector) ===
    pub regime: AtomicU8,                   // Calm/Normal/Volatile/Crisis
    pub prediction_error: AtomicF32,        // [0.0, 1.0]
    pub anomaly_count: AtomicU32,           // 0-16
    pub last_tier: AtomicU8,                // T0/T1/T2

    // === Accuracy & Performance (written by CalibrationTracker) ===
    pub accuracy: AtomicF32,                // [0.0, 1.0] recent prediction accuracy
    pub gate_pass_rate: AtomicF32,          // [0.0, 1.0] recent gate pass rate
    pub cost_efficiency: AtomicF32,         // $ per successful outcome

    // === Resource Health (written by BudgetTracker) ===
    pub resource_health: AtomicF32,         // [0.0, 1.0] budget remaining
    pub daily_spend_fraction: AtomicF32,    // [0.0, 1.0]
    pub vitality: AtomicF32,               // remaining_budget / initial_budget

    // === Affect (written by Daimon) ===
    pub pleasure: AtomicF32,                // [-1.0, 1.0] PAD P axis
    pub arousal: AtomicF32,                 // [-1.0, 1.0] PAD A axis
    pub dominance: AtomicF32,              // [-1.0, 1.0] PAD D axis
    pub behavioral_state: AtomicU8,         // Engaged/Struggling/Coasting/...

    // === Cognitive State (written by theta/delta) ===
    pub sleep_pressure: AtomicF32,          // [0.0, 1.0] accumulated toward delta
    pub causal_consistency: AtomicF32,      // [0.0, 1.0] lineage DAG health
    pub world_model_drift: AtomicF32,       // [0.0, 1.0] predicted vs actual

    // === Task Context (written by orchestrator) ===
    pub task_phase: AtomicU8,               // Understanding/Planning/.../Complete
    pub context_quality: AtomicU8,          // None/Insufficient/.../Comprehensive
    pub uncertainty: AtomicU8,              // High/Medium/Low
    pub stuck_count: AtomicU32,             // retries on current task

    // === Coordination (written by Mesh) ===
    pub threat_intensity: AtomicF32,        // [0.0, 1.0] pheromone field
    pub opportunity_intensity: AtomicF32,   // [0.0, 1.0]
    pub peer_count: AtomicU32,             // active mesh peers

    // === Clock (written by HeartbeatPolicy) ===
    pub gamma_interval_ms: AtomicU32,       // current gamma interval
    pub theta_interval_ms: AtomicU32,       // current theta interval
    pub ticks_since_theta: AtomicU32,       // gamma ticks since last theta

    // === Meta (written by various) ===
    pub uptime_secs: AtomicU64,            // seconds since Agent start
    pub total_episodes: AtomicU64,         // lifetime episode count
    pub last_updated_ms: AtomicU64,        // epoch millis of last write
}
```

### 4.1 Read Pattern

Any Cell reads CorticalState fields atomically with `Ordering::Relaxed` (eventual consistency within microseconds on modern hardware):

```rust
// Zero-cost read from any Cell
let regime = cortical.regime.load(Ordering::Relaxed);
let error = cortical.prediction_error.load(Ordering::Relaxed);
```

### 4.2 Write Discipline

Each field has exactly one writer. No field is written by multiple Cells. This eliminates contention without locks:

| Field Category | Writer |
|---|---|
| Regime & Environment | ProbeRegistry (gamma tick) |
| Accuracy & Performance | CalibrationTracker (theta tick) |
| Resource Health | BudgetTracker (continuous) |
| Affect | Daimon (gamma META-COGNIZE step) |
| Cognitive State | Orchestrator (task transitions) |
| Coordination | Mesh relay client (on receive) |
| Clock | HeartbeatPolicy (on interval change) |

---

## 5. Section Effects: Closing the Feedback Loop

The auction needs to learn which sections actually contribute to success. Section effects track, per bidder, the correlation between inclusion and gate-pass:

```rust
/// Section effect tracker: Beta distribution per bidder.
///
/// For each bidder, tracks:
///   successes = number of times the section was included AND gate passed
///   failures  = number of times the section was included AND gate failed
///
/// The posterior mean Beta(a, b) = a / (a + b) estimates the
/// probability that including this section leads to gate success.
///
/// Crate: `crates/roko-compose/src/section_effects.rs`
pub struct SectionEffectTracker {
    effects: HashMap<BidderId, BetaDistribution>,
}

pub struct BetaDistribution {
    pub alpha: f64,  // successes + prior
    pub beta: f64,   // failures + prior
}

impl BetaDistribution {
    pub fn new() -> Self {
        Self { alpha: 1.0, beta: 1.0 } // flat prior
    }

    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    pub fn update(&mut self, success: bool) {
        if success {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }
    }

    /// Thompson sampling: draw from posterior to explore
    pub fn sample(&self) -> f64 {
        // Beta(alpha, beta) sampling via Kumaraswamy approximation
        let u: f64 = rand::random();
        u.powf(1.0 / self.alpha) / (u.powf(1.0 / self.alpha)
            + (1.0 - u).powf(1.0 / self.beta))
    }
}

impl SectionEffectTracker {
    /// After a gate verdict, update all included sections.
    pub fn update(&mut self, included: &[BidderId], gate_passed: bool) {
        for bidder in included {
            self.effects
                .entry(bidder.clone())
                .or_insert_with(BetaDistribution::new)
                .update(gate_passed);
        }
    }

    /// Get the effect estimate for a bidder (used to modulate bids).
    pub fn effect(&self, bidder: &BidderId) -> f64 {
        self.effects
            .get(bidder)
            .map(|b| b.mean())
            .unwrap_or(0.5) // uninformative prior
    }
}
```

### 5.1 The Feedback Loop

```
Bidders submit bids
    |
    v
Auction allocates tokens (Compose Cell)
    |
    v
Context assembled -> LLM inference -> Action taken
    |
    v
Gate pipeline runs (Verify Cell)
    |
    v
Verdict (pass/fail) + which sections were included
    |
    v
SectionEffectTracker.update(included, passed)
    |
    v
Next tick: section effects modulate bid values
    |
    v (loop)
Bidders submit adjusted bids
```

Over time, sections that reliably correlate with gate success get higher effective bids (their effect multiplier is > 0.5). Sections that show no correlation drift toward 0.5 (neutral). Sections that anti-correlate (presence correlates with failure) get reduced bids.

---

## 6. Budget Tiers by Cognitive Speed

| Speed | Token Budget | Bidders Active | Rationale |
|---|---|---|---|
| Gamma T1 | ~4,000 tokens | Iteration, Domain, Playbook | Fast response, surgical context |
| Gamma T2 | ~32,000 tokens | All 8 bidders | Full deliberation |
| Theta | ~8,000 tokens | Iteration, Domain, Neuro | Reflection, not action |
| Delta | ~16,000 tokens | Neuro, Dream, Playbook | Consolidation, synthesis |

The budget for each speed is configurable:

```toml
[compose.auction]
gamma_t1_budget_tokens = 4000
gamma_t2_budget_tokens = 32000
theta_budget_tokens = 8000
delta_budget_tokens = 16000
```

---

## 7. VCG Pricing (Why Truthful Bidding)

The VCG price for bidder i is: "the total value other bidders lost because i was included." This means:

- If bidder i is replaced by something equally good, its price is high (others lost nothing; i gained a lot).
- If bidder i uniquely contributes value no one else can provide, its price is low (removing i would hurt everyone).

The key property: **no bidder can increase its allocation by inflating its bid.** Inflating forces you to pay more (VCG price increases) without gaining additional allocation. This makes the mechanism **incentive-compatible** -- bidders report true values.

For context allocation, the "price" is not monetary. It is a signal that the auction uses to weight future bids and to report to the Lens system for observability.

---

## What This Enables

- **Optimal context assembly**: The token budget goes to whoever will contribute most to the current task's success, proven by tracked section effects.
- **Self-improving allocation**: The Beta-distribution tracker learns which sections matter, adapting the allocation strategy over time without manual tuning.
- **Truthful bidding**: VCG pricing eliminates gaming -- no subsystem benefits from exaggerating its importance.
- **Affect-modulated attention**: The Daimon can shift attention (via bid modulation) based on emotional state, implementing Bower's mood-congruent retrieval.
- **Zero-latency coordination**: CorticalState gives all subsystems instant access to the Agent's current perceptual state without polling or querying.

## Feedback Loops

1. **Section effects -> bid modulation -> allocation -> gate outcome -> section effects** (Loop): The primary learning loop. Sections prove their worth through gate outcomes.
2. **CorticalState -> bidder modulation -> context quality -> task outcome -> CorticalState** (Loop): Perceptual state influences what context is assembled, which influences outcomes, which update the perceptual state.
3. **Budget pressure -> reduced token budget -> higher bid threshold -> only highest-value sections included -> potentially better signal-to-noise** (emergent): Budget pressure can paradoxically improve quality by forcing focus.

## Open Questions

1. Should section effects decay over time (recent outcomes weighted more than old ones)?
2. Should the auction support "package bids" (e.g., Neuro + Playbook together are worth more than separately)?
3. How should CorticalState handle the cold-start problem (no history for new Agents)?
4. Should there be a minimum guaranteed allocation per bidder to prevent starvation?

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| Define `AttentionAuction` Compose Cell | `crates/roko-compose/src/attention_auction.rs` | Partial (`vcg_allocate` exists) |
| Define `AttentionBidder` trait | `crates/roko-compose/src/bidder.rs` | Not started |
| Implement 8 bidder Cells | `crates/roko-compose/src/bidders/` | Partial (Neuro/Task/Research in orchestrate.rs) |
| Define `CorticalState` struct | `crates/roko-core/src/cortical.rs` | Not started |
| Implement `SectionEffectTracker` | `crates/roko-compose/src/section_effects.rs` | Not started |
| Wire auction into orchestrate.rs COMPOSE step | `crates/roko-cli/src/orchestrate.rs` | Not started |
| Add section effect feedback from gate verdicts | `crates/roko-cli/src/orchestrate.rs` | Not started |
| CorticalState persistence/restore | `crates/roko-core/src/cortical.rs` | Not started |
