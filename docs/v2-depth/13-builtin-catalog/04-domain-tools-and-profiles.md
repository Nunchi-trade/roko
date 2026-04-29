# 04 — Domain Tools and Profiles

> Domain-specific tool ecosystems as the Rack pattern (Graph + Macros + Slots). The chain
> domain's two-layer model is a Route Cell (adapter selection) composed with Connect Cells
> (protocol execution). Profiles are Rack macros that parameterize which Cells exist.

**Parent spec**: [14-TOOLS.md](../../unified/14-TOOLS.md), [03-GRAPH.md](../../unified/03-GRAPH.md)

---

## 1. Core Insight

Roko's kernel is domain-agnostic. The 16 built-in tools handle universal operations (files,
search, shell, web). Domain-specific capabilities (blockchain protocols, research APIs, deployment
services) are loaded as **domain plugins** — separate crates that register additional Connect
Cells.

The architectural pattern is the **Rack** (Graph + Macros + Slots):
- The Graph defines the domain's tool topology (which tools exist, how they compose)
- **Macros** (profile knobs) parameterize the Graph at boot time (which tools are active)
- **Slots** (jacks) are the connection points where the domain tools plug into the universal
  cognitive loop

The chain domain — 423+ DeFi tools covering 10+ protocols — is the reference implementation of
this pattern. It demonstrates the two-layer model: 8 adapter-facing tools visible to the LLM,
backed by a resolution layer that dispatches to protocol-specific handlers. This is a **Route
Cell** (select adapter) composed with a **Connect Cell** (execute protocol operation).

---

## 2. Two-Layer Tool Model

Exposing all 423+ chain tools directly to the LLM would consume ~38,000 tokens per turn just
for tool definitions. The two-layer model reduces this to ~1,200 tokens — a **94% reduction**.

### Layer 1: Adapter-Facing Tools (LLM sees these)

| Tool | Direction | Purpose |
|---|---|---|
| `preview_action` | Write (simulation) | Simulate an action before execution |
| `commit_action` | Write (execution) | Execute a previously previewed action |
| `cancel_action` | Write | Cancel a pending preview |
| `emergency_halt` | Write (privileged) | Halt all operations immediately |
| `query_state` | Read | Query portfolio, positions, balances |
| `search_context` | Read | Search market data, prices, pool state |
| `query_neuro` | Read | Query the knowledge Store for relevant memory |
| `update_directive` | Write | Update agent's operational directive |

### Layer 2: Protocol-Specific Handlers (Resolution layer)

When the LLM calls `preview_action` with `venue: "morpho"` and `action_type: "deposit"`,
the adapter resolution layer:

1. **Route Cell** — selects the Morpho protocol adapter based on venue + action_type
2. **Connect Cell** — executes the protocol-specific handler (Morpho deposit simulation)

```rust
/// Adapter resolution: Route Cell selecting the Connect Cell.
pub struct AdapterResolution {
    /// The internal Connect Cell to invoke.
    pub internal_tool: &'static ToolDef,
    /// Transformed parameters (adapter schema → internal schema).
    pub transformed_params: Value,
    /// Risk tier for safety Pipeline routing.
    pub risk_tier: RiskTier,
}
```

### Token Savings Breakdown

| Configuration | Tokens/turn | Savings |
|---|---|---|
| All 423+ tools directly exposed (baseline) | ~38,000 | — |
| 8 adapter-facing tools (two-layer model) | ~1,200 | **94%** |
| 8 adapter + 5 dormant skill descriptions | ~1,450 | **92%** |
| 8 adapter + 2 active skill loads | ~2,800 | **85%** |

Over an agent's lifetime (20 T1 turns/day at $0.001/1K tokens): ~$11.40/month saved per agent.
More importantly: smaller tool context means faster inference and better tool selection accuracy.

---

## 3. Profiles as Rack Macros

A profile is a **named parameterization** of the domain Rack. It determines which Cells are
present at runtime — not which are blocked by policy, but which **exist at all**.

### Chain Domain Profiles (13)

| Profile | Read Cells | Write Cells | Structural Property |
|---|---|---|---|
| `active` | All ~250 | All ~150 | Standard trading agent |
| `observatory` | All ~250 | **None** | Write adapters absent — cannot trade |
| `conservative` | All ~250 | ~40 | No leverage, no flashloans |
| `data` | ~40 | **None** | Analytics only |
| `trader` | ~60 | ~20 | Swap execution |
| `lp` | ~65 | ~25 | Liquidity provision |
| `vault` | ~75 | ~35 | ERC-4626 vault operations |
| `intelligence` | ~58 | **None** | MEV scoring, IL calculation |
| `learning` | ~52 | ~12 | Memory management |
| `identity` | ~60 | ~20 | ERC-8004 operations |
| `full` | All | All | All tools (except testnet) |
| `dev` | All | All + testnet | Full + local testnet tools |
| *(evaluation)* | Configurable | Configurable | Custom for eval harnesses |

### Structural Absence vs Policy Check

The critical distinction: an `observatory` agent does not have write adapters **filtered at
runtime**. The write adapters are **not registered**. The code path for executing trades does not
exist in the agent's process. This is a compile-time structural guarantee, not a policy check that
could be bypassed.

```rust
/// Profile filtering produces a structurally different Graph.
fn register_adapters(profile: &ToolProfile) -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();

    // All profiles get read adapters
    registry.add("query_state", "portfolio", &GET_PORTFOLIO_SNAPSHOT);
    registry.add("search_context", "price", &GET_TOKEN_PRICE);

    match profile {
        // Trader: read + swap adapters
        ToolProfile::Trader => {
            registry.add("preview_action", "swap", &SIMULATE_SWAP);
            registry.add("commit_action", "swap", &EXECUTE_SWAP);
        }
        // Observatory: read ONLY. No write adapters registered at all.
        ToolProfile::Observatory => {
            // Nothing added — structurally unable to write.
        }
        // Data: read ONLY. Not even all read adapters.
        ToolProfile::Data => {
            // Only data-category read tools.
        }
        // ...
    }

    registry
}
```

### Profile Composition

Profiles compose via comma-separated activation: `TOOL_PROFILE=trader,vault` loads the union
of both profiles' adapter sets. Composition rules:

- Tool sets merge by union
- If two profiles define the same adapter, the first in order wins
- Gates stack (both profiles' gates apply)
- Typed context merges (fields from both)

---

## 4. Route Cell: CascadeRouter and Tool Pruning

Even within a profile, not all tools are exposed on every tick. The **CascadeRouter** (a Route
Cell) selects which 12 or fewer tools to include in each inference call's context.

### Routing Criteria

The CascadeRouter uses EFE (Expected Free Energy) to select tools:

```rust
/// Tool selection via Route protocol (EFE-based).
pub fn select_tools_for_tick(
    available: &[&ToolDef],
    task_context: &TaskSignal,
    probe_results: &ProbeResults,
    regime: MarketRegime,
    max_tools: usize,  // Default: 12
) -> Vec<&ToolDef> {
    available.iter()
        .map(|tool| {
            let relevance = compute_relevance(tool, task_context);
            let cost = tool.tick_budget.expected_cost();
            let information_gain = estimate_info_gain(tool, probe_results);
            let efe = information_gain + relevance - cost;
            (tool, efe)
        })
        .sorted_by(|a, b| b.1.partial_cmp(&a.1).unwrap())
        .take(max_tools)
        .map(|(tool, _)| *tool)
        .collect()
}
```

### Tier-Based Tool Exposure

| Tier | Tools Exposed | Reasoning Depth | Cost |
|---|---|---|---|
| T0 (probe) | 0 (direct answer) | None | ~$0 |
| T1 (fast) | 3-6 | Shallow | Low |
| T2 (full) | 8-12 | Deep | Standard |

Most ticks are T0 or T1 (80-90%), needing only a small tool subset. The CascadeRouter ensures
the LLM sees only the most relevant tools, preventing confusion from irrelevant options.

---

## 5. Cross-Domain Composition

The Rack pattern is domain-agnostic. Multiple domain plugins can be active simultaneously,
each contributing its own adapter-facing tools:

```
Built-in (16 tools)              ← roko-std, always loaded
  + Chain domain (8 adapters)    ← roko-domain-chain, loaded when domain contains "chain"
  + Research domain (4 adapters) ← research-specific search, citation, analysis
  + Ops domain (3 adapters)      ← deployment, monitoring, alerting
  + MCP tools (N dynamic)        ← discovered at runtime from MCP servers
  = Agent's full tool set        ← filtered by profile + role + CascadeRouter per tick
```

### Domain Adapter Namespacing

Each domain contributes adapter-facing tools with domain-prefixed names when composition
requires disambiguation:

| Domain | Adapter Tools | Namespace |
|---|---|---|
| Chain | preview_action, commit_action, query_state, ... | (no prefix — primary domain) |
| Research | search_literature, cite, analyze_corpus | `research.*` |
| Ops | deploy, rollback, health_check | `ops.*` |
| Code | index_build, symbol_lookup, dependency_graph | `code.*` |

When a single domain is active, its tools use bare names. When multiple domains compose,
namespacing prevents collision.

---

## 6. Profile-Cognitive Integration

Profiles affect not just tool availability but how the entire cognitive loop behaves:

### Daimon (Affect) Modulation

```rust
/// Profile → PAD (Pleasure-Arousal-Dominance) baseline.
fn pad_baseline(profile: &ToolProfile) -> PadVector {
    match profile {
        // Active trading: high arousal (executing), high dominance
        ToolProfile::Active => PadVector { p: 0.0, a: 0.7, d: 0.6 },
        // Observatory: low arousal (watching), low dominance
        ToolProfile::Observatory => PadVector { p: 0.2, a: -0.3, d: -0.2 },
        // Data: neutral, monitoring
        ToolProfile::Data => PadVector { p: 0.0, a: -0.2, d: 0.0 },
        // Trader: high arousal (executing)
        ToolProfile::Trader => PadVector { p: 0.0, a: 0.5, d: 0.4 },
        // Learning: moderate arousal (exploring)
        ToolProfile::Learning => PadVector { p: 0.3, a: 0.2, d: 0.1 },
    }
}
```

### Dream Frequency

| Profile | Dream Cycle Frequency | Rationale |
|---|---|---|
| `active` | Standard (delta ~hours) | Balanced between action and consolidation |
| `observatory` | High (more dream time) | More observation → more to consolidate |
| `learning` | High | Memory self-improvement focus |
| `data` | Low | Less experiential data to consolidate |

### Tier Routing Bias

Lower arousal → more T0 probes, fewer T2 deep-reasoning ticks. An observatory agent naturally
operates mostly at T0/T1 (watching, not deciding), which matches its passive role and reduces
inference cost to ~0.3x of an active agent.

---

## 7. Ground Truth and Learning

Write tools in the chain domain produce ground-truth verification data that feeds the learning
system:

```rust
/// ToolResult for a chain domain write operation.
pub struct ChainWriteResult {
    pub data: Value,

    // Prediction (from simulation)
    pub expected_outcome: String,  // "Swap 1 ETH → 3200.45 USDC"

    // Reality (from receipt + balance check)
    pub actual_outcome: String,    // "Swap 1 ETH → 3198.12 USDC"

    // Evidence
    pub ground_truth_source: String,  // "receipt:0xabc... + balance_check"
}
```

When expected and actual diverge:
1. Episode is tagged with prediction error magnitude
2. Dream cycle replays the episode for heuristic revision
3. CascadeRouter adjusts: large prediction errors → route to deeper reasoning (T2) next time
4. Section effects updated: which prompt sections correlated with the error?

---

## 8. Protocol Coverage (Chain Domain)

The 423+ tools cover the DeFi landscape systematically:

| Category | Protocol Coverage | Tool Count | Read/Write Split |
|---|---|---|---|
| Data | Token prices, pool state, gas, balances | ~40 | 40/0 |
| Trading | Uniswap V3/V4, 1inch, CoW Protocol | ~20 | 8/12 |
| Lending | Aave V3, Morpho, Compound | ~27 | 15/12 |
| LP | Uniswap V3/V4 positions, fee collection | ~28 | 14/14 |
| Staking | Lido, Rocket Pool, Frax | ~16 | 8/8 |
| Restaking | EigenLayer, EtherFi | ~16 | 8/8 |
| Derivatives | GMX, dYdX, Panoptic | ~16 | 8/8 |
| Yield | Pendle, Yearn, Beefy | ~20 | 10/10 |
| Vault | ERC-4626 vault operations | ~40 | 20/20 |
| Safety | Simulation, risk scoring | ~16 | 16/0 |
| Intelligence | MEV, IL, slippage analysis | ~18 | 18/0 |
| Memory | Episode store, heuristic query | ~13 | 8/5 |
| Identity | ERC-8004, reputation, wallet | ~24 | 12/12 |
| Wallet | Key management, signing | ~8 | 4/4 |
| Streaming | Payment streams | ~6 | 3/3 |

### Multi-Chain Support

All chain tools operate across 11 networks via Alloy provider abstraction:
- Ethereum mainnet, Base, Arbitrum, Optimism, Polygon
- Base Sepolia, Arbitrum Sepolia (testnet)
- BSC, Avalanche, Fantom, zkSync Era

Chain selection is a parameter to the adapter, not a separate tool set.

---

## 9. Alloy Integration Pattern

On-chain operations use Alloy (Paradigm's Rust Ethereum toolkit) with `sol!` macro for
type-safe contract bindings:

```rust
use alloy::{sol, providers::Provider, primitives::*};

sol! {
    function slot0() external view returns (
        uint160 sqrtPriceX96, int24 tick,
        uint16 observationIndex, uint16 observationCardinality,
        uint16 observationCardinalityNext, uint8 feeProtocol, bool unlocked
    );
}

/// Read pool state — type-safe at compile time.
async fn read_slot0(provider: &dyn Provider, pool: Address) -> Result<Slot0Return> {
    let call = slot0Call {};
    let result = provider.call(call.abi_encode(), pool).await?;
    Ok(slot0Call::abi_decode_returns(&result)?)
}
```

Benefits over the legacy TypeScript approach:
- **Compile-time type safety**: Solidity signatures → Rust types. No ABI JSON parsing.
- **60% faster arithmetic**: native U256 vs JavaScript BigInt
- **Zero-copy decoding**: ABI response read directly from buffer
- **No codegen step**: `sol!` is a proc macro

---

## What This Enables

1. **94% token reduction** — the two-layer model lets agents reason over 423+ tools via 8
   adapter-facing interfaces, massively reducing per-turn context cost.
2. **Structural safety** — profiles make capabilities absent, not filtered. An observatory
   agent physically cannot trade.
3. **Domain composability** — multiple domain plugins compose into a single agent, each
   contributing Rack macros and adapter-facing tools.
4. **Progressive routing** — CascadeRouter ensures each tick sees only the most relevant
   tools, preventing LLM confusion from irrelevant options.
5. **Learning from ground truth** — chain write tools produce expected/actual pairs that
   feed the calibration loop automatically.

---

## Feedback Loops

- **Adapter selection accuracy**: when the Route Cell selects the wrong adapter (LLM specifies
  venue that maps to no handler), the error feeds adapter naming heuristics.
- **Profile → Daimon calibration**: arousal baselines affect tier routing. If an observatory
  agent frequently needs T2 (deep reasoning), the baseline is miscalibrated.
- **Tool pruning recall**: if agents frequently ask for tools that CascadeRouter excluded,
  the relevance model needs retraining (more context → better selection).
- **Ground truth divergence**: systematic prediction errors per protocol (e.g., Morpho deposits
  always slip more than simulated) feed protocol-specific simulation calibration.

---

## Open Questions

1. **Domain plugin hot-loading** — can a domain plugin be loaded/unloaded at runtime without
   restarting the agent? (Requires dynamic registry + graceful tool removal.)
2. **Cross-domain routing** — when an agent has both chain and research domains active, how
   does the CascadeRouter balance tools from different domains in the 12-tool budget?
3. **Profile inheritance** — should `conservative` inherit from `active` with restrictions,
   or define its own tool set independently? (Inheritance is implicit currently.)
4. **Adapter versioning** — when protocol upgrades change tool semantics (Uniswap V3 → V4),
   should old adapters remain available or force migration?

---

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| 16 built-in tools (roko-std) | `crates/roko-std/src/tool/builtin/` | Shipped |
| StaticToolRegistry + role filtering | `crates/roko-std/src/tool/registry.rs` | Shipped |
| CascadeRouter (tool pruning) | `crates/roko-learn/src/` | Shipped |
| Profile filtering (category-based) | `crates/roko-std/src/tool/` | Shipped |
| Chain domain crate (423+ tools) | (target: `crates/roko-domain-chain/`) | Planned |
| Two-layer adapter resolution | (target: `crates/roko-domain-chain/`) | Planned |
| Multi-domain composition | `crates/roko-agent/src/` | Planned |
| Profile → Daimon integration | `crates/roko-daimon/src/` | Wired |
| Ground truth → calibration pipeline | `crates/roko-learn/src/` | Wired |
| Alloy provider pool (11 chains) | (target: `crates/roko-domain-chain/`) | Planned |
