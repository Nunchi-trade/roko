# Technical Analysis -- gap checklist

Spec: `docs/20-technical-analysis/` (15 files).
Code: `crates/roko-core/`, `crates/roko-learn/`, `crates/roko-primitives/`, `crates/roko-index/`.

Overall: ~30% compliant. The `Oracle` trait, `Prediction`, `PredictionStore`,
`ResidualCorrector`, `CalibrationTracker`, and `HdcVector` primitives exist. Domain-specific
oracle implementations (Chain, Coding, Research) are spec-only. Advanced TA features (spectral
manifolds, causal discovery, sheaf geometry, tropical algebra) are entirely unimplemented.

## Compliant (no action needed)

- `Oracle` trait with `predict()` + `evaluate()` (doc 01) -- `crates/roko-core/src/prediction.rs:20`
- `OracleQuery` with domain, category, parameters, horizon (doc 01) -- `prediction.rs:37`
- `OracleDomain` enum: Chain, Coding, Research, Custom (doc 01) -- `prediction.rs:101`
- `Prediction` struct with value, confidence, interval, lineage (doc 01) -- `prediction.rs:352`
- `PredictionAccuracy` struct (doc 01) -- `prediction.rs:531`
- `PredictionStore` with register/track/resolve lifecycle (doc 01, 13) -- `prediction.rs:619`
- `ResidualCorrector` for bias correction (doc 13) -- `prediction.rs:763`
- `CalibrationTracker` per-(model, category) accuracy (doc 13) -- `prediction.rs:821`
- `HdcVector` 10,240-bit BSC with bind/bundle/permute/similarity (doc 06) -- `crates/roko-primitives/src/hdc.rs:30`
- `CorticalState` shared atomic signal bus (doc 05) -- `crates/roko-runtime/src/heartbeat.rs:269`
- `MultiPatchForager` + `should_stop_searching()` MVT stopping (doc 13) -- `crates/roko-compose/src/foraging.rs:25`

---

## Checklist

### TA-01: ChainOracle implementation
- [x] Implement ChainOracle with traditional + DeFi indicators

**Spec** (doc 02 `docs/20-technical-analysis/02-chain-oracles.md`): `ChainOracle` implements `Oracle` trait with two indicator families:

**Traditional TA indicators**: MA (simple/exponential moving average), RSI (14-period Relative Strength Index, Wilder 1978), Bollinger Bands (2-sigma from 20-period SMA), MACD (12/26/9 signal line crossover). These operate on price time series.

**DeFi-native indicators**: concentrated liquidity distribution (Uniswap V3 tick analysis), lending utilization (Aave/Morpho health factors), funding rates (perp vs spot premium), yield curves (PT/YT term structure), on-chain options skew (Panoptic put/call ratio).

**8 T0 chain probes** (zero-LLM, <100ms each): (1) gas_baseline — current gwei vs 7d average, (2) mempool_pressure — pending tx count trend, (3) liquidity_depth — combined TVL change rate, (4) funding_rate — perp funding vs threshold, (5) volatility — Garman-Klass estimator, (6) mev_density — sandwich/frontrun tx ratio per block, (7) block_time_drift — actual vs expected block time, (8) whale_flow — large tx volume vs average.

`predict()` returns a `Prediction` with `value: PredictionValue::Continuous(f64)` or `Categorical { outcomes, probabilities }`, `confidence: f64`, `interval: ConfidenceInterval`, `horizon: Duration`. `evaluate()` compares prediction against on-chain outcome Engram.

**Current code**: `Oracle` trait at `crates/roko-core/src/prediction.rs:20` with `predict()` and `evaluate()`. `OracleDomain::Chain` at line 101. `OracleQuery` at line 37 with `domain`, `category`, `parameters`, `horizon` fields. `Prediction` at line 352 with value, confidence, interval, lineage. `crates/roko-chain/src/` exists with type stubs but no Oracle implementation.

**What to change**: Create `crates/roko-chain/src/oracle.rs` with:
```rust
pub struct ChainOracle {
    rpc_provider: /* alloy provider */,
    indicators: Vec<Box<dyn ChainIndicator>>,
    probes: Vec<Box<dyn Probe>>,
}
impl Oracle for ChainOracle {
    async fn predict(&self, query: &OracleQuery, ctx: &Context) -> Result<Prediction>;
    async fn evaluate(&self, prediction: &Prediction, outcome: &Engram) -> Result<PredictionAccuracy>;
}
```
Start with MA, RSI, Bollinger as `ChainIndicator` implementations. Implement at least gas_baseline, liquidity_depth, volatility as T0 probes.

**Reference files**:
- `crates/roko-core/src/prediction.rs:20` — `Oracle` trait with `predict()` + `evaluate()`
- `crates/roko-core/src/prediction.rs:37` — `OracleQuery` struct
- `crates/roko-core/src/prediction.rs:101` — `OracleDomain::Chain`
- `crates/roko-core/src/prediction.rs:352` — `Prediction` struct with value, confidence, interval
- `crates/roko-chain/src/` — chain crate (implementation target)
- `crates/roko-runtime/src/heartbeat_probes.rs:25` — `Probe` trait (T0 chain probes should implement this)
- `docs/20-technical-analysis/02-chain-oracles.md` — full spec: indicators, probes, mirage-rs integration

**Accept when**:
- [x] `pub struct ChainOracle` in `crates/roko-learn/src/oracles/chain.rs`
- [x] `impl Oracle for ChainOracle` with working `predict()` and `evaluate()`
- [x] At least 3 traditional indicators (SMA, EMA, RSI, Bollinger) functional
- [x] At least 3 T0 chain probes via HeartbeatProbe (GasSpike, TvlDelta, PriceDelta, RSI, MACD)
- [x] `cargo test -p roko-learn`

**Verify**:
```bash
grep -rn 'struct ChainOracle' crates/roko-chain/src/ --include='*.rs'
grep -rn 'impl Oracle for ChainOracle' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```

**Priority**: P2 (Phase 2+)

---

### TA-02: CodingOracle implementation
- [x] Implement CodingOracle with software engineering indicators

**Spec** (doc 03 `docs/20-technical-analysis/03-coding-oracles.md`): `CodingOracle` implements `Oracle` with software engineering indicators:

**Prediction categories**: (1) build time prediction — trend analysis of `cargo build` duration, (2) test failure probability — per-file historical test pass rate, (3) complexity drift — McCabe cyclomatic complexity delta per commit (McCabe 1976), (4) dependency risk — dependency freshness/vulnerability score, (5) performance regression — benchmark trend detection.

**6 T0 coding probes** (zero-LLM, <100ms): (1) build_time_trend — current build time vs 7d rolling average, (2) test_pass_rate — test pass rate delta from last N runs, (3) complexity_delta — McCabe complexity change in modified files, (4) dependency_freshness — outdated dependency count / total, (5) churn_rate — lines changed per commit trend (Nagappan & Ball 2005), (6) file_coupling — co-change frequency between file pairs.

Tech debt feedback loops: complexity accumulates -> development slows -> more shortcuts -> more debt (Lehman 1980). The CodingOracle measures this and predicts when debt will cause failures.

**Current code**: No `CodingOracle` struct. `crates/roko-index/` has parser + graph + HDC indexing for code intelligence (`workspace.rs` for workspace analysis, `parser.rs` for AST parsing) but no oracle implementation. `crates/roko-runtime/src/heartbeat_probes.rs:139` has `EngineState` with coding fields (`build_time_secs`, `test_pass_rate`, `complexity_delta`) but these are probe inputs, not oracle outputs.

**What to change**: Create `CodingOracle` struct in `crates/roko-index/src/oracle.rs` (co-located with code analysis):
```rust
pub struct CodingOracle {
    workspace: WorkspaceAnalyzer,  // from roko-index
    history: Vec<BuildRecord>,     // build time history
}
impl Oracle for CodingOracle {
    async fn predict(&self, query: &OracleQuery, ctx: &Context) -> Result<Prediction>;
    async fn evaluate(&self, prediction: &Prediction, outcome: &Engram) -> Result<PredictionAccuracy>;
}
```
Start with build_time_trend and test_pass_rate probes. Use `roko-index` workspace analysis for complexity metrics.

**Reference files**:
- `crates/roko-core/src/prediction.rs:20` — `Oracle` trait to implement
- `crates/roko-core/src/prediction.rs:101` — `OracleDomain::Coding`
- `crates/roko-index/src/workspace.rs` — workspace-level code analysis (use for complexity metrics)
- `crates/roko-index/src/parser.rs` — AST parsing (use for McCabe complexity)
- `crates/roko-runtime/src/heartbeat_probes.rs:139` — `EngineState` coding fields (probe data source)
- `docs/20-technical-analysis/03-coding-oracles.md` — full spec: 5 prediction categories, 6 T0 probes, tech debt loops

**Accept when**:
- [x] `pub struct CodingOracle` implements `Oracle`
- [x] Build time prediction functional (compares to rolling average)
- [x] Test failure probability prediction functional (uses historical pass rate)
- [x] At least 3 T0 coding probes (build_time_trend, test_pass_rate, complexity_delta)
- [x] Uses complexity metrics (CodingOracle.observe_complexity() accepts workspace-computed deltas)
- [x] `cargo test -p roko-index` or hosting crate passes

**Verify**:
```bash
grep -rn 'struct CodingOracle' crates/ --include='*.rs'
grep -rn 'impl Oracle for CodingOracle' crates/ --include='*.rs'
cargo test --workspace
```

**Priority**: P2 (Phase 2+)

---

### TA-03: ResearchOracle implementation
- [x] Implement ResearchOracle with source reliability and completeness

**Spec** (doc 04 §ResearchOracle): `ResearchOracle` implements `Oracle` with source reliability,
completeness assessment, contradiction detection, replication probability, citation momentum.
p-hacking detection. Charnov stopping rule for research.

**Current code**: No `ResearchOracle` struct exists. Research commands exist in roko-cli
(`roko research`) but they do not use the Oracle trait.

**What to change**: Create `ResearchOracle` struct. Wire to the research subsystem for source
evaluation. Start with source reliability scoring and completeness assessment.

**Reference files**:
- `crates/roko-core/src/prediction.rs:20` -- `Oracle` trait to implement
- `crates/roko-core/src/prediction.rs:101` -- `OracleDomain::Research`
- `crates/roko-cli/src/research.rs` -- existing research commands
- `docs/20-technical-analysis/04-research-oracles.md` -- full spec

**Accept when**:
- [x] `pub struct ResearchOracle` implements `Oracle`
- [x] Source reliability scoring functional
- [x] Completeness assessment functional
- [x] `cargo test` for the crate passes

**Verify**:
```bash
grep -rn 'struct ResearchOracle' crates/ --include='*.rs'
cargo test --workspace
```

**Priority**: P2 (Phase 2+)

---

### TA-04: Generalized witness pipeline
- [x] Implement domain-agnostic data ingestion trait

**Spec** (doc 05 `docs/20-technical-analysis/05-witness-as-ta-generalized.md`): The `Witness` trait generalizes data ingestion across domains. Each domain oracle uses a witness to observe its environment and triage signals:

```rust
pub trait Witness: Send + Sync {
    /// Observe the environment and return raw signal data.
    async fn observe(&self, ctx: &Context) -> Result<Vec<Observation>>;

    /// Triage observations into CorticalState updates.
    /// Uses MIDAS-R for anomaly detection and DDSketch for quantile estimation.
    fn triage(&self, observations: &[Observation]) -> TriageResult;
}
```

**Domain witnesses**: (1) ChainWitness — observes blockchain state (blocks, logs, prices, mempool), (2) CodingWitness — observes filesystem + CI/CD (build results, test results, git history), (3) ResearchWitness — observes research corpus (new sources, citation updates, replication results).

**Triage pipeline**: MIDAS-R (Bhatia et al. 2020) for streaming anomaly detection, DDSketch (Masson et al. 2019) for quantile estimation. Triage determines whether a signal is anomalous and should trigger tier escalation.

**Three cognitive speeds in the witness**: T0 (reflex, <1ms) — pure probe evaluation, T1 (deliberative) — LLM analysis of anomalies, T2 (reflective) — cross-session pattern analysis.

**Current code** (`crates/roko-runtime/src/heartbeat.rs:269`): `CorticalState` exists with atomic signal bus. `crates/roko-runtime/src/heartbeat_probes.rs:25` has `Probe` trait and `EngineState`. No generalized `Witness` trait. No MIDAS-R or DDSketch implementations. Probes exist but are not organized as domain witnesses.

**What to change**: Define `Witness` trait in `crates/roko-core/src/` or `crates/roko-runtime/src/`. Implement `CodingWitness` that wraps the 6 coding T0 probes and observes filesystem state. Wire `triage()` output into `CorticalState` atomic writes.

**Reference files**:
- `crates/roko-runtime/src/heartbeat.rs:269` — `CorticalState` (T0 probe results written here)
- `crates/roko-runtime/src/heartbeat_probes.rs:25` — `Probe` trait, `EngineState` (witness reads probes)
- `crates/roko-core/src/prediction.rs:20` — `Oracle` trait (witnesses feed oracles)
- `docs/20-technical-analysis/05-witness-as-ta-generalized.md` — full spec: Witness trait, domain implementations, MIDAS-R/DDSketch triage

**Accept when**:
- [x] `Witness` trait defined with `observe()` + `triage()` methods
- [x] `CodingWitness` implements `Witness` using filesystem + CI/CD data (6 T0 probes)
- [x] Triage results include anomaly classifications (Normal/Elevated/Anomalous)
- [x] Threshold-based anomaly detection via Welford online z-scores
- [x] `cargo test -p roko-learn` passes (12 witness tests)

**Verify**:
```bash
grep -rn 'trait Witness\|CodingWitness' crates/ --include='*.rs' | grep -v target/
grep -rn 'CorticalState' crates/roko-runtime/src/ --include='*.rs'
cargo test -p roko-runtime
```

**Priority**: P2 (Phase 2+)

---

### TA-05: HDC pattern algebra for TA
- [x] Implement temporal encoding and cross-domain codebooks

**Spec** (doc 06 `docs/20-technical-analysis/06-hyperdimensional-ta.md`): HDC (Hyperdimensional Computing) provides pattern algebra for TA. The 10,240-bit BSC (Binary Sparse Code) vectors support:

**Role-filler composition**: bind named roles to values: `pattern = bind(ROLE_PRICE, price_vec) XOR bind(ROLE_VOLUME, volume_vec)`. This creates structured composite patterns where roles are retrievable.

**Temporal encoding**: `permute(vec, k)` shifts bits by k positions. A sequence `[v1, v2, v3]` is encoded as `bundle(permute(v1, 2), permute(v2, 1), v3)`. This is shift-invariant — the same temporal pattern at different timestamps has high similarity.

**Domain codebooks**: seed-based symbol allocation per domain. Each domain (Chain, Coding, Research) has a codebook of atomic symbols: `let gas_high = Codebook::chain().symbol("gas_high")`. Codebooks are deterministically generated from a domain seed, ensuring consistent encoding across sessions.

**Cross-domain resonance detection**: when similarity between patterns from different domains exceeds threshold 0.526, a "resonance" is detected — indicating a cross-domain structural analogy. Example: a gas spike pattern in Chain domain resonates with a build timeout pattern in Coding domain.

**Pattern store**: persistent store of HDC patterns with similarity-based retrieval. Consolidated during Dreams (Delta cycle).

**Current code** (`crates/roko-primitives/src/hdc.rs:30`): `HdcVector` struct with `bind()` (XOR, line 113), `bundle()` (majority vote, line 129), `permute()` (bit rotation, line 154), `similarity()` (normalized Hamming, line 223). These are the raw primitives. `crates/roko-index/src/hdc.rs` uses HDC for code intelligence indexing. **No codebook management. No role-filler helpers. No pattern store. No resonance detection.**

**What to change**: Add to `crates/roko-primitives/src/`:
1. `codebook.rs` — `Codebook` struct with `fn symbol(&self, name: &str) -> HdcVector` using deterministic seed-based allocation
2. `role_filler.rs` — `fn role_bind(role: &HdcVector, filler: &HdcVector) -> HdcVector` helper and `fn unbind(pattern: &HdcVector, role: &HdcVector) -> HdcVector`
3. `pattern_store.rs` — `PatternStore` with `insert(pattern: HdcVector, metadata: PatternMeta)` and `fn search(query: &HdcVector, threshold: f64) -> Vec<(PatternMeta, f64)>`
4. `resonance.rs` — `fn detect_resonance(pattern_a: &HdcVector, pattern_b: &HdcVector) -> Option<Resonance>` with threshold 0.526

**Reference files**:
- `crates/roko-primitives/src/hdc.rs:30` — `HdcVector` with bind(113)/bundle(129)/permute(154)/similarity(223)
- `crates/roko-index/src/hdc.rs` — HDC indexing in code intelligence (pattern reference for codebook usage)
- `docs/20-technical-analysis/06-hyperdimensional-ta.md` — full spec: role-filler, temporal encoding, codebooks, resonance at 0.526, pattern store

**Accept when**:
- [x] `Codebook` struct with deterministic seed-based symbol allocation
- [x] `role_bind()` / `unbind()` helpers for role-filler composition
- [x] At least one domain codebook (Coding) with 10+ symbols
- [x] `PatternStore` with similarity-based retrieval
- [x] Cross-domain resonance detection at threshold 0.526
- [x] `cargo test -p roko-primitives`

**Verify**:
```bash
grep -rn 'Codebook\|role_bind\|PatternStore\|resonance\|0.526' crates/roko-primitives/src/ --include='*.rs'
cargo test -p roko-primitives
```

**Priority**: P2 (Phase 2+)

---

### TA-06: Spectral liquidity manifolds
- [x] Implement Riemannian geometry for execution cost landscape

**Spec** (doc 07 `docs/20-technical-analysis/07-spectral-liquidity-manifolds.md`): Riemannian geometry models the DeFi execution cost landscape as a smooth manifold. The metric tensor at each point encodes the local cost structure:

```
g_ij(x) = | slippage_cost   cross_term      0              0             |
           | cross_term      gas_cost        0              0             |
           | 0               0              time_cost       0             |
           | 0               0              0              opportunity_cost|
```

**Core primitives**:
1. **MetricTensor** — symmetric positive-definite matrix at each point. `fn at(&self, point: &ManifoldPoint) -> Matrix4x4`. Computed from: slippage = f(amount, liquidity_depth), gas = f(base_fee, priority_fee, gas_limit), time = f(block_time, confirmation_blocks), opportunity = f(price_change_rate, hold_duration).
2. **Christoffel symbols** — `Γ^k_ij = ½ g^kl (∂_i g_jl + ∂_j g_il - ∂_l g_ij)`. Computed numerically from finite differences of the metric tensor.
3. **Geodesic solver** — `d²x^k/dt² + Γ^k_ij dx^i/dt dx^j/dt = 0`. Solve as system of ODEs using RK4 integration. The geodesic between two points is the minimum-cost execution path.
4. **Ricci scalar** — `R = g^ij R_ij` (trace of Ricci tensor). Positive = market instability (high curvature), negative = stable conditions. Used as a market stability indicator.
5. **Frechet mean** — `argmin_x Σ d²(x, x_i)` where d is geodesic distance. Provides a "center of mass" for aggregating multiple cost observations on the manifold.

**Current code**: No Riemannian geometry implementations. `crates/roko-primitives/src/` has HDC vectors and tier routing but no differential geometry. No matrix operations library (would need `nalgebra` or similar).

**What to change**: Add `crates/roko-primitives/src/manifold.rs`:
1. `MetricTensor` struct with `fn at(point: &[f64; 4]) -> [[f64; 4]; 4]`
2. `fn christoffel(metric: &MetricTensor, point: &[f64; 4]) -> [[[f64; 4]; 4]; 4]` via finite differences
3. `fn geodesic(metric: &MetricTensor, start: [f64; 4], end: [f64; 4], steps: usize) -> Vec<[f64; 4]>` via RK4
4. `fn frechet_mean(metric: &MetricTensor, points: &[[f64; 4]]) -> [f64; 4]` via iterative geodesic midpoint
5. Add `nalgebra` dependency for matrix operations

**Reference files**:
- `crates/roko-primitives/src/` — mathematical primitives crate (implementation target)
- `crates/roko-primitives/src/hdc.rs` — HDC vectors (pattern for primitives module structure)
- `docs/20-technical-analysis/07-spectral-liquidity-manifolds.md` — full spec: metric tensor, Christoffel, geodesics, Ricci, Frechet, parallel transport

**Accept when**:
- [x] `MetricTensor` struct with `at()` method for 4D cost manifold
- [x] Christoffel symbols computed via finite differences
- [x] Geodesic solver (RK4) finds minimum-cost path between two points
- [x] Frechet mean aggregates observations on the manifold
- [x] `cargo test -p roko-primitives`

**Verify**:
```bash
grep -rn 'MetricTensor\|Geodesic\|FrechetMean\|christoffel' crates/roko-primitives/src/ --include='*.rs'
cargo test -p roko-primitives
```

**Priority**: P2 (Phase 2+, research)

---

### TA-07: Adaptive signal metabolism
- [x] Implement replicator dynamics and fitness landscapes for signals

**Spec** (doc 08 `docs/20-technical-analysis/08-adaptive-signal-metabolism.md`): Signals (Engrams) are treated as organisms in a fitness landscape. Their population dynamics follow evolutionary biology:

**Replicator dynamics** (Taylor & Jonker 1978):
```
dx_i/dt = x_i * (f_i - φ)
```
where `x_i` = population fraction of signal type i, `f_i` = fitness of type i, `φ = Σ x_i * f_i` = average fitness. Signals with above-average fitness grow; below-average signals shrink.

**Hebbian learning** (Oja's rule): signal-to-outcome connection weights update via:
```
Δw = η * (y * x - y² * w)
```
where `x` = signal activation, `y` = outcome verification, `w` = current weight, `η` = learning rate. Self-normalizing: weights converge to principal eigenvector direction.

**SignalRegistry ecosystem**: maintains a population of active signal patterns with per-pattern fitness scores, birth/death rates, and speciation tracking. Fisher's fundamental theorem: the rate of fitness increase equals the genetic variance in fitness — more diverse signal populations adapt faster.

**Current code**: No `SignalRegistry` struct. No replicator dynamics. No Hebbian learning. `crates/roko-learn/src/` has learning infrastructure (episodes, bandits, efficiency) but not evolutionary signal dynamics. `crates/roko-core/src/` has Engram types but no fitness tracking on Engrams.

**What to change**: Add `crates/roko-learn/src/signal_metabolism.rs`:
1. `SignalRegistry` struct with `HashMap<SignalTypeId, SignalPopulation>` tracking population sizes and fitness scores
2. `fn replicator_step(registry: &mut SignalRegistry, dt: f64)` applying replicator dynamics update
3. `fn hebbian_update(weights: &mut [f64], signal: &[f64], outcome: f64, lr: f64)` implementing Oja's rule
4. `fn population_fitness_variance(registry: &SignalRegistry) -> f64` for Fisher's theorem monitoring
5. Speciation detection: when a signal pattern diverges beyond threshold, create a new species

**Reference files**:
- `crates/roko-learn/src/` — learning subsystem (implementation target)
- `crates/roko-core/src/` — Engram types, signal definitions
- `crates/roko-learn/src/episode_logger.rs` — episode data (source of fitness signals)
- `docs/20-technical-analysis/08-adaptive-signal-metabolism.md` — full spec: replicator dynamics, Oja's rule, Fisher's theorem, speciation, Red Queen

**Accept when**:
- [x] `SignalRegistry` struct with per-type population and fitness tracking
- [x] Replicator dynamics update: `dx_i/dt = x_i * (f_i - φ)`
- [x] Hebbian weight update (Oja's rule): self-normalizing, converges to principal eigenvector
- [x] Population fitness variance computed for monitoring
- [x] `cargo test -p roko-learn` passes

**Verify**:
```bash
grep -rn 'SignalRegistry\|replicator_step\|hebbian_update\|population_fitness' crates/roko-learn/src/ --include='*.rs'
cargo test -p roko-learn
```

**Priority**: P2 (Phase 2+, research)

---

### TA-08: Causal microstructure discovery
- [x] Implement causal discovery algorithms

**Spec** (doc 09 `docs/20-technical-analysis/09-causal-microstructure-discovery.md`): Pearl's causal hierarchy applied to agent signals:

**Level 1 — Association**: `P(Y|X)` — correlational patterns between signal time series. Already captured by Neuro's causal link distillation (textual/heuristic).

**Level 2 — Intervention**: `P(Y|do(X))` — what happens if we force X to a value? Use mirage-rs EVM fork to perform interventional experiments (e.g., "what if gas was 50 gwei?" by simulating at fixed gas).

**Level 3 — Counterfactual**: `P(Y_x|X=x', Y=y')` — what would have happened? Use Dream cycle (REM phase) to simulate alternative histories.

**PC algorithm** (Spirtes, Glymour, Scheines 2000): constraint-based causal discovery from observational data:
1. Start with complete undirected graph over variables
2. For each pair (X,Y), test conditional independence `X ⊥ Y | S` for all subsets S of neighbors
3. Remove edge if conditionally independent (using partial correlation test, threshold p < 0.05)
4. Orient edges using v-structures and acyclicity
Result: a DAG (directed acyclic graph) of causal relationships.

**Granger causality** (Granger 1969): X Granger-causes Y if past values of X improve prediction of Y beyond Y's own past. Test via F-statistic comparing restricted vs unrestricted autoregressive models. 4 DeFi extensions: (1) TVL → gas price, (2) whale transfers → price volatility, (3) funding rate → spot premium, (4) mempool patterns → MEV activity.

**Current code** (`crates/roko-neuro/src/distiller.rs:394`): `KnowledgeKind::CausalLink` exists. Causal link distillation extracts heuristic causal claims from LLM episodes (textual, not formal). `CAUSAL_LINK_HALF_LIFE_DAYS` at `crates/roko-neuro/src/lib.rs:64`. Causal link query tests at `crates/roko-neuro/src/knowledge_store.rs:2030`. **These are textual/LLM-extracted causal claims, not formal statistical causal discovery.**

**What to change**: Add `crates/roko-learn/src/causal.rs` or `crates/roko-primitives/src/causal.rs`:
1. `fn granger_test(x: &[f64], y: &[f64], lag: usize) -> GrangerResult` — F-test comparing restricted vs unrestricted AR models
2. `fn pc_algorithm(data: &[Vec<f64>], alpha: f64) -> CausalDag` — PC algorithm with conditional independence tests
3. `CausalDag` struct: nodes (variable IDs), directed edges (cause → effect), edge strengths
4. Wire Granger tests to signal time series from `.roko/signals.jsonl`
5. Integrate formal DAG with textual causal links from Neuro distiller

**Reference files**:
- `crates/roko-neuro/src/distiller.rs:394` — textual causal link distillation (complement with formal methods)
- `crates/roko-neuro/src/lib.rs:64` — `CAUSAL_LINK_HALF_LIFE_DAYS`
- `crates/roko-neuro/src/knowledge_store.rs:2030` — causal link query tests
- `crates/roko-learn/src/` — learning subsystem (implementation target)
- `docs/20-technical-analysis/09-causal-microstructure-discovery.md` — full spec: PC algorithm, Granger, NOTEARS/DAGMA, interventional discovery

**Accept when**:
- [x] Granger causality test implemented with F-statistic (Paulson approx critical value)
- [x] PC algorithm produces `CausalDag` from multivariate time series (conditional independence + v-structures)
- [x] `CausalDag` struct with nodes, directed edges, edge strengths
- [x] DeFi causal relationships discoverable (TVL -> gas price test included)
- [x] `cargo test -p roko-learn` passes (11 causal tests)

**Verify**:
```bash
grep -rn 'granger_test\|pc_algorithm\|CausalDag\|GrangerResult' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```

**Priority**: P2 (Phase 2+, research)

---

### TA-09: Predictive geometry and resonant patterns
- [x] Implement TDA persistence diagrams and pattern evolution

**Spec** (doc 10 `docs/20-technical-analysis/10-predictive-geometry-and-resonant-patterns.md`):

**Persistence diagrams** (Bubenik 2015, Carlsson 2009): Topological Data Analysis extracts shape features from time series data that are invariant to continuous deformation:
1. Embed time series via Takens delay embedding: `x(t) → [x(t), x(t-τ), x(t-2τ), ...]` in d-dimensional space
2. Build Vietoris-Rips simplicial complex at increasing scales ε
3. Track birth/death of topological features (connected components = H0, loops = H1, voids = H2) across scales
4. Output: persistence diagram = set of (birth, death) pairs. Long-lived features (far from diagonal) are genuine structure; short-lived are noise.
5. **Persistence landscape** (Bubenik): vectorization of persistence diagram into a Banach space element, enabling statistical operations (mean, variance, hypothesis testing).

**Resonant patterns**: Patterns are organisms with HDC vector genomes and fitness scores:
```rust
pub struct ResonantPattern {
    pub genome: HdcVector,      // 10,240-bit HDC encoding
    pub fitness: f64,           // prediction accuracy over lifetime
    pub age: u64,               // ticks since birth
    pub offspring_count: u32,   // reproduction success
    pub persistence: PersistenceDiagram, // topological signature
}
```
Patterns compete for attention budget via VCG auction (using existing `crates/roko-compose/src/auction.rs`). Lotka-Volterra dynamics govern predator-prey relationships between competing patterns. Price equation `ΔZ̄ = Cov(w,z)/w̄ + E(wΔz)/w̄` tracks evolutionary change.

**Current code**: `HdcVector` at `crates/roko-primitives/src/hdc.rs:30` with bind/bundle/permute/similarity. VCG auction at `crates/roko-compose/src/auction.rs:32`. No TDA, persistence diagrams, Lotka-Volterra, or `ResonantPattern` struct.

**What to change**: Add `crates/roko-primitives/src/tda.rs`:
1. `PersistenceDiagram` struct: `Vec<(f64, f64)>` birth-death pairs
2. `fn vietoris_rips(points: &[Vec<f64>], max_dim: usize) -> PersistenceDiagram` — simplicial complex computation
3. `fn persistence_landscape(diagram: &PersistenceDiagram, resolution: usize) -> Vec<Vec<f64>>` — vectorization
4. `fn takens_embedding(series: &[f64], dim: usize, tau: usize) -> Vec<Vec<f64>>` — delay embedding

Add `crates/roko-learn/src/resonant_patterns.rs`:
5. `ResonantPattern` struct with HDC genome, fitness, persistence signature
6. `fn lotka_volterra_step(patterns: &mut [ResonantPattern], dt: f64)` — population dynamics

**Reference files**:
- `crates/roko-primitives/src/hdc.rs:30` — HDC vectors (pattern genomes)
- `crates/roko-compose/src/auction.rs:32` — VCG auction (pattern competition mechanism)
- `docs/20-technical-analysis/10-predictive-geometry-and-resonant-patterns.md` — full spec

**Accept when**:
- [x] `PersistenceDiagram` computed from time series via Takens embedding + Vietoris-Rips
- [x] `persistence_landscape()` vectorizes diagram for statistical operations (+ L2 distance)
- [x] `ResonantPattern` struct with HDC genome, fitness, population dynamics
- [x] Lotka-Volterra dynamics for pattern competition (fitness-adjusted + genome similarity)
- [x] `cargo test -p roko-primitives` (16 TDA tests) and `cargo test -p roko-learn` (11 resonant tests)

**Verify**:
```bash
grep -rn 'PersistenceDiagram\|ResonantPattern\|persistence_landscape\|lotka_volterra' crates/ --include='*.rs' | grep -v target/
cargo test -p roko-primitives
```

**Priority**: P2 (Phase 2+, research)

---

### TA-10: Adversarial signal robustness
- [x] Implement robust statistics and adversarial detection for signals

**Spec** (doc 11 `docs/20-technical-analysis/11-adversarial-signal-robustness.md`):

**Robust statistics** (Huber 1964, Hampel 1974): replace standard estimators with breakdown-resistant alternatives:
1. **Trimmed mean** — discard top/bottom k% of values before averaging. `fn trimmed_mean(values: &[f64], trim_pct: f64) -> f64`. Breakdown point = trim_pct.
2. **MAD** (Median Absolute Deviation) — `MAD = median(|x_i - median(x)|) * 1.4826`. Robust scale estimator (breakdown point 50%). The 1.4826 factor makes it consistent with standard deviation for normal distributions.
3. **Hodges-Lehmann estimator** — median of all pairwise averages: `median((x_i + x_j)/2)`. Highly robust (breakdown 29%).
4. **Rank transform** — replace values with their ranks before analysis. Eliminates outlier influence entirely.

**HDC adversarial detection** (~10ns per check): for each incoming signal, compute `HdcVector::similarity(signal, prototype)` against known attack prototypes. If similarity > threshold (0.7), flag as adversarial. Attack prototype library:
- Chain: sandwich attack, oracle manipulation, flash loan, governance attack
- Coding: prompt injection, path traversal, dependency confusion
- Universal: replay attack, data poisoning, model extraction

**Red-team dreaming**: During Delta dream cycle (REM phase), generate adversarial scenarios by:
1. Take a successful episode
2. Mutate the input signals (flip bits in HDC vector)
3. Re-run the agent logic and check if defenses hold
4. Failed defenses become new training signals

**Current code**: `HdcVector::similarity()` at `crates/roko-primitives/src/hdc.rs:223` provides the core distance metric. No robust statistics module. No adversarial prototype library. `crates/roko-dreams/src/` has dream cycle runner (potential integration point for red-team dreaming).

**What to change**: Add `crates/roko-primitives/src/robust_stats.rs`:
1. `fn trimmed_mean(values: &mut [f64], trim_pct: f64) -> f64`
2. `fn mad(values: &[f64]) -> f64` — median absolute deviation * 1.4826
3. `fn hodges_lehmann(values: &[f64]) -> f64` — median of pairwise averages

Add `crates/roko-learn/src/adversarial.rs`:
4. `AdversarialDetector` struct with `attack_prototypes: Vec<(HdcVector, AttackType)>`
5. `fn check_signal(&self, signal: &HdcVector) -> Option<AttackType>` — HDC similarity check (~10ns)
6. `fn red_team_episode(episode: &Episode, mutations: usize) -> Vec<AdversarialScenario>` — generate adversarial variants

**Reference files**:
- `crates/roko-primitives/src/hdc.rs:223` — `HdcVector::similarity()` for prototype matching
- `crates/roko-dreams/src/` — Dreams crate (red-team dreaming integration)
- `crates/roko-learn/src/` — learning subsystem (adversarial detector integration)
- `docs/20-technical-analysis/11-adversarial-signal-robustness.md` — full spec: robust stats, HDC detection, red-team dreaming, certified robustness

**Accept when**:
- [x] `trimmed_mean()`, `mad()`, `hodges_lehmann()` implemented in `roko-primitives/src/robust_stats.rs`
- [x] `AdversarialDetector` with HDC prototype matching in `roko-learn/src/adversarial.rs`
- [x] Attack prototypes defined (chain, coding, universal domains)
- [x] `cargo test -p roko-primitives` and `cargo test -p roko-learn` pass

**Verify**:
```bash
grep -rn 'trimmed_mean\|mad\|hodges_lehmann\|AdversarialDetector' crates/ --include='*.rs' | grep -v target/
cargo test -p roko-primitives
```

**Priority**: P2 (Phase 2+, research)

---

### TA-11: Somatic TA and emergent multiscale intelligence
- [x] Wire somatic markers to TA subsystem

**Spec** (doc 12): Somatic markers (Damasio) as HDC bindings. PAD encoding. Somatic retrieval
(~63ns). 15% contrarian retrieval (Bower). IIT Phi over 9 TA subsystems (510 bipartitions).
MIB diagnostic. PID synergy detection (Williams & Beer).

**Current code** (`crates/roko-daimon/src/lib.rs:1000`): `SomaticMarker` struct exists.
`SomaticLandscape` at line 1101. `SomaticMarkerFiredEvent` at
`crates/roko-daimon/src/phase2_stubs.rs:389`. Contrarian blend weight at line 415.
`PadState` at `crates/roko-neuro/src/context.rs:148`. However, somatic markers are not
connected to the TA/Oracle subsystem.

**What to change**: Wire `SomaticMarker` retrieval into oracle `predict()` context. Add
somatic bias to prediction confidence. Implement IIT Phi metric if TA subsystems reach
sufficient count.

**Reference files**:
- `crates/roko-daimon/src/lib.rs:1000` -- `SomaticMarker` struct
- `crates/roko-daimon/src/lib.rs:1101` -- `SomaticLandscape` struct
- `crates/roko-daimon/src/phase2_stubs.rs:389` -- `SomaticMarkerFiredEvent`
- `crates/roko-neuro/src/context.rs:148` -- `PadState` struct
- `docs/20-technical-analysis/12-somatic-ta-and-emergent-multiscale.md` -- full spec

**Depends on**: TA-01 or TA-02 (need at least one domain oracle to wire somatic markers to)

**Accept when**:
- [x] Somatic markers influence oracle prediction context
- [x] 15% contrarian retrieval implemented
- [x] `cargo test` for the crate passes

**Verify**:
```bash
grep -rn 'SomaticMarker\|SomaticLandscape' crates/roko-daimon/src/ --include='*.rs'
grep -rn 'somatic' crates/roko-core/src/ --include='*.rs'
cargo test --workspace
```

**Priority**: P2 (Phase 2+)

---

### TA-12: Predictive foraging and active inference integration
- [x] Wire foraging + calibration into the oracle prediction loop

**Spec** (doc 13 `docs/20-technical-analysis/13-predictive-foraging-and-active-inference.md`): The complete prediction-resolution-calibration loop that wires existing primitives:

**Prediction lifecycle**: `register()` -> `track()` -> `resolve()` -> `feedback()`
1. Oracle produces `Prediction` via `predict()`
2. `PredictionStore::register(prediction)` stores it with a time horizon
3. When horizon expires, `PredictionStore::resolve(prediction_id, outcome)` evaluates accuracy
4. Resolution triggers:
   - `ResidualCorrector::correct(prediction, outcome)` — bias correction at ~50ns, updates running mean/variance of residuals
   - `CalibrationTracker::update(model, category, accuracy)` — per-(model, category) calibration statistics, tracks reliability decomposition (Murphy 1973)

**Thompson Sampling for oracle selection** (Thompson 1933): when multiple oracles can answer a query, sample from each oracle's posterior Beta distribution to select which one to query. This balances exploration (try uncertain oracles) with exploitation (prefer proven oracles).

**Charnov MVT stopping rule** (Charnov 1976): `MultiPatchForager::should_stop_searching()` determines when to stop querying additional oracles. When marginal gain from another query drops below the average gain rate, stop.

**Current code**:
- `PredictionStore` at `crates/roko-core/src/prediction.rs:619` — has `register()`, `resolve()` methods
- `ResidualCorrector` at line 763 — has `correct()` method, tracks running residual stats
- `CalibrationTracker` at line 821 — has `update()` and `accuracy_for(model, category)` methods
- `MultiPatchForager` at `crates/roko-compose/src/foraging.rs:25` — has `should_stop_searching()` MVT implementation
- `CascadeRouter` at `crates/roko-learn/src/cascade_router.rs` — has Thompson Sampling for model selection (can be adapted for oracle selection)
**Missing**: the wiring. `PredictionStore::resolve()` does NOT call `ResidualCorrector` or `CalibrationTracker`. No Thompson Sampling for oracle selection. `MultiPatchForager` not connected to oracle query batching.

**What to change**: This is primarily wiring work — the primitives exist:
1. In `PredictionStore::resolve()`, add calls to `ResidualCorrector::correct()` and `CalibrationTracker::update()` after computing accuracy
2. Add `OracleSelector` struct that wraps multiple `Box<dyn Oracle>` and uses Thompson Sampling (Beta distributions) to select which oracle to query — model it after `CascadeRouter`'s Thompson Sampling
3. Wire `MultiPatchForager::should_stop_searching()` to stop querying additional oracles when MVT threshold is crossed
4. Emit prediction resolution events for learning/feedback loops

**Reference files**:
- `crates/roko-core/src/prediction.rs:619` — `PredictionStore` (wire `resolve()` to corrector/tracker)
- `crates/roko-core/src/prediction.rs:763` — `ResidualCorrector` (call from resolve path)
- `crates/roko-core/src/prediction.rs:821` — `CalibrationTracker` (call from resolve path)
- `crates/roko-compose/src/foraging.rs:25` — `MultiPatchForager` with `should_stop_searching()` MVT
- `crates/roko-learn/src/cascade_router.rs` — `CascadeRouter` Thompson Sampling (pattern for OracleSelector)
- `docs/20-technical-analysis/13-predictive-foraging-and-active-inference.md` — full spec: prediction lifecycle, Thompson Sampling, MVT, EFE decomposition

**Accept when**:
- [x] `PredictionStore::resolve()` calls `ResidualCorrector::correct()` after accuracy computation
- [x] `PredictionStore::resolve()` calls `CalibrationTracker::update()` with model and category
- [x] `OracleSelector` uses Thompson Sampling to select from multiple oracles
- [x] `MultiPatchForager::should_stop_searching()` limits oracle query batching (via `OracleSelector::select_batch()`)
- [x] Prediction resolution events emitted for efficiency/learning logs
- [x] `cargo test -p roko-core`

**Verify**:
```bash
grep -rn 'ResidualCorrector\|CalibrationTracker' crates/roko-core/src/prediction.rs
grep -rn 'OracleSelector\|Thompson\|thompson' crates/ --include='*.rs' | grep -v target/
grep -rn 'should_stop_searching' crates/roko-compose/src/foraging.rs
cargo test -p roko-core
```

**Priority**: P1 (wiring existing primitives)

---

### TA-13: Sheaf-theoretic consistency
- [x] Implement cellular sheaves for oracle consistency checking

**Spec** (doc 14 `docs/20-technical-analysis/14-sheaf-tropical-geometry.md` §Sheaves): Cellular sheaves (Hansen & Ghrist 2019) provide a mathematical framework for checking local-to-global consistency across oracle predictions:

**Cellular sheaf** on a graph G = (V, E):
- Each vertex v has a stalk F(v) = vector space of local predictions (e.g., F(chain_oracle) = R^4 for [price, volume, gas, risk])
- Each edge e = (u,v) has a restriction map F(u) → F(e) ← F(v) that projects both endpoints into a shared comparison space
- A **global section** is an assignment of values to all vertices that is consistent under all restriction maps

**Sheaf Laplacian** L_F: `L_F = δ^T δ` where δ is the coboundary operator. The eigenvalues of L_F measure inconsistency:
- `λ_min(L_F) = 0` means perfect consistency (a global section exists)
- `λ_min(L_F) > 0` means the oracles disagree — the larger the value, the worse the disagreement
- The corresponding eigenvector identifies which oracle(s) are most inconsistent

**Application**: when ChainOracle says "price up" but CodingOracle says "build failures increasing" and ResearchOracle says "market stress," the sheaf Laplacian quantifies whether these are genuinely inconsistent or just different perspectives on the same situation.

**Current code**: No sheaf-related implementations in the codebase. `crates/roko-core/src/prediction.rs` has Oracle trait and Prediction struct. `crates/roko-primitives/src/` has mathematical primitives.

**What to change**: Add `crates/roko-primitives/src/sheaf.rs`:
1. `CellularSheaf` struct with `stalks: HashMap<NodeId, Vec<f64>>`, `restriction_maps: HashMap<EdgeId, Matrix>`
2. `fn coboundary(sheaf: &CellularSheaf, section: &[Vec<f64>]) -> Vec<Vec<f64>>` — δ operator
3. `fn laplacian(sheaf: &CellularSheaf) -> SparseMatrix` — L_F = δ^T δ
4. `fn inconsistency_score(sheaf: &CellularSheaf, predictions: &HashMap<NodeId, Vec<f64>>) -> f64` — λ_min(L_F) for given predictions
5. `fn most_inconsistent(sheaf: &CellularSheaf, predictions: &HashMap<NodeId, Vec<f64>>) -> NodeId` — eigenvector analysis

**Reference files**:
- `crates/roko-primitives/src/` — mathematical primitives crate (implementation target)
- `crates/roko-core/src/prediction.rs` — Oracle trait, Prediction struct (oracle predictions to check)
- `docs/20-technical-analysis/14-sheaf-tropical-geometry.md` — full spec: cellular sheaves, Laplacian, cohomology, sheaf neural networks

**Accept when**:
- [x] `CellularSheaf` data structure with stalks and restriction maps
- [x] Sheaf Laplacian L_F computable from coboundary operator
- [x] `inconsistency_score()` returns λ_min(L_F)
- [x] `most_inconsistent()` identifies the oracle causing disagreement
- [x] `cargo test -p roko-primitives`

**Verify**:
```bash
grep -rn 'CellularSheaf\|sheaf_laplacian\|inconsistency_score\|coboundary' crates/roko-primitives/src/ --include='*.rs'
cargo test -p roko-primitives
```

**Priority**: P2 (Phase 2+, research)

---

### TA-14: Tropical decision geometry
- [x] Implement tropical semiring and tropical attention

**Spec** (doc 14 `docs/20-technical-analysis/14-sheaf-tropical-geometry.md` §Tropical): Tropical algebra replaces standard arithmetic with (max, +) operations:

**Tropical semiring** (max-plus algebra):
- Addition: `a ⊕ b = max(a, b)` (take the maximum)
- Multiplication: `a ⊗ b = a + b` (standard addition)
- Zero element: `-∞` (additive identity)
- One element: `0` (multiplicative identity)

This turns piecewise-linear functions (like ReLU neural networks, decision trees, attention mechanisms) into polynomial operations in tropical algebra.

**Tropical polynomial**: `p(x) = ⊕_i (c_i ⊗ x^{⊗ a_i}) = max_i(c_i + a_i · x)`. This is just a max over affine functions — exactly how neural network layers work. Oracle decisions become tropical polynomials: the agent selects the action with maximum utility, which is a tropical max.

**Tropical attention** (Zhang et al. 2018): `Attention(Q,K,V) = softmax(QK^T/√d)V` is approximated in the tropical limit as `max_{j}(Q_i · K_j + V_j)`. This provides:
- Symbolic-neural fusion: attention patterns become piecewise-linear and interpretable
- Exact adversarial distances: the tropical polytope boundary gives the exact perturbation needed to change a decision (Alfarra et al. 2024)

**Tropical VCG**: VCG auction mechanism computed in tropical algebra. Bid evaluation becomes `max_i(bid_i + utility_i)` — piecewise-linear and exactly solvable.

**Current code**: No tropical algebra in the codebase. `crates/roko-compose/src/auction.rs:32` has standard VCG auction. `crates/roko-primitives/src/` has mathematical primitives.

**What to change**: Add `crates/roko-primitives/src/tropical.rs`:
1. `TropicalF64` newtype wrapping `f64` with `impl Add` → max, `impl Mul` → add:
   ```rust
   #[derive(Copy, Clone, Debug, PartialEq)]
   pub struct TropicalF64(pub f64);
   impl std::ops::Add for TropicalF64 {
       type Output = Self;
       fn add(self, rhs: Self) -> Self { Self(self.0.max(rhs.0)) }
   }
   impl std::ops::Mul for TropicalF64 {
       type Output = Self;
       fn mul(self, rhs: Self) -> Self { Self(self.0 + rhs.0) }
   }
   ```
2. `TropicalPolynomial` struct: `Vec<(TropicalF64, Vec<i32>)>` (coefficient, exponent vector)
3. `fn evaluate(poly: &TropicalPolynomial, point: &[TropicalF64]) -> TropicalF64` — tropical polynomial evaluation
4. `fn tropical_attention(q: &[f64], keys: &[Vec<f64>], values: &[f64]) -> f64` — max_j(Q·K_j + V_j)

**Reference files**:
- `crates/roko-primitives/src/` — mathematical primitives crate (implementation target)
- `crates/roko-compose/src/auction.rs:32` — VCG auction (extend with tropical variant)
- `docs/20-technical-analysis/14-sheaf-tropical-geometry.md` — full spec: tropical semiring, polynomials, attention, convexity, robustness

**Accept when**:
- [x] `TropicalF64` newtype with max-plus arithmetic
- [x] `TropicalPolynomial` evaluation works correctly
- [x] `tropical_attention()` computes attention via max-plus
- [x] `cargo test -p roko-primitives`

**Verify**:
```bash
grep -rn 'TropicalF64\|TropicalPolynomial\|tropical_attention\|max_plus' crates/roko-primitives/src/ --include='*.rs'
cargo test -p roko-primitives
```

**Priority**: P2 (Phase 2+, research)

---

### TA-15: Oracle-trait integration wiring — remaining feedback loops
- [x] Wire Router.feedback() and gate threshold EMA from prediction residuals

**Spec** (doc 01 §Integration with the Synapse traits): The Oracle trait integrates with all
six Synapse traits through defined injection points. Five integration items:

1. **PredictiveScorer** (`impl Scorer`) -- **DONE**. `PredictiveScorer` at `crates/roko-core/src/prediction.rs:992` implements `Scorer` (line 1074). Wraps `Arc<CalibrationTracker>`. `with_pragmatic_weight()` configurable per role. Wired in `orchestrate.rs:12956`.

2. **PredictionPolicy** (`impl Policy`) -- **DONE**. `PredictionPolicy` at `prediction.rs:1115` implements `Policy` (line 1156). Wraps `Arc<CalibrationTracker>`. `with_min_samples(6)` configurable. Wired in `orchestrate.rs:344`.

3. **Router feedback**: After `oracle.evaluate()`, call `router.feedback(&model_id,
   accuracy.accuracy)` to update bandit arms. This uses the same LinUCB + Thompson Sampling
   mechanism from CascadeRouter. **NOT YET WIRED** — CascadeRouter has `feedback()` method but
   it is not called after prediction resolution in the orchestration loop.

4. **Gate residual calibration**: Prediction residuals feed into `gate_thresholds.update_ema(
   category, residual.abs(), alpha: 0.1)` creating automatic gate tightening when oracles
   systematically overestimate. **NOT YET WIRED** — gate thresholds update from gate pass/fail
   but not from prediction residuals.

5. **EFE bidding**: Oracle predictions bid for attention budget via `composer.bid(
   "oracle_predictions", efe * urgency * affect_weight, prediction_context)`. **Phase 2+** — depends on VCG auction wiring (BEAT-07).

**Current code**:
- `PredictiveScorer` at `crates/roko-core/src/prediction.rs:992` — **exists**, implements `Scorer` at line 1074, wired in `orchestrate.rs:12956`
- `PredictionPolicy` at `prediction.rs:1115` — **exists**, implements `Policy` at line 1156, wired in `orchestrate.rs:344`
- `CalibrationTracker` at `prediction.rs:821` — provides `get_accuracy()`, `mean_residual()`, `accuracy_trend()`, consumed by both `PredictiveScorer` and `PredictionPolicy`
- `CascadeRouter::feedback()` at `crates/roko-learn/src/cascade_router.rs` — method exists but not called after prediction resolution
- `gate_thresholds.update_ema()` at `crates/roko-learn/src/gate_thresholds.rs` — method exists but not called from prediction residuals

**What to change**:
1. In `crates/roko-cli/src/orchestrate.rs`, after prediction evaluation calls, invoke `cascade_router.feedback(&model_id, accuracy)` to feed prediction accuracy back into model routing
2. After gate evaluation in the orchestrate loop, call `gate_thresholds.update_ema(category, residual.abs(), 0.1)` when prediction residuals are available
3. Emit prediction resolution events for observability

**Reference files**:
- `crates/roko-core/src/prediction.rs:992` — `PredictiveScorer` (done, wired at orchestrate.rs:12956)
- `crates/roko-core/src/prediction.rs:1115` — `PredictionPolicy` (done, wired at orchestrate.rs:344)
- `crates/roko-core/src/prediction.rs:821` — `CalibrationTracker` (consumed by both)
- `crates/roko-learn/src/cascade_router.rs` — `CascadeRouter.feedback()` (call after prediction resolution)
- `crates/roko-learn/src/gate_thresholds.rs` — adaptive gate threshold EMA (call with residuals)
- `docs/20-technical-analysis/01-oracle-trait.md` §Integration — full integration spec

**Depends on**: TA-12 (prediction lifecycle must be wired first)

**Accept when**:
- [x] `PredictiveScorer` implements `Scorer` and modulates confidence by oracle accuracy (exists at prediction.rs:992)
- [x] `PredictionPolicy` implements `Policy` with bias detection and trend detection (exists at prediction.rs:1115)
- [x] `CascadeRouter.feedback()` called after prediction resolution in orchestrate.rs
- [x] Gate threshold EMA updated from prediction residuals (via `AdaptiveThresholds::observe_residual()`)
- [x] `cargo test -p roko-core` passes
- [ ] `cargo clippy --workspace --no-deps` clean (pre-existing warnings)

**Verify**:
```bash
grep -rn 'PredictiveScorer\|PredictionPolicy' crates/roko-core/src/prediction.rs | head -5
grep -rn 'PredictiveScorer\|PredictionPolicy' crates/roko-cli/src/orchestrate.rs | head -5
grep -rn 'router\.feedback\|cascade_router.*feedback' crates/roko-cli/ --include='*.rs'
cargo test -p roko-core
```

**Priority**: P1 (wiring remaining feedback loops)

---

## Verify

```bash
cargo test -p roko-core
cargo test -p roko-primitives
cargo test -p roko-learn
cargo test --workspace
```
