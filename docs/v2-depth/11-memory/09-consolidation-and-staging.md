# Consolidation and Staging

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). Redesigns consolidation as Store protocol behavior with demurrage-driven staging, the confidence ladder as demurrage resistance, SHY renormalization as a periodic Functor, confirmation boost as a React Cell, and dream evolution as an optional Loop.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, demurrage, HDC fingerprints, tier progression), [02-CELL](../../unified/02-CELL.md) (Store protocol, Score protocol, Verify protocol, React protocol, Route protocol, Compose protocol, Functor pattern), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, Pipeline, Loop specializations), [demurrage-economics.md](../01-signal/demurrage-economics.md) (rate law, phase transitions, tier Markov chain)

**Source docs**: `docs/10-dreams/04-consolidation-and-staging.md`, `docs/10-dreams/05-dream-evolution.md`

---

## 1. The Staging Buffer IS a Store Partition

The staging buffer is not a separate system. It is a **partition of the Store** -- a named region within the same Store that holds promoted knowledge, subject to its own demurrage schedule. The partition boundary is a tag on the Signal, not a separate database.

Every Signal in the staging partition carries `partition: "staging"` in its metadata. Store queries default to excluding the staging partition unless the caller explicitly opts in. This is the same mechanism that separates cold storage from warm storage: partitions are metadata-driven filters on a single Store, not separate physical stores.

### Why a Partition, Not a Separate Store

Three reasons:

1. **Unified demurrage**: The same demurrage tick function processes staging Signals and promoted Signals. No separate expiry cron. The 14-day expiry for unvalidated staging entries is simply a high demurrage rate -- the Signal's balance reaches the cold floor in 14 days without reinforcement.

2. **HDC similarity queries span both**: When a new waking episode arrives and the system checks for confirmation matches, the HDC similarity search runs against the entire Store, including staging. A staging Signal with HDC similarity > 0.60 to a waking success episode gets a confirmation boost. No separate lookup path.

3. **Promotion is a tier transition, not a copy**: When a staging Signal reaches confidence >= 0.70, it is not copied to a "promoted store." Its partition tag changes from `"staging"` to `"promoted"` and its tier advances. The Signal stays in the same Store with the same content hash.

### The Staging Demurrage Schedule

Staging Signals use aggressive demurrage rates that implement the 14-day expiry window:

```rust
/// Demurrage configuration for the staging partition.
///
/// The rates are calibrated so that a Signal receiving zero reinforcement
/// hits the cold floor in approximately 14 days. This replaces the legacy
/// `expiration_window` with economics: Signals that earn reinforcement
/// survive; Signals that don't, expire naturally.
pub const STAGING_DEMURRAGE: DemurrageConfig = DemurrageConfig {
    // High flat tax: drains ~0.02 balance per day
    flat_tax_per_day: 0.020,
    // High exponential decay: halves balance in ~5 days without reinforcement
    exp_decay_per_day: 0.140,
    // Generous reinforcement bonus: each confirmation is worth ~5 days of survival
    confirmation_bonus: 0.10,
    // Initial balance for new staging entries
    initial_balance: 0.30,
    // Cold floor: below this, the Signal expires (moves to cold storage)
    cold_floor: 0.01,
    // No thaw for expired staging entries -- they stay cold
    thaw_eligible: false,
};
```

The math: with `beta = 0.140` and zero reinforcement, the half-life is `ln(2) / 0.140 = 4.95 days`. Starting from balance 0.30, the Signal crosses the cold floor (0.01) in approximately `ln(0.30/0.01) / 0.140 = 24.3 days`. The flat tax accelerates this to roughly 14 days. This matches the source spec's 14-day expiry window, but the mechanism is economic rather than calendar-based.

### Signal Schema for Staging Entries

```rust
/// A staging Signal. This is a regular Signal with staging-specific metadata.
///
/// The confidence ladder status is derived from the Signal's balance,
/// not stored as a separate enum. Balance bands map to ladder stages:
///   balance < 0.10  -> effectively expired (near cold floor)
///   0.10 - 0.15     -> Staged (just entered, no waking evidence)
///   0.15 - 0.22     -> Partially Validated (some evidence)
///   0.22 - 0.30     -> Validated (multiple confirmations)
///   >= 0.30         -> Promotion candidate (checked by promotion Trigger)
pub struct StagingSignal {
    /// The underlying Signal. All staging metadata lives here.
    pub signal: Signal,
}

impl StagingSignal {
    /// Create a new staging Signal from a dream hypothesis.
    pub fn from_hypothesis(
        content: String,
        source_phase: SourcePhase,
        source_episodes: Vec<EpisodeId>,
        hdc_fingerprint: HdcVector,
    ) -> Self {
        let mut signal = Signal::new(
            Kind::Hypothesis,
            content,
        );
        signal.balance = STAGING_DEMURRAGE.initial_balance;
        signal.tier = Tier::Transient;
        signal.hdc_fingerprint = Some(hdc_fingerprint);
        signal.partition = Partition::Staging;
        signal.provenance = Provenance::Dream {
            source_phase,
            source_episodes,
        };
        // Initial confidence ceiling: 0.30. All higher confidence
        // must come from waking validation (confirmation boost).
        signal.confidence = 0.25;

        Self { signal }
    }

    /// Derive the confidence ladder stage from the Signal's current balance.
    /// The ladder is not stored -- it is computed from the demurrage state.
    pub fn ladder_stage(&self) -> LadderStage {
        match self.signal.balance {
            b if b < 0.10 => LadderStage::Expired,
            b if b < 0.15 => LadderStage::Staged,
            b if b < 0.22 => LadderStage::PartiallyValidated,
            b if b < 0.30 => LadderStage::Validated,
            _ => LadderStage::PromotionCandidate,
        }
    }
}

pub enum LadderStage {
    Expired,
    Staged,
    PartiallyValidated,
    Validated,
    PromotionCandidate,
}

pub enum SourcePhase {
    NremReplay,
    NremCrossEpisode,
    RemCounterfactual,
    RemCombinational,
    RemTransformational,
    ThreatSimulation,
}
```

### Safety Constraints as Store Capacity Limits

The source spec defines four safety constraints. Each maps to a Store-level enforcement:

| Safety constraint | Store enforcement |
|---|---|
| Max 1,000 staging entries | `Store::staging_partition().max_signals = 1000`. On overflow, GC evicts lowest-balance entries first. |
| Max 3 contradictions per cycle | The staging write path counts Signals with `contradicts: Some(_)` written this cycle. After 3, further contradictions are rejected. |
| Initial confidence ceiling 0.30 | `STAGING_DEMURRAGE.initial_balance = 0.30`. No Signal enters staging with balance above 0.30. |
| 14-day expiry for unvalidated | The demurrage schedule above. No separate expiry timer. |

---

## 2. The Confidence Ladder IS Demurrage Resistance

The five-stage confidence ladder from the source spec maps directly onto balance bands in the demurrage system. Higher-confidence Signals resist demurrage better because they have accumulated more balance through reinforcement. The ladder is an emergent property of the economics, not a separate state machine.

### Balance-to-Ladder Mapping

```
Balance    Ladder Stage              Trust Level
------     ----------------------    ----------------------------------
< 0.10     Expired                   Below cold floor. Candidate for GC.
0.10-0.15  Staged (conf 0.20-0.30)   No waking evidence. Not used for decisions.
0.15-0.22  Partially Validated        Some evidence. Referenced but not relied upon.
            (conf 0.30-0.50)
0.22-0.30  Validated                  Multiple confirmations. Agent acts tentatively.
            (conf 0.50-0.70)
>= 0.30   Promotion Candidate        Ready for promotion to permanent Store.
            (conf >= 0.70)
```

The confidence field on the Signal tracks the epistemic confidence (how much evidence supports this hypothesis). The balance field tracks the economic confidence (how much attention credit this Signal has earned). They are correlated but not identical: a Signal can have high epistemic confidence but low balance if it hasn't been accessed recently, and vice versa.

The mapping between the two:

```rust
/// Sync confidence from balance after each demurrage tick.
///
/// Balance is the ground truth for survival. Confidence is the
/// epistemic estimate used by downstream consumers.
pub fn sync_confidence_from_balance(signal: &mut Signal) {
    // Staging partition: confidence tracks balance directly
    if signal.partition == Partition::Staging {
        // Map balance [0.10, 0.30+] to confidence [0.20, 0.70+]
        let raw = (signal.balance - 0.10) / 0.20; // 0..1 when balance in [0.10, 0.30]
        signal.confidence = 0.20 + raw.clamp(0.0, 1.0) * 0.50;
        // Cap at 0.70 -- promotion handles the jump above
        if signal.partition == Partition::Staging {
            signal.confidence = signal.confidence.min(0.70);
        }
    }
}
```

### Why Demurrage Resistance = Confidence

A Signal climbs the ladder by accumulating balance through confirmation boosts (see S3). Each boost is a reinforcement event on the demurrage ledger. The more confirmations, the higher the balance, the further the Signal is from the cold floor, the more it resists the daily demurrage drain.

This eliminates the "stuck at threshold" problem from the source spec. In the original design, a hypothesis could hover just below a promotion threshold indefinitely. With demurrage, a Signal that stops receiving confirmations decays back down the ladder. There is no stable resting state -- every Signal is either climbing (receiving confirmations) or falling (paying demurrage). This is the same insight as the Transient tier in the main demurrage system: Transient is unstable by design.

---

## 3. SHY Renormalization IS a Periodic Functor

Synaptic Homeostasis (SHY) renormalization -- the global downscaling of confidence during consolidation -- is a **Functor** applied to the entire Store partition. A Functor is a structure-preserving map: it transforms every Signal in a partition while preserving the Store's structure (ordering, partitioning, content hashes).

### The Renormalization Functor

```rust
/// SHY renormalization as a Functor over the staging Store partition.
///
/// Applies a global scale factor to all staging Signals, with protection
/// for high-confidence entries and a recency exemption. This implements
/// Tononi & Cirelli's SHY: during consolidation, all signals are
/// downscaled unless they have earned protection through validation.
///
/// The Functor is structure-preserving: it transforms balances but does
/// not add, remove, or reorder Signals. Content hashes are unchanged.
pub struct ShyRenormalizationFunctor {
    /// Global scaling factor. Values < 1.0 implement net downscaling.
    /// Causal evidence: Sawada et al., Science 2024.
    pub global_scale_factor: f64,           // default: 0.95

    /// Signals with confidence above this threshold are protected.
    pub protection_threshold: f64,          // default: 0.80

    /// Maximum confidence reduction per application.
    pub max_reduction: f64,                 // default: 0.05

    /// Recency window: Signals confirmed within this many hours are exempt.
    pub recency_window: Duration,           // default: 24h
}

impl ShyRenormalizationFunctor {
    /// Apply the Functor to every Signal in the staging partition.
    ///
    /// Returns the number of Signals affected and the total balance removed.
    pub fn apply(
        &self,
        store: &mut dyn Store,
        now: DateTime<Utc>,
    ) -> RenormalizationReport {
        let mut affected = 0;
        let mut total_removed = 0.0;

        for signal in store.partition_iter_mut(Partition::Staging) {
            // Exemption 1: high-confidence protection
            if signal.confidence >= self.protection_threshold {
                continue;
            }

            // Exemption 2: recently validated
            if let Some(last_validated) = signal.last_reinforced_at {
                if now - last_validated < self.recency_window {
                    continue;
                }
            }

            // Apply global downscaling
            let old_balance = signal.balance;
            let scaled = old_balance * self.global_scale_factor;
            let reduction = old_balance - scaled;

            // Cap the reduction
            let capped_reduction = reduction.min(self.max_reduction);
            signal.balance = old_balance - capped_reduction;

            total_removed += capped_reduction;
            affected += 1;
        }

        RenormalizationReport {
            signals_affected: affected,
            total_balance_removed: total_removed,
            timestamp: now,
        }
    }
}

pub struct RenormalizationReport {
    pub signals_affected: usize,
    pub total_balance_removed: f64,
    pub timestamp: DateTime<Utc>,
}
```

### When the Functor Fires

The renormalization Functor is applied once per consolidation cycle, after the NREM replay and REM imagination phases have generated new staging entries but before the promotion check runs. This ordering matters:

```
NREM replay   -> new hypotheses enter staging
REM imagine   -> more hypotheses enter staging
SHY Functor   -> global downscaling of existing staging entries
Promotion     -> entries that survived downscaling AND have balance >= 0.30 promote
```

The Functor's effect: it pushes borderline entries further from the promotion threshold. Only entries with sustained confirmation from waking experience survive both the daily demurrage drain AND the periodic SHY downscaling. This implements the biological principle: sleep consolidation is a filtering process, not just a storage process.

### Observability

The `RenormalizationReport` emitted after each Functor application feeds into the dream journal (see [17-advanced-dream-concepts](../../docs/10-dreams/17-advanced-dream-concepts.md)) and the telemetry pipeline. The key metric is `total_balance_removed / total_balance_before`: the fraction of the staging partition's total balance that was pruned. A healthy value is 3-7%. Below 3% means the Functor is not pruning enough (protection threshold too low or too many recent validations). Above 7% means the Functor is too aggressive (scale factor too low).

---

## 4. Confirmation Boost IS a React Cell

The confirmation boost -- the mechanism by which waking evidence raises a staging Signal's confidence -- is a **React Cell**. A React Cell watches a Pulse stream on Bus and emits side effects. The confirmation boost Cell watches for Verify verdict Pulses and updates staging Signals whose HDC fingerprints match the verified content.

### The React Cell

```toml
# Graph definition for the confirmation boost React Cell.
# This Cell is always active -- it runs continuously during waking operation,
# watching the Bus for Verify verdict Pulses.

[[graph.cells]]
id = "confirmation-boost"
protocol = "React"
description = "Watch Verify verdict Pulses; boost matching staging Signals"

# Input: Verify verdict Pulses on Bus topic "verify.verdict"
[[graph.edges]]
from = "bus:verify.verdict"
to = "confirmation-boost.in"
filter = "verdict.passed == true"

# Output: updated staging Signals written back to Store
[[graph.edges]]
from = "confirmation-boost.out"
to = "store:staging"
```

```rust
/// Confirmation boost as a React Cell.
///
/// Watches the Bus for Verify verdict Pulses (topic: "verify.verdict").
/// When a verdict passes AND the verified content's HDC fingerprint matches
/// a staging Signal with similarity > 0.60, the staging Signal receives
/// a confirmation boost to its balance.
///
/// The boost formula: new_balance = old_balance + boost * (max_balance - old_balance)
/// This gives diminishing returns -- each successive confirmation adds less,
/// preventing runaway accumulation from repeated similar events.
pub struct ConfirmationBoostCell {
    /// Boost factor per independent confirmation.
    pub boost_factor: f64,                  // default: 0.15

    /// Minimum HDC similarity to count as a match.
    pub similarity_threshold: f64,          // default: 0.60

    /// Maximum balance a staging Signal can reach via boosts alone.
    /// Promotion happens at 0.30; this cap prevents over-accumulation.
    pub max_balance: f64,                   // default: 0.40

    /// Minimum time between the staging Signal's creation and the
    /// confirming episode. Prevents self-confirmation.
    pub min_age: Duration,                  // default: 1h

    /// Refutation penalty when a matching verdict FAILS.
    pub refutation_penalty: f64,            // default: 0.10
}

impl ConfirmationBoostCell {
    /// React to a Verify verdict Pulse.
    ///
    /// Returns a list of staging Signals that were boosted or penalized.
    pub async fn react(
        &self,
        pulse: &Pulse,
        store: &mut dyn Store,
    ) -> Vec<ConfirmationEvent> {
        let verdict: &Verdict = pulse.payload();
        let episode_fingerprint = pulse.hdc_fingerprint();
        let now = Utc::now();
        let mut events = Vec::new();

        // Find staging Signals whose HDC fingerprint is similar
        let candidates = store.query_partition(
            Partition::Staging,
            HdcQuery::similar_to(episode_fingerprint, self.similarity_threshold),
        ).await;

        for mut candidate in candidates {
            // Guard: candidate must have been created before the episode
            let age = now - candidate.created_at;
            if age < self.min_age {
                continue;
            }

            // Guard: episode must not itself be dream-generated
            if pulse.provenance().is_dream() {
                continue;
            }

            let similarity = candidate.hdc_fingerprint
                .as_ref()
                .map(|fp| fp.similarity(episode_fingerprint))
                .unwrap_or(0.0);

            if similarity < self.similarity_threshold {
                continue;
            }

            if verdict.passed {
                // Confirmation: boost balance with diminishing returns
                let old = candidate.balance;
                let boost = self.boost_factor * (self.max_balance - old);
                candidate.balance = (old + boost).min(self.max_balance);
                candidate.last_reinforced_at = Some(now);

                events.push(ConfirmationEvent {
                    signal_ref: candidate.ref_(),
                    kind: ConfirmationKind::Boosted,
                    old_balance: old,
                    new_balance: candidate.balance,
                    similarity,
                    episode_ref: pulse.source_ref(),
                });
            } else {
                // Refutation: penalize balance
                let old = candidate.balance;
                candidate.balance = (old - self.refutation_penalty).max(0.0);

                // If the staging Signal contradicted existing knowledge,
                // boost the existing knowledge's confidence
                if let Some(contradicts_ref) = &candidate.contradicts {
                    if let Some(mut existing) = store.get(contradicts_ref).await {
                        existing.balance += 0.10;
                        store.update(existing).await;
                    }
                }

                events.push(ConfirmationEvent {
                    signal_ref: candidate.ref_(),
                    kind: ConfirmationKind::Refuted,
                    old_balance: old,
                    new_balance: candidate.balance,
                    similarity,
                    episode_ref: pulse.source_ref(),
                });
            }

            store.update(candidate).await;
        }

        // Publish confirmation events as Pulses for downstream consumers
        for event in &events {
            let pulse = Pulse::new(
                Topic::parse("staging.confirmation"),
                event.clone(),
            );
            bus.publish(pulse).await;
        }

        events
    }
}

pub struct ConfirmationEvent {
    pub signal_ref: SignalRef,
    pub kind: ConfirmationKind,
    pub old_balance: f64,
    pub new_balance: f64,
    pub similarity: f64,
    pub episode_ref: SignalRef,
}

pub enum ConfirmationKind {
    Boosted,
    Refuted,
}
```

### Independence Criteria

Not every matching verdict counts as an "independent confirmation." The source spec requires four conditions. The React Cell enforces them:

| Condition | Enforcement in React Cell |
|---|---|
| HDC similarity > 0.60 | `self.similarity_threshold` in the HDC query |
| Occurred after hypothesis creation | `age < self.min_age` guard |
| Successful outcome (passed all gates) | `filter = "verdict.passed == true"` on the Bus edge |
| Not dream-generated | `pulse.provenance().is_dream()` guard |

### The Confirmation-Demurrage Feedback Loop

The confirmation boost and the demurrage drain form a closed feedback loop:

```
Waking episode succeeds
    -> Verify verdict Pulse on Bus (topic: "verify.verdict")
    -> ConfirmationBoostCell.react() fires
    -> Matching staging Signal gets balance boost
    -> Signal climbs the confidence ladder
    -> At balance >= 0.30, promotion Trigger fires
    -> Signal moves from Partition::Staging to Partition::Promoted
    -> Signal now subject to promoted demurrage schedule (lower rates)
    -> Signal must continue earning reinforcement to survive long-term
```

Without confirmation: the staging demurrage drains the Signal in ~14 days. With one confirmation per week: the boost offsets the drain and the Signal slowly climbs. With multiple confirmations: the Signal reaches the promotion threshold and escapes the aggressive staging demurrage.

---

## 5. Dream Evolution IS an Optional Loop

The EVOLUTION phase -- memetic selection, tournament recombination, MAP-Elites diversity search -- is a **Loop**: a Graph with a feedback edge from output back to input. It fires conditionally (20+ promotions since last EVOLUTION cycle) and operates on the promoted partition of the Store.

### The Evolution Loop Graph

```toml
[graph]
id = "dream-evolution-loop"
kind = "Loop"
description = "Evolutionary selection and recombination of promoted knowledge"
trigger = "promotion_count >= 20"

# Score Cell: memetic fitness evaluation
[[graph.cells]]
id = "fitness-scorer"
protocol = "Score"
description = "Evaluate each promoted Signal's memetic fitness"

# Route Cell: tournament selection
[[graph.cells]]
id = "tournament-selector"
protocol = "Route"
description = "Select parents via tournament among scored Signals"

# Compose Cell: recombination
[[graph.cells]]
id = "recombiner"
protocol = "Compose"
description = "Combine selected parents into candidate Signals"

# Verify Cell: fitness threshold check
[[graph.cells]]
id = "fitness-gate"
protocol = "Verify"
description = "Gate candidates by minimum fitness threshold"

# Store Cell: write survivors back
[[graph.cells]]
id = "archive-writer"
protocol = "Store"
description = "Write surviving candidates to staging or MAP-Elites archive"

# Edges: linear pipeline with feedback
[[graph.edges]]
from = "store:promoted"
to = "fitness-scorer.in"

[[graph.edges]]
from = "fitness-scorer.out"
to = "tournament-selector.in"

[[graph.edges]]
from = "tournament-selector.out"
to = "recombiner.in"

[[graph.edges]]
from = "recombiner.out"
to = "fitness-gate.in"

[[graph.edges]]
from = "fitness-gate.passed"
to = "archive-writer.in"

# Feedback edge: archive contents inform next fitness scoring
[[graph.edges]]
from = "archive-writer.out"
to = "fitness-scorer.context"
kind = "feedback"
```

### Score Cell: Memetic Fitness

The fitness scorer evaluates each promoted Signal against the agent's recent episode history. The fitness function measures whether the Signal's presence correlates with successful outcomes:

```rust
/// Memetic fitness as a Score Cell.
///
/// Fitness = success_rate_when_referenced / success_rate_when_not_referenced
///
/// Uses Bayesian estimation with Monte Carlo sampling to account for
/// small sample sizes and confounding variables.
pub struct MemeticFitnessScorer {
    /// Minimum observations before evaluating fitness.
    pub min_observations: usize,            // default: 5

    /// Bayesian confidence threshold for classification.
    pub confidence_threshold: f64,          // default: 0.75

    /// Monte Carlo samples for posterior estimation.
    pub mc_samples: usize,                  // default: 10_000
}

impl MemeticFitnessScorer {
    /// Score a promoted Signal's memetic fitness.
    pub fn score(
        &self,
        signal: &Signal,
        episodes: &[Episode],
    ) -> FitnessScore {
        let referenced: Vec<_> = episodes.iter()
            .filter(|e| e.active_signals.contains(&signal.ref_()))
            .collect();
        let unreferenced: Vec<_> = episodes.iter()
            .filter(|e| !e.active_signals.contains(&signal.ref_()))
            .collect();

        if referenced.len() < self.min_observations {
            return FitnessScore::uncertain(signal.ref_());
        }

        let successes_ref = referenced.iter().filter(|e| e.succeeded).count();
        let successes_unref = unreferenced.iter().filter(|e| e.succeeded).count();

        // Monte Carlo estimation of P(fitness > 1.0)
        let mut count_above_one = 0;
        let mut samples = Vec::with_capacity(self.mc_samples);
        for _ in 0..self.mc_samples {
            let s_ref = sample_beta(
                successes_ref as f64 + 1.0,
                (referenced.len() - successes_ref) as f64 + 1.0,
            );
            let s_unref = sample_beta(
                successes_unref as f64 + 1.0,
                (unreferenced.len() - successes_unref) as f64 + 1.0,
            );
            let f = s_ref / s_unref.max(0.001);
            samples.push(f);
            if f > 1.0 { count_above_one += 1; }
        }

        let prob_beneficial = count_above_one as f64 / self.mc_samples as f64;
        let classification = if prob_beneficial > self.confidence_threshold {
            FitnessClassification::Beneficial
        } else if (1.0 - prob_beneficial) > self.confidence_threshold {
            FitnessClassification::Harmful
        } else {
            FitnessClassification::Uncertain
        };

        FitnessScore {
            signal_ref: signal.ref_(),
            point_estimate: mean(&samples),
            prob_beneficial,
            classification,
        }
    }
}

pub struct FitnessScore {
    pub signal_ref: SignalRef,
    pub point_estimate: f64,
    pub prob_beneficial: f64,
    pub classification: FitnessClassification,
}

pub enum FitnessClassification {
    Beneficial,     // P(fitness > 1.0) > threshold -> boost half-life 1.5x
    Harmful,        // P(fitness < 1.0) > threshold -> accelerate demurrage
    Uncertain,      // insufficient evidence -> no change
}
```

### Route Cell: Tournament Selection

Tournament selection picks parents for recombination. It samples `tournament_size` candidates from the fitness-scored pool and selects the fittest. This applies evolutionary pressure without discarding diversity entirely.

```rust
/// Tournament selection as a Route Cell.
///
/// Samples tournament_size candidates, selects the fittest.
/// Elitism guarantees top-fitness Signals survive unchanged.
pub struct TournamentSelector {
    pub tournament_size: usize,             // default: 4
    pub elitism_fraction: f64,              // default: 0.10
    pub max_population: usize,              // default: 200
}

impl TournamentSelector {
    /// Select parents for recombination.
    ///
    /// Returns pairs of parent SignalRefs for the Compose Cell.
    pub fn route(
        &self,
        scored: &[FitnessScore],
    ) -> Vec<(SignalRef, SignalRef)> {
        let mut pairs = Vec::new();

        // Elitism: top fraction passes through unchanged
        let elite_count = (scored.len() as f64 * self.elitism_fraction)
            .ceil() as usize;
        let mut sorted = scored.to_vec();
        sorted.sort_by(|a, b| b.point_estimate
            .partial_cmp(&a.point_estimate).unwrap());

        // Pair non-elite Signals via tournament
        let candidates: Vec<_> = sorted[elite_count..].to_vec();
        let pair_count = (self.max_population - elite_count) / 2;

        for _ in 0..pair_count {
            let parent_a = tournament_pick(&candidates, self.tournament_size);
            let parent_b = tournament_pick(&candidates, self.tournament_size);
            if parent_a.signal_ref != parent_b.signal_ref {
                pairs.push((parent_a.signal_ref.clone(), parent_b.signal_ref.clone()));
            }
        }

        pairs
    }
}
```

### Compose Cell: Recombination

The recombination Cell takes parent pairs and produces offspring Signals using HDC vector operations:

```rust
/// Recombination as a Compose Cell.
///
/// Uses HDC bundling + permutation to produce offspring Signals
/// that are related to both parents but distinct from either.
pub struct RecombinationComposer {
    pub crossover_rate: f64,                // default: 0.70
    pub mutation_rate: f64,                 // default: 0.15
}

impl RecombinationComposer {
    /// Compose offspring from parent pairs.
    pub fn compose(
        &self,
        parents: &[(Signal, Signal)],
        rng: &mut impl Rng,
    ) -> Vec<Signal> {
        let mut offspring = Vec::new();

        for (parent_a, parent_b) in parents {
            if rng.gen::<f64>() > self.crossover_rate {
                continue; // No crossover for this pair
            }

            // HDC recombination: bundle + permute
            let fp_a = parent_a.hdc_fingerprint.as_ref().unwrap();
            let fp_b = parent_b.hdc_fingerprint.as_ref().unwrap();
            let shift = rng.gen_range(1..64);
            let child_fp = HdcVector::bundle(&[fp_a, &fp_b.permute(shift)]);

            // Optional mutation: random bit flips
            let child_fp = if rng.gen::<f64>() < self.mutation_rate {
                child_fp.mutate(0.05) // flip ~5% of bits
            } else {
                child_fp
            };

            let mut child = Signal::new(
                Kind::StrategyFragment,
                format!(
                    "Evolved from {} x {}",
                    parent_a.ref_().short(),
                    parent_b.ref_().short(),
                ),
            );
            child.hdc_fingerprint = Some(child_fp);
            child.partition = Partition::Staging;
            child.balance = STAGING_DEMURRAGE.initial_balance;
            child.provenance = Provenance::Evolution {
                parent_a: parent_a.ref_(),
                parent_b: parent_b.ref_(),
            };

            offspring.push(child);
        }

        offspring
    }
}
```

### MAP-Elites Archive

The optional MAP-Elites archive maintains a quality-diversity grid indexed by behavioral descriptors. Each cell holds the highest-fitness Signal for that behavioral niche:

```rust
/// MAP-Elites archive for quality-diversity search.
///
/// Behavioral descriptors: task_domain, complexity_level, time_horizon.
/// Each cell holds the highest-fitness Signal for that niche.
/// QD-score (sum of all cell qualities) is monotonically non-decreasing.
pub struct MapElitesArchive {
    pub bins_per_dim: usize,                // default: 10
    pub max_archive: usize,                 // default: 1000
    pub min_quality: f64,                   // default: 0.30
    grid: HashMap<GridCoord, ArchiveEntry>,
}

impl MapElitesArchive {
    /// Insert a candidate if it improves its cell.
    /// Returns true if the candidate was inserted.
    pub fn try_insert(
        &mut self,
        signal: &Signal,
        fitness: f64,
        descriptors: &[f64],
    ) -> bool {
        if fitness < self.min_quality {
            return false;
        }

        let coord = self.discretize(descriptors);

        match self.grid.entry(coord) {
            Entry::Vacant(v) => {
                v.insert(ArchiveEntry {
                    signal_ref: signal.ref_(),
                    fitness,
                    descriptors: descriptors.to_vec(),
                });
                true
            }
            Entry::Occupied(mut o) => {
                if fitness > o.get().fitness {
                    o.insert(ArchiveEntry {
                        signal_ref: signal.ref_(),
                        fitness,
                        descriptors: descriptors.to_vec(),
                    });
                    true
                } else {
                    false
                }
            }
        }
    }

    /// QD-score: sum of all occupied cell fitness values.
    /// Monotonically non-decreasing by construction (cells only improve).
    pub fn qd_score(&self) -> f64 {
        self.grid.values().map(|e| e.fitness).sum()
    }

    /// Coverage: fraction of cells occupied.
    pub fn coverage(&self) -> f64 {
        self.grid.len() as f64 / self.max_archive as f64
    }
}
```

---

## 6. Promotion as a Trigger Cell

When a staging Signal's balance reaches the promotion threshold, a **Trigger Cell** fires the promotion workflow. This is not a periodic scan -- it is event-driven, triggered by the confirmation boost React Cell updating a Signal's balance.

```rust
/// Promotion Trigger: fires when a staging Signal reaches balance >= 0.30.
///
/// Promotion is a partition change + tier transition, not a copy.
/// The Signal stays in the same Store with the same content hash.
pub struct PromotionTrigger {
    pub balance_threshold: f64,             // default: 0.30
}

impl PromotionTrigger {
    /// Check and promote a Signal after a confirmation boost.
    pub async fn check_and_promote(
        &self,
        signal: &mut Signal,
        store: &mut dyn Store,
    ) -> Option<PromotionEvent> {
        if signal.partition != Partition::Staging {
            return None;
        }
        if signal.balance < self.balance_threshold {
            return None;
        }

        // Step 1: Assign knowledge type from generation mode
        let kind = classify_knowledge_type(&signal.provenance);

        // Step 2: Transition partition and tier
        let old_partition = signal.partition;
        signal.partition = Partition::Promoted;
        signal.tier = Tier::Working; // Enters Working tier of the main demurrage schedule
        signal.kind = kind;

        // Step 3: Write updated Signal to Store
        store.update(signal.clone()).await;

        // Step 4: Emit promotion Pulse
        let event = PromotionEvent {
            signal_ref: signal.ref_(),
            from_partition: old_partition,
            to_partition: Partition::Promoted,
            assigned_kind: kind,
            balance_at_promotion: signal.balance,
            timestamp: Utc::now(),
        };

        let pulse = Pulse::new(
            Topic::parse("staging.promoted"),
            event.clone(),
        );
        bus.publish(pulse).await;

        Some(event)
    }
}

/// Map dream generation mode to knowledge Signal kind.
fn classify_knowledge_type(provenance: &Provenance) -> Kind {
    match provenance {
        Provenance::Dream { source_phase: SourcePhase::NremReplay, .. } => Kind::Insight,
        Provenance::Dream { source_phase: SourcePhase::NremCrossEpisode, .. } => Kind::Insight,
        Provenance::Dream { source_phase: SourcePhase::RemCounterfactual, .. } => Kind::Heuristic,
        Provenance::Dream { source_phase: SourcePhase::RemCombinational, .. } => Kind::Insight,
        Provenance::Dream { source_phase: SourcePhase::RemTransformational, .. } => Kind::StrategyFragment,
        Provenance::Dream { source_phase: SourcePhase::ThreatSimulation, .. } => Kind::Warning,
        Provenance::Evolution { .. } => Kind::StrategyFragment,
        _ => Kind::Insight,
    }
}
```

---

## 7. The Full Consolidation Graph

Putting it all together, the consolidation system is a Graph with five Cells wired by Bus Pulses:

```
                          Bus: "verify.verdict"
                                |
                                v
                    +---------------------------+
                    | ConfirmationBoostCell     |  React
                    | (watches verdict Pulses)  |
                    +---------------------------+
                                |
                     updates staging Signals
                                |
                                v
                    +---------------------------+
                    | PromotionTrigger          |  Trigger
                    | (fires on balance >= 0.30)|
                    +---------------------------+
                                |
                     emits "staging.promoted" Pulse
                                |
                                v
                    +---------------------------+
                    | ShyRenormalizationFunctor |  Functor (periodic, per cycle)
                    | (global downscaling)      |
                    +---------------------------+
                                |
                     downscales all staging Signals
                                |
         (when 20+ promotions)  |
                                v
               +------------------------------------+
               | Dream Evolution Loop               |  Loop (conditional)
               |  Score -> Route -> Compose -> Gate |
               +------------------------------------+
                                |
                     new candidates -> staging
```

The first two Cells (ConfirmationBoost and PromotionTrigger) run continuously during waking operation. The Functor runs once per consolidation cycle. The Evolution Loop fires conditionally after sufficient promotions accumulate.

---

## What This Enables

1. **Unified economics**: Staging, promotion, and expiry are all demurrage-driven. No separate timers, no separate expiry cron, no separate confidence tracking. One mechanism -- demurrage with reinforcement -- governs everything.

2. **Emergent confidence ladder**: The five-stage ladder is not a state machine with explicit transitions. It emerges from balance dynamics. A Signal's position on the ladder is always computable from its current balance. This eliminates state synchronization bugs between "confidence" and "status" fields.

3. **Self-regulating staging capacity**: The 14-day expiry is not a hard cutoff -- it is the natural consequence of aggressive demurrage. A Signal that receives even one confirmation survives longer. A Signal that receives frequent confirmations can stay in staging indefinitely, accumulating evidence until it reaches the promotion threshold. The system self-regulates: valuable hypotheses survive, worthless ones expire, without explicit expiry logic.

4. **Evolution as optional optimization**: The Evolution Loop is an overlay, not a requirement. Without it, consolidation still works: staging, demurrage, confirmation, promotion. The Loop adds quality-diversity search on top, but the base system is complete without it.

5. **Bus-driven reactivity**: The confirmation boost fires in response to Verify verdict Pulses, not on a polling interval. This means staging Signals get updated immediately when relevant waking evidence appears, not at the next scheduled consolidation cycle.

---

## Feedback Loops

1. **Confirm -> Boost -> Promote -> Better heuristics -> More successes -> More confirmations**: The virtuous cycle. Staging Signals that help the agent succeed generate the evidence that promotes them. Once promoted, they enter the agent's active knowledge, making it more likely the agent succeeds again.

2. **Refute -> Penalize -> Expire -> Cleaner Store -> Less noise -> Better confirmations**: The immune response. Refuted hypotheses lose balance, approach the cold floor, and expire. This clears noise from the staging partition, making HDC similarity queries more precise for remaining hypotheses.

3. **SHY Functor -> Downscale -> Only confirmed survive -> Higher promotion quality**: The filter. Periodic downscaling ensures that only hypotheses with sustained waking evidence survive to promotion. Without SHY, borderline hypotheses could accumulate just enough balance from a single confirmation to hover near the threshold indefinitely.

4. **Evolution -> New candidates -> Staging -> Confirm/Expire -> Evolution input**: The innovation cycle. Evolutionary recombination produces novel candidates that enter staging. Some survive confirmation and get promoted, feeding the next Evolution cycle with richer material. Failed candidates expire, wasting only the staging slot they occupied.

5. **Promotion count -> Trigger Evolution -> Diversify strategies -> Better performance -> More promotions**: The growth spiral. More promotions trigger Evolution, which diversifies the strategy repertoire, which improves performance on a wider range of tasks, which produces more promotions.

---

## Open Questions

1. **Staging-to-promoted demurrage discontinuity**: When a Signal is promoted, it transitions from the aggressive staging demurrage schedule (half-life ~5 days) to the standard promoted schedule (half-life per kind, e.g., 69 days for Heuristic). This is a sharp discontinuity in the dynamics. Should there be a transitional demurrage schedule for recently-promoted Signals that ramps from staging rates to promoted rates over a week?

2. **Cross-agent confirmation**: In a fleet, should a confirmation from Agent B count as an independent confirmation for Agent A's staging hypothesis? The confirmation boost Cell currently requires `pulse.provenance().is_dream() == false` but does not check agent identity. Cross-agent confirmation could accelerate promotion but also introduces the risk of correlated evidence (agents working on similar tasks confirm each other's hypotheses without true independence).

3. **SHY Functor timing with Evolution**: The Functor runs before promotion checks. The Evolution Loop runs after sufficient promotions. But the Functor also affects Signals that the Evolution Loop might want to use as parents. Should the Evolution Loop snapshot the pre-Functor state, or should it operate on the post-Functor state? The former preserves fitness scores; the latter reflects the "true" post-consolidation balance.

4. **MAP-Elites descriptor selection**: The behavioral descriptors for the MAP-Elites archive (task_domain, complexity_level, time_horizon) are hardcoded. Should the descriptor dimensions themselves evolve based on which dimensions show the most performance variation? This is meta-evolution -- evolving the search space, not just the solutions.

5. **Staging partition GC under load**: With 1,000 max staging entries and aggressive demurrage, the GC burden is modest. But during a burst of dream cycles (e.g., post-crisis intensive threat rehearsal), the staging partition could churn hundreds of entries per cycle. Should GC be batched (once per cycle) or incremental (per entry expiry)?
