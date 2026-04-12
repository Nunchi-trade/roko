# Causal Microstructure Discovery

> Correlation is not causation. The causal discovery subsystem uses Pearl's structural causal models, Granger causality, and interventional experiments (via mirage-rs simulation) to discover genuine causal relationships in structured domains.

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
