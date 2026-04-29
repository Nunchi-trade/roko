# Causal Discovery and Adversarial Robustness

> Depth for [09-causal-microstructure-discovery.md](../../docs/20-technical-analysis/09-causal-microstructure-discovery.md) and [11-adversarial-signal-robustness.md](../../docs/20-technical-analysis/11-adversarial-signal-robustness.md). Reframes causal reasoning as a Graph of Score Cells that discover causal structure through intervention testing, and adversarial robustness as an immune system extension that protects predictions from manipulation.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, lineage, HDC fingerprint), [02-CELL](../../unified/02-CELL.md) (Cell, Score protocol, Verify protocol, Connect protocol), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, Loop pattern, Pipeline pattern), [06-MEMORY](../../unified/06-MEMORY.md) (dreams, REM imagination), [16-SECURITY](../../unified/16-SECURITY.md) (immune system)

---

## Part I: Causal Discovery as a Graph of Score Cells

## 1. Pearl's Three Levels as Cell Taxonomy

Judea Pearl's causal hierarchy (Pearl 2009, *Causality*) defines three levels of causal reasoning. Each level maps to a specific Cell type in the unified model:

| Pearl's level | Question | Cell type | Implementation |
|---|---|---|---|
| **L1: Association** (seeing) | "What is P(Y given X)?" | Store query (correlational) | Standard `Store.query()` with statistical aggregation |
| **L2: Intervention** (doing) | "What happens to Y if I do X?" | Connect Cell executing changes | mirage-rs EVM fork, CI pipeline trigger, A/B test |
| **L3: Counterfactual** (imagining) | "Would Y have happened if X hadn't?" | Dream cycle's REM imagination | Structural equation inversion in offline consolidation |

Most prediction systems operate exclusively at Level 1 -- they detect correlations but cannot distinguish cause from coincidence. The causal discovery subsystem moves agents up the hierarchy, enabling predictions that survive regime changes (because they model the generative mechanism, not just the statistical association).

```rust
/// Pearl's causal hierarchy as Cell taxonomy.
///
/// L1 (Association) = Score Cell computing P(Y|X) from Store observations
/// L2 (Intervention) = Connect Cell executing do(X) and observing Y
/// L3 (Counterfactual) = Dream Cell inverting structural equations
///
/// The hierarchy forms a Pipeline: L1 generates hypotheses,
/// L2 tests them via intervention, L3 refines them via imagination.
///
/// Location: `crates/roko-learn/src/causal/`
pub enum CausalLevel {
    /// Observational: compute conditional probabilities from Store.
    Association,
    /// Interventional: execute actions and observe consequences.
    Intervention,
    /// Counterfactual: reason about what would have been.
    Counterfactual,
}
```

---

## 2. Structural Causal Model as a Graph

A Structural Causal Model (SCM) is literally a Graph -- directed edges represent causal relationships, nodes represent variables. In unified terms, it is a **Graph of Score Cells** where each Cell rates the strength of a causal edge:

```rust
/// Structural Causal Model expressed as a Graph of Score Cells.
///
/// Each vertex is a variable (gas_price, block_time, slippage, etc.)
/// Each directed edge is a causal claim: "X causes Y with strength s"
/// Edge strength is rated by a Score Cell using conditional independence tests.
///
/// The graph IS the causal model. Updating the graph IS learning causality.
///
/// Location: `crates/roko-learn/src/causal/scm.rs`
pub struct StructuralCausalModel {
    /// Variables in the model (vertices of the Graph).
    pub variables: Vec<CausalVariable>,

    /// Directed edges with strength scores.
    /// Each edge is a claim: "intervention on source changes target"
    pub edges: Vec<CausalEdge>,

    /// Structural equations: Y = f_Y(parents(Y), noise_Y)
    /// Each variable's value is determined by its parents plus noise.
    pub equations: HashMap<VariableId, StructuralEquation>,
}

pub struct CausalVariable {
    pub id: VariableId,
    pub name: String,
    pub domain: OracleDomain,
    /// Current observed value (from Store query).
    pub value: Option<f64>,
    /// Is this variable exogenous (no parents) or endogenous?
    pub exogenous: bool,
}

pub struct CausalEdge {
    pub source: VariableId,
    pub target: VariableId,
    /// Causal strength: how much does intervening on source change target?
    pub strength: f64,
    /// Confidence in this edge's existence (from discovery algorithm).
    pub confidence: f64,
    /// Evidence: which tests support this edge?
    pub evidence: Vec<CausalEvidence>,
}

pub struct StructuralEquation {
    /// Y = f(parents) + noise
    /// The function f learned from interventional data.
    pub function: Box<dyn Fn(&[f64]) -> f64 + Send + Sync>,
    /// Noise distribution parameters.
    pub noise_std: f64,
}
```

---

## 3. PC Algorithm as a Score Cell

The PC algorithm (Spirtes, Glymour & Scheines 2000) discovers causal structure from observational data by testing conditional independence. In unified terms, it is a Score Cell that rates the presence of causal edges:

```rust
/// PC Algorithm expressed as a Score Cell.
///
/// For each pair of variables (X, Y), the PC algorithm tests:
///   "Are X and Y conditionally independent given some subset Z?"
///
/// If independent: no direct causal edge between X and Y.
/// If dependent given all conditioning sets: edge exists.
///
/// The "score" is the edge existence probability.
///
/// Location: `crates/roko-learn/src/causal/pc.rs`
pub struct PCAlgorithmCell {
    /// Significance level for independence tests.
    alpha: f64,  // default: 0.05
    /// Maximum conditioning set size to test.
    max_conditioning_size: usize,  // default: 3
    /// Independence test implementation.
    test: ConditionalIndependenceTest,
}

impl ScoreProtocol for PCAlgorithmCell {
    /// Rate a candidate causal edge by testing conditional independence.
    async fn rate(&self, signal: &Signal, ctx: &CellContext) -> Score {
        let edge_candidate = CausalEdgeCandidate::from_signal(signal)?;
        let x = edge_candidate.source;
        let y = edge_candidate.target;

        // Get observation history from Store
        let observations = ctx.store.query(
            Query::variables(&[x, y])
                .with_all_parents()
                .time_range(ctx.lookback_window)
        ).await;

        // Test conditional independence: X ⊥ Y | Z for all subsets Z
        let mut min_p_value = 1.0;
        for z_subset in power_sets_up_to(
            &edge_candidate.potential_confounders,
            self.max_conditioning_size,
        ) {
            let p_value = self.test.test(&observations, x, y, &z_subset);
            min_p_value = min_p_value.min(p_value);

            if p_value > self.alpha {
                // Conditionally independent -- no direct causal edge
                return Score {
                    confidence: 1.0 - p_value, // low confidence in edge
                    utility: 0.0,
                    ..Default::default()
                };
            }
        }

        // Not conditionally independent given any subset -- edge likely exists
        Score {
            confidence: 1.0 - min_p_value,
            utility: edge_candidate.estimated_intervention_value(),
            ..Default::default()
        }
    }
}
```

---

## 4. Granger Causality with DeFi Extensions

Granger causality tests temporal precedence: "X Granger-causes Y if past values of X help predict future values of Y beyond what past values of Y alone provide." Four DeFi-specific extensions adapt this to on-chain data:

```rust
/// Granger causality with 4 DeFi extensions.
///
/// Standard Granger: compare AR(p) model of Y with and without X lags.
///   F-test: does adding X lags significantly improve prediction?
///
/// DeFi extensions:
///   1. Block-time alignment (irregular timestamps -> block-indexed)
///   2. Cross-chain lag (account for bridge latency between L1/L2)
///   3. MEV-aware (exclude sandwich transactions from causal inference)
///   4. Liquidity-weighted (weight observations by pool depth)
///
/// Location: `crates/roko-learn/src/causal/granger.rs`
pub struct GrangerCausalityCell {
    /// Maximum lag order to test.
    max_lag: usize,  // default: 10 blocks
    /// Significance level.
    alpha: f64,      // default: 0.05
    /// DeFi extensions to apply.
    extensions: DeFiExtensions,
}

pub struct DeFiExtensions {
    /// Align to block timestamps (handles irregular block times).
    pub block_time_alignment: bool,
    /// Account for cross-chain bridge latency.
    pub cross_chain_lag: Option<Duration>,
    /// Exclude known MEV transactions from sample.
    pub mev_filter: bool,
    /// Weight observations by pool liquidity depth.
    pub liquidity_weighting: bool,
}

impl ScoreProtocol for GrangerCausalityCell {
    async fn rate(&self, signal: &Signal, ctx: &CellContext) -> Score {
        let candidate = CausalEdgeCandidate::from_signal(signal)?;

        // Get time series data from Store
        let x_series = ctx.store.time_series(candidate.source, self.max_lag * 2).await;
        let y_series = ctx.store.time_series(candidate.target, self.max_lag * 2).await;

        // Apply DeFi extensions
        let (x_aligned, y_aligned) = self.extensions.align(&x_series, &y_series);

        // Restricted model: Y_t = a_0 + Σ a_i * Y_{t-i} + ε
        let restricted_rss = fit_ar_model(&y_aligned, self.max_lag).residual_sum_squares();

        // Unrestricted model: Y_t = a_0 + Σ a_i * Y_{t-i} + Σ b_j * X_{t-j} + ε
        let unrestricted_rss = fit_ar_model_with_exogenous(
            &y_aligned, &x_aligned, self.max_lag
        ).residual_sum_squares();

        // F-test: does X significantly improve Y prediction?
        let n = y_aligned.len() as f64;
        let p = self.max_lag as f64;
        let f_stat = ((restricted_rss - unrestricted_rss) / p)
            / (unrestricted_rss / (n - 2.0 * p - 1.0));
        let p_value = f_distribution_p_value(f_stat, p, n - 2.0 * p - 1.0);

        Score {
            confidence: if p_value < self.alpha { 1.0 - p_value } else { p_value },
            utility: (restricted_rss - unrestricted_rss) / restricted_rss, // R^2 improvement
            ..Default::default()
        }
    }
}
```

---

## 5. Intervention Testing via Connect Cell

Level 2 causal reasoning requires actually doing things and observing consequences. The Connect Cell executes interventions via mirage-rs (EVM fork simulation), CI pipeline triggers, or A/B test deployment:

```rust
/// Intervention Cell: executes do(X) and observes the effect on Y.
///
/// Uses the Connect protocol to interact with external systems:
///   - Chain domain: mirage-rs EVM fork (simulate trades, observe price impact)
///   - Coding domain: CI pipeline trigger (change code, observe test results)
///   - Research domain: A/B test deployment (show different content, observe engagement)
///
/// The intervention is a CAUSAL experiment: it establishes that
/// X -> Y (not just that X correlates with Y) by actively manipulating X
/// and observing whether Y changes as predicted by the structural equations.
///
/// Location: `crates/roko-learn/src/causal/intervention.rs`
pub struct InterventionCell {
    /// The intervention to perform.
    pub intervention: Intervention,
    /// Connect Cell for external system interaction.
    pub connector: Box<dyn ConnectProtocol>,
}

pub enum Intervention {
    /// EVM fork simulation: set variable to value and observe downstream.
    EvmFork {
        variable: VariableId,
        set_value: f64,
        observe: Vec<VariableId>,
    },
    /// CI pipeline trigger: make a code change and observe test results.
    CiTrigger {
        change: CodeChange,
        observe_metrics: Vec<CodingMetric>,
    },
    /// Controlled experiment: A/B split with measurement.
    Experiment {
        treatment: Treatment,
        control: Treatment,
        metric: ResearchMetric,
        sample_size: usize,
    },
}

impl ConnectProtocol for InterventionCell {
    async fn execute(&self, ctx: &CellContext) -> Result<Signal> {
        match &self.intervention {
            Intervention::EvmFork { variable, set_value, observe } => {
                // Fork EVM state, set variable, simulate forward, observe
                let fork = self.connector.connect("mirage-rs").await?;
                fork.set_state(variable, set_value).await?;
                fork.simulate_forward(10).await?; // 10 blocks forward

                let observations: Vec<(VariableId, f64)> = observe.iter()
                    .map(|v| (*v, fork.observe(v)))
                    .collect();

                Ok(Signal::builder()
                    .kind(Kind::InterventionResult)
                    .body(Body::InterventionOutcome {
                        intervened_on: *variable,
                        set_to: *set_value,
                        observed: observations,
                    })
                    .build())
            }
            // ... similar for CiTrigger and Experiment
            _ => todo!()
        }
    }
}
```

---

## 6. Counterfactual Reasoning in Dream Cycle

Level 3 (counterfactual) reasoning is the most powerful but also the most speculative. It runs during the Dream cycle's REM phase, where the agent imagines "what would have happened if X had been different":

```rust
/// Counterfactual reasoning via structural equation inversion.
///
/// Given SCM with structural equations:
///   Y = f_Y(parents(Y), noise_Y)
///
/// Counterfactual Y_{X=x'} is computed in three steps:
///   1. ABDUCTION: infer noise values from observed data
///      noise_Y = observed_Y - f_Y(observed_parents)
///   2. ACTION: modify the structural equation for X
///      Replace X = f_X(...) with X = x' (the counterfactual value)
///   3. PREDICTION: propagate forward with new X, same noise
///      Y_cf = f_Y(new_parents_with_X=x', noise_Y)
///
/// This runs during REM (creative/imaginative phase) because:
///   - It generates novel scenarios (not observed in reality)
///   - It tests causal claims against hypothetical realities
///   - It refines structural equations by checking consistency
///
/// Location: `crates/roko-dreams/src/rem/counterfactual.rs`
pub struct CounterfactualCell {
    scm: Arc<StructuralCausalModel>,
}

impl CounterfactualCell {
    /// "What would Y have been if X had been x' instead of x?"
    pub async fn counterfactual(
        &self,
        observed: &Observation,
        intervene_on: VariableId,
        counterfactual_value: f64,
    ) -> CounterfactualResult {
        // Step 1: ABDUCTION -- infer noise from observations
        let noises = self.scm.abduct(observed);

        // Step 2: ACTION -- set the counterfactual value
        let mut modified_scm = self.scm.clone();
        modified_scm.set_exogenous(intervene_on, counterfactual_value);

        // Step 3: PREDICTION -- propagate with same noise, new value
        let counterfactual_state = modified_scm.forward_with_noise(&noises);

        CounterfactualResult {
            query: format!("If {} had been {}", intervene_on, counterfactual_value),
            factual: observed.clone(),
            counterfactual: counterfactual_state,
            causal_effect: counterfactual_state.get(intervene_on)
                .map(|cf| cf - observed.get(intervene_on).unwrap_or(0.0)),
        }
    }
}
```

---

## Part II: Adversarial Robustness as Immune System

## 7. The Immune System Analogy

Adversarial robustness is not a bolted-on defense layer. It is the **immune system** -- a 5-layer Pipeline Graph (spec doc 16) that protects the agent's predictions from manipulation. Each layer corresponds to a biological immune mechanism:

| Layer | Biological analogue | Cell type | What it does |
|---|---|---|---|
| **Skin** | Physical barrier | Verify Cell (schema validation) | Reject malformed inputs |
| **Innate immunity** | Pattern recognition receptors | Score Cell (HDC prototype matching) | Detect known attack patterns at ~10ns |
| **Adaptive immunity** | T-cells and B-cells | Score Cell (robust statistics) | Detect anomalies without known patterns |
| **Inflammation** | Cytokine signaling | React Cell (alerting) | Escalate and coordinate response |
| **Memory** | Immunological memory | Store Cell (attack pattern library) | Remember and recognize future attacks |

---

## 8. HDC Prototype Matching -- Nanosecond Attack Detection

The first line of defense: compare incoming Signals against known attack pattern prototypes using HDC Hamming similarity. Cost: ~10ns per comparison (XOR + popcount on 10,240 bits). For 1,000 known attack patterns: ~10 microseconds total.

```rust
/// HDC prototype matching: innate immunity for Signals.
///
/// Known adversarial patterns are encoded as HDC prototype vectors.
/// Incoming Signals are compared against all prototypes at ~10ns each.
/// Matches above threshold trigger the appropriate response.
///
/// This is fast enough to run at Gamma frequency on EVERY observation.
/// It is the "pattern recognition receptor" of the immune system.
///
/// Location: `crates/roko-primitives/src/hdc/immune.rs`
pub struct PrototypeMatcherCell {
    /// Known attack pattern prototypes.
    prototypes: Vec<AttackPrototype>,
    /// Similarity threshold for match (default: 0.6, tighter than
    /// cross-domain resonance at 0.526 because false positives are costly).
    threshold: f64,
}

pub struct AttackPrototype {
    /// HDC vector encoding the attack pattern structure.
    pub vector: HdcVector,
    /// Human-readable attack name.
    pub name: String,
    /// Domain this attack targets.
    pub domain: OracleDomain,
    /// Severity if matched (0.0 to 1.0).
    pub severity: f64,
    /// Recommended response.
    pub response: AdversarialResponse,
}

pub enum AdversarialResponse {
    /// Widen prediction intervals (increase uncertainty quantification).
    WidenIntervals { factor: f64 },
    /// Suppress action for duration (wait for manipulation to pass).
    SuppressAction { duration: Duration },
    /// Escalate to T2 deep reasoning for manual analysis.
    Escalate,
    /// Quarantine the Signal (remove from active consideration).
    Quarantine,
}

impl VerifyProtocol for PrototypeMatcherCell {
    async fn verify(&self, signal: &Signal, _ctx: &CellContext) -> Verdict {
        let signal_hv = signal.hdc_fingerprint();

        let matches: Vec<(f64, &AttackPrototype)> = self.prototypes.iter()
            .filter_map(|proto| {
                let sim = signal_hv.hamming_similarity(&proto.vector);
                if sim > self.threshold { Some((sim, proto)) } else { None }
            })
            .collect();

        if matches.is_empty() {
            Verdict { pass: true, reward: 1.0, evidence: Evidence::Clean, message: "No attack patterns matched".into() }
        } else {
            let worst = matches.iter().max_by(|a, b| a.0.partial_cmp(&b.0).unwrap()).unwrap();
            Verdict {
                pass: worst.1.severity < 0.8, // Only hard-reject at high severity
                reward: 1.0 - worst.1.severity,
                evidence: Evidence::AttackMatch {
                    pattern: worst.1.name.clone(),
                    similarity: worst.0,
                    severity: worst.1.severity,
                },
                message: format!(
                    "Attack pattern '{}' matched at similarity {:.3}",
                    worst.1.name, worst.0
                ),
            }
        }
    }
}
```

---

## 9. Robust Statistics as Adaptive Immunity

When no known prototype matches but the Signal is still suspicious, robust statistics detect anomalies. These replace naive aggregation (mean, std) with estimators that tolerate up to 50% contaminated data:

```rust
/// Robust statistics: adaptive immunity against unknown attacks.
///
/// Three robust estimators that tolerate adversarial contamination:
///   1. Trimmed mean: remove top/bottom k% before averaging
///   2. Hodges-Lehmann: median of all pairwise averages
///   3. MAD (Median Absolute Deviation): robust spread estimator
///
/// These are Score Cells that rate "how anomalous is this Signal
/// relative to robust estimates of the population?"
///
/// Location: `crates/roko-learn/src/robust/statistics.rs`
pub struct RobustStatisticsCell {
    /// Trim fraction for trimmed mean (default: 0.1 = remove 10% each end).
    trim_fraction: f64,
    /// Anomaly threshold in MAD units (default: 5.0).
    anomaly_threshold: f64,
    /// Sliding window of recent observations for baseline.
    window_size: usize,
}

impl ScoreProtocol for RobustStatisticsCell {
    async fn rate(&self, signal: &Signal, ctx: &CellContext) -> Score {
        let value = signal.numeric_value()?;

        // Get recent observations from Store for baseline
        let recent = ctx.store.query(
            Query::same_kind_and_domain(signal)
                .time_range(ctx.lookback_window)
                .limit(self.window_size)
        ).await;

        let values: Vec<f64> = recent.iter().map(|s| s.numeric_value().unwrap()).collect();

        // Robust location estimate: trimmed mean
        let location = trimmed_mean(&values, self.trim_fraction);

        // Robust scale estimate: MAD
        let scale = median_absolute_deviation(&values);

        // Anomaly score: how many MAD units from robust center?
        let anomaly_score = if scale > 0.0 {
            (value - location).abs() / scale
        } else {
            0.0
        };

        let is_anomalous = anomaly_score > self.anomaly_threshold;

        Score {
            confidence: if is_anomalous { 0.1 } else { 0.9 },
            novelty: anomaly_score / self.anomaly_threshold, // normalized anomaly
            utility: if is_anomalous { -1.0 } else { 1.0 },
            ..Default::default()
        }
    }
}

/// Trimmed mean: robust location estimator.
fn trimmed_mean(values: &[f64], trim_fraction: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let trim_count = (sorted.len() as f64 * trim_fraction) as usize;
    let trimmed = &sorted[trim_count..sorted.len() - trim_count];
    trimmed.iter().sum::<f64>() / trimmed.len() as f64
}

/// MAD: robust scale estimator. Breakdown point = 50%.
fn median_absolute_deviation(values: &[f64]) -> f64 {
    let median = median(values);
    let deviations: Vec<f64> = values.iter().map(|v| (v - median).abs()).collect();
    median(&deviations) * 1.4826  // consistency factor for normal distribution
}
```

---

## 10. Red-Team Dreaming -- Self-Immunization Loop

The Dream cycle's REM phase serves double duty: it generates creative pattern recombinations (doc 04) AND adversarial perturbations. Red-team dreaming generates attacks against the agent's own predictions and tests whether the immune system catches them:

```rust
/// Red-team dreaming: self-immunization via Dream cycle.
///
/// During REM phase, the Dream Cell:
///   1. Takes high-confidence predictions from recent episodes
///   2. Generates adversarial perturbations that would fool the Oracle
///   3. Tests whether PrototypeMatcher or RobustStatistics catches them
///   4. If NOT caught: adds new prototype to the attack library
///   5. If caught: validates that the immune system is working
///
/// This is a self-immunization Loop: the agent attacks itself to
/// discover vulnerabilities, then patches them by adding new prototypes.
///
/// Location: `crates/roko-dreams/src/rem/red_team.rs`
pub struct RedTeamDreamCell {
    /// How many adversarial perturbations to generate per dream cycle.
    perturbation_count: usize,  // default: 20
    /// Perturbation magnitude (in HDC space: number of bits to flip).
    perturbation_strength: usize,  // default: 512 bits (5% of 10,240)
}

impl RedTeamDreamCell {
    /// Generate adversarial perturbations and test the immune system.
    pub async fn red_team(
        &self,
        recent_predictions: &[PredictionClaim],
        immune_system: &ImmuneSystemPipeline,
    ) -> Vec<NewPrototype> {
        let mut new_prototypes = Vec::new();

        for prediction in recent_predictions.iter().take(self.perturbation_count) {
            // Generate adversarial perturbation
            let original_hv = prediction.signal.hdc_fingerprint();
            let perturbed = original_hv.flip_random_bits(self.perturbation_strength);

            // Construct adversarial Signal
            let adversarial_signal = Signal::builder()
                .kind(prediction.signal.kind())
                .hdc_fingerprint(perturbed)
                .body(prediction.signal.body().clone())
                .score(Score { confidence: 0.95, ..Default::default() }) // high confidence (deceptive)
                .build();

            // Test: does the immune system catch this?
            let verdict = immune_system.verify(&adversarial_signal).await;

            if verdict.pass {
                // NOT caught -- this is a vulnerability. Add new prototype.
                new_prototypes.push(NewPrototype {
                    vector: perturbed,
                    name: format!("red-team-{}-perturbation-of-{}", now_ms(), prediction.id()),
                    severity: 0.5, // medium severity (discovered in simulation)
                    response: AdversarialResponse::WidenIntervals { factor: 1.5 },
                });
            }
            // If caught: immune system working correctly. No action needed.
        }

        new_prototypes
    }
}
```

---

## 11. Certified Robustness as a Verify Cell

For safety-critical predictions, randomized smoothing provides mathematically certified robustness -- a guarantee that the prediction does not change within a certified radius of the input:

```rust
/// Certified robustness via randomized smoothing.
///
/// Given an input Signal x and a base predictor f:
///   1. Add Gaussian noise to x, producing N noisy copies
///   2. Run f on each noisy copy
///   3. The "smoothed" prediction = majority vote across noisy copies
///   4. The certified radius R = (sigma/2) * (Phi^{-1}(p_A) - Phi^{-1}(p_B))
///      where p_A = fraction voting for top class, p_B = fraction for runner-up
///
/// Within radius R, no adversarial perturbation can change the prediction.
/// This is a MATHEMATICAL guarantee, not a heuristic.
///
/// Cost: N forward passes per certification (N=100 typical, ~50ms total).
/// Use case: safety-critical predictions where false confidence is dangerous.
///
/// Location: `crates/roko-learn/src/robust/certified.rs`
pub struct CertifiedRobustnessCell {
    /// Number of noisy samples for certification.
    n_samples: usize,  // default: 100
    /// Noise standard deviation (controls certified radius).
    sigma: f64,  // default: 0.25
    /// Minimum certification confidence.
    min_confidence: f64,  // default: 0.99
}

impl VerifyProtocol for CertifiedRobustnessCell {
    async fn verify(&self, signal: &Signal, ctx: &CellContext) -> Verdict {
        let base_prediction = ctx.oracle.predict_from_signal(signal).await?;

        // Generate N noisy copies and run predictions
        let mut votes: HashMap<PredictedValue, usize> = HashMap::new();
        for _ in 0..self.n_samples {
            let noisy = signal.add_gaussian_noise(self.sigma);
            let noisy_pred = ctx.oracle.predict_from_signal(&noisy).await?;
            *votes.entry(noisy_pred.quantized()).or_default() += 1;
        }

        // Top two vote counts
        let mut counts: Vec<usize> = votes.values().cloned().collect();
        counts.sort_unstable_by(|a, b| b.cmp(a));
        let p_a = counts[0] as f64 / self.n_samples as f64;
        let p_b = counts.get(1).copied().unwrap_or(0) as f64 / self.n_samples as f64;

        // Certified radius (Cohen et al. 2019)
        let certified_radius = (self.sigma / 2.0)
            * (normal_ppf(p_a) - normal_ppf(p_b));

        let is_certified = p_a >= self.min_confidence && certified_radius > 0.0;

        Verdict {
            pass: is_certified,
            reward: certified_radius,
            evidence: Evidence::CertifiedRobustness {
                radius: certified_radius,
                confidence: p_a,
                n_samples: self.n_samples,
            },
            message: format!(
                "Certified robust within radius {:.4} (confidence {:.3})",
                certified_radius, p_a
            ),
        }
    }
}
```

---

## 12. The Immune System Pipeline Graph

The complete adversarial robustness system is a Pipeline pattern -- each layer can reject, transform, or pass through:

```toml
# immune-pipeline.toml -- Adversarial robustness as a Pipeline Graph

[graph]
name = "immune-system"
pattern = "Pipeline"

[[cells]]
name = "schema-validation"
protocol = "Verify"
layer = "skin"
# Reject malformed inputs (missing fields, invalid types)

[[cells]]
name = "prototype-matcher"
protocol = "Verify"
layer = "innate"
config.threshold = 0.6
config.max_prototypes = 1000
# ~10ns per comparison, rejects known attack patterns

[[cells]]
name = "robust-statistics"
protocol = "Score"
layer = "adaptive"
config.trim_fraction = 0.1
config.anomaly_threshold = 5.0
# Detect unknown anomalies via robust estimators

[[cells]]
name = "causal-consistency"
protocol = "Score"
layer = "adaptive"
# Check if Signal is consistent with the causal model

[[cells]]
name = "alerting"
protocol = "React"
layer = "inflammation"
watches = ["immune.alerts.*"]
# Coordinate response to detected threats

[[cells]]
name = "prototype-library"
protocol = "Store"
layer = "memory"
# Store new attack patterns discovered by red-team dreaming

# Pipeline edges (sequential, each can reject)
[[edges]]
from = "schema-validation"
to = "prototype-matcher"

[[edges]]
from = "prototype-matcher"
to = "robust-statistics"

[[edges]]
from = "robust-statistics"
to = "causal-consistency"

[[edges]]
from = "causal-consistency"
to = "alerting"
```

---

## What This Enables

1. **Causal predictions that survive regime changes**: By modeling generative mechanisms (not just correlations), Oracle predictions remain valid when the environment shifts -- the causal structure is more stable than statistical associations.

2. **Intervention testing without real-world cost**: mirage-rs EVM forks allow testing "what would happen if?" at zero cost. CI pipeline triggers can be sandboxed. This gives Level 2 causal evidence without market risk.

3. **Nanosecond attack detection**: HDC prototype matching at ~10ns per comparison enables real-time adversarial detection at Gamma frequency. Known attacks are caught before they can influence predictions.

4. **Self-immunizing defense**: Red-team dreaming discovers new vulnerabilities during offline consolidation and patches them by adding prototypes. The immune system strengthens while the agent sleeps.

5. **Mathematical robustness guarantees**: Certified robustness via randomized smoothing provides provable bounds on prediction stability -- no adversarial perturbation within the certified radius can change the output.

---

## Feedback Loops

| Loop | Participants | Signal | Timescale |
|---|---|---|---|
| **Causal structure discovery** | PC algorithm Score Cell + Store observations | Edge confidence scores | Theta (per-task) |
| **Intervention validation** | Connect Cell + SCM update | Confirmed/refuted edges | On-demand (Connect) |
| **Counterfactual refinement** | Dream REM Cell + SCM equations | Refined structural equations | Delta (offline) |
| **Immune prototype update** | Red-team Dream Cell -> Prototype Store | New attack prototypes | Delta (per dream cycle) |
| **Robustness certification** | CertifiedRobustnessCell + Oracle | Certified radius per prediction | On-demand (safety-critical) |

---

## Open Questions

1. **Causal discovery sample complexity**: The PC algorithm requires O(n^d) observations where d is the maximum in-degree of the causal graph. For dense graphs (many confounders), this can require thousands of observations. Should we limit graph density and accept incomplete discovery?

2. **Intervention ethics in production**: mirage-rs forks are safe (simulation). But coding domain interventions (triggering CI) and research domain interventions (A/B tests) have real costs. How should the system budget interventional experiments?

3. **Prototype generalization**: Current prototypes are specific patterns. Should there be abstract prototype "families" that generalize? E.g., "any sandwich-like pattern" rather than "specific sandwich with these parameters"?

4. **Adversarial adaptation**: Sophisticated adversaries may adapt to the immune system. Red-team dreaming tests against the current immune system but cannot anticipate an adversary who is also adapting. Is there a stable equilibrium, or is this an arms race?

5. **Certified robustness cost**: 100 forward passes per certification is expensive for real-time predictions. Should certification be selective (only safety-critical predictions) or is there a way to amortize the cost across similar inputs?

---

## Implementation Tasks

- [ ] Implement `StructuralCausalModel` as a Graph data structure in `crates/roko-learn/src/causal/scm.rs`
- [ ] Implement PC algorithm as a Score Cell with conditional independence testing
- [ ] Implement Granger causality with 4 DeFi extensions (block-time alignment, cross-chain lag, MEV filter, liquidity weighting)
- [ ] Wire mirage-rs as a Connect Cell for EVM fork intervention testing
- [ ] Add counterfactual reasoning to `crates/roko-dreams/src/rem/` (abduction + action + prediction)
- [ ] Implement `PrototypeMatcherCell` with HDC Hamming similarity in `crates/roko-primitives/src/hdc/immune.rs`
- [ ] Implement robust statistics Cell (trimmed mean, Hodges-Lehmann, MAD) in `crates/roko-learn/src/robust/`
- [ ] Implement red-team dreaming in `crates/roko-dreams/src/rem/red_team.rs`
- [ ] Add certified robustness Cell with randomized smoothing in `crates/roko-learn/src/robust/certified.rs`
- [ ] Wire immune system as a Pipeline Graph in the verification path of orchestrate.rs
