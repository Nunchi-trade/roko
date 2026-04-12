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

## Current Status and Gaps

**Specified**: Full dimension definitions for coding and chain domains in `refactoring-prd/09-innovations.md` §XIX.F. Domain registration via configuration. Cross-domain transfer via structural analogy.

**Not implemented**: No `StrategySpaceComputer` trait or implementation exists in any crate. Dimension computation functions are not written. The `roko.toml` configuration section for strategy space is not parsed.

**Dependency**: The strategy space is required by the somatic landscape (see [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md)), which is itself not yet implemented.

---

## Cross-references

- See [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md) for k-d tree query over strategy space
- See [07-15-percent-contrarian-retrieval.md](./07-15-percent-contrarian-retrieval.md) for contrarian retrieval in strategy space
- See topic [04-knowledge](../06-neuro/INDEX.md) for HDC encoding of strategy coordinates
- See [11-coding-agent-integration.md](./11-coding-agent-integration.md) for coding-specific dimension computation
