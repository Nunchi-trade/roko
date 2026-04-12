# Configuration and Operator Model

> **Layer**: L1 Framework (roles, tools, model routing, capabilities)
>
> **Prerequisites**: `docs/17-lifecycle/01-agent-creation.md` (agent manifest), `docs/17-lifecycle/02-provisioning.md` (provisioning pipeline)
>
> **Synapse traits**: Policy (operator controls implemented as Policy trait observers), Router (model selection configured via config), Composer (budget constraints configured via config)


> **Implementation**: Specified

---

## Overview

Agent configuration in Roko follows a four-file model with a strict override hierarchy. The operator (the human who creates, funds, configures, and monitors an agent) has a defined set of controls — from gentle steering to hard kill — that respect agent autonomy while maintaining human oversight. This document specifies the configuration files, their loading order, hot-reload semantics, and the operator freedom hierarchy.

---

## Four Configuration Files

| File | Written by | Purpose | Hot-reloadable |
|------|-----------|---------|----------------|
| `roko.toml` | Operator | Infrastructure, inference, budget, Neuro, Mesh, tools | Partial (see below) |
| `STRATEGY.md` | Operator (or AI autofill) | Agent goals, tactics, risk bounds, domain-specific parameters | Yes (full) |
| `PLAYBOOK.md` | Agent (via Dream integration) | Machine-evolved heuristics, distilled from experience | Read-only to operator |
| `hermes.yaml` | Agent (via Mesh) | Peer discovery, Mesh topology, gossip configuration | Read-only to operator |

### `roko.toml` — Infrastructure Configuration

The primary configuration file. Operator-authored, version-controlled, merged from four sources in priority order.

```toml
# roko.toml — Full annotated example

[agent]
name = "agent-V1StGXR8_Z5j"
prompt = "Monitor ETH/USDC liquidity on Uniswap V3 and rebalance"
mode = "self-hosted"           # "hosted" | "self-hosted"
domain = "chain"               # "chain" | "coding" | "research" | "general"

[inference]
default_model = "claude-haiku-4-5"
escalation_model = "claude-sonnet-4-6"
critical_model = "claude-opus-4-6"
# gateway_url = "https://inference.roko.dev"   # For hosted inference
max_tokens_per_turn = 4096
temperature = 0.7

[inference.routing]
# Three cognitive speeds (Gamma, Theta, Delta)
gamma_model = "claude-haiku-4-5"       # ~5-15s reactive
theta_model = "claude-sonnet-4-6"      # ~75s reflective
delta_model = "claude-opus-4-6"        # ~hours consolidation

[neuro]
path = ".roko/neuro/"
max_engrams = 50000
decay_model = "ebbinghaus"

[neuro.tiers]
transient_multiplier = 0.1     # Fast decay: 0.1× base half-life
working_multiplier = 0.5       # Moderate decay: 0.5× base half-life
consolidated_multiplier = 1.0  # Standard decay: 1.0× base half-life
persistent_multiplier = 5.0    # Slow decay: 5.0× base half-life

[neuro.types]
# Six knowledge types with base half-lives (in hours)
insight_half_life = 168        # 1 week
heuristic_half_life = 336      # 2 weeks
warning_half_life = 72         # 3 days
causal_link_half_life = 504    # 3 weeks
strategy_fragment_half_life = 168  # 1 week
anti_knowledge_half_life = 720 # 30 days

[mesh]
enabled = false
# relay_url = "wss://mesh.roko.dev/v1/ws"
# collective_id = "my-team"

[tools]
profile = "standard"           # "minimal" | "standard" | "full"

[budget]
max_daily_inference_usd = 10.0
# max_total_usd = 1000.0      # Optional hard cap

[heartbeat]
# Adaptive clock configuration
gamma_interval_secs = 15       # Reactive loop (5-15s)
theta_interval_secs = 75       # Reflective loop (~75s)
delta_interval_hours = 6       # Consolidation cycle (hours)

# --- Domain-specific sections (chain domain only) ---
[chain]
network = "base"               # "base" | "base-sepolia" | "anvil"
# wallet.mode = "delegation"
# wallet.smart_account = "0x..."

[chain.korai]
# KORAI/DAEJI token configuration
demurrage_rate = 0.01          # 1% annual demurrage
```

### Hot-Reload Scope

| Section | Hot-reloadable | Requires restart |
|---------|---------------|-----------------|
| `[agent]` | No | Yes (name, mode changes need reprovisioning) |
| `[inference]` | Yes | No (model changes take effect next turn) |
| `[neuro]` | Partial | `path` requires restart; `max_engrams`, tiers, types are hot |
| `[mesh]` | No | Yes (connection changes need re-handshake) |
| `[tools]` | Yes | No (profile changes take effect next tool load) |
| `[budget]` | Yes | No (limit changes take effect immediately) |
| `[heartbeat]` | Yes | No (interval changes take effect next tick) |
| `[chain]` | No | Yes (network changes need reprovisioning) |

### Config Loading Priority

Four sources, in descending priority:

1. **CLI flags**: `--model claude-opus-4-6` overrides everything
2. **Environment variables**: `ROKO_INFERENCE_DEFAULT_MODEL=claude-opus-4-6`
3. **TOML config file**: `roko.toml` at the specified path
4. **Built-in defaults**: Hardcoded sane defaults in the `roko-core` crate

Secrets (API keys, wallet keys) are only accepted from environment variables, keystore files, or interactive stdin prompts — never from CLI flags or config files. Shell history, process listings, and crash reports never contain key material.

---

### `STRATEGY.md` — Agent Goals

The operator-authored (or AI-generated) strategy document. This is the human-to-agent instruction surface — the primary way operators communicate intent.

```markdown
# Strategy: ETH/USDC LP Management

## Objective
Maximize fee revenue from Uniswap V3 ETH/USDC concentrated liquidity positions
while maintaining capital preservation during high-volatility events.

## Assets
- ETH
- USDC

## Entry Conditions
- ETH 24h volatility < 5%
- Pool fee tier: 0.3% (3000)
- Minimum pool TVL: $10M

## Exit Conditions
- ETH 24h volatility > 8% (emergency exit)
- Position out of range for > 4 hours
- Unrealized loss > 2% of position value

## Risk Bounds
- Max position size: 50% of portfolio
- Max concurrent positions: 3
- Stop loss: -5% per position
- Max daily gas: $10

## Heartbeat
- tick_interval: 60s
- cron: "*/5 * * * *"  # Check positions every 5 minutes
```

`STRATEGY.md` is fully hot-reloadable. Changing the strategy file sends a "steer" signal to the cognitive loop, which picks up the new strategy on its next planning cycle. No restart needed.

### `PLAYBOOK.md` — Machine-Evolved Heuristics

Written exclusively by the agent's Dream integration cycle. Contains procedural knowledge distilled from experience. The operator can read `PLAYBOOK.md` but cannot modify it — this is the agent's learned knowledge.

Example (auto-generated):

```markdown
# Playbook (auto-generated by Dream integration)

## Heuristic: Gas Timing
- Optimal rebalance window: 02:00-06:00 UTC (lowest gas)
- Confidence: 0.87
- Validated: 147 observations over 3 weeks
- Source: NREM replay consolidation, cycle #42

## Heuristic: Volatility Regime Detection
- VIX proxy > 25 correlates with ETH hourly vol > 3% (r² = 0.72)
- When detected: reduce position size by 40%, widen tick range by 2x
- Confidence: 0.91
- Validated: 23 regime transitions
- Source: REM imagination + Mattar-Daw prioritized replay
```

### `hermes.yaml` — Mesh Configuration

Written by the Mesh subsystem. Contains peer discovery information, gossip topology, and Collective membership. The operator configures Mesh enablement and relay URL in `roko.toml`; the Mesh subsystem manages the operational state.

---

## Operator Freedom Hierarchy

The operator has five levels of control, from least to most disruptive:

### Level 1: Steer (Non-Disruptive)

Edit `STRATEGY.md`. The agent picks up the new strategy on its next planning cycle. No interruption to current work. The agent may take several ticks to fully adapt to the new strategy.

**Example**: Change risk bounds, adjust target assets, modify entry/exit conditions.

### Level 2: Constrain (Bounded Disruption)

Modify `roko.toml` hot-reloadable sections. Changes take effect immediately but do not interrupt in-progress work.

**Example**: Reduce budget limits, change model routing, adjust heartbeat intervals.

### Level 3: Pause (Reversible Disruption)

Suspend the cognitive loop. The agent stops processing but retains all state. Resume restores the agent to its pre-pause state.

```bash
roko pause                     # Suspend cognitive loop
roko resume                    # Resume from where it left off
```

**Example**: Investigate unusual behavior, wait for market conditions to stabilize.

### Level 4: Restart (State-Preserving Disruption)

Stop and restart the agent process. Neuro state is preserved on disk. The agent resumes from its last persisted state.

```bash
roko restart                   # Stop, restart, resume from persisted state
```

**Example**: Apply config changes that require restart (mesh URL, network, agent name).

### Level 5: Kill (Irreversible)

Immediately terminate the agent. Resources released. Neuro state preserved on disk (not destroyed). The user can later back up the Neuro and create a new agent.

```bash
roko delete                    # Clean shutdown (see 06-agent-deletion.md)
roko delete --force            # Immediate kill (no graceful shutdown)
```

**Example**: Agent exhibiting harmful behavior, unrecoverable state, operator decision to stop.

---

## Operator Responsibilities

The operator is responsible for:

1. **Strategy definition**: What the agent should do (STRATEGY.md)
2. **Resource allocation**: How much the agent can spend (budget config)
3. **Model selection**: Which LLMs the agent uses (inference config)
4. **Tool access**: Which tools are available (tool profile)
5. **Mesh policy**: Whether the agent shares knowledge with others (Mesh config)
6. **Lifecycle management**: When to create, pause, restart, delete

The operator is NOT responsible for:

1. **Tactical decisions**: The agent decides how to execute the strategy
2. **Knowledge management**: The agent manages its own Neuro via Ebbinghaus decay
3. **Behavioral adaptation**: The Daimon adjusts behavioral states based on performance
4. **Dream scheduling**: The Dream subsystem decides when to consolidate

This separation ensures that the agent maintains genuine autonomy within operator-defined bounds — critical for the anti-proletarianization mandate (Stiegler 2010, 2018). An agent that merely executes operator instructions without developing its own understanding is a proletarianized agent — it has lost the capacity for genuine knowledge. The operator sets goals; the agent develops competence.

---

## Configuration Validation

All configuration changes are validated before application:

```rust
/// Validate a configuration change against safety constraints.
pub fn validate_config_change(
    current: &AgentConfig,
    proposed: &AgentConfig,
) -> Result<Vec<ConfigWarning>, ConfigError> {
    let mut warnings = Vec::new();

    // Budget can only decrease, not increase, without explicit confirmation
    if proposed.budget.max_daily_inference_usd > current.budget.max_daily_inference_usd {
        warnings.push(ConfigWarning::BudgetIncrease {
            old: current.budget.max_daily_inference_usd,
            new: proposed.budget.max_daily_inference_usd,
        });
    }

    // Model changes are always allowed but logged
    if proposed.inference.default_model != current.inference.default_model {
        warnings.push(ConfigWarning::ModelChange {
            old: current.inference.default_model.clone(),
            new: proposed.inference.default_model.clone(),
        });
    }

    // Neuro path changes require restart
    if proposed.neuro.path != current.neuro.path {
        return Err(ConfigError::RequiresRestart("neuro.path"));
    }

    Ok(warnings)
}
```

---

## MCP Config Passthrough

Roko supports Model Context Protocol (MCP) configuration passthrough. If the operator specifies an MCP config in `roko.toml`, it is passed to the agent dispatch layer:

```toml
[agent]
mcp_config = "/path/to/mcp-config.json"
```

The MCP config is passed via `--mcp-config` to the underlying agent process, enabling integration with external MCP servers for tool augmentation.

---

## Related Topics

- `docs/17-lifecycle/01-agent-creation.md` — How config is initially generated
- `docs/17-lifecycle/04-funding-and-budgets.md` — Budget configuration details
- `docs/04-daimon/INDEX.md` — Daimon behavioral states and PAD computation
- `docs/05-dreams/INDEX.md` — Dream integration and PLAYBOOK.md generation
