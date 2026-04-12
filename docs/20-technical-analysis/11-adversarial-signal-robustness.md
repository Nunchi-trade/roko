# Adversarial Signal Robustness

> Every domain has adversaries who manipulate signals. MEV searchers manipulate prices. Attackers manipulate supply chains. p-hackers manipulate statistics. The adversarial robustness subsystem defends predictions through HDC prototype matching, robust statistics, and red-team dreaming.

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
