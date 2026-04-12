# 03 — Chain Domain Plugin: 423+ DeFi Tools

> The chain domain plugin — 423+ DeFi tools as ONE domain plugin, not the core framework.
> Two-layer tool model, adapter pattern, high-level architecture, protocol coverage.


> **Implementation**: Built

---

## Framing

**The chain domain plugin is one domain plugin among many.** Roko's kernel is domain-agnostic.
The 423+ DeFi tools covering Uniswap, Aave, Morpho, Pendle, Lido, EigenLayer, GMX, Panoptic,
and other protocols are implemented as a chain domain crate (`roko-domain-chain`, target name;
currently in legacy `golem-tools`). A coding agent, research agent, or operations agent does
NOT load these tools by default. Only agents configured with `domain = "chain"` load the chain
domain plugin.

This distinction is architecturally enforced: the chain tools are a separate crate that
depends on `roko-core` (for ToolDef, ToolContext, ToolResult), but `roko-core` does NOT depend
on the chain crate. Dependencies flow strictly downward per the five-layer architecture.

---

## Two-Layer Tool Model

The agent's LLM never calls protocol-specific tools directly. It calls **8 adapter-facing
tools** (`preview_action`, `commit_action`, `cancel_action`, `emergency_halt`, `query_state`,
`search_context`, `query_neuro`, `update_directive`). Each call resolves to a specific ToolDef
handler through the **Tool Adapter Registry** maintained by the chain domain plugin.

### Why Two Layers?

Exposing all 423+ tools directly to the LLM would require ~38,000 tokens per turn just for
tool definitions. The two-layer model reduces this to ~1,200 tokens — a **94% reduction**.

The LLM sees a small, stable interface. The adapter layer resolves to the right protocol-specific handler based on action type, venue, and parameters.

### Adapter Resolution

When the LLM calls:

```json
{
  "tool": "preview_action",
  "params": {
    "action_type": "deposit",
    "venue": "morpho",
    "asset": "USDC",
    "amount": "50000000000"
  }
}
```

The adapter layer:
1. Resolves `venue: "morpho"` to the Morpho protocol adapter
2. Constructs calldata via Alloy's `sol!` macro
3. Classifies the risk tier
4. Returns an `AdapterResolution` routing to the internal handler

```rust
pub struct AdapterResolution {
    /// The internal ToolDef to invoke.
    pub internal_tool: &'static ToolDef,
    /// Transformed parameters (adapter-facing schema → internal schema).
    pub transformed_params: serde_json::Value,
    /// Risk tier for ActionPermit routing.
    pub risk_tier: RiskTier,
}

#[derive(Debug, Clone, Copy)]
pub enum RiskTier {
    Routine,     // Read-only, informational
    Standard,    // Bounded value write
    Elevated,    // Large value or complex operation
    High,        // Cross-chain, V4 hooks, leverage
    Critical,    // Ownership/admin operations
}
```

### Profile-Specific Adapter Sets

Each profile gets a different set of adapters. The `data` profile is **structurally unable**
to trade — it has no `preview_action` or `commit_action` adapters. Not gated by a flag. Not
blocked by a policy check. The routing entries don't exist.

```rust
fn register_adapters(profile: &ToolProfile) -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();

    // All profiles get query adapters
    registry.add("query_state", "portfolio", &GET_PORTFOLIO_SNAPSHOT);
    registry.add("search_context", "price", &GET_TOKEN_PRICE);
    registry.add("search_context", "pool", &GET_POOL_INFO);

    match profile {
        ToolProfile::Trader | ToolProfile::Full | ToolProfile::Dev => {
            registry.add("preview_action", "swap", &SIMULATE_SWAP);
            registry.add("commit_action", "swap", &EXECUTE_SWAP);
        }
        ToolProfile::Lp | ToolProfile::Full | ToolProfile::Dev => {
            registry.add("preview_action", "add_liquidity", &SIMULATE_ADD_LIQUIDITY);
            registry.add("commit_action", "add_liquidity", &EXECUTE_ADD_LIQUIDITY);
            registry.add("preview_action", "remove_liquidity", &SIMULATE_REMOVE_LIQUIDITY);
            registry.add("commit_action", "remove_liquidity", &EXECUTE_REMOVE_LIQUIDITY);
        }
        ToolProfile::Vault | ToolProfile::Full | ToolProfile::Dev => {
            registry.add("preview_action", "deposit", &VAULT_PREVIEW_DEPOSIT);
            registry.add("commit_action", "deposit", &VAULT_DEPOSIT);
            registry.add("preview_action", "withdraw", &VAULT_PREVIEW_WITHDRAW);
            registry.add("commit_action", "withdraw", &VAULT_WITHDRAW);
        }
        ToolProfile::Data => {
            // Read-only. NO preview_action or commit_action adapters.
        }
        _ => {}
    }

    registry
}
```

The `observatory` profile is architecturally distinct. An observatory agent loads only read
tools, meaning the code path for executing trades doesn't exist at runtime — not blocked by
a policy check, but **structurally absent**. The observatory agent watches the market, dreams
about what it observes, publishes structural insights to the collective mesh, and consumes
resources at 0.3× the rate of an active agent (no gas costs, reduced inference).

---

## High-Level Architecture

### Primary Path: Agent via Synapse Loop

```
Agent (Cognitive Loop)
       |
       v
+--- Framework Layer (L1) ----------------------------------+
|  Chain domain plugin: 8 adapter-facing tools               |
|  Safety capabilities: PolicyCage + spending enforcement     |
|  Permits: ActionPermit lifecycle                            |
+--------+---------------------------------------------------+
         |
         v
+--- Tool Adapter Registry ----------------------------------+
|  Adapter-facing tool → ToolDef handler resolution           |
|  Profile-filtered: trader, vault, lp, etc.                  |
+--------+---------------------------------------------------+
         |
         v
+--- Chain Domain Crate -------------------------------------+
|  Data (~40) | Trading (~20) | Lending (~27) | LP (~28)     |
|  Staking (~16) | Restaking (~16) | Derivatives (~16)       |
|  Yield (~20) | Vault (~40) | Safety (~16) | Intel (~18)    |
|  Memory (~13) | Identity (~24) | Wallet (~8) | Stream (6)  |
|  423+ tools, all ToolDef + handler functions                |
+--------+---------------------------------------------------+
         | (unchanged from here down)
         v
  [Neuro → Safety Hook Chain → Alloy Provider → Signer → Revm]
```

### Secondary Path: External Agents via A2A

```
External Agent
       |
       v
+--- A2A Interface -----------------------------------------+
|  JSON-RPC 2.0 task lifecycle                               |
|  Agent Card at /.well-known/agent.json                     |
|  Imports handlers from chain domain crate                  |
+--------+--------------------------------------------------+
         |
         v
  [Same chain domain handler layer]
```

### Shared Lower Stack

```
+--- Neuro (Knowledge Layer) --------------------------------+
|  LanceDB (episodic) | SQLite (semantic) | Filesystem (strat)|
|  Reflexion | ExpeL | Ebbinghaus decay | Dream hooks         |
|  Optional: active when `learning` profile is on             |
+--------+---------------------------------------------------+
         |
         v
+--- Safety Hook Chain --------------------------------------+
|  on_tool_call chain: safety → permits → risk → filter      |
|  Capability token minting + consumption                    |
|  Revm simulation (pre-flight fork)                         |
|  PolicyCage enforcement + behavioral state gating          |
+--------+---------------------------------------------------+
         |
         v
+--- Alloy Provider Layer ----------------------------------+
|  sol! macro for type-safe contract bindings                |
|  11 chains | RPC pool | retry | block caching             |
+--------+---------------------------------------------------+
         |
         v
+--- Signer Abstraction ------------------------------------+
|  Local key | Privy (HTTP) | Safe | ZeroDev | generic Alloy|
|  All normalized to Alloy's Signer trait                    |
+--------+---------------------------------------------------+
         |
         v
+--- TypeScript Sidecar ------------------------------------+
|  Unix socket IPC (~1-5ms)                                  |
|  Uniswap SDK math (smart-order-router, v3-sdk, v4-sdk)    |
|  Called only for routing/position math not yet ported       |
+-----------------------------------------------------------+
```

---

## Alloy Integration

All on-chain interaction uses Alloy (Paradigm's Rust Ethereum toolkit). The `sol!` macro
generates type-safe Rust bindings from Solidity function signatures at compile time.

```rust
use alloy::{sol, providers::Provider, primitives::*};

sol! {
    /// Uniswap V3 Pool slot0 read.
    function slot0() external view returns (
        uint160 sqrtPriceX96,
        int24 tick,
        uint16 observationIndex,
        uint16 observationCardinality,
        uint16 observationCardinalityNext,
        uint8 feeProtocol,
        bool unlocked
    );

    /// ERC-20 balance check.
    function balanceOf(address owner) external view returns (uint256);

    /// ERC-4626 vault deposit.
    function deposit(uint256 assets, address receiver) external returns (uint256 shares);
}

/// Read pool state using type-safe bindings.
async fn read_slot0(provider: &dyn Provider, pool: Address) -> Result<Slot0Return> {
    let call = slot0Call {};
    let result = provider.call(call.abi_encode(), pool).await?;
    Ok(slot0Call::abi_decode_returns(&result)?)
}
```

Advantages over previous viem (TypeScript) approach:
- **Type-safe at compile time**: Solidity signatures compile to Rust types. No ABI JSON, no
  runtime decode errors.
- **No codegen step**: The `sol!` macro runs at compile time as a procedural macro.
- **60% faster arithmetic**: Alloy's `U256` operations in native Rust vs JavaScript BigInt.
- **Zero-copy decoding**: ABI decoding reads directly from the response buffer.

### TypeScript Sidecar

Uniswap SDKs are 50,000+ lines of TypeScript. Porting them would take months. Instead, a
co-located Node.js process handles SDK math via Unix domain socket (~1-5ms latency):

```rust
pub struct SidecarClient {
    socket_path: PathBuf,
}

impl SidecarClient {
    pub async fn find_best_route(
        &self,
        token_in: Address,
        token_out: Address,
        amount: U256,
        chain_id: u64,
    ) -> Result<SwapRoute> {
        let params = serde_json::json!({
            "tokenIn": token_in.to_string(),
            "tokenOut": token_out.to_string(),
            "amount": amount.to_string(),
            "chainId": chain_id,
        });
        let result = self.call("findBestRoute", params).await?;
        Ok(serde_json::from_value(result)?)
    }
}
```

The sidecar runs `@uniswap/smart-order-router`, `v3-sdk`, `v4-sdk`, `permit2-sdk`, and
`uniswapx-sdk`. It starts automatically with the agent and restarts on crash.

---

## Revm Simulation

Pre-flight simulation uses Revm (Rust EVM implementation) instead of `eth_call`. Revm
provides a local fork of chain state supporting multi-step simulation, state inspection, and
gas profiling.

```rust
pub async fn simulate_swap(
    ctx: &ToolContext,
    chain_id: u64,
    calldata: &[u8],
    to: Address,
    value: U256,
) -> Result<SimulationResult> {
    let mut fork = ctx.revm_fork(chain_id)?;

    // Execute the transaction in the fork
    let result = fork.transact(calldata, to, value)?;

    // Inspect state changes
    let balance_before = fork.balance_of(ctx.signer_address(), token)?;
    let balance_after = fork.balance_of_post(ctx.signer_address(), token)?;

    Ok(SimulationResult {
        success: result.is_success(),
        gas_used: result.gas_used(),
        output: result.output().to_vec(),
        state_changes: fork.diff(),
        balance_delta: balance_after - balance_before,
    })
}
```

Advantages over `eth_call`:
- **Multi-step simulation**: Execute approve + swap + verify in one fork
- **State inspection**: Read balances before and after without separate calls
- **No RPC round-trips**: One fork creation, then all simulation is local
- **Deterministic gas**: Fork snapshot at specific block, not racing against mempool

---

## Tool Pruning

The task classifier in the model routing system (CascadeRouter) determines which 12 or fewer
tools to expose per tick. The rest are deferred — present in the adapter registry but not
included in the LLM's context window for that inference call. The classifier reads current
probe results and regime to select the most relevant tool subset.

This is a key component of the dual-process cognition system. Most ticks are T0 (direct tool
call, no LLM) or T1 (fast model, shallow reasoning), which need only a small tool subset. Only
T2 ticks (full model, deep reasoning) might need broader tool access, and even then the
classifier selects the most relevant subset.

---

## mirage-rs Integration

The `mirage-rs` crate (`crates/mirage-rs/`, 141 tests) provides an in-process EVM simulator
for testing chain domain tools without connecting to real networks. It supports:

- Fork from any chain at any block
- Time travel (`testnet_time_travel` tool)
- Account impersonation
- Transaction simulation with state diff
- Gas profiling

mirage-rs is used in three contexts:
1. **Tool testing**: Unit and integration tests for chain domain tools
2. **Pre-flight simulation**: The `safety_simulate_transaction` tool
3. **Development**: The `dev` profile's testnet tools for local development

See `07-tool-testing.md` for the complete testing architecture using mirage-rs.
