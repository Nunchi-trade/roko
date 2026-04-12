# 02 — Tool Categories Taxonomy

> The 17 chain domain categories, prefix conventions, chain support matrix, and risk tier
> scale. Plus how categories interact with profiles for tool filtering.

---

## Overview

Every `ToolDef` has a `category: Category` field that drives profile filtering. The chain
domain plugin defines 17 categories covering DeFi protocols, on-chain operations, analytics,
memory, identity, and testing. The built-in tools (see `01-builtin-tools.md`) use a separate,
simpler category set (File I/O, Search, Execution, Web, Planning/Orchestration).

**Important framing note:** The 17 categories below are specific to the **chain domain plugin**.
They represent one domain's tool taxonomy. Other domains (coding, research, ops) define their
own categories. The chain domain is documented in detail here because it is the most complex
domain plugin, with 423+ tools. But it is still a plugin — not the framework's default (see
`refactoring-prd/05-agent-types.md` for the domain-agnostic agent architecture).

---

## The 17 Chain Domain Categories

| Category | Prefix | Description | Approx. Tool Count |
|---|---|---|---|
| `data` | `data_` | On-chain data reads, pool state, token info, portfolio, P&L | ~40 |
| `trading` | `uniswap_` | Swap execution, quotes, approvals, order management | ~20 |
| `lp` | `uniswap_` | Liquidity provision and position management | ~28 |
| `vault` | `vault_` | ERC-4626 vault operations | ~40 |
| `lending` | `aave_`, `morpho_` | Supply, borrow, repay, health factor monitoring | ~27 |
| `staking` | `lido_`, `rocketpool_` | Liquid staking deposits, withdrawals, reward tracking | ~16 |
| `restaking` | `eigenlayer_` | Restaking, AVS delegation, LRT management | ~16 |
| `derivatives` | `gmx_`, `panoptic_` | Perpetuals, options, hedging strategies | ~16 |
| `yield` | `yearn_`, `pendle_`, `ethena_` | Yield aggregators, PT/YT tokenization | ~20 |
| `safety` | `safety_` | Simulation, risk assessment, circuit breakers | ~16 |
| `intelligence` | `intel_` | MEV scoring, IL calc, venue comparison, regime classification | ~18 |
| `memory` | `memory_` | Neuro (formerly Grimoire) episodic and semantic memory | ~13 |
| `identity` | `identity_` | ERC-8004 agent identity, reputation | ~24 |
| `wallet` | `wallet_` | Wallet policy, funding, session keys | ~8 |
| `streaming` | `stream_` | Event bus live data subscriptions | ~6 |
| `testnet` | `testnet_` | Local Anvil testnet management | ~5 |
| `bootstrap` | `bootstrap_` | First-run setup and provisioning | ~3 |
| | | **Total** | **423+** |

---

## Prefix Convention

All tool names follow `<prefix>_<action>_<subject>`. The prefix identifies the protocol or
subsystem.

### Protocol Prefixes (Chain Domain)

| Prefix | Protocol | Example |
|---|---|---|
| `uniswap_` | Uniswap V2/V3/V4/UniswapX | `uniswap_execute_swap` |
| `aave_` | Aave V3 | `aave_supply_collateral` |
| `morpho_` | Morpho Blue | `morpho_supply_market` |
| `curve_` | Curve Finance | `curve_get_pool_info` |
| `lido_` | Lido | `lido_stake_eth` |
| `rocketpool_` | Rocket Pool | `rocketpool_deposit_eth` |
| `eigenlayer_` | EigenLayer | `eigenlayer_delegate_avs` |
| `pendle_` | Pendle | `pendle_buy_pt` |
| `yearn_` | Yearn V3 | `yearn_deposit_vault` |
| `ethena_` | Ethena | `ethena_stake_usde` |
| `gmx_` | GMX V2 | `gmx_open_position` |
| `panoptic_` | Panoptic | `panoptic_buy_option` |

### Subsystem Prefixes

| Prefix | Subsystem | Example |
|---|---|---|
| `data_` | On-chain data reads | `data_get_token_price` |
| `safety_` | Safety and simulation | `safety_simulate_transaction` |
| `intel_` | Intelligence/analytics | `intel_assess_mev_risk` |
| `memory_` | Neuro memory | `memory_store_episode` |
| `identity_` | ERC-8004 identity | `identity_verify_agent` |
| `wallet_` | Wallet management | `wallet_get_status` |
| `vault_` | ERC-4626 vault ops | `vault_deposit` |
| `stream_` | Live data streams | `stream_subscribe_price` |
| `testnet_` | Local testnet | `testnet_time_travel` |
| `bootstrap_` | First-run setup | `bootstrap_setup_wallet` |

---

## Chain Support Matrix

Not every tool category is available on every chain. This matrix shows write operation
availability (read operations are available on all chains via RPC):

| Category | Ethereum | Base | Unichain | Arbitrum | Optimism | Polygon | BNB | Avalanche |
|---|---|---|---|---|---|---|---|---|
| trading (Uniswap) | V2/V3/V4 | V2/V3/V4 | V3/V4 | V3 | V3 | V3 | V3 | V3 |
| lending (Aave) | Yes | Yes | No | Yes | Yes | Yes | No | Yes |
| lending (Morpho) | Yes | Yes | No | No | No | No | No | No |
| staking (Lido) | Yes | No | No | No | No | No | No | No |
| restaking (EigenLayer) | Yes | No | No | No | No | No | No | No |
| derivatives (GMX) | No | No | No | Yes | No | No | No | Yes |
| derivatives (Panoptic) | Yes | Yes | Yes | No | No | No | No | No |
| yield (Pendle) | Yes | No | No | Yes | No | No | No | No |
| vault (ERC-4626) | Yes | Yes | Yes | No | No | No | No | No |

Chain support is declared in each tool's `ToolDef` via `supported_chains: &[u64]`. The adapter
layer rejects calls targeting unsupported chains with `CHAIN_NOT_SUPPORTED` before any on-chain
interaction.

---

## Risk Tier Scale

Tools use a three-layer risk classification that gates behavioral state transitions and
custody constraints:

### Layer 1 (Low Risk)

Reads, quotes, balance checks. No on-chain state mutation. No capability token required.
No ActionPermit.

**Examples:** `data_get_token_price`, `uniswap_get_pool_info`, `aave_get_health_factor`

### Layer 2 (Medium Risk)

Swaps, standard DeFi operations (supply, withdraw, stake). Require `Capability<WriteTool>`.
Subject to spending limits and PolicyCage validation. Standard ActionPermit.

**Examples:** `uniswap_execute_swap`, `aave_supply_collateral`, `lido_stake_eth`

**Escalation:** Operations exceeding value thresholds escalate to Layer 3:
- Swaps > $50K USD → Elevated
- Lending supply/withdraw > $100K USD → Elevated
- Cross-chain operations → High

### Layer 3 (High Risk)

Leveraged positions, novel protocols, flash loans, options. Require `Capability<WriteTool>`
plus additional Revm simulation before execution. Behavioral-state-gated: blocked when the
agent is in Struggling or Resting states (the new architecture's equivalent of the legacy
conservation/declining phases — see `refactoring-prd/08-translation-guide.md` for the
translation).

**Examples:** `gmx_open_position`, `panoptic_buy_option`, cross-chain bridge operations

### Risk Tier Classification Table (Chain Domain)

| Category | Default Risk Tier | Escalation Conditions |
|---|---|---|
| Lending (supply/withdraw) | Standard | > 100K USD: Elevated |
| LP (add/remove liquidity) | Elevated | V4 hooks: High |
| Swaps | Standard | > 50K USD: Elevated, cross-chain: High |
| Staking | Standard | > 100K USD: Elevated |
| Ownership/admin operations | Critical | Always |

Risk tiers feed into the ActionPermit system. Standard-tier permits execute immediately.
Elevated and above route through additional verification.

---

## Tool Module Breakdown

| Module | Count | Category | Capability | Write Ops |
|---|---|---|---|---|
| On-chain data reads | 9 | data | Read | 0 |
| Trading (Uniswap core) | 5 | trading | Write | 4 |
| Uniswap API | ~20 | trading | Write | ~12 |
| Lending (Aave, Morpho, MakerDAO) | ~27 | lending | Write | ~18 |
| LP management | ~28 | lp | Write | ~18 |
| Vault core | ~40 | vault | Write | ~25 |
| Staking (Lido, Rocket Pool) | ~16 | staking | Write | ~8 |
| Restaking (EigenLayer, LRTs) | ~16 | restaking | Write | ~10 |
| Derivatives (GMX, Panoptic) | ~16 | derivatives | Write | ~10 |
| Yield (Yearn, Pendle, Convex, Ethena) | ~20 | yield | Write | ~12 |
| Bridge + Aggregator | 9 | trading | Write | 4 |
| Safety and simulation | ~16 | safety | Read/Write | ~4 |
| Intelligence and analytics | ~18 | intelligence | Read | 0 |
| Memory | ~13 | memory | Write | ~5 |
| Identity + Wallet | ~24 | identity/wallet | Read/Write | ~8 |
| Streaming | 6 | streaming | Read | 0 |
| Testnet | 5 | testnet | Write | 4 |
| Bootstrap | 3 | bootstrap | Write | 2 |
| **Total** | **423+** | | | **~150** |

All write operations pass through the full safety hook chain (see `04-safety-hooks.md`).

---

## Profile-to-Category Mapping

Profiles compose categories. A profile defines which categories are loaded at boot. The
`data` category is implicitly included in all profiles.

| Profile | data | trading | lending | staking | restaking | derivatives | yield | lp | vault | safety | intelligence | memory | identity | wallet | streaming | testnet | bootstrap |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| active | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | — | Y |
| observatory | Y | — | — | — | — | — | — | — | — | — | Y | — | — | — | — | — | — |
| conservative | Y | Y* | — | — | — | — | — | Y* | — | Y | — | — | — | — | — | — | — |
| data | Y | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — |
| trader | Y | Y | — | — | — | — | — | — | — | Y | — | — | — | — | — | — | — |
| lp | Y | — | — | — | — | — | — | Y | — | Y | — | — | — | — | — | — | — |
| vault | Y | — | — | — | — | — | — | — | Y | Y | — | — | — | — | — | — | — |
| intelligence | Y | — | — | — | — | — | — | — | — | — | Y | — | — | — | — | — | — |
| learning | Y | — | — | — | — | — | — | — | — | — | Y | Y | — | — | — | — | — |
| identity | Y | — | — | — | — | — | — | — | — | — | — | — | Y | — | — | — | — |
| full | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | — | Y |
| dev | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y |

\* `conservative` includes a restricted subset of trading and LP write tools — no leverage,
no flashloans, no complex multi-hop strategies.

### Profile Composition

Profiles compose — `TOOL_PROFILE=trader,vault` activates both trader and vault categories:

```rust
let allowed = resolve_profile_categories(profile);
let tools: Vec<&ToolDef> = ALL_TOOL_DEFS
    .iter()
    .filter(|t| allowed.contains(&t.category))
    .collect();
```

### Fine-Grained Overrides

The config file supports per-tool enable/disable that takes precedence over profiles:

```toml
# roko.toml
[tools]
profile = "trader"
enable = ["intel_compute_vpin", "intel_compute_lvr"]
disable = ["uniswap_submit_uniswapx_order"]
```

---

## Capability Gating

Three capability gates control tool registration. A tool requiring a capability that isn't
present is silently skipped during registration.

| Capability | Required By | How It's Satisfied |
|---|---|---|
| `wallet` | All write tools (trading, LP, vault, safety) | A signer is configured (`ROKO_WALLET_*` env or `roko.toml [wallet]`) |
| `uniswap_api` | API-backed tools | `ROKO_UNISWAP_API_KEY` is set |
| `memory` | Memory and self-improvement tools | `ROKO_MEMORY_ENABLED=true` and `learning` profile active |

Capability checking happens once at boot. A `data` profile with no wallet loads all read tools
without error. A `trader` profile without a wallet logs a warning and skips write tools.

---

## Category Interaction with Cognitive Subsystems

Categories don't just filter tools — they also influence how cognitive subsystems interact with
tool results:

| Category | Neuro Storage | Daimon Influence | Dream Replay |
|---|---|---|---|
| `data` | Store as Transient-tier knowledge | No affect change | Rarely replayed |
| `trading` | Store as Working-tier episodes | Success → Pleasure↑, failure → Pleasure↓ | Frequently replayed |
| `safety` | Store as Consolidated warnings | Blocking → Dominance↓ | Always replayed on failure |
| `intelligence` | Store as Insights, may promote to Persistent | Novelty detection → Arousal↑ | Replayed during Delta consolidation |
| `memory` | Self-referential — stores knowledge about knowledge | No direct affect | Not replayed (meta-level) |

This interaction is automatic. The cognitive loop (see `00-ALWAYS-READ-FIRST.md`) processes
tool results through the VERIFY → PERSIST → ADAPT → META-COGNIZE pipeline regardless of
category. The category metadata simply informs how each subsystem weights the result.
