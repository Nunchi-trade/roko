# 05 — Tool Profiles & Configuration

> Profile-based tool loading, 13 chain domain profiles, configuration hierarchy,
> environment variables, and fine-grained overrides.


> **Implementation**: Shipping

---

## Overview

Tool profiles control which tools are loaded at agent boot. A profile is a named selection of
tool categories — it determines the agent's structural capabilities. An agent with the `data`
profile is **structurally unable** to trade: the write tool adapters don't exist in its
registry. This is not a runtime policy check — it's a structural absence.

Profiles are set in `roko.toml` or via the `TOOL_PROFILE` environment variable. They compose —
`TOOL_PROFILE=trader,vault` activates both trader and vault categories.

---

## The 13 Chain Domain Profiles

These profiles are specific to the chain domain plugin. Other domains (coding, research, ops)
define their own role-based access via the `StaticToolRegistry.for_role()` mechanism (see
`01-builtin-tools.md`).

| Profile | Read Tools | Write Tools | Use Case |
|---|---|---|---|
| `active` | All ~250 | All ~150 | Standard active trading agent — full read + write access |
| `observatory` | All ~250 | None | Observatory phenotype — observes, dreams, publishes, never trades |
| `conservative` | All ~250 | ~40 (restricted) | Risk-averse owner configuration |
| `data` | ~40 | None | Read-only analytics, monitoring, portfolio tracking |
| `trader` | ~60 | ~20 | Swap execution, quotes, approvals, MEV assessment |
| `lp` | ~65 | ~25 | Liquidity provision, position management, fee collection |
| `vault` | ~75 | ~35 | ERC-4626 vault operations, am-AMM bidding |
| `intelligence` | ~58 | None | MEV scoring, IL calculation, venue comparison |
| `learning` | ~52 | ~12 | Memory management, self-improvement |
| `identity` | ~60 | ~20 | ERC-8004 identity, reputation, wallet |
| `full` | All | All | All tools registered (except testnet) |
| `dev` | All | All + testnet | Full + local testnet tools |
| *(evaluation)* | Configurable | Configurable | Custom profile for evaluation harnesses |

### Profile Semantics

**`active`** and **`full`** are equivalent for tool access — both load all categories except
testnet. The distinction is semantic: `active` implies a trading agent, `full` implies a
general-purpose agent that happens to have all tools available.

**`observatory`** is architecturally distinct. An observatory agent loads only read tools and
intelligence tools. The code path for executing trades doesn't exist at runtime. The agent
watches the market, processes data through its cognitive loop (Dreams consolidation at Delta
frequency), and publishes insights to the collective mesh. It consumes resources at ~0.3×
the rate of an active agent (no gas costs, reduced inference).

**`conservative`** includes a restricted subset of trading and LP write tools — no leverage,
no flashloans, no complex multi-hop strategies. This is the recommended profile for
risk-averse owners who want automated trading with guardrails.

**`dev`** extends `full` with testnet tools (`testnet_time_travel`, `testnet_mine_blocks`,
etc.) for local development against Anvil or mirage-rs. Not used in production.

---

## Profile Filtering Mechanism

Profile filtering uses the `ToolDef.category` field. Filtering happens once at initialization:

```rust
/// Resolve which categories a profile includes.
fn resolve_profile_categories(profile: &str) -> HashSet<Category> {
    match profile {
        "active" | "full" => ALL_CATEGORIES.iter().copied().collect(),
        "observatory" => [Category::Data, Category::Intelligence].into(),
        "conservative" => [Category::Data, Category::Trading, Category::Lp, Category::Safety].into(),
        "data" => [Category::Data].into(),
        "trader" => [Category::Data, Category::Trading, Category::Safety].into(),
        "lp" => [Category::Data, Category::Lp, Category::Safety].into(),
        "vault" => [Category::Data, Category::Vault, Category::Safety].into(),
        "intelligence" => [Category::Data, Category::Intelligence].into(),
        "learning" => [Category::Data, Category::Intelligence, Category::Memory].into(),
        "identity" => [Category::Data, Category::Identity].into(),
        "dev" => ALL_CATEGORIES.iter().copied().chain([Category::Testnet]).collect(),
        _ => [Category::Data].into(), // fallback
    }
}

// Applied at boot:
let allowed = resolve_profile_categories(profile);
let tools: Vec<&ToolDef> = ALL_TOOL_DEFS
    .iter()
    .filter(|t| allowed.contains(&t.category))
    .collect();
```

---

## Configuration Hierarchy

Configuration follows a precedence chain (highest priority first):

1. **CLI flags** — `--profile trader --disable uniswap_submit_uniswapx_order`
2. **Environment variables** — `TOOL_PROFILE=trader`, `ROKO_TOOL_DISABLE=...`
3. **Config file** — `roko.toml` `[tools]` section
4. **Defaults** — `active` profile if nothing specified

### roko.toml Configuration

```toml
[tools]
# Profile name (or comma-separated list for composition)
profile = "trader"

# Per-tool overrides (take precedence over profile)
enable = ["intel_compute_vpin", "intel_compute_lvr"]
disable = ["uniswap_submit_uniswapx_order"]

[tools.safety]
# Spending limits
max_per_tick_usd = 10000.0
max_per_day_usd = 100000.0
# Rate limits
max_writes_per_minute = 10
# Simulation
require_simulation = true
simulation_gas_multiplier = 1.2

[tools.cache]
# Tool result caching
ttl_seconds = 15
max_entries = 1000
```

### Environment Variables

All environment variables use the `ROKO_` prefix (renamed from the legacy `GOLEM_`/`BARDO_`
prefixes):

| Variable | Purpose | Example |
|---|---|---|
| `TOOL_PROFILE` | Profile selection | `trader,vault` |
| `ROKO_WALLET_TYPE` | Custody mode | `delegation`, `embedded`, `local_key` |
| `ROKO_WALLET_KEY` | Local key (dev only) | Hex-encoded private key |
| `ROKO_UNISWAP_API_KEY` | Uniswap Trading API access | API key string |
| `ROKO_MEMORY_ENABLED` | Enable Neuro memory integration | `true` |
| `ROKO_SUBGRAPH_URL` | Subgraph endpoint | URL |
| `ROKO_RPC_URL` | RPC endpoint override | URL |
| `ROKO_TOOL_DISABLE` | Disable specific tools | Comma-separated names |
| `ROKO_TOOL_ENABLE` | Enable specific tools | Comma-separated names |
| `ROKO_SIMULATION_REQUIRED` | Require pre-flight simulation | `true` |

### Three-Tier API Key Model

| Tier | Permissions | Required For |
|---|---|---|
| **Read** | Pool data, prices, quotes | All profiles |
| **Feedback** | Read + route quality reporting | Trader, LP profiles with analytics |
| **Write** | Read + Feedback + order submission | Trader, LP profiles with execution |

The read tier is sufficient for most tools. Write tier is only needed for tools that submit
orders via the Uniswap Trading API (UniswapX, Smart Order Router). Feedback tier enables
quality-of-execution reporting that improves routing over time.

---

## Data Source Configuration

### Data Source Matrix

| Data Source | Used By | Configuration |
|---|---|---|
| **RPC (Alloy)** | All chain reads/writes | `ROKO_RPC_URL` or `roko.toml [rpc]` |
| **Subgraph** | Historical data, analytics | `ROKO_SUBGRAPH_URL` or `roko.toml [subgraph]` |
| **Uniswap API** | Smart order routing, UniswapX | `ROKO_UNISWAP_API_KEY` |
| **Coingecko/DeFiLlama** | Token prices, TVL | Optional, falls back to on-chain |
| **Neuro (local)** | Episodic memory, semantic search | `ROKO_MEMORY_ENABLED` |

### Caching Strategy

Tool results are cached to reduce RPC calls and improve latency:

| Data Type | TTL | Eviction |
|---|---|---|
| Token prices | 15s | LRU, max 1000 entries |
| Pool state | 15s | LRU, per-pool |
| Gas prices | 12s | Single entry, refreshed on access |
| Balance snapshots | 30s | Per-address, invalidated on write |
| Subgraph queries | 60s | LRU, max 500 entries |

---

## Error Taxonomy

Tool errors follow a structured taxonomy for consistent handling:

| Error Code | Category | Retryable | Example |
|---|---|---|---|
| `CHAIN_NOT_SUPPORTED` | Configuration | No | Tool called for unsupported chain |
| `WALLET_NOT_CONFIGURED` | Configuration | No | Write tool called without wallet |
| `INSUFFICIENT_BALANCE` | State | No (without deposit) | Not enough tokens for operation |
| `SLIPPAGE_EXCEEDED` | Market | Yes (with new quote) | Price moved beyond tolerance |
| `GAS_ESTIMATION_FAILED` | RPC | Yes | RPC returned error on gas estimate |
| `SIMULATION_FAILED` | Safety | No (until params change) | Revm simulation reverted |
| `RATE_LIMITED` | Safety | Yes (after cooldown) | Too many operations per window |
| `CAPABILITY_EXPIRED` | Safety | Yes (re-preview) | ActionPermit timed out |
| `POLICY_REJECTED` | Safety | No | PolicyCage blocked the operation |
| `HALLUCINATION_DETECTED` | Safety | No | Address/amount doesn't match known state |

---

## Profile Interaction with Cognitive Subsystems

The profile affects not just tool availability but how cognitive subsystems behave:

| Profile | Daimon Modulation | Dream Frequency | Neuro Priority |
|---|---|---|---|
| `active` | Full PAD range | Standard (Delta ~hours) | Balanced across categories |
| `observatory` | Low arousal (watching) | High (more Dream time) | Intelligence prioritized |
| `trader` | High arousal (executing) | Standard | Trading episodes prioritized |
| `learning` | Moderate (exploring) | High | Memory self-improvement |
| `data` | Low (monitoring) | Low | Data quality insights |

The Daimon (motivation/affect system) reads the profile to calibrate its Pleasure-Arousal-Dominance (PAD) vector baselines. An observatory agent naturally has lower arousal (it's watching, not acting), which in turn affects tier routing — lower arousal means more T0 probes and fewer T2 deep-reasoning ticks, which is exactly right for a passive observation role.
