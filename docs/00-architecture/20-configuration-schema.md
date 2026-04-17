# Cross-system configuration schema

> Layer 0 Kernel -- Configuration Management
> Status: **Implemented** -- `RokoConfig` in `crates/roko-core/src/config/schema.rs` (~2,600 lines)
> Canonical source: `crates/roko-core/src/config/schema.rs`, `roko.toml`
> Cross-references: [15-crate-map.md](15-crate-map.md), [04-decay-variants.md](04-decay-variants.md), [25-attention-as-currency.md](25-attention-as-currency.md), [01-naming-and-glossary.md](01-naming-and-glossary.md)

> **Implementation**: Shipping

---

## Purpose

Roko has 60+ configurable parameters spread across 20+ config structs. This document catalogs every parameter, its default, valid range, and interdependencies. It serves as the single reference for `roko.toml` authors and for validation logic.

REF12 extends that canonical surface with a dedicated demurrage section for durable-memory economics. Even where the shipping schema still trails the architecture, the keys below are the intended contract for tuning `balance`, reinforcement, and cold-tier thresholds. See also [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md).

---

## 1. Schema structure

The `RokoConfig` struct is the root. Every section is a separate struct with serde defaults, so a bare `roko.toml` produces a fully populated config.

```rust
pub struct RokoConfig {
    pub config_version: u32,        // Migration tracking
    pub schema_version: u32,        // Migration tracking
    pub project: ProjectConfig,     // Project metadata
    pub prd: PrdConfig,             // PRD lifecycle
    pub agent: AgentConfig,         // Agent/model settings
    pub providers: HashMap<String, ProviderConfig>,  // Provider registry
    pub models: HashMap<String, ModelProfile>,       // Model registry
    pub gates: GatesConfig,         // Verification gates
    pub routing: RoutingConfig,     // Model routing
    pub pipeline: PipelineConfig,   // Complexity-to-pipeline mapping
    pub budget: BudgetConfig,       // Spend/token budgets
    pub conductor: ConductorConfig, // Meta-orchestrator
    pub watcher: WatcherConfig,     // File-system watcher
    pub learning: LearningConfig,   // Learning subsystem
    pub tui: TuiConfig,            // Terminal UI
    pub serve: ServeConfig,        // HTTP API
    pub scheduler: SchedulerConfig, // Cron scheduler
    pub webhooks: WebhooksConfig,   // Webhook ingress
    pub subscriptions: Vec<SubscriptionConfig>, // Event subscriptions
    pub server: ServerConfig,       // HTTP server/gateway
    pub deploy: DeployConfig,       // Cloud deployment
    pub perplexity: PerplexityConfig, // Perplexity-specific
    pub gemini: GeminiConfig,       // Gemini-specific
}
```

The next schema revision adds a top-level `[demurrage]` section. Treat that section as canonical for the architecture docs even if the shipping `RokoConfig` has not yet absorbed it.

---

## 2. Parameter catalog

### 2.1 Project (`[project]`)

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `name` | String | `"roko-project"` | any | Human-readable project name |
| `root` | String | `"."` | valid path | Project root directory |
| `fresh_base_branch` | String | `"main"` | any | Base branch for fresh checkouts |

### 2.2 PRD (`[prd]`)

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `auto_plan` | bool | `false` | -- | Generate plan when PRD is promoted |

### 2.3 Agent (`[agent]`)

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `default_model` | String | `"claude-sonnet-4-6"` | valid model slug | Default model for all roles |
| `default_backend` | String | `"claude"` | valid backend label | Default backend/provider family |
| `default_effort` | String | `"medium"` | low, medium, high, max | Default reasoning effort |
| `context_limit_k` | u32 | `200` | 1 - 1,000 | Context window limit in thousands of tokens |
| `roles` | HashMap\<String, RoleOverride\> | empty | -- | Per-role model/parameter overrides |

Per-role overrides (`[agent.roles.<name>]`):

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `role` | Option\<String\> | None (use section name) | any non-empty role label | Override the runtime role/contract label |
| `model` | Option\<String\> | None (use default) | valid model slug | Model override for this role |
| `backend` | Option\<String\> | None | valid backend label | Backend override for this role |
| `effort` | Option\<String\> | None | low, medium, high, max | Reasoning effort override |
| `context_limit_k` | Option\<u32\> | None | 1 - 1,000 | Context window override in thousands of tokens |
| `tools` | Option\<Vec\<String\>\> | None | tool names / globs | Role-local tool whitelist |
| `budget` | Option\<AgentBudget\> | None | -- | Per-turn token and cost caps |
| `thresholds` | Option\<AgentThresholds\> | None | -- | Adaptive gate-threshold overrides |
| `routing_overrides` | Option\<RoutingOverrides\> | None | -- | Force backend/tier during routing |
| `turn_budget_usd` | Option\<f32\> | None | 0.0+ | Legacy per-turn USD cap; folded into `budget` |

### 2.4 Providers (`[providers.<name>]`)

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `kind` | ProviderKind | required | anthropic, openai, openrouter, ollama, gemini, perplexity | Protocol family |
| `api_key_env` | Option\<String\> | None | env var name | Environment variable for API key |
| `base_url` | Option\<String\> | None | valid URL | Custom API endpoint |
| `max_retries` | u32 | `3` | 0 - 10 | Max retry attempts |
| `timeout_secs` | u64 | `300` | 10 - 3,600 | Request timeout |
| `rate_limit_rpm` | Option\<u32\> | None | 1 - 100,000 | Requests per minute limit |

### 2.5 Gates (`[gates]`)

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `clippy` | bool | `true` | -- | Enable clippy gate |
| `test` | bool | `true` | -- | Enable test gate |
| `compile` | bool | `true` | -- | Enable compile gate |
| `fmt` | bool | `false` | -- | Enable format check gate |
| `diff` | bool | `true` | -- | Enable diff review gate |
| `max_iterations` | u32 | `5` | 1 - 20 | Max retry iterations per task |
| `timeout_secs` | u64 | `300` | 30 - 3,600 | Gate execution timeout |

### 2.6 Routing (`[routing]`)

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `mode` | String | `"auto_override"` | "auto_override", "static", "bandit" | Routing strategy |
| `exploration_rate` | f32 | `0.1` | 0.0 - 1.0 | Thompson sampling exploration rate |
| `min_samples` | u32 | `10` | 1 - 100 | Min samples before bandit arms are trusted |
| `cost_weight` | f32 | `0.3` | 0.0 - 1.0 | Weight of cost in reward signal |
| `latency_weight` | f32 | `0.1` | 0.0 - 1.0 | Weight of latency in reward signal |
| `quality_weight` | f32 | `0.6` | 0.0 - 1.0 | Weight of pass rate in reward signal |

Constraint: `cost_weight + latency_weight + quality_weight` must equal 1.0.

### 2.7 Pipeline (`[pipeline]`)

Per-complexity-band settings (`[pipeline.trivial]`, `[pipeline.standard]`, `[pipeline.complex]`):

| Parameter | Type | Default (standard) | Range | Description |
|---|---|---|---|---|
| `strategist` | bool | `true` | -- | Run strategist stage before implementation |
| `reviewer` | bool | `true` | -- | Run reviewer stage after implementation |
| `max_iterations` | u32 | `5` | 1 - 20 | Max gate retry iterations |
| `model_tier` | String | `"sonnet"` | "haiku", "sonnet", "opus" | Default model tier |

### 2.8 Budget (`[budget]`)

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `max_plan_usd` | f32 | `50.0` | 0.01 - 10,000.0 | Max spend per plan |
| `max_session_usd` | f32 | `100.0` | 0.01 - 50,000.0 | Max spend per session |
| `max_task_usd` | f32 | `10.0` | 0.01 - 1,000.0 | Max spend per task |
| `warn_threshold` | f32 | `0.8` | 0.1 - 0.99 | Fraction of budget that triggers warning |
| `block_threshold` | f32 | `0.95` | 0.5 - 1.0 | Fraction of budget that blocks new tasks |

Constraint: `warn_threshold < block_threshold`.

### 2.9 Conductor (`[conductor]`)

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `max_agents` | u32 | `4` | 1 - 32 | Max concurrent agents |
| `circuit_breaker_threshold` | u32 | `5` | 1 - 50 | Failures before circuit opens |
| `circuit_breaker_reset_secs` | u64 | `300` | 30 - 3,600 | Seconds before half-open retry |
| `health_check_interval_secs` | u64 | `60` | 10 - 600 | Seconds between health checks |
| `enable_watchers` | bool | `true` | -- | Enable file system watchers |
| `enable_scheduler` | bool | `true` | -- | Enable cron scheduler |

### 2.10 Learning (`[learning]`)

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `auto_refresh_playbook` | bool | `true` | -- | Refresh playbook after successful tasks |
| `auto_extract_skills` | bool | `true` | -- | Extract skills from successful episodes |
| `episode_retention_days` | u32 | `30` | 1 - 365 | Days to retain episode logs |
| `pattern_min_frequency` | u32 | `3` | 2 - 50 | Min occurrences for trigram pattern |
| `experiment_min_samples` | u32 | `25` | 10 - 1,000 | Min samples per experiment variant |
| `adaptive_thresholds` | bool | `true` | -- | Enable EMA-based gate threshold adaptation |
| `cascade_router_persistence` | bool | `true` | -- | Persist cascade router state |

`episode_retention_days` remains a coarse cap for raw logs, but REF12 shifts durable-knowledge freshness away from fixed retention windows and toward demurrage-governed `balance` on Engrams, playbooks, and distilled heuristics.

### 2.10a Demurrage (`[demurrage]`, specified by REF12)

This section tunes the durable-memory attention economy. The values below come from [tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md) and are read alongside the glossary in [01-naming-and-glossary.md](01-naming-and-glossary.md).

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `flat_tax_per_day` | f64 | `0.01` | `>= 0.0` | Flat carrying cost `r` charged against an Engram's `balance` each day. |
| `exp_decay_per_day` | f64 | `0.005` | `>= 0.0` | Exponential term `β` that compounds demurrage as balance persists idle. |
| `min_balance` | f64 | `0.0` | `>= 0.0` | Floor below which an Engram becomes a candidate for cold-tier freeze. |
| `cited_bonus` | f64 | `0.05` | `>= 0.0` | Reinforcement added when other Engrams cite this Engram in lineage. |
| `retrieved_bonus` | f64 | `0.02` | `>= 0.0` | Reinforcement added when retrieval actually surfaces the Engram. |
| `gated_bonus` | f64 | `0.03` | `>= 0.0` | Reinforcement added when a gate compares against the Engram and it holds up. |
| `surprised_bonus` | f64 | `0.15` | `>= 0.0` | Novelty-heavy reinforcement for prediction error or informative surprise. |
| `agent_quoted_bonus` | f64 | `0.08` | `>= 0.0` | Reinforcement added when an agent explicitly quotes or references the Engram in output. |
| `policy_confidence_tax` | f64 | `0.002` | `>= 0.0` | Demurrage applied to learned Policy confidences so stale thresholds become challengeable again. |

Illustrative shape:

```toml
[demurrage]
flat_tax_per_day      = 0.01
exp_decay_per_day     = 0.005
min_balance           = 0.0
cited_bonus           = 0.05
retrieved_bonus       = 0.02
gated_bonus           = 0.03
surprised_bonus       = 0.15
agent_quoted_bonus    = 0.08
policy_confidence_tax = 0.002
```

### 2.11 TUI (`[tui]`)

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `refresh_rate_ms` | u32 | `250` | 50 - 5,000 | Dashboard refresh interval |
| `theme` | String | `"rosedust"` | "rosedust", "default" | Color theme |

### 2.12 Server (`[server]`)

| Parameter | Type | Default | Range | Description |
|---|---|---|---|---|
| `bind` | String | `"127.0.0.1:3000"` | valid socket addr | Listen address |
| `workers` | u32 | `4` | 1 - 64 | HTTP worker threads |
| `request_timeout_secs` | u64 | `30` | 5 - 300 | Request timeout |

---

## 3. Interdependencies

Some parameters constrain each other. The config loader validates these on startup:

| Constraint | Parameters | Validation |
|---|---|---|
| Budget ordering | `warn_threshold`, `block_threshold` | `warn < block` |
| Reward weights | `cost_weight`, `latency_weight`, `quality_weight` | Sum == 1.0 (within 0.01 tolerance) |
| Max agents vs budget | `max_agents`, `max_session_usd` | `max_agents * max_task_usd <= max_session_usd` (warning, not error) |
| Gate iterations vs pipeline | `gates.max_iterations`, `pipeline.*.max_iterations` | Pipeline value overrides gate default |
| Model availability | `agent.default_model`, provider entries | Default model's provider must exist in `providers` |
| Demurrage floor | `demurrage.min_balance` | Must be non-negative so freeze candidates are well-defined |
| Demurrage bonuses | `demurrage.*_bonus` | Bonuses should be non-negative; negative reinforcement belongs in scorer/gate outcomes, not config |

REF12 also changes interpretation, not just schema shape: `learning.episode_retention_days` manages raw log retention, while durable retrieval quality is expected to come from demurrage-driven `effective_weight` rather than a standalone decay multiplier.

---

## 4. Validation rules

```rust
impl RokoConfig {
    pub fn validate(&self) -> Vec<ConfigWarning> {
        let mut warnings = Vec::new();

        // Budget ordering
        if self.budget.warn_threshold >= self.budget.block_threshold {
            warnings.push(ConfigWarning::BudgetThresholdOrder);
        }

        // Reward weight sum
        let sum = self.routing.cost_weight
            + self.routing.latency_weight
            + self.routing.quality_weight;
        if (sum - 1.0).abs() > 0.01 {
            warnings.push(ConfigWarning::RewardWeightSum { actual: sum });
        }

        // Default model has a provider
        if !self.providers.is_empty() {
            let has_provider = self.providers.values()
                .any(|p| p.models_include(&self.agent.default_model));
            if !has_provider {
                warnings.push(ConfigWarning::OrphanDefaultModel {
                    model: self.agent.default_model.clone(),
                });
            }
        }

        warnings
    }
}
```

---

## 5. Runtime overrides

Parameters can be overridden at runtime via three mechanisms, in priority order:

| Priority | Mechanism | Scope | Persistence |
|---|---|---|---|
| 1 (highest) | CLI flags | Single invocation | None |
| 2 | Environment variables | Session | None |
| 3 (lowest) | `roko.toml` | Persistent | On disk |

### 5.1 CLI flag mapping

```bash
roko plan run plans/ --max-agents 8 --max-plan-usd 200.0
```

Maps to: `conductor.max_agents = 8`, `budget.max_plan_usd = 200.0`.

### 5.2 Environment variable mapping

```bash
ROKO_CONDUCTOR_MAX_AGENTS=8 roko plan run plans/
```

Convention: `ROKO_` prefix, section and field joined by `_`, all uppercase.

### 5.3 Override resolution

```
fn resolve(field: &str) -> Value {
    if let Some(v) = cli_flag(field) { return v; }
    if let Some(v) = env_var(field)  { return v; }
    return toml_value(field);
}
```

---

## 6. Schema versioning

The config carries two version numbers:

| Field | Purpose | Current |
|---|---|---|
| `config_version` | Layout version for migration tooling | 2 |
| `schema_version` | Semantic version for the parameter set | 2 |

When a breaking change is introduced (renamed field, changed default, removed parameter), bump `schema_version` and add a migration function:

```rust
fn migrate_v1_to_v2(old: &toml::Value) -> Result<toml::Value, MigrationError> {
    // Rename [agent.model] -> [agent.default_model]
    // Move [agent.mcp] -> [agent.mcp_config]
    // ...
}
```

The config loader checks `schema_version` on load and runs migrations sequentially if needed.

---

## 7. Minimal config example

A bare-minimum `roko.toml` that uses all defaults:

```toml
config_version = 2
schema_version = 2

[project]
name = "my-project"
```

Every other section uses serde defaults. The system is fully functional with just a project name.

---

## 8. Full config example

```toml
config_version = 2
schema_version = 2

[project]
name = "roko"
root = "."
fresh_base_branch = "main"

[prd]
auto_plan = false

[agent]
default_model = "claude-sonnet-4-6"
max_turns = 200
max_tool_calls = 100

[agent.roles.implementer]
model = "claude-opus-4-6"

[agent.roles.reviewer]
model = "claude-sonnet-4-6"

[providers.anthropic]
kind = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"
max_retries = 3
timeout_secs = 300

[gates]
clippy = true
test = true
compile = true
diff = true
max_iterations = 5

[routing]
mode = "auto_override"
exploration_rate = 0.1
cost_weight = 0.3
latency_weight = 0.1
quality_weight = 0.6

[budget]
max_plan_usd = 50.0
max_session_usd = 100.0
max_task_usd = 10.0
warn_threshold = 0.8
block_threshold = 0.95

[conductor]
max_agents = 4
circuit_breaker_threshold = 5

[learning]
auto_refresh_playbook = true
auto_extract_skills = true
adaptive_thresholds = true
```

---

## 9. Error handling

| Condition | Response |
|---|---|
| Missing `roko.toml` | Use `RokoConfig::default()` with all serde defaults |
| Malformed TOML | Return parse error with line number and column |
| Unknown field | Warn but do not fail (forwards compatibility) |
| Wrong type for field | Return type mismatch error with expected and actual types |
| Schema version mismatch | Run migration chain; fail if no migration path exists |
| Validation warning | Print warning, continue with config as-is |
| Validation error (e.g., negative budget) | Refuse to start; print error |

---

## 10. Test criteria

1. `RokoConfig::default()` passes `validate()` with zero warnings.
2. Minimal TOML (just `[project] name = "x"`) deserializes and validates.
3. Full TOML with all sections deserializes and validates.
4. Budget threshold violation (`warn >= block`) produces a validation warning.
5. Reward weight sum != 1.0 produces a validation warning.
6. CLI flag overrides TOML value for the same parameter.
7. Environment variable overrides TOML value for the same parameter.
8. CLI flag overrides environment variable for the same parameter.
9. Unknown TOML field does not cause deserialization failure.
10. Schema version migration from v1 to v2 produces a valid v2 config.

---

## Cross-References

- [15-crate-map.md](15-crate-map.md) -- Crate layout including roko-core
- `crates/roko-core/src/config/schema.rs` -- All config structs and defaults
- `crates/roko-core/src/config/` -- Config loading, validation, migration
- `crates/roko-cli/src/` -- CLI flag definitions that map to config overrides
