# Causal Microstructure Discovery

> Correlation is not causation. The causal discovery subsystem uses Pearl's structural causal models, Granger causality, and interventional experiments (via mirage-rs simulation) to discover genuine causal relationships in structured domains.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [01-oracle-trait](./01-oracle-trait.md) for prediction integration, [06-hyperdimensional-ta](./06-hyperdimensional-ta.md) for pattern encoding
**Key sources**: `bardo-backup/prd/23-ta/04-causal-microstructure-discovery.md`

---

## Pearl's causal hierarchy in Roko

Judea Pearl's causal hierarchy (Pearl, 2009, *Causality*) defines three levels of causal reasoning. Roko implements all three:

| Level | Question | Roko implementation |
|---|---|---|
| **L1: Association** (seeing) | "What is the probability of Y given X?" | Standard TA indicators (correlation, regression) |
| **L2: Intervention** (doing) | "What happens to Y if I do X?" | Mirage-rs simulation (do-operator) |
| **L3: Counterfactual** (imagining) | "Would Y have occurred if X hadn't happened?" | Dreams counterfactual engine (REM phase) |

Most TA systems operate exclusively at Level 1. They detect correlations — "when RSI drops below 30, price tends to rise." But correlation is not causation. The RSI drop and the price rise might both be caused by a third variable (e.g., a whale liquidation). Acting on correlation without understanding causation leads to fragile strategies that fail when the causal structure changes.

Roko's causal discovery subsystem moves the agent up Pearl's hierarchy, enabling genuinely causal predictions that survive regime changes.

---

## Structural Causal Model (SCM)

```rust
/// A structural causal model (SCM) following Pearl's formalism.
///
/// An SCM is a tuple (U, V, F) where:
/// - U: exogenous (external, unobserved) variables
/// - V: endogenous (internal, observed) variables
/// - F: structural equations V_i = f_i(Pa(V_i), U_i)
///   where Pa(V_i) are the parents of V_i in the causal graph
pub struct StructuralCausalModel {
    /// Exogenous variables (external factors).
    pub exogenous: Vec<Variable>,

    /// Endogenous variables (observed state).
    pub endogenous: Vec<Variable>,

    /// Structural equations: each variable is a function of its parents.
    pub equations: HashMap<VariableId, StructuralEquation>,

    /// The causal DAG (directed acyclic graph).
    pub graph: CausalGraph,
}

pub struct Variable {
    pub id: VariableId,
    pub name: String,
    pub domain: VariableDomain,
}

pub enum VariableDomain {
    Continuous { min: f64, max: f64 },
    Discrete(Vec<String>),
    Binary,
}

/// A structural equation: V_i = f(Pa(V_i), U_i).
pub struct StructuralEquation {
    /// The variable this equation defines.
    pub target: VariableId,

    /// Parent variables (causal inputs).
    pub parents: Vec<VariableId>,

    /// The functional form.
    pub function: Box<dyn Fn(&HashMap<VariableId, f64>) -> f64 + Send + Sync>,

    /// Exogenous noise distribution.
    pub noise: NoiseDistribution,
}
```

### The do-operator

Pearl's do-operator `do(X = x)` intervenes on the model by setting variable X to value x and removing all incoming edges to X. This breaks the causal mechanism that normally determines X, allowing us to measure the pure causal effect of X on downstream variables:

```rust
/// Apply the do-operator to an SCM.
///
/// do(X = x):
///   1. Set X = x (override its structural equation)
///   2. Remove all incoming edges to X in the causal graph
///   3. Propagate through remaining structural equations
///
/// Returns the distribution of downstream variables under intervention.
pub fn do_intervention(
    scm: &StructuralCausalModel,
    variable: VariableId,
    value: f64,
) -> InterventionalDistribution {
    // Create a modified SCM with X fixed
    let mut modified = scm.clone();

    // Replace X's equation with a constant
    modified.equations.insert(variable, StructuralEquation {
        target: variable,
        parents: vec![],  // no parents — intervention severs incoming arrows
        function: Box::new(move |_| value),
        noise: NoiseDistribution::Constant(0.0),
    });

    // Remove incoming edges to X in the graph
    modified.graph.remove_incoming_edges(variable);

    // Propagate through the modified model
    modified.propagate()
}
```

---

## Causal discovery algorithms

### PC Algorithm (Spirtes, Glymour, Scheines, 2000)

The PC algorithm discovers the causal graph structure from observational data:

```rust
/// The PC (Peter-Clark) algorithm for causal graph discovery.
///
/// Spirtes, Glymour, & Scheines (2000), "Causation, Prediction, and Search"
///
/// 1. Start with a complete undirected graph
/// 2. Remove edges based on conditional independence tests
/// 3. Orient edges based on v-structures (colliders)
/// 4. Apply Meek's orientation rules
///
/// Output: a Partially Directed Acyclic Graph (PDAG)
pub fn pc_algorithm(
    data: &DataFrame,
    alpha: f64,  // significance level for independence tests (typically 0.05)
    max_conditioning_set: usize,
) -> CausalGraph {
    let variables: Vec<VariableId> = data.columns().collect();
    let mut graph = CausalGraph::complete_undirected(&variables);

    // Phase I: Edge removal via conditional independence
    for conditioning_size in 0..=max_conditioning_set {
        for (x, y) in graph.edges() {
            let neighbors = graph.neighbors(x);
            for conditioning_set in neighbors.combinations(conditioning_size) {
                if conditional_independence_test(data, x, y, &conditioning_set, alpha) {
                    graph.remove_edge(x, y);
                    graph.add_separation_set(x, y, conditioning_set);
                    break;
                }
            }
        }
    }

    // Phase II: Orient v-structures
    for (x, z) in graph.undirected_edges() {
        for y in graph.common_neighbors(x, z) {
            if !graph.separation_set(x, z).contains(&y) {
                // x → y ← z is a v-structure (collider)
                graph.orient(x, y);
                graph.orient(z, y);
            }
        }
    }

    // Phase III: Meek's orientation rules
    graph.apply_meek_rules();

    graph
}
```

### Granger causality with DeFi extensions

Granger causality (Granger, 1969) tests whether past values of X help predict Y beyond Y's own past values. Four extensions adapt it to DeFi:

```rust
/// Granger causality test with DeFi-specific extensions.
///
/// Base test: does X_{t-k} Granger-cause Y_t?
/// H0: past values of X add no predictive power for Y
/// H1: past values of X improve Y prediction
pub struct GrangerCausalityTest {
    /// Maximum lag order to test.
    pub max_lag: usize,
    /// Significance level.
    pub alpha: f64,
}

impl GrangerCausalityTest {
    /// Extension 1: Block-aware Granger causality.
    ///
    /// Standard Granger assumes uniform time steps.
    /// Blockchain data has variable block times and MEV-induced
    /// ordering effects. This extension uses block number as the
    /// time index and accounts for intra-block ordering.
    pub fn block_aware(&self, x: &TimeSeries, y: &TimeSeries, blocks: &[Block]) -> GrangerResult;

    /// Extension 2: Cross-protocol Granger causality.
    ///
    /// Tests whether events on Protocol A Granger-cause events on
    /// Protocol B. Accounts for different time granularities across
    /// protocols (Uniswap has per-swap data, Aave has per-block updates).
    pub fn cross_protocol(&self, x: &ProtocolSeries, y: &ProtocolSeries) -> GrangerResult;

    /// Extension 3: Multi-chain Granger causality.
    ///
    /// Tests whether events on Chain A Granger-cause events on Chain B.
    /// Accounts for bridge latency and cross-chain message propagation.
    pub fn multi_chain(&self, x: &ChainSeries, y: &ChainSeries) -> GrangerResult;

    /// Extension 4: MEV-adjusted Granger causality.
    ///
    /// Removes MEV-induced spurious correlations (sandwich attacks
    /// create artificial dependencies between transactions that are
    /// not genuinely causal).
    pub fn mev_adjusted(&self, x: &TimeSeries, y: &TimeSeries, mev_labels: &[bool]) -> GrangerResult;
}
```

---

## Interventional discovery via mirage-rs

The deepest causal reasoning requires interventions — actively changing variables and observing effects. In the chain domain, mirage-rs enables this without risking real assets:

```rust
/// Interventional causal discovery using mirage-rs simulation.
///
/// The agent constructs causal hypotheses from observational data,
/// then tests them by simulating interventions:
///
/// 1. Observe: "When pool TVL drops, gas spikes."
/// 2. Hypothesize: "TVL drop → liquidation cascade → gas spike" (causal)
///    vs. "Both caused by external event (whale movement)" (confounded)
/// 3. Intervene: In mirage-rs, force TVL to drop while holding
///    external factors constant.
/// 4. Observe: If gas spikes in the simulation, the causal hypothesis
///    is supported. If not, it's confounded.
pub struct InterventionalDiscovery {
    /// The simulation environment.
    mirage: Arc<MirageSimulator>,

    /// The current causal model.
    scm: StructuralCausalModel,

    /// Hypotheses to test.
    hypotheses: Vec<CausalHypothesis>,
}

pub struct CausalHypothesis {
    /// Hypothesized cause.
    pub cause: VariableId,

    /// Hypothesized effect.
    pub effect: VariableId,

    /// Hypothesized mechanism (intermediate variables).
    pub mechanism: Vec<VariableId>,

    /// Confidence in this hypothesis.
    pub confidence: f64,

    /// Test results (from interventional experiments).
    pub test_results: Vec<InterventionResult>,
}

pub struct InterventionResult {
    /// The intervention applied.
    pub intervention: (VariableId, f64),

    /// The observed effect.
    pub observed_effect: f64,

    /// The predicted effect (from the causal model).
    pub predicted_effect: f64,

    /// Whether the hypothesis was supported.
    pub supported: bool,
}

impl InterventionalDiscovery {
    /// Run an interventional experiment.
    pub async fn test_hypothesis(
        &self,
        hypothesis: &CausalHypothesis,
    ) -> InterventionResult {
        // Fork the current chain state in mirage-rs
        let fork = self.mirage.fork_current_state().await;

        // Apply the intervention (set cause variable to a specific value)
        fork.set_variable(hypothesis.cause, 0.5).await;  // e.g., reduce TVL by 50%

        // Advance the simulation for the hypothesized propagation time
        fork.advance_blocks(10).await;

        // Observe the effect variable
        let observed = fork.get_variable(hypothesis.effect).await;
        let predicted = do_intervention(&self.scm, hypothesis.cause, 0.5)
            .mean(hypothesis.effect);

        InterventionResult {
            intervention: (hypothesis.cause, 0.5),
            observed_effect: observed,
            predicted_effect: predicted,
            supported: (observed - predicted).abs() < 0.1,
        }
    }
}
```

### Coding domain causal discovery

In the coding domain, interventional experiments use the workspace itself as the simulation environment:

```rust
/// Coding causal discovery: test whether code change X causes test failure Y.
///
/// Example hypothesis: "Modifying auth.rs causes security_tests to fail"
///
/// Interventional test:
///   1. Create a workspace snapshot (git stash or worktree)
///   2. Apply the change to auth.rs
///   3. Run security_tests
///   4. If they fail: hypothesis supported
///   5. If they pass: hypothesis not supported (the failure was confounded)
pub async fn test_coding_hypothesis(
    hypothesis: &CodingCausalHypothesis,
    workspace: &Workspace,
) -> InterventionResult {
    let snapshot = workspace.create_snapshot().await?;

    // Apply the change (intervention)
    workspace.apply_change(&hypothesis.change).await?;

    // Run the tests (observe effect)
    let test_result = workspace.run_tests(&hypothesis.affected_tests).await?;

    // Restore snapshot
    workspace.restore_snapshot(&snapshot).await?;

    InterventionResult {
        intervention: hypothesis.change.clone(),
        observed_effect: test_result.pass_rate,
        predicted_effect: hypothesis.predicted_pass_rate,
        supported: (test_result.pass_rate - hypothesis.predicted_pass_rate).abs() < 0.1,
    }
}
```

---

## Backdoor criterion — Controlling for confounders

```rust
/// The backdoor criterion (Pearl, 2009).
///
/// A set Z satisfies the backdoor criterion relative to (X, Y) if:
/// 1. No node in Z is a descendant of X
/// 2. Z blocks every path between X and Y that contains an arrow INTO X
///
/// If Z satisfies the backdoor criterion, the causal effect of X on Y
/// can be computed from observational data:
///
/// P(Y | do(X)) = Σ_z P(Y | X, Z=z) × P(Z=z)
///
/// This is the "adjustment formula" — it converts observational
/// probabilities into interventional ones without running experiments.
pub fn backdoor_adjustment(
    graph: &CausalGraph,
    x: VariableId,
    y: VariableId,
    z: &[VariableId],
    data: &DataFrame,
) -> Option<f64> {
    // Verify backdoor criterion
    if !graph.satisfies_backdoor(x, y, z) {
        return None;
    }

    // Compute adjustment formula
    let mut causal_effect = 0.0;
    for z_values in data.unique_values(z) {
        let p_y_given_xz = data.conditional_probability(y, &[(x, 1.0)], z, &z_values);
        let p_z = data.marginal_probability(z, &z_values);
        causal_effect += p_y_given_xz * p_z;
    }

    Some(causal_effect)
}
```

---

## Dream-based counterfactual discovery

During REM Dreams, the agent generates counterfactual scenarios using the causal model:

```rust
/// Counterfactual generation during REM phase.
///
/// "What would have happened if X had been different?"
///
/// Pearl's three-step counterfactual:
/// 1. Abduction: Use evidence to determine exogenous variables U
/// 2. Action: Modify the SCM with do(X = x')
/// 3. Prediction: Propagate through modified model
pub fn generate_counterfactual(
    scm: &StructuralCausalModel,
    evidence: &HashMap<VariableId, f64>,
    intervention: (VariableId, f64),
) -> HashMap<VariableId, f64> {
    // Step 1: Abduction — infer exogenous variables from evidence
    let exogenous = scm.abduct(evidence);

    // Step 2: Action — apply intervention to modified model
    let modified = scm.intervene(intervention.0, intervention.1);

    // Step 3: Prediction — propagate with inferred exogenous values
    modified.propagate_with_exogenous(&exogenous)
}
```

Counterfactual discovery is unique to Level 3 of Pearl's hierarchy. No competitor agent framework operates at this level. Combined with HDC encoding, discovered causal relationships are stored as `CausalLink` knowledge entries in the Neuro subsystem:

```rust
// Store discovered causal link
neuro.store(KnowledgeEntry {
    kind: KnowledgeType::CausalLink,
    content: format!(
        "Causal link discovered: {} → {} (effect size: {:.3}, confidence: {:.2})",
        cause_name, effect_name, effect_size, confidence
    ),
    hdc_vector: HdcVector::bind(&cause_hv, &effect_hv),  // HDC encoding
    confidence,
    tier: KnowledgeTier::Working,
    ..Default::default()
}).await?;
```

---

## Implementation details

### PC algorithm: conditional independence test

The PC algorithm uses the **partial correlation test** as its conditional independence test. For continuous variables X, Y conditioned on set Z:

```rust
/// Conditional independence test via partial correlation.
///
/// Tests H0: X ⊥ Y | Z (X is independent of Y given Z).
///
/// Method: Compute partial correlation r_{XY|Z} from the
/// correlation matrix using recursive formula (Baba et al., 2004).
/// Convert to a test statistic via Fisher's z-transform:
///   z = 0.5 * ln((1+r)/(1-r)) * sqrt(n - |Z| - 3)
///
/// Under H0, z ~ N(0,1). Reject H0 if |z| > z_{alpha/2}.
pub fn conditional_independence_test(
    data: &DataFrame,
    x: VariableId,
    y: VariableId,
    z: &[VariableId],
    alpha: f64,
) -> bool {
    let n = data.n_rows();
    let r_xy_z = partial_correlation(data, x, y, z);
    let z_stat = 0.5 * ((1.0 + r_xy_z) / (1.0 - r_xy_z)).ln()
        * ((n as f64 - z.len() as f64 - 3.0).max(1.0)).sqrt();
    let critical = normal_quantile(1.0 - alpha / 2.0); // two-sided
    z_stat.abs() < critical // true = independent
}
```

### Significance level adaptation

The significance level `alpha` adapts based on the number of variables and available data:

| n_variables | n_observations | Recommended alpha |
|---|---|---|
| < 10 | > 1000 | 0.05 (standard) |
| 10 - 50 | > 1000 | 0.01 (Bonferroni-style correction) |
| > 50 | > 1000 | 0.001 |
| any | < 100 | 0.10 (relaxed, low power) |

```rust
/// Adapt alpha based on problem scale.
///
/// Uses Bonferroni-like correction: alpha_adj = base_alpha / n_tests_estimate.
/// The n_tests estimate is O(p^2) where p = number of variables.
pub fn adaptive_alpha(n_variables: usize, n_observations: usize, base_alpha: f64) -> f64 {
    let n_tests = n_variables * (n_variables - 1) / 2; // upper bound on pairwise tests
    let bonferroni = base_alpha / n_tests.max(1) as f64;
    // Floor at 1e-6 to prevent test from never rejecting
    let alpha = bonferroni.max(1e-6);
    // Relax if sample size is too small for the correction
    if n_observations < 10 * n_variables {
        (alpha * 10.0).min(0.1)
    } else {
        alpha
    }
}
```

### Maximum conditioning set formula

The PC algorithm tests conditional independence with conditioning sets of increasing size. The maximum conditioning set size controls the computational cost:

```
max_conditioning_set = min(
    max_neighbors - 1,         // can't condition on more than the neighbor count
    floor(log2(n_observations)) - 1,  // statistical power limit
    user_max                   // user override (default: 5)
)
```

**Rationale**: Conditioning on k variables requires estimating a (k+2)-dimensional distribution. With n observations, you need roughly `2^(k+2)` samples for reliable estimation. So `k < log2(n) - 2` is the practical limit.

### Meek's orientation rules

After v-structure orientation, Meek's four rules orient remaining undirected edges:

```
Rule 1 (Acyclicity): If X → Y — Z and X and Z are not adjacent, orient Y → Z.
Rule 2 (Directed path): If X → Y → Z and X — Z, orient X → Z.
Rule 3 (Two directed paths): If X — Y, X → Z, X → W, Y — Z, Y — W,
        and Z and W are not adjacent, orient X → Y.
Rule 4 (Transitive closure): If X — Y, Y → Z, X — Z, orient X → Z.
```

Apply rules repeatedly until no new orientations are produced. This converges in at most O(p^2) iterations where p is the number of variables.

### Granger causality extensions

#### Time alignment for cross-protocol tests

DeFi protocols produce observations at different cadences. Before running Granger tests, time-align the series:

```rust
/// Align two time series to a common time grid.
///
/// Method: snap each observation to the nearest grid point.
/// Grid spacing = max(cadence_x, cadence_y).
/// Missing values are forward-filled (last observation carried forward).
pub fn time_align(
    x: &TimeSeries,
    y: &TimeSeries,
    grid_spacing: Duration,
) -> (Vec<f64>, Vec<f64>) {
    let start = x.start().min(y.start());
    let end = x.end().max(y.end());
    let n_points = ((end - start).as_secs_f64() / grid_spacing.as_secs_f64()).ceil() as usize;

    let mut aligned_x = Vec::with_capacity(n_points);
    let mut aligned_y = Vec::with_capacity(n_points);

    for i in 0..n_points {
        let t = start + grid_spacing * i as u32;
        aligned_x.push(x.value_at_or_before(t).unwrap_or(f64::NAN));
        aligned_y.push(y.value_at_or_before(t).unwrap_or(f64::NAN));
    }

    (aligned_x, aligned_y)
}
```

#### MEV label generation

The MEV-adjusted Granger test requires labels identifying which transactions are MEV-related:

```rust
/// Heuristic MEV labeling for transaction sequences.
///
/// Labels a transaction as MEV if any of:
/// 1. It is part of a sandwich bundle (buy-victim-sell in same block).
/// 2. It is a backrun (immediately follows a large swap in the same block).
/// 3. It interacts with a known MEV relay (Flashbots, MEV-Boost builder).
/// 4. Its gas price is >3x the block median (priority fee bidding).
pub fn label_mev_transactions(txs: &[Transaction], block: &Block) -> Vec<bool> {
    let median_gas = median_gas_price(txs);
    txs.iter().map(|tx| {
        is_sandwich_component(tx, txs)
            || is_backrun(tx, txs)
            || is_known_mev_relay(&tx.from)
            || tx.gas_price > median_gas * 3.0
    }).collect()
}
```

#### Bridge latency model for multi-chain tests

Cross-chain Granger tests must account for message propagation delay:

| Bridge type | Typical latency | Model |
|---|---|---|
| Native bridge (L1 -> L2) | 1 - 15 minutes | Fixed lag = 10 minutes |
| Third-party bridge (LayerZero, Wormhole) | 2 - 30 minutes | Fixed lag = 15 minutes |
| Optimistic rollup -> L1 | 7 days (dispute period) | Fixed lag = 7 days |
| ZK rollup -> L1 | 1 - 4 hours (proof generation) | Fixed lag = 2 hours |

The Granger test lag order `max_lag` is set to `ceil(bridge_latency / grid_spacing) + 2` to account for the bridge latency plus a buffer.

### Do-operator on code: supported change formats

The coding-domain do-operator supports these intervention types:

```rust
/// Supported code intervention formats for causal experiments.
pub enum CodeIntervention {
    /// Modify a function body.
    FunctionBody {
        file: PathBuf,
        function_name: String,
        new_body: String,
    },
    /// Add/remove a dependency.
    DependencyChange {
        crate_name: String,
        action: DepAction, // Add, Remove, ChangeVersion
    },
    /// Modify a configuration value.
    ConfigChange {
        key: String,
        old_value: String,
        new_value: String,
    },
    /// Apply a diff patch.
    Patch {
        diff: String, // unified diff format
    },
}

/// Observable timing model for coding experiments.
///
/// After applying a code change, measure these observables:
pub struct CodingObservables {
    pub compile_time_ms: u64,
    pub test_pass_rate: f64,
    pub test_duration_ms: u64,
    pub clippy_warning_count: usize,
    pub binary_size_bytes: u64,
}
```

**Snapshot/restore**: Uses `git stash` for lightweight snapshots. For heavier experiments (dependency changes), creates a temporary git worktree. Restore is `git stash pop` or worktree deletion.

### Backdoor adjustment: handling high-cardinality Z

When the conditioning set Z contains high-cardinality variables (many unique values), direct enumeration of Z values is infeasible. Two mitigation strategies:

1. **Binning**: For continuous Z variables, bin into quantiles (default: 10 bins). This trades precision for tractability.

2. **Propensity score**: Replace Z with a 1-dimensional propensity score `e(Z) = P(X=1|Z)`. The backdoor adjustment becomes `sum over e(Z) bins of P(Y|X, e(Z)) * P(e(Z))`.

```rust
/// Backdoor adjustment with propensity score dimensionality reduction.
///
/// When |Z| > max_cardinality, collapses Z into a propensity score.
pub fn backdoor_adjustment_propensity(
    graph: &CausalGraph,
    x: VariableId,
    y: VariableId,
    z: &[VariableId],
    data: &DataFrame,
    max_cardinality: usize,  // default: 100
    n_bins: usize,           // default: 10
) -> Option<f64> {
    if !graph.satisfies_backdoor(x, y, z) {
        return None;
    }

    let z_cardinality: usize = z.iter()
        .map(|v| data.n_unique(v))
        .product();

    if z_cardinality <= max_cardinality {
        // Direct adjustment
        return backdoor_adjustment(graph, x, y, z, data);
    }

    // Propensity score collapse
    let propensity = logistic_regression(data, x, z);
    let binned_propensity = quantile_bin(&propensity, n_bins);
    backdoor_adjustment(graph, x, y, &[binned_propensity], data)
}
```

### Adequacy detection for backdoor sets

Automatically find a valid backdoor set (if one exists):

```rust
/// Find a minimal valid backdoor adjustment set.
///
/// Algorithm: start with all non-descendants of X. Test the backdoor
/// criterion. If valid, greedily remove variables while maintaining
/// validity. Returns None if no valid backdoor set exists.
pub fn find_backdoor_set(
    graph: &CausalGraph,
    x: VariableId,
    y: VariableId,
) -> Option<Vec<VariableId>> {
    let descendants_x = graph.descendants(x);
    let candidates: Vec<VariableId> = graph.all_variables()
        .filter(|v| *v != x && *v != y && !descendants_x.contains(v))
        .collect();

    // Start with full candidate set and prune
    let mut z = candidates.clone();
    if !graph.satisfies_backdoor(x, y, &z) {
        return None; // no valid backdoor set exists
    }

    // Greedy minimization
    for candidate in &candidates {
        let reduced: Vec<_> = z.iter().filter(|v| *v != candidate).cloned().collect();
        if graph.satisfies_backdoor(x, y, &reduced) {
            z = reduced;
        }
    }

    Some(z)
}
```

### Intervention hypothesis confidence threshold

Hypotheses are accepted when the observed effect matches the predicted effect within a tolerance:

```
|observed_effect - predicted_effect| < tolerance

tolerance = max(0.1, 0.2 * |predicted_effect|)
```

The 0.1 absolute floor prevents rejection of hypotheses with small predicted effects due to noise. The 0.2 relative component accounts for model imprecision scaling with effect size.

Confidence updates after each test:

```
if supported:
    confidence = confidence + 0.1 * (1.0 - confidence)   // diminishing increase
if not supported:
    confidence = confidence * 0.7                         // 30% penalty
```

A hypothesis is promoted to "confirmed" at confidence >= 0.8 (requires ~5 supporting tests from a neutral prior of 0.5). It is demoted to "rejected" at confidence < 0.1 (requires ~3 consecutive failures from 0.5).

### Test criteria

- **PC algorithm on known DAG**: Given data generated from X -> Y -> Z, the PC algorithm recovers the correct structure.
- **Conditional independence calibration**: On independent data, the test rejects at rate <= alpha.
- **Granger test on known causal series**: When X(t) = X(t-1) + noise, Y(t) = 0.5*X(t-1) + Y(t-1) + noise, the test detects X -> Y.
- **Do-operator correctness**: In a confounded model (X <- Z -> Y, X -> Y), `do(X)` gives a different result than conditioning on X.
- **Backdoor adjustment**: On synthetic data with known causal effect, the adjusted estimate is within 10% of the true effect.
- **Coding intervention round-trip**: After applying and restoring a CodeIntervention, the workspace is in its original state.
- **MEV label accuracy**: On a set of labeled Flashbots bundles, the heuristic achieves >90% recall.

---

## Academic foundations

- Pearl, J. (2009). *Causality: Models, Reasoning, and Inference*. 2nd ed. Cambridge University Press. — SCM formalism, do-calculus, backdoor criterion.
- Spirtes, P., Glymour, C., & Scheines, R. (2000). *Causation, Prediction, and Search*. 2nd ed. MIT Press. — PC algorithm.
- Granger, C. W. J. (1969). "Investigating Causal Relations by Econometric Models and Cross-spectral Methods." *Econometrica*, 37(3), 424-438. — Granger causality.
- Pearl, J. (2019). "The seven tools of causal inference." *Communications of the ACM*, 62(3), 54-60. — Accessible overview of the causal hierarchy.
- Peters, J., Janzing, D., & Schölkopf, B. (2017). *Elements of Causal Inference*. MIT Press. — Modern causal discovery algorithms.

---

## Cross-references

- See [01-oracle-trait.md](./01-oracle-trait.md) for how causal models feed oracle predictions
- See [02-chain-oracles.md](./02-chain-oracles.md) for mirage-rs simulation integration
- See [03-coding-oracles.md](./03-coding-oracles.md) for coding-domain causal discovery
- See [10-predictive-geometry-and-resonant-patterns.md](./10-predictive-geometry-and-resonant-patterns.md) for topological constraints on causal graphs
- See [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) for how causal models inform active inference
