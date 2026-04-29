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
demurrage_rate = 0.01          # 1% annual demurrage (planned)
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

REF23 adds a user-facing continuity rule above raw pause/resume mechanics: permissions and ambiguity decisions can be remembered within the current session, but that remembered state must remain session-scoped and explicitly clearable. `roko session forget` and `roko init --reset-permissions` therefore belong to the same lifecycle story as `pause` and `resume`, because they control what operational context carries forward when a user switches surfaces or resumes work later. See [../12-interfaces/21-user-ux-running-agents.md](../12-interfaces/21-user-ux-running-agents.md) and [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md).

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

## GitOps Configuration Management

For operators managing agent fleets, Roko supports a GitOps model where Git is the single source of truth for all agent configuration. This applies the four principles codified by the OpenGitOps specification (v1.0, 2021): **declarative**, **versioned and immutable**, **pulled automatically**, **continuously reconciled**.

### Architecture

```
Git Repository (source of truth)
  environments/
    production/
      roko.toml          # Agent infrastructure config
      STRATEGY.md         # Agent goals
      hooks/              # Lifecycle hooks
    staging/
      roko.toml
      STRATEGY.md
          │
          │  poll (1m interval)
          ▼
┌──────────────────┐
│  Config Watcher   │ ← Runs inside roko process (self-hosted)
│  (Reconciler)     │   or as control plane component (hosted)
└────────┬─────────┘
         │  compare desired vs actual
         │
    ┌────▼────────────┐
    │  Drift Detection │
    └────┬────────────┘
         │
    ┌────▼──────────────────────────┐
    │  Apply Changes / Self-Heal    │
    │  (hot-reload or restart)      │
    └───────────────────────────────┘
```

### GitOps Config Spec

```rust
/// GitOps configuration source, analogous to an ArgoCD Application
/// or Flux GitRepository + Kustomization.
///
/// When enabled, the agent polls the Git repository at `poll_interval`
/// and reconciles its configuration against the desired state.
///
/// Crate: `roko-core`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitOpsConfig {
    /// Git repository URL (HTTPS or SSH).
    pub repo_url: String,

    /// Branch, tag, or full commit SHA. Default: "main".
    pub target_revision: String,

    /// Relative path within the repo to this agent's config directory.
    pub path: String,

    /// How often to check for changes. Default: 60s.
    /// Minimum: 30s. Maximum: 3600s.
    /// ArgoCD default: 180s. Flux default: 60s.
    pub poll_interval: Duration,

    /// Auto-sync: automatically apply changes detected in Git.
    /// When false, changes are reported but require `roko config apply`.
    pub auto_sync: bool,

    /// Self-heal: revert manual config changes back to Git state.
    /// Requires `auto_sync: true`.
    /// ArgoCD equivalent: `spec.syncPolicy.automated.selfHeal`.
    pub self_heal: bool,

    /// Prune: remove config sections absent from the desired state.
    /// When false, orphaned config sections are preserved.
    /// Safety: defaults to false to prevent accidental deletion.
    pub prune: bool,

    /// Number of past config states to retain for rollback.
    /// Each state is identified by Git commit SHA.
    /// Default: 10. Range: 1-100.
    pub revision_history_limit: usize,

    /// Retry policy for failed sync attempts.
    pub retry: GitOpsRetryPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitOpsRetryPolicy {
    /// Maximum retry attempts. -1 = unlimited.
    pub limit: i32,
    /// Initial backoff duration.
    pub initial_backoff: Duration,
    /// Backoff multiplier.
    pub factor: f64,
    /// Maximum backoff duration.
    pub max_backoff: Duration,
}

/// Result of a drift detection pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfigDrift {
    /// Actual state matches desired state from Git.
    InSync { revision: String },
    /// Actual state diverges from desired state.
    Drifted {
        revision: String,
        diverged_keys: Vec<String>,
        last_known_good: String,
    },
    /// Git source unreachable (network error, auth failure).
    SourceUnreachable { error: String },
}

impl Default for GitOpsConfig {
    fn default() -> Self {
        Self {
            repo_url: String::new(),
            target_revision: "main".into(),
            path: ".".into(),
            poll_interval: Duration::from_secs(60),
            auto_sync: true,
            self_heal: true,
            prune: false,
            revision_history_limit: 10,
            retry: GitOpsRetryPolicy {
                limit: 5,
                initial_backoff: Duration::from_secs(5),
                factor: 2.0,
                max_backoff: Duration::from_secs(180),
            },
        }
    }
}
```

### TOML Configuration

```toml
[gitops]
enabled = true
repo_url = "https://github.com/org/agent-configs.git"
target_revision = "main"
path = "environments/production"
poll_interval_secs = 60
auto_sync = true
self_heal = true
prune = false
revision_history_limit = 10

[gitops.retry]
limit = 5
initial_backoff_secs = 5
factor = 2.0
max_backoff_secs = 180
```

### Reconciliation Algorithm

```
reconcile():
  1. fetch(repo_url, target_revision) → desired_state
  2. read_current_config() → actual_state
  3. diff = compare(desired_state, actual_state)

  4. if diff.is_empty():
       emit ConfigDrift::InSync { revision }
       return

  5. emit ConfigDrift::Drifted { diverged_keys }

  6. if !auto_sync:
       log_drift(diff)
       notify_operator(diff)
       return

  7. for key in diff.changed_keys:
       if is_hot_reloadable(key):
         hot_apply(key, desired_state[key])
       else:
         schedule_restart(key, desired_state[key])

  8. for key in diff.removed_keys:
       if prune:
         remove(key)
       else:
         log_orphan(key)

  9. save_revision_history(revision, actual_state)
 10. emit sync_success(revision)
```

### Rollback

```bash
# List available config revisions
roko config history

# Show diff between current and specific revision
roko config diff abc123f

# Rollback to a previous Git revision
roko config rollback abc123f

# Rollback with dry-run
roko config rollback abc123f --dry-run
```

Rollback replays a prior Git commit's config state through the same reconciliation pipeline. The rollback itself is recorded in history, creating an audit trail.

### Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitops_defaults_are_safe() {
        let config = GitOpsConfig::default();
        assert!(config.auto_sync);
        assert!(config.self_heal);
        assert!(!config.prune, "prune defaults to false for safety");
        assert_eq!(config.revision_history_limit, 10);
    }

    #[test]
    fn poll_interval_minimum_enforced() {
        let config = GitOpsConfig {
            poll_interval: Duration::from_secs(10), // below minimum
            ..Default::default()
        };
        // Runtime should clamp to 30s minimum
        let clamped = config.poll_interval.max(Duration::from_secs(30));
        assert_eq!(clamped, Duration::from_secs(30));
    }

    #[test]
    fn drift_detection_reports_diverged_keys() {
        let drift = ConfigDrift::Drifted {
            revision: "abc123".into(),
            diverged_keys: vec!["inference.default_model".into()],
            last_known_good: "def456".into(),
        };
        if let ConfigDrift::Drifted { diverged_keys, .. } = drift {
            assert_eq!(diverged_keys.len(), 1);
        }
    }

    #[test]
    fn retry_backoff_is_bounded() {
        let retry = GitOpsRetryPolicy {
            limit: 5,
            initial_backoff: Duration::from_secs(5),
            factor: 2.0,
            max_backoff: Duration::from_secs(180),
        };
        // After 5 doublings: 5 * 2^5 = 160s, still under 180s max
        let delay = retry.initial_backoff.as_secs_f64()
            * retry.factor.powi(5);
        assert!(delay <= retry.max_backoff.as_secs_f64());
    }
}
```

---

## Cross-References

- `docs/17-lifecycle/01-agent-creation.md` — How config is initially generated
- `docs/17-lifecycle/04-funding-and-budgets.md` — Budget configuration details
- `docs/04-daimon/INDEX.md` — Daimon behavioral states and PAD computation
- `docs/05-dreams/INDEX.md` — Dream integration and PLAYBOOK.md generation
