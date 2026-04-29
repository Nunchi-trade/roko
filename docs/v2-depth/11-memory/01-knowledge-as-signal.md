# Knowledge as Signal

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). How knowledge types, tiers, and demurrage emerge from Signal primitives rather than requiring a separate knowledge subsystem.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal struct, Kind system, demurrage balance, HDC fingerprint), [02-CELL](../../unified/02-CELL.md) (Store protocol, Verify protocol, Score protocol, predict-publish-correct), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, Loop, Trigger), [demurrage-economics](../01-signal/demurrage-economics.md) (rate law, phase space, tier transitions as Markov chain), [scoring-and-calibration](../01-signal/scoring-and-calibration.md) (Score-Verify-Score loop, temperature scaling)

---

## 1. The Redundancy Problem

The original Neuro system maintained its own type hierarchy (`KnowledgeKind`), its own retention mechanism (`KnowledgeTier` with multipliers), its own decay model (Ebbinghaus half-life with tier adjustments), and its own persistence layer (`KnowledgeStore` backed by JSONL). Each of these duplicated machinery that the unified primitives already provide:

| Neuro concept | Unified primitive that subsumes it |
|---|---|
| `KnowledgeKind` (6 variants) | Signal `Kind` (extensible, content-addressed) |
| `KnowledgeTier` (4 levels) | Store retention policy (tier multiplier on demurrage rate) |
| `half_life_days` per entry | Signal `balance` + demurrage rate law |
| `confidence` field | Signal `Score.confidence` axis |
| `KnowledgeStore` (JSONL file) | Store protocol Cell (`.roko/signals.jsonl`) |
| `hdc_vector: Option<Vec<u8>>` | Signal `hdc_fingerprint: HdcVector` (always present) |
| `frozen: bool` | Signal tier `Frozen` + cold Store backend |

The goal is not to remove the Neuro crate. The goal is to show that every Neuro concept is a *specialization* of a Signal concept, not an independent invention. When knowledge types are Signal Kinds, the same demurrage engine, the same Store protocol, and the same Verify pipeline serve knowledge without any knowledge-specific machinery.

---

## 2. Knowledge Types Are Signal Kinds

Signal carries a `Kind` enum. Knowledge types map directly into it. No wrapper, no adapter, no second type system.

```rust
/// Unified Kind system. Knowledge types are first-class variants,
/// not a separate enum that must be bridged.
///
/// Each variant declares its own default demurrage rates (flat tax `r`
/// and exponential decay `beta`) so the demurrage engine does not need
/// a knowledge-specific lookup table -- it reads rates from the Kind.
#[non_exhaustive]
pub enum Kind {
    // --- Structural Kinds (non-knowledge) ---
    Text,
    Code,
    Task,
    Verdict,
    Episode,
    Config,
    Compound(Vec<Kind>),

    // --- Knowledge Kinds ---
    /// Compact causal observation distilled from episodes.
    /// Base half-life: ~35 days (beta = 0.02).
    Insight,

    /// Behavioral rule with when/then clauses and a mandatory falsifier.
    /// Base half-life: ~69 days (beta = 0.01).
    Heuristic,

    /// Danger signal. Short-lived by design.
    /// Base half-life: ~3.5 days (beta = 0.20).
    Warning,

    /// Directed cause-effect relationship between two Signals.
    /// Base half-life: ~87 days (beta = 0.008).
    CausalLink,

    /// Reusable approach fragment, composable into plans.
    /// Base half-life: ~23 days (beta = 0.03).
    StrategyFragment,

    /// Negative knowledge: what NOT to do.
    /// Base half-life: ~35 days (beta = 0.02). Floor balance 0.30.
    AntiKnowledge,

    /// Emergent co-citation cluster of heuristics.
    /// Base half-life: ~69 days (beta = 0.01, half standard rate).
    Worldview,
}
```

### 2.1 Why Kind Carries Demurrage Rates

Each Kind declares two constants: `r` (flat tax per day) and `beta` (exponential decay per day). The demurrage engine reads these from the Signal's Kind at tick time. This eliminates the need for a lookup table indexed by knowledge type -- the rates travel with the data.

```rust
impl Kind {
    /// Default flat demurrage tax per day for this Kind.
    pub const fn default_flat_tax(&self) -> f64 {
        match self {
            Kind::Warning           => 0.100,
            Kind::StrategyFragment  => 0.020,
            Kind::Insight           => 0.010,
            Kind::AntiKnowledge     => 0.010,
            Kind::Heuristic         => 0.005,
            Kind::CausalLink        => 0.007,
            Kind::Episode           => 0.005,
            Kind::Worldview         => 0.005,
            Kind::Verdict           => 0.002,
            Kind::Text | Kind::Code => 0.001,
            _                       => 0.010, // sensible default
        }
    }

    /// Default exponential decay rate per day for this Kind.
    pub const fn default_exp_decay(&self) -> f64 {
        match self {
            Kind::Warning           => 0.200,
            Kind::StrategyFragment  => 0.030,
            Kind::Insight           => 0.020,
            Kind::AntiKnowledge     => 0.020,
            Kind::Heuristic         => 0.010,
            Kind::CausalLink        => 0.008, // was 0.017 in older spec
            Kind::Episode           => 0.010,
            Kind::Worldview         => 0.010,
            Kind::Verdict           => 0.003,
            Kind::Text | Kind::Code => 0.001,
            _                       => 0.020,
        }
    }

    /// Unreinforced half-life: ln(2) / beta.
    /// This is the time for balance to halve with zero reinforcement
    /// and zero flat tax.
    pub fn unreinforced_half_life_days(&self) -> f64 {
        f64::ln(2.0) / self.default_exp_decay()
    }

    /// Whether this Kind has a minimum balance floor that prevents
    /// full decay (e.g., AntiKnowledge must remain retrievable).
    pub const fn balance_floor(&self) -> Option<f64> {
        match self {
            Kind::AntiKnowledge => Some(0.30),
            _ => None,
        }
    }
}
```

### 2.2 The Derived Half-Life Table

The half-life table is not a configuration artifact. It is a derived consequence of each Kind's `beta` constant. No separate table needs to be maintained or synchronized.

| Kind | beta | Half-life (days) | Interpretation |
|---|---|---|---|
| Warning | 0.200 | 3.5 | Danger signal; expires in under a week |
| StrategyFragment | 0.030 | 23 | Approaches go stale in evolving codebases |
| Insight | 0.020 | 35 | Observations need periodic confirmation |
| AntiKnowledge | 0.020 | 35 | Mistakes stay relevant about a month (floor prevents full decay) |
| Heuristic | 0.010 | 69 | Proven rules persist for ~2 months |
| Episode | 0.010 | 69 | Raw learning data persists ~2 months |
| Worldview | 0.010 | 69 | Aggregate knowledge matches Heuristic durability |
| CausalLink | 0.008 | 87 | Causal models need time to be tested |
| Verdict | 0.003 | 231 | Audit evidence persists most of a year |
| Text/Code | 0.001 | 693 | Structural data persists ~2 years |

These are unreinforced half-lives. Any reinforcement extends them. A heavily cited Heuristic can persist indefinitely despite a 69-day base half-life. The flat tax `r` ensures that even at very low balance, there is a constant drain preventing zombie Signals from hovering near zero forever.

---

## 3. Tiers Are Store-Level Retention Policies

The four-tier system (Transient, Working, Consolidated, Persistent) is not a knowledge-layer concept. It is a **Store-level retention policy** that modulates demurrage parameters. Any Signal in any Store can carry a tier. Knowledge Signals happen to use tiers heavily because knowledge lifetimes vary by orders of magnitude.

```rust
/// Retention tier. Stored on the Signal, enforced by the Store protocol.
///
/// Tiers are orthogonal to Kind. A Warning can be Persistent (rare but
/// possible: a permanent safety constraint). An Insight can be Transient
/// (just arrived, not yet validated). The 6x4 matrix of Kind x Tier
/// emerges naturally from two independent metadata fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tier {
    /// Just arrived. High charge multiplier pushes toward quick
    /// promotion or quick death. No comfortable resting state.
    Transient,
    /// Actively used working memory. Moderate charge.
    Working,
    /// Cross-validated, reliable. Low charge, broad reinforcement.
    Consolidated,
    /// Bedrock knowledge. Near-zero charge. Requires contradiction
    /// to dislodge.
    Persistent,
    /// Below cold threshold. Body archived, hash retained.
    Frozen,
}

impl Tier {
    /// Multiplier on the demurrage charge rate.
    /// Higher multiplier = faster decay.
    ///
    /// Transient Signals pay 2x the base rate (unstable, must prove
    /// themselves quickly). Persistent Signals pay 0.1x (deeply stable).
    pub const fn charge_multiplier(&self) -> f64 {
        match self {
            Tier::Transient    => 2.0,
            Tier::Working      => 1.0,
            Tier::Consolidated => 0.5,
            Tier::Persistent   => 0.1,
            Tier::Frozen       => 0.0, // no charge while frozen
        }
    }

    /// Multiplier on reinforcement bonuses.
    /// Transient Signals earn more per reinforcement (bootstrapping).
    /// Persistent Signals earn less (they do not need it).
    pub const fn reinforcement_multiplier(&self) -> f64 {
        match self {
            Tier::Transient    => 1.5,
            Tier::Working      => 1.0,
            Tier::Consolidated => 0.75,
            Tier::Persistent   => 0.5,
            Tier::Frozen       => 0.0,
        }
    }

    /// Balance band for this tier. A Signal whose balance falls
    /// outside its tier's band is a candidate for promotion or demotion.
    pub const fn balance_band(&self) -> (f64, f64) {
        match self {
            Tier::Transient    => (0.00, 0.35),
            Tier::Working      => (0.35, 0.80),
            Tier::Consolidated => (0.80, 1.20),
            Tier::Persistent   => (1.20, f64::INFINITY),
            Tier::Frozen       => (0.00, 0.00),
        }
    }
}
```

### 3.1 The 6x4 Matrix Emerges From Two Orthogonal Fields

The effective half-life of any knowledge Signal is determined by exactly two fields on the Signal struct: `kind` and `tier`. No third parameter is needed.

```
effective_half_life(signal) = ln(2) / (signal.kind.default_exp_decay()
                                       * signal.tier.charge_multiplier())
```

This produces a 6x4 matrix:

| Kind \ Tier | Transient (2.0x) | Working (1.0x) | Consolidated (0.5x) | Persistent (0.1x) |
|---|---|---|---|---|
| **Warning** (beta=0.20) | 1.7 days | 3.5 days | 6.9 days | 34.7 days |
| **StrategyFragment** (beta=0.03) | 11.6 days | 23.1 days | 46.2 days | 231 days |
| **Insight** (beta=0.02) | 17.3 days | 34.7 days | 69.3 days | 347 days |
| **Heuristic** (beta=0.01) | 34.7 days | 69.3 days | 139 days | 693 days |
| **CausalLink** (beta=0.008) | 43.3 days | 86.6 days | 173 days | 866 days |
| **Verdict** (beta=0.003) | 115 days | 231 days | 462 days | 2,310 days |

No configuration file stores this table. It is a derived consequence of the Kind's `beta` divided by the Tier's charge multiplier. If you change `beta` on Insight, every Insight at every tier immediately adjusts. If you change the Transient charge multiplier, every Kind at Transient immediately adjusts.

The old system stored `half_life_days` as a per-entry field and multiplied it by `KnowledgeTier::multiplier()`. The redesign eliminates that field: the half-life is computed, not stored.

### 3.2 Why Tiers Are Not Redundant With Demurrage

One might ask: if demurrage already controls retention through the balance field, why have tiers at all? Could balance alone handle everything?

Balance alone cannot express **asymmetric promotion and demotion**. The tier system encodes a qualitative judgment that demurrage economics alone cannot make:

1. **Promotion requires evidence.** A Signal cannot promote from Transient to Working just by having a high balance. It must also pass Verify Cells -- a qualitative gate that the balance mechanism cannot enforce. Balance is necessary but not sufficient for promotion.

2. **Demotion is asymmetric.** Demotion from Persistent requires contradiction evidence, not just low balance. A Persistent Signal with slowly declining balance (because it is rarely retrieved) should not be demoted -- it may simply be foundational knowledge that is assumed rather than cited. Contradiction is a qualitative signal that the tier system can interpret but the balance field cannot.

3. **Charge multipliers create basins of attraction.** The 2.0x charge on Transient makes it an unstable equilibrium: Signals either promote quickly or die. The 0.1x charge on Persistent makes it deeply stable. These dynamics are impossible to reproduce with a single balance field and a single decay rate.

The tier is a **phase variable** in the dynamical system. The balance is a **continuous state variable**. Both are needed because knowledge retention involves both continuous dynamics (gradual decay) and discrete phase transitions (promotion/demotion).

---

## 4. Demurrage Is the Signal Primitive's Decay Mechanism

Demurrage is not a Neuro-specific feature bolted onto Signals for the knowledge use case. Every Signal in Store carries a `balance` and pays demurrage. Knowledge Signals simply make the most visible use of it.

The full derivation appears in [demurrage-economics.md](../01-signal/demurrage-economics.md). The key points for the knowledge context:

### 4.1 The Unified Demurrage Tick

The Store protocol Cell runs a periodic demurrage tick on all Signals it holds. The tick does not distinguish knowledge Signals from other Signals. It reads `kind.default_flat_tax()`, `kind.default_exp_decay()`, `tier.charge_multiplier()`, and `tier.reinforcement_multiplier()` from the Signal's own metadata.

```rust
/// Demurrage tick applied by the Store protocol Cell.
///
/// This function knows nothing about knowledge types. It reads rates
/// from the Signal's Kind and Tier, which are part of the Signal
/// primitive itself.
pub fn demurrage_tick(
    signal: &mut Signal,
    dt_days: f64,
    novelty: f64,
    reinforcement: Option<ReinforceKind>,
) {
    let r = signal.kind.default_flat_tax();
    let beta = signal.kind.default_exp_decay();
    let charge_mult = signal.tier.charge_multiplier();
    let reinforce_mult = signal.tier.reinforcement_multiplier();

    // 1. Charge: flat tax + proportional drain, scaled by tier
    let flat_charge = r * dt_days * charge_mult;
    let prop_charge = beta * signal.balance * dt_days * charge_mult;
    signal.balance -= flat_charge + prop_charge;
    signal.demurrage_paid += flat_charge + prop_charge;

    // 2. Reinforce if earned
    if let Some(kind) = reinforcement {
        let base_bonus = kind.bonus();
        signal.balance += base_bonus * novelty * reinforce_mult;
    }

    // 3. Kind-specific floor enforcement (e.g., AntiKnowledge floor)
    if let Some(floor) = signal.kind.balance_floor() {
        signal.balance = signal.balance.max(floor);
    }

    // 4. Global cold threshold
    if signal.balance < COLD_THRESHOLD && signal.tier != Tier::Frozen {
        // Candidate for cold storage archival
    }

    signal.last_touched_at = chrono::Utc::now();
}
```

The function has no `match` on knowledge types. It has no `if signal.kind.is_knowledge()` branch. A Verdict Signal, a Config Signal, and an Insight Signal all flow through the same code path. The *behavior* differs because the *rates* differ, and the rates are declared on the Kind.

### 4.2 Ebbinghaus as Degenerate Case

The old Neuro system used Ebbinghaus decay: `weight = 2^(-age / half_life)`. This is recovered from the demurrage model when flat tax `r = 0` and no reinforcement occurs:

```
balance(t) = balance_0 * exp(-beta * t)
```

Which is equivalent to `2^(-t / half_life)` when `half_life = ln(2) / beta`.

Demurrage generalizes Ebbinghaus by adding two mechanisms that Ebbinghaus lacks:

1. **Flat tax** (`r`): ensures even very low-balance Signals continue to drain, preventing accumulation of near-zero zombie entries.
2. **Reinforcement**: active use restores balance, creating an economic feedback loop where useful knowledge persists and unused knowledge decays.

The old `half_life_days` field on `KnowledgeEntry` is superseded by computing `ln(2) / (kind.default_exp_decay() * tier.charge_multiplier())` on the fly. The stored value is `balance`, not `half_life_days`.

---

## 5. Tier Promotion Is Predict-Publish-Correct on the Store Protocol

The predict-publish-correct (PPC) pattern from [02-CELL.md](../../unified/02-CELL.md) is the universal learning mechanism: every Cell predicts its output, publishes the prediction as a Pulse, and then the Verify protocol compares prediction to ground truth. Tier promotion fits this pattern exactly.

### 5.1 The Tier Transition as a Verify Cell

A `TierVerifier` Cell implements the Verify protocol. It takes a Signal and a `TierTransitionProposal` as input, runs the appropriate evidence check, and produces a binary verdict.

```rust
/// A Verify Cell that evaluates tier transition proposals.
///
/// This Cell implements the pattern:
///   1. Predict: the current tier implies an expected balance trajectory.
///   2. Observe: actual balance, gate results, and confirmation evidence.
///   3. Decide: promote, demote, or hold.
///
/// By making tier transitions a Verify Cell, the transition logic can
/// be composed into different Graphs. A strict project might require
/// 10 gate passes for promotion. A lenient one might require 2.
pub struct TierVerifier {
    /// Promotion thresholds, configurable per deployment.
    promotion_criteria: PromotionCriteria,
    /// Demotion thresholds.
    demotion_criteria: DemotionCriteria,
}

pub struct PromotionCriteria {
    /// Transient -> Working: gate passes where this Signal was in context.
    pub transient_to_working_gate_passes: u32,    // default: 3
    /// Working -> Consolidated: independent confirmations from
    /// different agents or contexts.
    pub working_to_consolidated_confirmations: u32, // default: 5
    /// Consolidated -> Persistent: validator attestations.
    pub consolidated_to_persistent_validators: u32, // default: 3
}

pub struct DemotionCriteria {
    /// Persistent -> Consolidated: unresolved contradictions.
    pub persistent_demotion_contradictions: u32,    // default: 1
    /// Consolidated -> Working: gate failures with Signal in context.
    pub consolidated_demotion_gate_failures: u32,   // default: 2
    /// Working -> Transient: consecutive gate failures OR balance below
    /// this threshold.
    pub working_demotion_consecutive_failures: u32, // default: 3
    pub working_demotion_balance_floor: f64,        // default: 0.15
}

impl Cell for TierVerifier {
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Verify]
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Input: [0] = the Signal under evaluation
        //        [1] = a Signal containing TierTransitionProposal
        let subject = &input[0];
        let proposal: TierTransitionProposal =
            serde_json::from_value(input[1].body.clone())?;

        // Gather evidence from Store
        let evidence = self.gather_evidence(subject, ctx).await?;

        // Evaluate
        let verdict = match proposal.direction {
            Direction::Promotion => {
                self.check_promotion(subject, &evidence)
            }
            Direction::Demotion => {
                self.check_demotion(subject, &evidence)
            }
        };

        // Publish prediction error as Pulse (PPC pattern)
        let prediction_error = if verdict.approved {
            0.0 // transition was expected
        } else {
            1.0 // transition was rejected
        };
        ctx.bus.publish(Pulse::prediction_error(
            "tier-verifier",
            prediction_error,
        )).await?;

        // Return verdict as Signal
        let verdict_signal = Signal::builder(Kind::Verdict)
            .body(serde_json::to_value(&verdict)?)
            .source(vec![subject.ref_(), input[1].ref_()])
            .build();

        Ok(vec![verdict_signal])
    }
}

#[derive(Serialize, Deserialize)]
pub struct TierTransitionProposal {
    pub signal_ref: SignalRef,
    pub current_tier: Tier,
    pub proposed_tier: Tier,
    pub direction: Direction,
}

#[derive(Serialize, Deserialize)]
pub struct TierVerdict {
    pub approved: bool,
    pub from: Tier,
    pub to: Option<Tier>,
    pub evidence_summary: String,
    pub gate_pass_count: u32,
    pub confirmation_count: u32,
    pub contradiction_count: u32,
}
```

### 5.2 Why a Cell Instead of a Function

The old system checked tier transitions inside `KnowledgeStore::decay()` as an inline conditional. Making it a Verify Cell brings three advantages:

1. **Composability.** Different Graphs can use different `TierVerifier` configurations. A safety-critical project can require 10 gate passes for promotion. A rapid-prototyping project can require 1. The transition logic is not hardcoded into the Store.

2. **Observability.** Every tier transition attempt produces a Verdict Signal with a full evidence summary. Operators can query Store for all `Kind::Verdict` Signals where the body contains `TierVerdict`, producing an audit trail of every promotion and demotion.

3. **Feedback.** The PPC prediction error Pulse feeds back into the Score protocol. If the TierVerifier is consistently wrong (proposing promotions that get rejected downstream), its confidence calibration adjusts. This is the Score-Verify-Score loop applied to tier management itself.

### 5.3 The Promotion Pipeline as a Graph

The full promotion check is a Graph of Cells, not a single function call:

```toml
[graph]
name = "tier-promotion-pipeline"

[[nodes]]
id = "balance-check"
cell = "roko:balance-threshold"
protocol = "Score"
# Scores the Signal's balance against its tier's upper band.

[[nodes]]
id = "evidence-gather"
cell = "roko:evidence-collector"
protocol = "Store"
# Queries Store for gate verdicts referencing this Signal.

[[nodes]]
id = "tier-verify"
cell = "roko:tier-verifier"
protocol = "Verify"
# Runs promotion/demotion criteria against gathered evidence.

[[nodes]]
id = "tier-apply"
cell = "roko:tier-updater"
protocol = "Store"
# If verdict approves, writes the new tier to the Signal in Store.

[[nodes]]
id = "tier-react"
cell = "roko:tier-event-publisher"
protocol = "React"
# Publishes Pulse on Bus: knowledge.promoted or knowledge.demoted.

[[edges]]
from = "balance-check"
to = "evidence-gather"

[[edges]]
from = "evidence-gather"
to = "tier-verify"

[[edges]]
from = "tier-verify"
to = "tier-apply"

[[edges]]
from = "tier-apply"
to = "tier-react"
```

Each node can be replaced independently. Want to add an LLM-judge step before Persistent promotion? Insert it between `evidence-gather` and `tier-verify`. Want to skip the balance check for operator-promoted Signals? Remove the edge from `balance-check`. The Graph topology encodes the policy; the Cells encode the mechanics.

---

## 6. Cybernetic Loops

The knowledge system requires three feedback structures to remain healthy: a Lens for observing knowledge metabolism, a Loop for tuning demurrage rates, and a Trigger + Graph for garbage collection.

### 6.1 Lens: Knowledge Health Observability

A Lens is a read-only cross-cut Functor that observes Signals flowing through a pipeline and publishes metrics as Pulses without altering the flow. The knowledge health Lens tracks three diagnostic dimensions.

```rust
/// Knowledge health metrics observed by the Memory Lens.
///
/// Published as a Pulse on topic "knowledge.health" at the end of
/// each demurrage cycle.
#[derive(Serialize, Deserialize)]
pub struct KnowledgeHealthMetrics {
    pub timestamp: DateTime<Utc>,

    // --- Metabolic rate ---
    /// Ratio of reinforcement earned to demurrage charged this cycle.
    /// > 1.0 = knowledge is growing. < 1.0 = knowledge is shrinking.
    pub metabolic_rate: f64,
    /// Total demurrage charged across all Signals this cycle.
    pub total_charged: f64,
    /// Total reinforcement earned across all Signals this cycle.
    pub total_reinforced: f64,

    // --- Immune load ---
    /// Number of AntiKnowledge Signals currently warm.
    pub anti_knowledge_count: usize,
    /// Fraction of Verify verdicts that were failures this cycle.
    /// High failure rate suggests the knowledge base contains stale
    /// or incorrect entries that are contaminating agent context.
    pub gate_failure_rate: f64,
    /// Number of contradiction-triggered demotions this cycle.
    pub contradiction_demotions: usize,

    // --- Diversity ---
    /// Shannon entropy across Kind variants in the warm Store.
    /// Low entropy = monoculture (e.g., all Insights, no CausalLinks).
    pub kind_entropy: f64,
    /// Shannon entropy across Tier levels.
    /// Low entropy = stagnation (e.g., everything stuck at Working).
    pub tier_entropy: f64,
    /// Fraction of warm Signals that are unique (HDC novelty > 0.5).
    /// Low uniqueness = redundancy (many near-duplicate entries).
    pub uniqueness_ratio: f64,

    // --- Distribution ---
    /// Count of Signals per Tier.
    pub tier_counts: BTreeMap<Tier, usize>,
    /// Balance histogram (10 bins from 0.0 to 2.0+).
    pub balance_histogram: Vec<(f64, usize)>,
    /// Top 10 Signals by balance (attention leaderboard).
    pub attention_leaderboard: Vec<(SignalRef, f64)>,
}
```

**Metabolic rate** is the single most important diagnostic. A metabolic rate consistently below 1.0 means the system is forgetting faster than it is learning -- knowledge is a net drain. A rate consistently above 2.0 means the system is hoarding -- too much survives without adequate challenge. The healthy range is 0.8 to 1.5.

**Immune load** tracks the system's ability to reject bad knowledge. A rising `gate_failure_rate` when knowledge Signals are in the context pack suggests contamination. The AntiKnowledge count is a proxy for how many known-bad patterns the system is actively guarding against.

**Diversity** prevents cognitive monoculture. If `kind_entropy` drops (e.g., the system only produces Insights and never CausalLinks), the knowledge graph is shallow. If `uniqueness_ratio` drops, the system is accumulating redundant entries that waste context window space.

### 6.2 Loop: Adaptive Demurrage Tuning

The demurrage rates declared on each Kind are defaults. A Loop Cell can adjust them based on the KnowledgeHealthMetrics, closing the feedback edge between observation and policy.

```rust
/// Adaptive demurrage tuning Loop.
///
/// Observes KnowledgeHealthMetrics (via Bus subscription to
/// "knowledge.health") and adjusts demurrage rates to maintain
/// the target stationary distribution across tiers.
///
/// This is a textbook control loop:
///   - Setpoint: target tier distribution (e.g., 15% Transient,
///     35% Working, 30% Consolidated, 10% Persistent, 10% Frozen).
///   - Error: difference between current and target distribution.
///   - Controller: PID-style adjustment to charge multipliers.
///   - Actuator: updated DemurrageConfig written to Store.
pub struct DemurrageTuner {
    /// Target stationary distribution.
    target_distribution: BTreeMap<Tier, f64>,
    /// PID gains. Conservative defaults: slow adaptation.
    kp: f64,  // proportional gain, default 0.01
    ki: f64,  // integral gain, default 0.001
    kd: f64,  // derivative gain, default 0.005
    /// Accumulated integral error per tier.
    integral_error: BTreeMap<Tier, f64>,
    /// Previous error per tier (for derivative term).
    prev_error: BTreeMap<Tier, f64>,
    /// Bounds on charge multiplier adjustment to prevent runaway.
    adjustment_bounds: (f64, f64), // default (0.5, 4.0)
}

impl DemurrageTuner {
    /// Compute adjusted charge multipliers given current metrics.
    ///
    /// Returns a map of Tier -> adjusted charge multiplier.
    /// The Store protocol applies these on the next demurrage tick.
    pub fn tune(
        &mut self,
        metrics: &KnowledgeHealthMetrics,
        total_warm_signals: usize,
    ) -> BTreeMap<Tier, f64> {
        let mut adjustments = BTreeMap::new();

        for (tier, &target_frac) in &self.target_distribution {
            let current_count = metrics.tier_counts
                .get(tier).copied().unwrap_or(0);
            let current_frac = current_count as f64
                / total_warm_signals.max(1) as f64;

            // PID error: positive means too many at this tier
            let error = current_frac - target_frac;

            // Integral accumulation (with anti-windup clamp)
            let integral = self.integral_error
                .entry(*tier).or_default();
            *integral = (*integral + error).clamp(-1.0, 1.0);

            // Derivative
            let prev = self.prev_error
                .entry(*tier).or_default();
            let derivative = error - *prev;
            *prev = error;

            // PID output: how much to adjust the charge multiplier
            let adjustment = self.kp * error
                + self.ki * *integral
                + self.kd * derivative;

            // Apply to base multiplier with bounds
            let base = tier.charge_multiplier();
            let adjusted = (base + adjustment)
                .clamp(self.adjustment_bounds.0, self.adjustment_bounds.1);

            adjustments.insert(*tier, adjusted);
        }

        adjustments
    }
}
```

The Loop has a natural operating rhythm: it runs once per demurrage cycle (default: every 6 hours). It publishes adjusted multipliers as a Config Signal to Store, which the next demurrage tick reads. The adjustment bounds prevent the controller from oscillating or driving multipliers to extreme values.

**Critical design decision:** the tuner adjusts *charge multipliers* (how fast things decay), not *reinforcement multipliers* (how much use restores). This is deliberate. Reinforcement is driven by actual agent behavior and should not be artificially inflated. Decay rate is a policy knob that the system can safely adjust.

### 6.3 Garbage Collection as a Scheduled Graph

In the old system, garbage collection was a function call: `KnowledgeStore::gc()` scanned all entries, removed those below `DEFAULT_GC_MIN_CONFIDENCE`, and rewrote the JSONL file atomically. This was a monolithic operation with no observability, no composability, and no scheduling mechanism.

In the redesign, GC is a scheduled Graph triggered by a Trigger Cell:

```toml
[graph]
name = "knowledge-gc"

# Trigger: run every 6 hours
[[nodes]]
id = "gc-trigger"
cell = "roko:cron-trigger"
protocol = "Trigger"
config = { cron = "0 */6 * * *" }

# Step 1: Scan for candidates below cold threshold
[[nodes]]
id = "gc-scan"
cell = "roko:balance-scanner"
protocol = "Store"
config = { threshold = 0.05 }

# Step 2: Attempt consolidation before deletion
[[nodes]]
id = "gc-consolidate"
cell = "roko:gc-consolidator"
protocol = "Compose"
# Tries to merge low-balance Signals into existing higher-balance
# Signals of the same Kind. If successful, the low-balance Signal
# is superseded rather than destroyed.

# Step 3: Freeze survivors that could not be consolidated
[[nodes]]
id = "gc-freeze"
cell = "roko:cold-archiver"
protocol = "Store"
# Moves payload to cold storage. Retains hash and HDC fingerprint
# in warm index for future thaw discovery.

# Step 4: Publish GC summary
[[nodes]]
id = "gc-report"
cell = "roko:gc-reporter"
protocol = "React"
# Emits Pulse on "knowledge.gc" with counts: consolidated, frozen,
# destroyed.

[[edges]]
from = "gc-trigger"
to = "gc-scan"

[[edges]]
from = "gc-scan"
to = "gc-consolidate"

[[edges]]
from = "gc-consolidate"
to = "gc-freeze"

[[edges]]
from = "gc-freeze"
to = "gc-report"
```

The critical innovation is step 2: **consolidation before destruction**. The old GC destroyed entries below threshold. The new GC first attempts to *merge* dying entries into surviving relatives. If three nearly identical Insights are all decaying, one can absorb the others' evidence (source episodes, confirmations) and survive with a boosted balance. Only Signals that cannot be consolidated are frozen.

```rust
/// Attempt to consolidate a dying Signal into a surviving relative.
///
/// Returns Some(survivor_ref) if consolidation succeeded, None if
/// no suitable survivor was found.
pub async fn try_consolidate(
    dying: &Signal,
    store: &dyn Store,
) -> Result<Option<SignalRef>> {
    // Find Signals of the same Kind with HDC similarity > 0.7
    let candidates = store.query_similar(
        &dying.hdc_fingerprint,
        10,                              // top 10 neighbors
        Some(dying.kind.clone()),         // same Kind only
    ).await?;

    // Pick the candidate with the highest balance
    let survivor = candidates.iter()
        .filter(|c| c.balance > dying.balance * 2.0) // must be healthier
        .max_by(|a, b| a.balance.partial_cmp(&b.balance).unwrap());

    if let Some(target) = survivor {
        // Transfer evidence: add dying's source episodes to survivor
        let mut updated = target.clone();
        updated.source.extend(dying.source.iter().cloned());
        updated.source.dedup();

        // Boost survivor balance by a fraction of dying's remaining balance
        updated.balance += dying.balance * 0.5;

        // Record the lineage: survivor now descends from dying
        updated.source.push(dying.ref_());

        store.update(updated.clone()).await?;

        Ok(Some(target.ref_()))
    } else {
        Ok(None) // no suitable survivor; proceed to freeze
    }
}
```

This transforms GC from a destructive operation (knowledge is lost) into a consolidation operation (knowledge is compressed). The system's total information decreases, but the surviving Signals are richer for having absorbed the evidence of the dying ones.

---

## 7. Contradiction Resolution

One gap the original system did not address: what happens when two Signals with contradictory content are both at high tiers? For example, two Persistent Heuristics:

- Heuristic A: "When deploying Rust services, always run `cargo test` first."
- Heuristic B: "When deploying Rust services, skip `cargo test` if only config files changed."

These are not contradictions in the logical sense (B is a refinement of A), but the system must detect when two knowledge Signals with opposing recommendations are both in the active context pack.

### 7.1 Contradiction Detection via HDC

Contradictions are detected using the HDC bind operation. If Signal A and Signal B are of the same Kind and their HDC fingerprints are *moderately* similar (Hamming distance 0.3-0.5, meaning related but not identical), and their content carries opposing valence (one is AntiKnowledge referencing the other's domain, or their Score.confidence axes diverge by more than 0.3), a contradiction Pulse is published.

```rust
/// Detect potential contradictions between a new Signal and
/// existing warm Signals in the same Kind.
pub async fn check_contradictions(
    new_signal: &Signal,
    store: &dyn Store,
    bus: &dyn Bus,
) -> Result<Vec<ContradictionReport>> {
    let neighbors = store.query_similar(
        &new_signal.hdc_fingerprint,
        20,
        Some(new_signal.kind.clone()),
    ).await?;

    let mut reports = Vec::new();

    for neighbor in &neighbors {
        let similarity = new_signal.hdc_fingerprint
            .hamming_similarity(&neighbor.hdc_fingerprint);

        // "Related but different" zone: similar enough to be about the
        // same topic, different enough to potentially disagree.
        if similarity < 0.3 || similarity > 0.7 {
            continue;
        }

        // Check for opposing valence indicators
        let confidence_divergence = (new_signal.score.confidence
            - neighbor.score.confidence).abs();

        let is_anti = new_signal.kind == Kind::AntiKnowledge
            || neighbor.kind == Kind::AntiKnowledge;

        if confidence_divergence > 0.3 || is_anti {
            let report = ContradictionReport {
                signal_a: new_signal.ref_(),
                signal_b: neighbor.ref_(),
                similarity,
                confidence_divergence,
                is_anti_knowledge_involved: is_anti,
            };

            // Publish contradiction Pulse for the tier system to act on
            bus.publish(Pulse::new(
                Topic::parse("knowledge.contradiction"),
                Kind::Verdict,
                serde_json::to_value(&report)?,
            )).await?;

            reports.push(report);
        }
    }

    Ok(reports)
}
```

### 7.2 Resolution Strategies

When a contradiction is detected between two established Signals, the system does not automatically pick a winner. Instead, it applies one of three strategies depending on the tier relationship:

| Scenario | Strategy |
|---|---|
| Both Transient/Working | Let demurrage decide: the one that earns more reinforcement survives |
| One Consolidated, one lower | The Consolidated Signal is presumed correct; the lower is demoted |
| Both Consolidated or higher | Trigger a **contradiction review**: both Signals are tagged `under_review`, and the next Verify Cell that encounters both must produce a resolution Verdict |

The contradiction review is itself a Graph that can be composed with different resolution policies -- human-in-the-loop review, LLM judge arbitration, or majority-vote among agents who have used both Signals.

---

## 8. AntiKnowledge as Immune Memory

AntiKnowledge deserves special treatment because it inverts the normal demurrage incentive. Most Signals *should* decay when unused. AntiKnowledge Signals should *persist* even when unused, because their value is in *preventing* actions rather than *enabling* them. You do not "use" the knowledge that `rm -rf /` is dangerous by running it. You use it by *not* running it.

The unified model handles this through the `balance_floor` mechanism on Kind:

```rust
Kind::AntiKnowledge => Some(0.30),
```

This means an AntiKnowledge Signal's balance never drops below 0.30, regardless of demurrage charges. At the Working tier with charge multiplier 1.0, an AntiKnowledge Signal with no reinforcement eventually converges to balance 0.30 and stays there indefinitely. It is retrievable. It pays demurrage (balance does decline toward the floor). But it never freezes.

The floor creates a third class of knowledge lifecycle:

1. **Normal Signals**: decay toward zero, eventually freeze.
2. **Frozen Signals**: exempt from demurrage, dormant until thawed.
3. **Floor Signals** (AntiKnowledge): pay demurrage but cannot die. The floor acts as a permanent subsidy from the system to retain negative knowledge.

### 8.1 The HDC Admission Guard

When a new Signal is ingested, the Store checks its HDC similarity against warm AntiKnowledge entries. This implements an immune response: knowledge that resembles a known failure pattern is either discounted or rejected.

| HDC similarity to AntiKnowledge | Action |
|---|---|
| < 0.5 | No action (unrelated) |
| 0.5 - 0.7 | Log warning, attach taint label |
| 0.7 - 0.9 | Discount confidence by 0.5x |
| > 0.9 | Reject ingestion entirely |

This is the existing `ANTI_KNOWLEDGE_WARN_THRESHOLD`, `ANTI_KNOWLEDGE_DISCOUNT_THRESHOLD`, and `ANTI_KNOWLEDGE_REJECT_THRESHOLD` from the current codebase, expressed as a Store-level admission policy rather than a Neuro-specific function.

---

## What This Enables

1. **One demurrage engine for all data.** Episodes, Verdicts, Config, and knowledge Signals all flow through the same `demurrage_tick()`. No Neuro-specific decay code. When the demurrage engine improves (e.g., better novelty computation), all Signal types benefit.

2. **Composable retention policies.** Tier promotion is a Graph of Cells, not hardcoded logic. Different projects can compose different promotion pipelines from the same Cell building blocks.

3. **GC that consolidates rather than destroys.** Dying knowledge is merged into surviving relatives before it is frozen. The system compresses rather than amputates.

4. **Observable knowledge metabolism.** The Lens publishes health metrics every cycle. Operators can see whether the knowledge system is growing, shrinking, or stagnant -- and the Loop can automatically adjust demurrage rates to maintain the target distribution.

5. **Contradiction as first-class event.** Opposing knowledge Signals trigger a structured resolution process rather than silently coexisting.

6. **No separate knowledge type system.** `KnowledgeKind` is eliminated as a standalone enum. Knowledge types are `Kind` variants. `KnowledgeTier` is eliminated as a standalone enum. Tiers are `Tier` on Signal. `KnowledgeEntry` is eliminated as a standalone struct. Knowledge entries are Signals. The Neuro crate becomes a collection of Cells (TierVerifier, GC Graph, Distiller, Contradiction Detector) that operate on Signals through the standard protocols.

---

## Feedback Loops

Five feedback loops govern knowledge health:

1. **Use-Reinforce-Survive** (positive, self-amplifying): Useful Signals get retrieved, retrieval earns reinforcement, reinforcement keeps balance above cold threshold, balance keeps the Signal available for future retrieval. Checked by novelty weighting: the 10th retrieval earns less than the 1st.

2. **Novelty-Bonus-Reduce-Novelty** (self-regulating): Novel Signals earn larger reinforcement bonuses. As similar Signals accumulate, novelty decreases, bonuses shrink, and only the most distinctive Signal in a cluster survives. This is the anti-hoarding mechanism.

3. **Contradiction-Demote-Decay-Freeze** (immune response): Contradicted Signals are demoted. Demotion increases charge multiplier. Higher charge accelerates balance decline. Low balance triggers freeze. The immune system actively expels bad knowledge.

4. **GC-Consolidate-Boost-Survive** (compression): GC candidates are merged into healthier relatives before freezing. The surviving Signal gains evidence from the dying one, boosting its balance. This converts entropy (many low-balance copies) into negentropy (fewer, richer Signals).

5. **Observe-Tune-Charge-Distribute** (metabolic control): The Lens observes tier distribution. The Loop computes error against the target distribution. The tuner adjusts charge multipliers. Adjusted multipliers change the rate at which Signals move between tiers. The tier distribution shifts toward the target. The Lens observes the new distribution. This is a classical PID control loop with a 6-hour period.

---

## Open Questions

1. **Is the tier system actually necessary?** Demurrage alone could handle retention if the charge rate were a continuous function of accumulated evidence rather than a discrete 4-level multiplier. The tier system's value is in creating named basins of attraction that operators can reason about ("this Signal is Consolidated" is more legible than "this Signal's effective charge multiplier is 0.52"). But the discrete jumps at tier boundaries introduce discontinuities in the dynamics. Could a continuous retention curve (e.g., `charge_multiplier = f(confirmation_count, balance, age)`) achieve the same retention semantics without the phase transition artifacts?

2. **What prevents Persistent ossification?** A Persistent Signal with charge multiplier 0.1x decays extremely slowly. If the environment changes (e.g., the project switches from Rust to Go), Persistent knowledge about Rust idioms is now useless but will persist for years. The contradiction-demotion mechanism only fires when a *specific* contradicting Signal is ingested. Gradual irrelevance (no contradictions, just declining utility) is harder to detect. Should the system track a "last useful retrieval" timestamp and demote Signals that have not been retrieved in N demurrage cycles, regardless of tier?

3. **How should cross-Space demurrage work?** When Signals are shared across Spaces (isolation boundaries), should they carry their balance with them or start fresh? If carried, a Signal rich in Space A gets a free ride in Space B. If reset, useful cross-Space knowledge must re-prove itself. The answer may depend on the trust relationship between Spaces.

4. **Should reinforcement from multiple agents compound?** In a multi-agent setting, if 10 agents all retrieve the same Signal in the same cycle, does it earn 10x reinforcement? This rewards consensus but can create popularity bias where mediocre-but-popular knowledge crowds out novel-but-niche knowledge. A diminishing-returns curve (e.g., `effective_reinforcement = base * ln(1 + agent_count)`) might balance consensus and diversity.

5. **What is the right GC consolidation threshold?** The `try_consolidate` function merges dying Signals into relatives with HDC similarity > 0.7. If the threshold is too high, few merges occur and GC reverts to pure destruction. If too low, distinct knowledge entries are incorrectly merged, losing nuance. The threshold may need to be Kind-specific: Heuristics with subtle when-clause differences should not merge at 0.7, but Insights about the same pattern probably should.

6. **How does the Ebbinghaus degenerate case interact with tier multipliers?** The old system stored `half_life_days` explicitly and multiplied by `KnowledgeTier::multiplier()`. The new system derives half-life from `beta / charge_multiplier`. These give slightly different trajectories because the old system applied the tier multiplier to the half-life (stretching the exponential), while the new system applies it to the decay rate (changing the exponent). The difference is small for moderate multipliers but diverges for extreme ones (e.g., Transient at 2.0x). Is this divergence acceptable, or does migration require a backward-compatibility shim?
