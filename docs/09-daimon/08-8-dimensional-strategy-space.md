# 8-Dimensional Strategy Space

> The domain-configurable coordinate system that locates every strategy attempt in an 8D space, enabling k-d tree somatic lookup and pattern matching across agent histories.


> **Implementation**: Built

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md)
**Key sources**: `refactoring-prd/09-innovations.md` §XIX.F, `refactoring-prd/03-cognitive-subsystems.md` §2

---

## Abstract

The somatic landscape (see [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md)) is a k-d tree over an 8-dimensional strategy space. But what are the 8 dimensions? They are **domain-configurable axes** that characterize a strategy attempt in terms meaningful to the agent's domain. A coding agent's dimensions (Complexity, Risk, Novelty, Confidence, Time Pressure, Scope, Reversibility, Dependency Depth) differ from a chain agent's dimensions (Volatility, Liquidity, Correlation, Leverage, Time Horizon, Concentration, Counterparty Risk, Regulatory Exposure).

The 8D strategy space serves three purposes:
1. **Somatic marker storage**: Every marker in the k-d tree has coordinates in this space, enabling nearest-neighbor queries that find emotionally similar past strategies.
2. **Strategy classification**: The coordinates provide a compact feature vector for classifying strategy types, enabling the agent to recognize when it's in familiar vs. unfamiliar territory.
3. **Cross-agent transfer**: Agents with the same domain configuration can share somatic markers because their coordinates are commensurable.

---

## Why 8 Dimensions?

### Dimensionality Trade-offs

The choice of 8 dimensions balances three constraints:

1. **Expressiveness**: Too few dimensions conflate distinct strategy types. With 3 dimensions, "high-complexity, low-risk, novel" and "high-complexity, low-risk, familiar" would be indistinguishable — but they require very different approaches.

2. **Computational efficiency**: k-d tree performance degrades with dimensionality. For D dimensions and N points, nearest-neighbor search is efficient when N >> 2^D. At D=8, this requires N >> 256 markers for efficient search — easily achievable. At D=16, N >> 65,536 would be needed, which is impractical for early-stage agents.

3. **Human interpretability**: An operator examining a somatic marker at coordinates [0.8, 0.3, 0.9, 0.2, 0.7, 0.4, 0.1, 0.6] should be able to understand what kind of strategy it represents. With 8 named dimensions, each coordinate is meaningful. With 32 dimensions, interpretation becomes impractical.

### Alternative Dimensionalities Considered

| Dimensions | Pros | Cons | Decision |
|---|---|---|---|
| 3 (P-A-D only) | Fast, simple | Can't distinguish strategies with same emotional profile | Rejected — insufficient |
| 5 (Big Five analog) | Psychology-backed | Not enough for domain-specific strategy features | Rejected — too few |
| **8** | **Good k-d tree performance, interpretable, domain-specific** | **Requires domain configuration** | **Selected** |
| 16 | Very expressive | k-d tree degradation, hard to populate | Rejected — too many |
| Arbitrary (HDC) | Maximum flexibility | No spatial structure for k-d tree | Used elsewhere (Neuro), not here |

---

## Coding Agent Dimensions

For agents working on software engineering tasks, the 8 dimensions are:

| # | Dimension | Range | Low (0.0) | High (1.0) | Measurement Source |
|---|---|---|---|---|---|
| 1 | **Complexity** | [0, 1] | Simple change (rename, formatting) | Multi-file refactor, architecture change | Cyclomatic complexity of affected code, file count |
| 2 | **Risk** | [0, 1] | Test-covered, well-understood code | Untested, critical path, safety-sensitive | Test coverage, gate rung level, dependency criticality |
| 3 | **Novelty** | [0, 1] | Familiar crate/module, repeat task | New crate, new API, first encounter | Playbook match confidence, Neuro similarity score |
| 4 | **Confidence** | [0, 1] | Low Daimon confidence, unfamiliar territory | High confidence, proven approach | Daimon `confidence` field, somatic landscape valence |
| 5 | **Time Pressure** | [0, 1] | No deadline, background task | Imminent deadline, blocking other tasks | Deadline proximity, blocker count, queue wait time |
| 6 | **Scope** | [0, 1] | Single function, localized change | System-wide change, public API modification | Lines changed estimate, file count, symbol graph extent |
| 7 | **Reversibility** | [0, 1] | Easily reverted (new file, additive) | Hard to revert (schema migration, data loss) | Git diff analysis, database schema changes, file deletions |
| 8 | **Dependency Depth** | [0, 1] | Leaf module, no dependents | Core library, many reverse deps | Dependency graph depth, reverse dependency count |

### Example Strategy Coordinates (Coding)

| Task | Complexity | Risk | Novelty | Confidence | Time | Scope | Reversibility | Deps |
|---|---|---|---|---|---|---|---|---|
| Fix typo in comment | 0.05 | 0.01 | 0.02 | 0.95 | 0.10 | 0.02 | 0.99 | 0.01 |
| Add unit test | 0.15 | 0.10 | 0.20 | 0.80 | 0.20 | 0.10 | 0.95 | 0.05 |
| Wire existing module | 0.40 | 0.30 | 0.40 | 0.60 | 0.50 | 0.35 | 0.70 | 0.40 |
| Refactor error handling | 0.70 | 0.60 | 0.30 | 0.50 | 0.30 | 0.65 | 0.40 | 0.70 |
| New crate from scratch | 0.80 | 0.40 | 0.90 | 0.35 | 0.40 | 0.80 | 0.80 | 0.20 |
| Migrate database schema | 0.85 | 0.90 | 0.50 | 0.30 | 0.70 | 0.90 | 0.05 | 0.85 |

The Euclidean distance between "Fix typo" and "Migrate database schema" is approximately 2.1 — they are far apart in strategy space. A somatic marker from a database migration experience would not fire for a typo fix. But "Wire existing module" and "Refactor error handling" are closer (distance ~0.8), so somatic markers can transfer between them.

---

## Chain Agent Dimensions

For agents working in DeFi/blockchain domains, the 8 dimensions are:

| # | Dimension | Range | Low (0.0) | High (1.0) | Measurement Source |
|---|---|---|---|---|---|
| 1 | **Volatility** | [0, 1] | Stable market, low price movement | High volatility, rapid price changes | Price delta, historical volatility |
| 2 | **Liquidity** | [0, 1] | Deep pools, easy execution | Thin markets, high slippage risk | Pool depth, bid-ask spread |
| 3 | **Correlation** | [0, 1] | Assets move independently | High correlation across portfolio | Cross-asset correlation matrix |
| 4 | **Leverage** | [0, 1] | No leverage, spot only | High leverage, liquidation risk | Collateral ratio, margin utilization |
| 5 | **Time Horizon** | [0, 1] | Short-term (< 1 hour) | Long-term (> 1 week) | Position duration, strategy type |
| 6 | **Concentration** | [0, 1] | Diversified across assets | Concentrated in single asset/pool | Portfolio Herfindahl index |
| 7 | **Counterparty Risk** | [0, 1] | Trustless protocol | Bridge, CEX dependency | Smart contract audit status, bridge exposure |
| 8 | **Regulatory Exposure** | [0, 1] | Clearly unregulated | Potentially regulated asset/activity | Jurisdiction analysis, token classification |

---

## Dimension Computation

### Automatic vs. Manual Coordinates

Most dimensions can be computed automatically from observable state:

```rust
pub trait StrategySpaceComputer {
    /// Compute the 8D strategy coordinates for a proposed action.
    fn compute_coords(&self, action: &Action, context: &Context) -> [f64; 8];
}

/// Coding domain implementation.
pub struct CodingStrategySpace;

impl StrategySpaceComputer for CodingStrategySpace {
    fn compute_coords(&self, action: &Action, context: &Context) -> [f64; 8] {
        [
            self.compute_complexity(action, context),
            self.compute_risk(action, context),
            self.compute_novelty(action, context),
            self.compute_confidence(context),
            self.compute_time_pressure(context),
            self.compute_scope(action, context),
            self.compute_reversibility(action, context),
            self.compute_dependency_depth(action, context),
        ]
    }
}
```

**Complexity** is computed from the task description and affected files — cyclomatic complexity of touched functions, number of files in the change set, depth of the call graph being modified.

**Risk** combines test coverage of affected code, gate rung level (higher rungs = stricter checks = higher stakes), and whether the code is on a critical path (determined by reverse dependency analysis).

**Novelty** is the inverse of the Neuro's similarity score — how far is this task from the nearest knowledge entry? A task that closely matches a proven playbook has low novelty. A task in an unfamiliar crate has high novelty.

**Confidence** is read directly from the Daimon's `AffectState.confidence` field, which aggregates gate outcomes and task results.

**Time Pressure** is computed from deadline proximity (if set), blocker count (tasks blocking on this one), and queue wait time.

**Scope** estimates the extent of the change — single function, multiple functions, whole file, multiple files, multiple crates.

**Reversibility** analyzes the diff characteristics — additive changes (new files, new functions) are highly reversible; destructive changes (file deletions, schema migrations) are not.

**Dependency Depth** queries the dependency graph to determine how many other components depend on the affected code.

### Domain Registration

New domains register their dimension definitions at configuration time:

```toml
# roko.toml
[daimon.strategy_space]
domain = "coding"
dimensions = [
    { name = "complexity", source = "task_analysis" },
    { name = "risk", source = "coverage_and_gates" },
    { name = "novelty", source = "neuro_similarity" },
    { name = "confidence", source = "daimon_state" },
    { name = "time_pressure", source = "scheduler" },
    { name = "scope", source = "diff_analysis" },
    { name = "reversibility", source = "diff_analysis" },
    { name = "dependency_depth", source = "dep_graph" },
]
```

This makes the strategy space fully configurable per domain while maintaining the fixed 8-dimensional structure that the k-d tree and somatic landscape require.

---

## Cross-Domain Transfer

### Structural Analogy via Dimension Mapping

When agents from different domains share a mesh, their strategy spaces are incompatible at the dimension level (coding's "Complexity" ≠ chain's "Volatility"). But structural patterns can transfer through dimension mapping:

```
Coding:  high_complexity + high_risk + low_confidence → cautious approach
Chain:   high_volatility + high_leverage + low_confidence → cautious approach

Both map to: high_dimension_1 + high_dimension_2 + low_dimension_4
```

The mapping is coarse — it preserves the shape of the strategy profile (which dimensions are high/low) without preserving the specific meanings. This enables cross-domain somatic transfer: a coding agent's "cautious approach to complex risky tasks" marker can inform a chain agent's "cautious approach to volatile leveraged positions" through structural similarity.

This cross-domain transfer is facilitated by HDC encoding (see topic [04-knowledge](../06-neuro/INDEX.md)), where the 8D coordinates are encoded as hyperdimensional vectors using bind and permute operations. Structural similarity in the 8D space produces Hamming similarity in the HDC space.

---

## Academic Foundations

- Damasio, A.R. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain*. Putnam.
- Bechara, A., Damasio, H., Tranel, D., & Damasio, A.R. (1997). "Deciding advantageously before knowing the advantageous strategy." *Science*, 275(5304), 1293–1295.
- Kleyko, D. et al. (2022). "Vector Symbolic Architectures as a Computing Framework for Emerging Hardware." *ACM Computing Surveys*, 55(13s), 1–55.

---

## Automatic Extraction Algorithms

Each dimension requires a concrete extraction algorithm that converts observable state into a [0, 1] value. Below are the algorithms for the coding domain.

### Dimension 1: Complexity

Complexity captures how structurally difficult the change is. Two signals feed it: cyclomatic complexity of affected code and the change set size.

```rust
impl CodingStrategySpace {
    fn compute_complexity(&self, action: &Action, context: &Context) -> f64 {
        // Signal 1: Cyclomatic complexity of affected functions.
        // Requires code analysis — either static analysis via roko-index
        // or a heuristic based on the task description.
        let cc = if let Some(index) = context.code_index() {
            // If the code index is available, compute exact cyclomatic complexity
            let affected_fns = index.functions_in_files(&action.affected_files);
            let avg_cc = affected_fns.iter()
                .map(|f| f.cyclomatic_complexity as f64)
                .sum::<f64>() / affected_fns.len().max(1) as f64;
            avg_cc
        } else {
            // Heuristic fallback: estimate from task description keywords
            estimate_complexity_from_description(&action.description)
        };

        // Signal 2: Change set size (number of files)
        let file_count = action.affected_files.len() as f64;

        // Combine: normalize each signal to [0, 1] and average
        let cc_normalized = sigmoid_normalize(cc, 5.0, 2.0);   // midpoint=5, steepness=2
        let files_normalized = sigmoid_normalize(file_count, 5.0, 1.5);

        0.6 * cc_normalized + 0.4 * files_normalized
    }
}

/// Sigmoid normalization: maps [0, inf) to [0, 1] with a configurable midpoint.
/// At x = midpoint, output = 0.5.
fn sigmoid_normalize(x: f64, midpoint: f64, steepness: f64) -> f64 {
    1.0 / (1.0 + (-steepness * (x - midpoint)).exp())
}
```

**Does complexity require code analysis?** Not strictly. The code index (`roko-index`) provides exact cyclomatic complexity for Rust files by parsing AST nodes. When the index isn't available (the agent is working on a new crate, or the index hasn't been built yet), the fallback estimates complexity from the task description using keyword heuristics:

| Keyword pattern | Estimated CC |
|---|---|
| "rename", "typo", "comment" | 1-2 |
| "add test", "add field" | 3-5 |
| "refactor", "wire module" | 6-10 |
| "rewrite", "migrate", "new crate" | 10-20 |

The heuristic is coarse but sufficient for somatic marker placement. Exact CC matters for the code index; the strategy space only needs approximate positioning.

### Dimension 2: Risk

Risk combines test coverage, gate strictness, and dependency criticality.

```rust
fn compute_risk(&self, action: &Action, context: &Context) -> f64 {
    // Signal 1: Test coverage of affected code (inverted — less coverage = more risk)
    let coverage = context.test_coverage_for_files(&action.affected_files)
        .unwrap_or(0.0);  // no data → assume uncovered
    let coverage_risk = 1.0 - coverage;  // 0% coverage → risk 1.0

    // Signal 2: Gate rung level → [0, 1]
    // Gate rungs: 0 (advisory) through 5 (strict)
    let rung = context.gate_rung_for_task(&action.task_id) as f64;
    let rung_risk = rung / 5.0;  // rung 5 → 1.0, rung 0 → 0.0

    // Signal 3: Dependency criticality
    // How many other crates/modules depend on the affected code?
    let reverse_deps = context.reverse_dependency_count(&action.affected_files) as f64;
    let dep_risk = sigmoid_normalize(reverse_deps, 5.0, 1.0);

    // Weighted combination — coverage matters most
    0.50 * coverage_risk + 0.25 * rung_risk + 0.25 * dep_risk
}
```

**Gate rung quantization**: The 6-rung gate pipeline (rungs 0-5) maps linearly to [0, 1]. Rung 0 (advisory, no enforcement) produces risk 0.0. Rung 5 (strict, all gates must pass) produces risk 1.0. This captures the idea that tasks assigned to higher rungs are higher stakes.

### Dimension 3: Novelty

Novelty is the inverse of similarity to known patterns.

```rust
fn compute_novelty(&self, action: &Action, context: &Context) -> f64 {
    // Query the NeuroStore for the nearest playbook match
    let embedding = context.embed_task_description(&action.description);
    let nearest = context.neuro_store().nearest_playbook(&embedding);

    match nearest {
        Some(match_result) => {
            // Similarity in [0, 1]. High similarity = low novelty.
            1.0 - match_result.similarity
        }
        None => {
            // No playbooks exist → maximum novelty
            1.0
        }
    }
}
```

**Similarity baseline**: The query searches the entire NeuroStore, not just recent episodes. This means novelty is relative to the agent's complete history. A task that matches a playbook from 1,000 episodes ago has low novelty even if the agent hasn't encountered anything similar recently. This is intentional — somatic markers from that old playbook should still fire.

For agents in their first 50 episodes, novelty will be high for most tasks. This pushes the strategy coordinates into the high-novelty region of the landscape, where there are few somatic markers and the agent relies on analytical reasoning. As the NeuroStore fills, novelty drops for familiar task types.

### Dimension 7: Reversibility

Reversibility analyzes the diff to determine how easily a change can be undone.

```rust
fn compute_reversibility(&self, action: &Action, context: &Context) -> f64 {
    let diff = context.estimated_diff(action);

    let mut reversibility = 1.0;  // start optimistic

    // File deletions are hard to reverse
    reversibility -= 0.3 * (diff.deleted_files.len() as f64).min(1.0);

    // Schema migrations are hard to reverse
    if diff.touches_migration_files {
        reversibility -= 0.4;
    }

    // Modifying public API signatures reduces reversibility
    let pub_api_changes = diff.public_api_changes.len() as f64;
    reversibility -= 0.1 * pub_api_changes.min(3.0);

    // Additive-only changes (new files, new functions) are highly reversible
    if diff.is_purely_additive() {
        reversibility = reversibility.max(0.85);
    }

    reversibility.clamp(0.0, 1.0)
}
```

**Timing**: Reversibility is computed from the *estimated* diff, not the actual diff. The estimate comes from the task description and affected file list. The actual diff doesn't exist yet when the strategy coordinates are computed (the agent hasn't started the task). After the task completes, the actual reversibility could be computed from the real diff and used to update the somatic marker — this is a future refinement.

**Location**: The `estimated_diff` method lives in the `CodingStrategySpace` implementation. It reads the file list from the action, checks the code index for public API signatures in those files, and applies heuristic rules. It does not require running `git diff`.

---

## Domain Registration and Cross-Domain Transfer

### Registering a New Domain

Domain registration happens through `roko.toml`. When a new domain is registered, it defines 8 dimension names and their extraction sources:

```rust
pub struct DomainRegistration {
    pub name: String,
    pub dimensions: [DimensionDef; 8],
}

pub struct DimensionDef {
    pub name: String,
    /// Which subsystem provides the raw signal.
    pub source: DimensionSource,
    /// Weight for distance calculations (default: 1.0).
    pub weight: f64,
}

pub enum DimensionSource {
    /// Read from code analysis (roko-index).
    TaskAnalysis,
    /// Read from test coverage + gate config.
    CoverageAndGates,
    /// Read from NeuroStore similarity.
    NeuroSimilarity,
    /// Read from Daimon affect state.
    DaimonState,
    /// Read from task scheduler metadata.
    Scheduler,
    /// Read from estimated diff analysis.
    DiffAnalysis,
    /// Read from the dependency graph.
    DepGraph,
    /// Custom extraction function (name of a registered extractor).
    Custom(String),
}
```

**Custom extractors**: For domain-specific dimensions that don't map to any built-in source, the `Custom` variant references a named extraction function registered at startup. For example, a chain agent could register `"volatility_oracle"` that queries an external price feed.

### Cross-Domain Transfer: Who Implements, Algorithm

Cross-domain transfer is implemented by the `StrategyTransferMapper` in `roko-daimon`:

```rust
pub struct StrategyTransferMapper {
    /// Maps source domain dimension indices to target domain dimension indices.
    /// Index correspondence is by structural role, not by name.
    dimension_map: [(usize, usize); 8],
}

impl StrategyTransferMapper {
    /// Build a mapping between two domains based on structural analogy.
    ///
    /// The algorithm:
    /// 1. Classify each dimension by its behavioral role:
    ///    - "difficulty" (maps to Complexity, Volatility)
    ///    - "danger" (maps to Risk, Leverage)
    ///    - "familiarity" (maps to Novelty, Correlation)
    ///    - "self_assessment" (maps to Confidence)
    ///    - "urgency" (maps to Time Pressure, Time Horizon)
    ///    - "breadth" (maps to Scope, Concentration)
    ///    - "recoverability" (maps to Reversibility, Counterparty Risk)
    ///    - "coupling" (maps to Dependency Depth, Regulatory Exposure)
    ///
    /// 2. Match dimensions across domains by shared role.
    /// 3. For unmatched dimensions, fall back to positional mapping.
    pub fn from_domains(source: &DomainRegistration, target: &DomainRegistration) -> Self {
        let mut dimension_map = [(0usize, 0usize); 8];
        for (i, src_dim) in source.dimensions.iter().enumerate() {
            let role = classify_role(&src_dim.name);
            let target_idx = target.dimensions.iter()
                .position(|d| classify_role(&d.name) == role)
                .unwrap_or(i);  // fallback: same position
            dimension_map[i] = (i, target_idx);
        }
        Self { dimension_map }
    }

    /// Transfer a somatic marker's coordinates from source domain to target domain.
    pub fn transfer(&self, source_coords: &[f64; 8]) -> [f64; 8] {
        let mut target_coords = [0.5; 8];  // default: midpoint
        for &(src_idx, tgt_idx) in &self.dimension_map {
            target_coords[tgt_idx] = source_coords[src_idx];
        }
        target_coords
    }
}
```

The role classification (`classify_role`) maps dimension names to abstract behavioral roles. This is a heuristic, not a precise semantic mapping. The roles are coarse on purpose — `"danger"` covers both code risk and financial leverage because the behavioral response is similar (increase caution, escalate model tier).

### Dimension Weighting

Each dimension can carry a different weight for distance calculations. The weights determine how much each dimension contributes to the nearest-neighbor search in the somatic landscape.

```rust
pub struct DimensionWeights {
    /// Per-dimension weights. Default: [1.0; 8] (equal weighting).
    pub weights: [f64; 8],
}

impl DimensionWeights {
    /// Apply weights to strategy coordinates before k-d tree insertion.
    /// Weighted coordinates are used for distance computation;
    /// original coordinates are stored in the marker for display/analysis.
    pub fn apply(&self, coords: &[f64; 8]) -> [f64; 8] {
        let mut weighted = [0.0; 8];
        for i in 0..8 {
            weighted[i] = coords[i] * self.weights[i].sqrt();
        }
        weighted
    }
}
```

**Combination method**: Weighted squared Euclidean distance. The `sqrt` on the weight is because the distance function squares the coordinate difference. Applying `sqrt(w)` to each coordinate before squaring produces `w * (a_i - b_i)^2` in the final distance, which is a weighted sum.

**Default weights**: All 1.0 (equal). Domain-specific overrides can be set in `roko.toml`:

```toml
[daimon.strategy_space]
domain = "coding"
dimension_weights = [1.0, 1.5, 1.2, 1.0, 0.8, 1.0, 1.3, 0.7]
# Risk (1.5) and Reversibility (1.3) weighted higher — these dimensions
# produce the strongest somatic responses in coding tasks.
# Time Pressure (0.8) and Dependency Depth (0.7) weighted lower —
# these are contextual factors that vary more between tasks.
```

**Why weighted sum, not max?** The max operator would make a single extreme dimension dominate the distance, ignoring all other dimensions. A task that is high-risk but low-complexity would look identical to a task that is high-risk and high-complexity. The weighted sum preserves the contribution of every dimension while allowing domain operators to tune relative importance.

---

## Resource Pressure Scalar

When the agent is running low on budget (token budget, time budget, or compute budget), the strategy space coordinates are compressed toward the conservative region. The resource pressure scalar modulates all 8 dimensions:

```rust
pub struct ResourcePressure {
    /// Token budget remaining as fraction of total. Range: [0, 1].
    pub token_budget_remaining: f64,
    /// Time budget remaining as fraction of total. Range: [0, 1].
    pub time_budget_remaining: f64,
}

impl ResourcePressure {
    /// Compute the scalar that compresses strategy coordinates under pressure.
    ///
    /// Formula: scalar = min(token_remaining, time_remaining)^0.5
    ///
    /// At 100% budget: scalar = 1.0 (no compression)
    /// At 25% budget:  scalar = 0.5 (moderate compression)
    /// At 6.25% budget: scalar = 0.25 (strong compression)
    /// At 0% budget:   scalar = 0.0 (maximum compression — conservative region)
    pub fn scalar(&self) -> f64 {
        let min_remaining = self.token_budget_remaining
            .min(self.time_budget_remaining);
        min_remaining.sqrt().clamp(0.0, 1.0)
    }

    /// Apply resource pressure to strategy coordinates.
    /// Compresses coordinates toward [0.5; 8] (the neutral midpoint),
    /// then biases toward conservative region.
    pub fn apply(&self, coords: &[f64; 8]) -> [f64; 8] {
        let s = self.scalar();
        let mut compressed = [0.0; 8];
        for i in 0..8 {
            // Interpolate toward midpoint (0.5) as pressure increases
            compressed[i] = s * coords[i] + (1.0 - s) * 0.5;
        }
        compressed
    }
}
```

**Effect on somatic lookup**: Under resource pressure, all coordinates compress toward the center of the strategy space. This causes the k-d tree query to hit markers in the "moderate" region — markers from cautious, well-understood tasks. The agent reverts to proven approaches when budget is low, regardless of the task's actual characteristics.

**Integration point**: Resource pressure is applied after dimension computation and before k-d tree query:

```
compute_coords(action, context)        // raw 8D coordinates
    → DimensionWeights::apply()        // weighted coordinates
    → ResourcePressure::apply()        // compressed if under budget pressure
    → SomaticLandscape::query()        // k-d tree lookup
```

### Error Handling

| Error | Cause | Response |
|---|---|---|
| Code index unavailable | `roko-index` not built for workspace | Fall back to keyword heuristic for Complexity |
| Test coverage data missing | No coverage report | Assume 0% coverage (maximum risk) |
| NeuroStore empty | New agent, no playbooks | Novelty = 1.0 for all tasks |
| Dependency graph unavailable | Workspace not fully resolved | Dependency Depth defaults to 0.5 (midpoint) |
| Budget data missing | No budget configured | Resource pressure scalar = 1.0 (no compression) |
| NaN in extracted dimension | Buggy extractor | Clamp to 0.5 (midpoint), log warning |

### Test Criteria

| Test | Condition | Expected |
|---|---|---|
| Sigmoid normalize at midpoint | `sigmoid_normalize(5.0, 5.0, 2.0)` | Returns 0.5 |
| Complexity from code index | Function with CC=10, 3 files | Complexity > 0.6 |
| Complexity fallback | No code index, task says "refactor" | Complexity between 0.4-0.7 |
| Risk from zero coverage | 0% test coverage, rung 3, 2 reverse deps | Risk > 0.6 |
| Gate rung quantization | Rung 0 → 0.0, rung 5 → 1.0 | Linear mapping |
| Novelty for new agent | Empty NeuroStore | Novelty = 1.0 |
| Novelty for repeat task | Exact playbook match (similarity 0.95) | Novelty = 0.05 |
| Reversibility for additive change | New file only, no deletions | Reversibility >= 0.85 |
| Reversibility for schema migration | Touches migration file | Reversibility <= 0.6 |
| Resource pressure at 25% budget | token_budget_remaining = 0.25 | Scalar = 0.5, coordinates compressed |
| Resource pressure at 100% budget | Full budget | Scalar = 1.0, coordinates unchanged |
| Cross-domain transfer | Coding "Risk" → Chain "Leverage" via "danger" role | Coordinates mapped correctly |
| Dimension weights applied | Weight 1.5 on Risk dimension | Weighted coordinate = coord * sqrt(1.5) |
| NaN dimension clamped | Extractor returns NaN | Dimension set to 0.5 |

---

## Current Status and Gaps

**Specified**: Full dimension definitions for coding and chain domains in `refactoring-prd/09-innovations.md` §XIX.F. Domain registration via configuration. Cross-domain transfer via structural analogy.

**Implemented**: `roko-daimon` now owns both the persisted `StrategySpaceDefinition` and the coordinate projection layer. `roko.toml` parses `[daimon.strategy_space]`, `DaimonState` persists the selected definition, and a real `StrategySpaceComputer` abstraction now projects normalized task / episode observations into the 8D space. The built-in coding extractor is used for live orchestration and dream replay, while custom domains can already register labels and remain operational through the shared normalized projection path.

**Still missing**: Dedicated non-coding extractors such as a chain-native `StrategySpaceComputer`, richer cross-domain mapping logic, and tighter coupling between strategy-space computation and the cross-subsystem VCG market.

**Dependency**: The strategy space is now consumed by the implemented somatic landscape (see [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md)); the remaining work is on richer extractors and integration depth, not on basic availability.

---

## Cross-References

- See [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md) for k-d tree query over strategy space
- See [07-15-percent-contrarian-retrieval.md](./07-15-percent-contrarian-retrieval.md) for contrarian retrieval in strategy space
- See topic [04-knowledge](../06-neuro/INDEX.md) for HDC encoding of strategy coordinates
- See [11-coding-agent-integration.md](./11-coding-agent-integration.md) for coding-specific dimension computation
