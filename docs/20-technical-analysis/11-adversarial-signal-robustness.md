# Adversarial Signal Robustness

> Every domain has adversaries who manipulate signals. MEV searchers manipulate prices. Attackers manipulate supply chains. p-hackers manipulate statistics. The adversarial robustness subsystem defends predictions through HDC prototype matching, robust statistics, and red-team dreaming.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [06-hyperdimensional-ta](./06-hyperdimensional-ta.md) for HDC encoding, [09-causal-microstructure-discovery](./09-causal-microstructure-discovery.md) for causal analysis
**Key sources**: `bardo-backup/prd/23-ta/08-adversarial-signal-robustness.md`

---

## Adversarial signal decomposition

Every observed signal is a mixture of genuine information and adversarial manipulation. The first defense is decomposition — separating the signal into components and identifying which parts are trustworthy:

```rust
/// Decompose a signal into genuine and adversarial components.
///
/// The decomposition model:
///   observed = genuine + adversarial + noise
///
/// Genuine: reflects actual state (market fundamentals, code quality)
/// Adversarial: intentional manipulation (MEV, supply chain, p-hacking)
/// Noise: random, zero-mean disturbance
///
/// Identification uses multiple methods:
/// - Statistical outlier detection (robust statistics)
/// - Causal consistency (does this signal fit the causal model?)
/// - HDC prototype matching (does this match a known attack pattern?)
/// - Cross-source verification (do independent sources agree?)
pub struct AdversarialDecomposer {
    /// Robust statistics engine.
    robust_stats: RobustStatistics,

    /// Causal model for consistency checking.
    causal_model: Arc<StructuralCausalModel>,

    /// HDC prototypes of known attack patterns.
    attack_prototypes: Vec<HdcVector>,

    /// Cross-source verifier.
    cross_verifier: CrossSourceVerifier,
}

pub struct SignalDecomposition {
    /// Estimated genuine component.
    pub genuine: f64,

    /// Estimated adversarial component.
    pub adversarial: f64,

    /// Estimated noise component.
    pub noise: f64,

    /// Confidence in the decomposition.
    pub confidence: f64,

    /// If adversarial component is significant: which attack pattern matched?
    pub attack_match: Option<AttackPatternMatch>,
}
```

---

## HDC prototype matching — Nanosecond attack detection

Known adversarial patterns are encoded as HDC prototype vectors. Incoming signals are compared against all prototypes via Hamming similarity:

```rust
/// HDC prototype matching for adversarial pattern detection.
///
/// Cost: ~10ns per prototype comparison (XOR + popcount on 10,240 bits).
/// For 1,000 known attack patterns: ~10µs total.
///
/// This is fast enough to run at Gamma frequency on every observation.
pub struct PrototypeMatcher {
    /// Known adversarial pattern prototypes.
    prototypes: Vec<PrototypeEntry>,

    /// Similarity threshold for match detection (typically 0.6).
    threshold: f64,
}

pub struct PrototypeEntry {
    /// The HDC prototype vector.
    pub vector: HdcVector,

    /// Human-readable name of the attack pattern.
    pub name: String,

    /// Domain this prototype belongs to.
    pub domain: OracleDomain,

    /// Severity if this pattern is detected.
    pub severity: f64,

    /// Recommended response.
    pub response: AdversarialResponse,
}

pub enum AdversarialResponse {
    /// Widen prediction intervals (increase uncertainty).
    WidenIntervals(f64),

    /// Suppress action (wait for the adversarial activity to pass).
    SuppressAction(Duration),

    /// Escalate to T2 (deep reasoning) for manual analysis.
    EscalateToT2,

    /// Emit a Warning to Neuro.
    EmitWarning(String),
}

impl PrototypeMatcher {
    /// Match an observation against all prototypes.
    ///
    /// Returns all prototypes with similarity above threshold,
    /// sorted by similarity (most similar first).
    pub fn match_prototypes(&self, observation: &HdcVector) -> Vec<(f64, &PrototypeEntry)> {
        self.prototypes.iter()
            .filter_map(|proto| {
                let sim = observation.hamming_similarity(&proto.vector);
                if sim > self.threshold {
                    Some((sim, proto))
                } else {
                    None
                }
            })
            .sorted_by(|a, b| b.0.partial_cmp(&a.0).unwrap())
            .collect()
    }
}
```

### Domain-specific attack prototypes

```rust
/// Chain domain attack prototypes.
pub fn chain_attack_prototypes(codebook: &DeFiCodebook) -> Vec<PrototypeEntry> {
    vec![
        // Sandwich attack: buy before victim, sell after
        PrototypeEntry {
            vector: encode_sandwich_pattern(codebook),
            name: "sandwich_attack".into(),
            domain: OracleDomain::Chain,
            severity: 0.8,
            response: AdversarialResponse::SuppressAction(Duration::from_secs(12)),
        },

        // Oracle manipulation: flash loan → manipulate price feed → profit
        PrototypeEntry {
            vector: encode_oracle_manipulation_pattern(codebook),
            name: "oracle_manipulation".into(),
            domain: OracleDomain::Chain,
            severity: 0.9,
            response: AdversarialResponse::EscalateToT2,
        },

        // Governance attack: flash loan → vote → profit
        PrototypeEntry {
            vector: encode_governance_attack_pattern(codebook),
            name: "governance_attack".into(),
            domain: OracleDomain::Chain,
            severity: 0.95,
            response: AdversarialResponse::EmitWarning("Potential governance attack detected".into()),
        },

        // JIT liquidity sniping
        PrototypeEntry {
            vector: encode_jit_sniping_pattern(codebook),
            name: "jit_sniping".into(),
            domain: OracleDomain::Chain,
            severity: 0.5,
            response: AdversarialResponse::WidenIntervals(0.2),
        },
    ]
}

/// Coding domain attack prototypes.
pub fn coding_attack_prototypes(codebook: &CodingCodebook) -> Vec<PrototypeEntry> {
    vec![
        // Dependency confusion: malicious package name squatting
        PrototypeEntry {
            vector: encode_dep_confusion_pattern(codebook),
            name: "dependency_confusion".into(),
            domain: OracleDomain::Coding,
            severity: 0.9,
            response: AdversarialResponse::EscalateToT2,
        },

        // Typosquatting: similar package name with malicious payload
        PrototypeEntry {
            vector: encode_typosquatting_pattern(codebook),
            name: "typosquatting".into(),
            domain: OracleDomain::Coding,
            severity: 0.85,
            response: AdversarialResponse::EmitWarning("Potential typosquatting detected".into()),
        },

        // Build artifact tampering
        PrototypeEntry {
            vector: encode_build_tampering_pattern(codebook),
            name: "build_tampering".into(),
            domain: OracleDomain::Coding,
            severity: 0.9,
            response: AdversarialResponse::EscalateToT2,
        },
    ]
}
```

---

## Robust statistics — Defending numerical estimates

When adversarial manipulation is suspected, standard statistics (mean, variance) are unreliable. Robust estimators resist contamination:

```rust
/// Robust statistics for adversarial-contaminated data.
///
/// Each estimator resists a fraction of contaminated data points
/// (the "breakdown point"). Standard mean has breakdown point 0
/// (one outlier can destroy it). These estimators have breakdown
/// points of 25-50%.
pub struct RobustStatistics;

impl RobustStatistics {
    /// Trimmed mean: discard the top and bottom α fraction before averaging.
    ///
    /// Breakdown point: α (typically 0.1-0.25).
    /// Removes extreme values that may be adversarial.
    pub fn trimmed_mean(data: &[f64], alpha: f64) -> f64 {
        let n = data.len();
        let trim = (n as f64 * alpha) as usize;
        let mut sorted = data.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let trimmed = &sorted[trim..n - trim];
        trimmed.iter().sum::<f64>() / trimmed.len() as f64
    }

    /// Hodges-Lehmann estimator: median of all pairwise averages.
    ///
    /// Breakdown point: ~29%.
    /// More efficient than trimmed mean for symmetric distributions.
    pub fn hodges_lehmann(data: &[f64]) -> f64 {
        let n = data.len();
        let mut pairwise_means = Vec::with_capacity(n * (n + 1) / 2);

        for i in 0..n {
            for j in i..n {
                pairwise_means.push((data[i] + data[j]) / 2.0);
            }
        }

        pairwise_means.sort_by(|a, b| a.partial_cmp(b).unwrap());
        pairwise_means[pairwise_means.len() / 2]
    }

    /// Winsorized variance: clip extreme values before computing variance.
    ///
    /// Breakdown point: α.
    /// More stable than standard variance under contamination.
    pub fn winsorized_variance(data: &[f64], alpha: f64) -> f64 {
        let n = data.len();
        let trim = (n as f64 * alpha) as usize;
        let mut sorted = data.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let lower = sorted[trim];
        let upper = sorted[n - trim - 1];

        let winsorized: Vec<f64> = data.iter()
            .map(|&x| x.clamp(lower, upper))
            .collect();

        let mean = winsorized.iter().sum::<f64>() / n as f64;
        winsorized.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64
    }

    /// Median Absolute Deviation (MAD): robust scale estimator.
    ///
    /// Breakdown point: 50% (highest possible).
    /// MAD = median(|x_i - median(x)|)
    /// Scaled MAD: 1.4826 * MAD ≈ standard deviation for Gaussian data.
    pub fn mad(data: &[f64]) -> f64 {
        let median = Self::median(data);
        let deviations: Vec<f64> = data.iter().map(|x| (x - median).abs()).collect();
        Self::median(&deviations)
    }

    /// Rank-order transformation: replace values with ranks.
    ///
    /// Completely eliminates magnitude-based manipulation.
    /// Preserves ordinal relationships but discards scale information.
    pub fn rank_transform(data: &[f64]) -> Vec<f64> {
        let n = data.len() as f64;
        let mut indexed: Vec<_> = data.iter().enumerate().collect();
        indexed.sort_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap());

        let mut ranks = vec![0.0; data.len()];
        for (rank, (original_idx, _)) in indexed.iter().enumerate() {
            ranks[*original_idx] = rank as f64 / n;
        }
        ranks
    }

    fn median(data: &[f64]) -> f64 {
        let mut sorted = data.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = sorted.len();
        if n % 2 == 0 {
            (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
        } else {
            sorted[n / 2]
        }
    }
}
```

---

## Signal cross-validation

Multiple independent signals predicting the same outcome should agree. Disagreement indicates either adversarial manipulation or model error:

```rust
/// Cross-validate signals predicting the same outcome.
///
/// If multiple independent signals (e.g., different data sources
/// predicting the same price) disagree significantly, at least one
/// is compromised. The cross-validator identifies outlier signals.
pub struct SignalCrossValidator {
    /// Maximum acceptable disagreement (MAD-based).
    max_disagreement: f64,
}

impl SignalCrossValidator {
    /// Validate a set of predictions for the same outcome.
    pub fn validate(&self, predictions: &[(SignalId, f64)]) -> CrossValidationResult {
        let values: Vec<f64> = predictions.iter().map(|(_, v)| *v).collect();
        let median = RobustStatistics::median(&values);
        let mad = RobustStatistics::mad(&values);

        let outliers: Vec<SignalId> = predictions.iter()
            .filter(|(_, v)| (*v - median).abs() > self.max_disagreement * mad * 1.4826)
            .map(|(id, _)| *id)
            .collect();

        CrossValidationResult {
            consensus: median,
            spread: mad * 1.4826,  // scaled MAD ≈ std dev
            outlier_signals: outliers,
            is_consistent: outliers.is_empty(),
        }
    }
}
```

---

## Red-team dreaming — Adversarial simulation

During Delta-frequency Dreams, the agent runs adversarial simulations against its own strategies:

```rust
/// Red-team dreaming: the agent attacks its own strategies.
///
/// During REM Dreams, the agent generates adversarial scenarios
/// and tests whether its current predictions and strategies survive.
///
/// Algorithm:
/// 1. Select the agent's top N active strategies
/// 2. For each strategy, generate K adversarial perturbations
/// 3. Simulate each perturbation (via mirage-rs or workspace snapshot)
/// 4. If the strategy fails under perturbation:
///    a. Demote the strategy's confidence
///    b. Store the adversarial scenario as a Warning in Neuro
///    c. Generate a defensive modification
///
/// This is how agents develop adversarial robustness WITHOUT
/// encountering real attacks.
pub struct RedTeamDreaming {
    /// The agent's current active strategies.
    strategies: Vec<StrategyFragment>,

    /// Adversarial perturbation generators per domain.
    perturbation_generators: HashMap<OracleDomain, Box<dyn PerturbationGenerator>>,

    /// Simulation environment.
    simulator: Arc<dyn Simulator>,
}

pub trait PerturbationGenerator: Send + Sync {
    /// Generate adversarial perturbations for a strategy.
    fn generate(
        &self,
        strategy: &StrategyFragment,
        n_perturbations: usize,
    ) -> Vec<AdversarialPerturbation>;
}

pub struct AdversarialPerturbation {
    /// What was changed (e.g., "2x slippage", "5x gas", "correlation breakdown").
    pub description: String,

    /// The perturbation as a state modification.
    pub modification: StateModification,

    /// Severity of the adversarial scenario.
    pub severity: f64,
}

impl RedTeamDreaming {
    /// Run one red-team dreaming cycle.
    pub async fn dream_cycle(&self) -> Vec<RedTeamResult> {
        let mut results = Vec::new();

        for strategy in &self.strategies {
            let domain = strategy.domain();
            if let Some(generator) = self.perturbation_generators.get(&domain) {
                let perturbations = generator.generate(strategy, 5);

                for perturbation in perturbations {
                    let outcome = self.simulator
                        .simulate_with_perturbation(strategy, &perturbation)
                        .await;

                    let survived = outcome.success_rate > 0.5;

                    results.push(RedTeamResult {
                        strategy_id: strategy.id,
                        perturbation: perturbation.description.clone(),
                        survived,
                        outcome_detail: outcome,
                    });

                    if !survived {
                        // Strategy failed — this is a discovered vulnerability
                        // Store as Warning in Neuro during dream integration phase
                    }
                }
            }
        }

        results
    }
}
```

### Chain-domain adversarial perturbations

```rust
/// Chain-specific adversarial perturbations for red-team dreaming.
pub struct ChainPerturbationGenerator;

impl PerturbationGenerator for ChainPerturbationGenerator {
    fn generate(
        &self,
        strategy: &StrategyFragment,
        n: usize,
    ) -> Vec<AdversarialPerturbation> {
        vec![
            // What if slippage is 2x higher than expected?
            AdversarialPerturbation {
                description: "2x slippage spike".into(),
                modification: StateModification::ScaleVariable("slippage", 2.0),
                severity: 0.6,
            },
            // What if gas is 5x higher?
            AdversarialPerturbation {
                description: "5x gas spike".into(),
                modification: StateModification::ScaleVariable("gas_price", 5.0),
                severity: 0.7,
            },
            // What if correlations break down?
            AdversarialPerturbation {
                description: "Correlation breakdown: ETH/BTC decorrelation".into(),
                modification: StateModification::BreakCorrelation("eth", "btc"),
                severity: 0.8,
            },
            // What if a pool is drained (rug pull)?
            AdversarialPerturbation {
                description: "Pool drain: 90% liquidity removal".into(),
                modification: StateModification::ScaleVariable("pool_liquidity", 0.1),
                severity: 0.95,
            },
            // What if a sandwich attack targets the strategy?
            AdversarialPerturbation {
                description: "Sandwich attack on primary swap".into(),
                modification: StateModification::InjectSandwich("primary_swap"),
                severity: 0.7,
            },
        ].into_iter().take(n).collect()
    }
}
```

---

## Integration with the Daimon

Adversarial detection feeds directly into the Daimon PAD vector:

- **Detection of adversarial activity** → increases Arousal (urgency)
- **Failed red-team defense** → decreases Dominance (confidence)
- **Successful defense** → increases Pleasure (positive outcome)

This creates a feedback loop: adversarial pressure raises arousal, which routes more cycles to T2 (deep reasoning), which enables more thorough analysis.

---

## Implementation details

### Attack prototype HDC encoding

Each attack prototype is encoded as an HDC vector that captures the structural signature of the attack pattern. The encoding method varies by domain:

```rust
/// Encode a chain attack prototype as an HDC vector.
///
/// The encoding captures the temporal and structural fingerprint of the attack.
/// For a sandwich attack:
///   TEMPORAL_PATTERN(
///     BIND(swap_role, large_buy),       // step 0: frontrun
///     BIND(swap_role, victim_trade),     // step 1: victim
///     BIND(swap_role, large_sell),       // step 2: backrun
///   )
/// bundled with:
///   BIND(timing_role, same_block),       // timing constraint
///   BIND(profit_role, positive),         // profit indicator
///
/// The resulting vector matches any structurally similar sandwich pattern
/// regardless of specific tokens, amounts, or protocols.
pub fn encode_attack_prototype(
    pattern: &AttackPatternDef,
    codebook: &DeFiCodebook,
) -> HdcVector {
    // Encode the temporal sequence of actions
    let temporal = encode_temporal_pattern(
        &pattern.steps.iter()
            .map(|step| encode_ta_state(&step.role_filler_pairs(codebook)))
            .collect::<Vec<_>>()
    );

    // Encode structural constraints (timing, profit, etc.)
    let constraints: Vec<HdcVector> = pattern.constraints.iter()
        .map(|c| c.encode(codebook))
        .collect();

    // Bundle temporal pattern with constraints
    let mut all = vec![temporal];
    all.extend(constraints);
    HdcVector::bundle(&all)
}

/// For coding domain attacks, encode the supply chain signature:
///   BIND(package_role, name_similarity_vector),
///   BIND(action_role, install_hook | postinstall_script),
///   BIND(timing_role, recent_publish),
pub fn encode_coding_attack_prototype(
    pattern: &CodingAttackDef,
    codebook: &CodingCodebook,
) -> HdcVector {
    let components: Vec<HdcVector> = pattern.indicators.iter()
        .map(|indicator| {
            let role = codebook.role_for(indicator.kind);
            let filler = codebook.encode_indicator_value(&indicator.value);
            role.xor(&filler)
        })
        .collect();
    HdcVector::bundle(&components)
}
```

### Prototype selection: count and update procedure

| Domain | Initial prototype count | Source | Update cadence |
|---|---|---|---|
| Chain | 20-50 | Known MEV patterns from Flashbots data, historical exploits | Delta frequency (daily) |
| Coding | 15-30 | Known supply chain attacks from OSV/advisories | Weekly or on new advisory |
| Research | 5-10 | Known p-hacking patterns from replication crisis literature | Monthly |

**Update procedure**:

1. When a new attack is confirmed (by red-team dreaming or external report), encode it as a prototype.
2. Compute similarity to existing prototypes. If max similarity > 0.8, update the existing prototype via bundle (strengthens shared structure).
3. If max similarity <= 0.8, add as a new prototype.
4. Prune prototypes with zero matches in the last 30 days, unless they represent critical attack classes (severity >= 0.9).

```rust
/// Update prototypes with a newly confirmed attack pattern.
pub fn update_prototypes(
    prototypes: &mut Vec<PrototypeEntry>,
    new_attack: &HdcVector,
    metadata: PrototypeMetadata,
    merge_threshold: f64,  // default: 0.8
) {
    let best_match = prototypes.iter_mut()
        .map(|p| (p, new_attack.hamming_similarity(&p.vector)))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

    match best_match {
        Some((existing, sim)) if sim > merge_threshold => {
            // Merge: bundle existing with new to strengthen shared structure
            existing.vector = existing.vector.bundle_with(new_attack);
            existing.last_matched = now_ms();
        }
        _ => {
            // Add as new prototype
            prototypes.push(PrototypeEntry {
                vector: new_attack.clone(),
                name: metadata.name,
                domain: metadata.domain,
                severity: metadata.severity,
                response: metadata.response,
            });
        }
    }
}
```

### Robust statistics: adaptive trim fraction

The trim fraction alpha for the trimmed mean should adapt based on the suspected contamination rate:

```rust
/// Adaptive trim fraction selection.
///
/// If adversarial activity is detected (prototype match or cross-source
/// disagreement), increase alpha to resist heavier contamination.
///
/// Default: alpha = 0.10 (handles up to 10% contamination).
/// Under adversarial pressure: alpha = min(0.25, 2 * estimated_contamination).
/// Maximum useful alpha: 0.25 (trimming more than 25% from each tail
/// discards too much genuine data).
pub fn adaptive_trim_alpha(
    adversarial_detected: bool,
    estimated_contamination: Option<f64>,
) -> f64 {
    match (adversarial_detected, estimated_contamination) {
        (false, _) => 0.10,                                    // baseline
        (true, Some(rate)) => (2.0 * rate).clamp(0.10, 0.25),  // adaptive
        (true, None) => 0.20,                                   // conservative default
    }
}
```

### Hodges-Lehmann caching for n > 1000

The Hodges-Lehmann estimator computes the median of all `n*(n+1)/2` pairwise averages. For n = 1000, this is ~500K pairs (fast). For n > 1000, the O(n^2) cost becomes significant:

```rust
/// Hodges-Lehmann estimator with subsampling for large n.
///
/// For n <= 1000: exact computation (500K pairs, ~1ms).
/// For n > 1000: subsample to 1000 points, compute exactly on the subsample.
///   Error bound: O(1/sqrt(1000)) = ~3% relative to exact.
///   The subsample is drawn without replacement using reservoir sampling.
///
/// Alternative for very large n: use the Johnson-Ethier approximation
/// (Hodges-Lehmann ~ median + O(1/n)), but this loses robustness.
pub fn hodges_lehmann_cached(data: &[f64], max_exact_n: usize) -> f64 {
    if data.len() <= max_exact_n {
        return RobustStatistics::hodges_lehmann(data);
    }

    // Subsample
    let sample = reservoir_sample(data, max_exact_n);
    RobustStatistics::hodges_lehmann(&sample)
}
```

### MAD scaling constant

The MAD is scaled by 1.4826 to estimate the standard deviation under a Gaussian distribution. This constant equals `1 / Phi_inv(3/4)` where `Phi_inv` is the inverse normal CDF. For non-Gaussian distributions, the constant differs:

| Distribution | Correct scaling constant | When to use |
|---|---|---|
| Gaussian | 1.4826 | Default assumption |
| Laplace (heavy-tailed) | 1.0 | When data has fat tails (common in DeFi) |
| Uniform | 1.1547 | When data is bounded |
| Unknown | 1.4826 | Safe default (overestimates for fat tails, conservative) |

### Cross-source verification

```rust
/// Cross-source verification configuration.
pub struct CrossSourceConfig {
    /// Minimum number of independent sources required for verification.
    /// Default: 3. With fewer sources, mark the signal as unverified.
    pub min_independent_sources: usize,

    /// Maximum acceptable MAD-normalized disagreement between sources.
    /// Default: 3.0 (sources within 3 MAD-scaled deviations agree).
    pub max_disagreement_mad: f64,

    /// Reliability weights per source (higher = more trusted).
    /// Sources with reliability < 0.3 are excluded from consensus.
    pub source_weights: HashMap<SourceId, f64>,
}

impl CrossSourceConfig {
    /// Compute reliability-weighted consensus.
    ///
    /// Each source contributes proportionally to its reliability weight.
    /// The consensus is the weighted median (robust to a single bad source).
    pub fn weighted_consensus(&self, predictions: &[(SourceId, f64)]) -> f64 {
        let filtered: Vec<(f64, f64)> = predictions.iter()
            .filter_map(|(id, v)| {
                let w = self.source_weights.get(id).copied().unwrap_or(0.5);
                if w >= 0.3 { Some((*v, w)) } else { None }
            })
            .collect();
        weighted_median(&filtered)
    }
}
```

### Red-team dreaming: strategy selection and severity

Red-team dreaming selects strategies to attack based on exposure and novelty:

```rust
/// Select strategies for red-team dreaming.
///
/// Priority order:
/// 1. Strategies with highest current exposure (most capital/attention at risk).
/// 2. Strategies that have not been red-teamed in the last 7 days.
/// 3. Strategies that survived all previous red-teams (they may have
///    undiscovered vulnerabilities).
pub fn select_red_team_targets(
    strategies: &[StrategyFragment],
    red_team_history: &HashMap<StrategyId, DateTime>,
    max_targets: usize,
) -> Vec<&StrategyFragment> {
    let mut scored: Vec<_> = strategies.iter()
        .map(|s| {
            let exposure_score = s.exposure_value();
            let staleness = red_team_history.get(&s.id)
                .map(|last| (now() - *last).as_secs_f64() / 86400.0)
                .unwrap_or(30.0); // never tested = 30 days stale
            let survived_all = s.red_team_failures == 0;
            let priority = exposure_score * staleness * if survived_all { 2.0 } else { 1.0 };
            (s, priority)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scored.into_iter().take(max_targets).map(|(s, _)| s).collect()
}
```

**Perturbation severity levels**:

| Severity | Description | Example perturbations |
|---|---|---|
| 0.0 - 0.3 | Mild | 10% price change, 2x gas, minor slippage |
| 0.3 - 0.6 | Moderate | 30% price change, 5x gas, correlation weakening |
| 0.6 - 0.8 | Severe | 50% price change, pool 50% drained, correlation breakdown |
| 0.8 - 1.0 | Extreme | 90% price crash, pool fully drained, sandwich + frontrun combo |

**Success/failure criteria**: A strategy "survives" a perturbation if its simulated PnL remains above -10% (chain domain) or its test pass rate remains above 80% (coding domain). Failure triggers confidence demotion and a Warning stored in Neuro.

### Somatic marker formation from adversarial events

When adversarial activity triggers an `AdversarialResponse`, the event feeds into the Daimon PAD vector and forms a somatic marker:

```rust
/// Map AdversarialResponse to Daimon PAD changes.
pub fn adversarial_response_to_pad(response: &AdversarialResponse) -> PadDelta {
    match response {
        AdversarialResponse::WidenIntervals(amount) => PadDelta {
            pleasure: -0.1,                // mild negative valence
            arousal: 0.2 * amount,         // proportional urgency
            dominance: -0.1,               // slight loss of control
        },
        AdversarialResponse::SuppressAction(duration) => PadDelta {
            pleasure: -0.2,
            arousal: 0.4,                  // significant urgency
            dominance: -0.3,               // loss of agency (can't act)
        },
        AdversarialResponse::EscalateToT2 => PadDelta {
            pleasure: -0.3,
            arousal: 0.6,                  // high urgency
            dominance: -0.5,               // significant loss of control
        },
        AdversarialResponse::EmitWarning(_) => PadDelta {
            pleasure: -0.15,
            arousal: 0.3,
            dominance: -0.2,
        },
    }
}

/// Form a somatic marker for an adversarial event.
///
/// The marker binds the attack pattern vector with the PAD response,
/// enabling future fast retrieval: "I've seen something like this before,
/// and it felt bad."
pub fn form_adversarial_marker(
    attack_vector: &HdcVector,
    response: &AdversarialResponse,
    codebook: &AffectCodebook,
) -> SomaticMarker {
    let pad = adversarial_response_to_pad(response);
    let affect_hv = encode_pad(pad.pleasure, pad.arousal, pad.dominance, codebook);
    let marker_hv = attack_vector.xor(&affect_hv);

    SomaticMarker {
        marker_hv,
        pattern_hv: attack_vector.clone(),
        affect_hv,
        pleasure: pad.pleasure,
        arousal: pad.arousal,
        dominance: pad.dominance,
        strength: 1.5, // adversarial markers start stronger (high-salience events)
        episode_sources: vec![],
        created_at_ms: now_ms(),
    }
}
```

### Test criteria

- **Prototype matching recall**: Against a test set of 100 known sandwich attacks, the PrototypeMatcher detects >= 90% with threshold = 0.6.
- **Prototype matching precision**: Against 1000 random (non-attack) observations, false positive rate < 5%.
- **Robust statistics breakdown**: The trimmed mean with alpha = 0.25 survives 25% contamination (estimate within 10% of true mean).
- **Hodges-Lehmann accuracy**: Subsampled estimate (n=1000 from n=10000) is within 5% of exact estimate.
- **Cross-source consensus**: When 2 of 3 sources agree and 1 is adversarial, the consensus matches the honest majority.
- **Red-team coverage**: After 10 dream cycles, every strategy with exposure > 0 has been red-teamed at least once.
- **Somatic marker formation**: After an adversarial event, a somatic marker exists with correct PAD sign (negative pleasure, positive arousal).

---

## Academic foundations

- Huber, P. J. (1964). "Robust Estimation of a Location Parameter." *Annals of Mathematical Statistics*, 35(1), 73-101. — Robust statistics foundations.
- Hodges, J. L., & Lehmann, E. L. (1963). "Estimates of Location Based on Rank Tests." *Annals of Mathematical Statistics*, 34(2), 598-611. — Hodges-Lehmann estimator.
- Hampel, F. R. (1974). "The Influence Curve and its Role in Robust Estimation." *JASA*, 69(346), 383-393. — Influence functions and breakdown points.
- Pearl, J. (2009). *Causality*. Cambridge University Press. — Causal consistency checking.
- Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6). — HDC prototype matching performance.

---

## Cross-references

- See [02-chain-oracles.md](./02-chain-oracles.md) for MEV detection context
- See [04-research-oracles.md](./04-research-oracles.md) for p-hacking detection
- See [06-hyperdimensional-ta.md](./06-hyperdimensional-ta.md) for HDC prototype encoding
- See [09-causal-microstructure-discovery.md](./09-causal-microstructure-discovery.md) for causal consistency checks
- See [12-somatic-ta-and-emergent-multiscale.md](./12-somatic-ta-and-emergent-multiscale.md) for Daimon integration
