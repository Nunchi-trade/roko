# 19 -- Configuration

> Configuration is a Signal. `Kind::Config` carries content-addressed, versioned, lineage-tracked, demurrage-decayed configuration state. Runtime overrides resolve through a Compose Cell. Schema validation runs as a Verify Cell. Hot reload fires from a Trigger Cell. The same five primitives that govern every other subsystem govern configuration.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal, Kind, content addressing, demurrage), [02-CELL](02-CELL.md) (9 protocols, Compose, Verify, Trigger), [03-GRAPH](03-GRAPH.md) (Graph composition), [13-TRIGGERS](13-TRIGGERS.md) (Trigger Cell, file watcher)

---

## 1. Config as Signal

Configuration is not special-cased infrastructure. It is data that:
- has a content hash (identical configs produce the same SHA-256),
- has a version (`schema_version` tracks breaking changes),
- has lineage (each config derives from its predecessor plus an override source),
- decays (stale config is worse than no config -- a six-month-old routing weight is probably wrong).

Configuration is a Signal with `Kind::Config`. It participates in the same Store, Bus, and protocol system as every other Signal. Config Signals are content-addressed, carry lineage, can be queried by HDC similarity, and are subject to demurrage.

```rust
/// Configuration as a Signal.
///
/// A config Signal carries the full or partial configuration state.
/// Content-addressed: the same config values produce the same hash.
/// Versioned: schema_version tracks breaking changes.
/// Lineage: parent_hashes point to the config(s) it was derived from.
pub fn config_signal(config: &RokoConfig, source: ConfigSource) -> Signal {
    Signal {
        kind: Kind::Config,
        payload: serde_json::to_value(config).expect("config is serializable"),
        metadata: SignalMetadata {
            schema_version: config.schema_version,
            source: source.to_string(),
            hash: ContentHash::compute(&canonical_bytes(config)),
            parent_hashes: vec![],  // set by the Compose protocol
            demurrage_balance: 1.0,
            ..Default::default()
        },
    }
}
```

---

## 2. ConfigSource and Priority

Every config value has a provenance. Sources are ordered by priority -- higher-priority sources override lower ones for the same field.

```rust
/// Where a config value came from, in priority order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// Priority 0: L4 structural adaptation proposed this value.
    /// Lowest -- evolution is a suggestion, not a mandate.
    Evolved { proposal_id: String },

    /// Priority 1: the roko.toml file on disk.
    TomlFile(PathBuf),

    /// Priority 2: environment variable.
    EnvVar(String),

    /// Priority 3: CLI flag.
    CliFlag(String),

    /// Priority 4: runtime override via HTTP API.
    ApiOverride { principal: String },
}

impl ConfigSource {
    fn priority(&self) -> u8 {
        match self {
            ConfigSource::Evolved { .. } => 0,
            ConfigSource::TomlFile(_) => 1,
            ConfigSource::EnvVar(_) => 2,
            ConfigSource::CliFlag(_) => 3,
            ConfigSource::ApiOverride { .. } => 4,
        }
    }
}
```

Resolution: `CLI (3) > Env (2) > TOML (1) > Evolved (0)`. Each field resolves independently -- the highest-priority source providing a value for that field wins. API overrides (priority 4) are runtime-only and do not persist to disk.

### Environment Variable Convention

```
ROKO_SECTION_FIELD  ->  section.field

ROKO_CONDUCTOR_MAX_AGENTS=8       -> conductor.max_agents = 8
ROKO_BUDGET_MAX_PLAN_USD=200      -> budget.max_plan_usd = 200.0
ROKO_ROUTING_COST_WEIGHT=0.4      -> routing.cost_weight = 0.4
ROKO_AGENT_DEFAULT_MODEL=opus     -> agent.default_model = "opus"
```

Convention: `ROKO_` prefix, section and field joined by `_`, all uppercase. `${VAR}` expansion inside TOML string values also supported: `rpc_url = "${ETH_RPC_URL}"`.

---

## 3. Config Compose Cell

The three override sources (CLI > env > TOML > evolved) are a **Compose protocol** (02-CELL.md) that merges config Signals by priority. The Compose Cell assembles a composite config from multiple partial inputs.

```rust
/// Compose Cell: merges config Signals by priority.
///
/// Input: up to 4 config Signals (evolved, toml, env, cli), each partial.
/// Output: 1 merged config Signal with full lineage.
///
/// Priority: API > CLI > Env > TOML > Evolved > Default.
/// Each field resolves independently: the highest-priority source
/// that provides a value for that field wins.
pub struct ConfigComposeCell;

impl Cell for ConfigComposeCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Compose] }
    fn name(&self) -> &str { "config-compose" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut sources = input.into_iter()
            .map(|s| {
                let source: ConfigSource = extract_source(&s)?;
                let priority = source.priority();
                Ok((priority, s))
            })
            .collect::<Result<Vec<_>, CellError>>()?;
        sources.sort_by(|a, b| b.0.cmp(&a.0));

        // Start with compiled defaults
        let mut merged = RokoConfig::default();

        // Apply in reverse priority order (lowest first, highest wins)
        let mut parent_hashes = Vec::new();
        for (_, signal) in sources.iter().rev() {
            apply_partial_config(&mut merged, &signal.payload)?;
            parent_hashes.push(signal.hash());
        }

        // Build merged config Signal with lineage
        let mut result = config_signal(&merged, ConfigSource::Composed);
        result.metadata.parent_hashes = parent_hashes;

        Ok(vec![result])
    }
}
```

### Resolution Algorithm

```rust
fn resolve_field(field: &str, sources: &[ConfigSignal]) -> Value {
    // Sources sorted by priority (highest first)
    for source in sources {
        if let Some(value) = source.get_field(field) {
            return value;
        }
    }
    // Fall through to compiled default
    RokoConfig::default().get_field(field)
}
```

The merged config Signal records all parent hashes. To trace where a value came from: walk the lineage. To see what changed: diff the current config Signal against its parent. The same lineage system used for every other Signal.

---

## 4. Config Verify Cell

Configuration validation is a **Verify protocol Cell** (02-CELL.md). It takes a config Signal as input and emits a Verdict Signal.

```rust
/// Verify Cell: config schema validation.
///
/// Checks 7 invariants plus type correctness, provider existence,
/// schema version compatibility, and unknown field detection.
pub struct ConfigVerifyCell;

impl Cell for ConfigVerifyCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn name(&self) -> &str { "config-verify" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let config: RokoConfig = serde_json::from_value(input[0].payload.clone())?;
        let mut warnings = Vec::new();
        let mut errors = Vec::new();

        // Check all 7 invariants (see section 4.1)
        validate_invariants(&config, &mut errors, &mut warnings);

        // Schema version migration check
        if config.schema_version < CURRENT_SCHEMA_VERSION {
            warnings.push(format!(
                "Config schema version {} is behind current {}; migration available",
                config.schema_version, CURRENT_SCHEMA_VERSION
            ));
        }

        let passed = errors.is_empty();
        let verdict = Signal::new(
            Kind::Verdict,
            ConfigVerdict { passed, errors, warnings },
        );
        Ok(vec![verdict])
    }
}
```

### 4.1 The 7 Invariants

| # | Invariant | Parameters | Rule |
|---|---|---|---|
| 1 | Budget ordering | `warn_threshold`, `block_threshold` | `warn < block` |
| 2 | Reward weight sum | `cost_weight + latency_weight + quality_weight` | `== 1.0` (tolerance 0.01) |
| 3 | Agent capacity vs budget | `max_agents * max_task_usd` | `<= max_session_usd` (warning, not error) |
| 4 | Gate iterations hierarchy | `gates.max_iterations`, `pipeline.*.max_iterations` | Pipeline overrides gate |
| 5 | Provider existence | `agent.default_model` | Provider for this model must exist in `[providers]` |
| 6 | Demurrage floor | `demurrage.min_balance` | `>= 0.0` |
| 7 | Demurrage bonuses | `demurrage.*_bonus` | `>= 0.0` (negative reinforcement belongs in Verify, not config) |

### 4.2 Error Handling

| Condition | Response |
|---|---|
| Missing `roko.toml` | Use `RokoConfig::default()` -- system is functional with just defaults |
| Malformed TOML | Parse error with line/column. Refuse to start. |
| Unknown field | Warn but continue (forward compatibility) |
| Wrong type | Type mismatch error with expected vs actual |
| Schema version mismatch | Run migration chain; fail if no migration path |
| Validation warning | Log warning, continue |
| Validation error | Refuse to start, print error |

---

## 5. Config Watch Trigger

When `roko.toml` changes on disk, the system reloads without restart. This is a **Trigger Cell** ([13-TRIGGERS](13-TRIGGERS.md)) watching the config file.

```rust
/// Trigger Cell: watches roko.toml for changes and triggers reload.
///
/// When the file changes:
/// 1. Read the new config from disk.
/// 2. Create a new Config Signal with TomlFile source.
/// 3. Route through ConfigComposeCell (merge with env and CLI).
/// 4. Route through ConfigVerifyCell.
/// 5. If verification passes: publish on "config.reloaded" topic.
/// 6. If verification fails: publish "config.reload_failed" with errors.
///    The old config remains active.
pub struct ConfigWatchTrigger {
    watch_path: PathBuf,
    debounce_ms: u64,  // default: 500ms
}

impl Cell for ConfigWatchTrigger {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Trigger] }
    fn name(&self) -> &str { "config-watch-trigger" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let event = extract_file_event(&input[0])?;
        if event.path != self.watch_path {
            return Ok(vec![]);
        }

        // Debounce: skip if last reload was too recent
        if event.elapsed_since_last < Duration::from_millis(self.debounce_ms) {
            return Ok(vec![]);
        }

        // Read new config
        let toml_content = tokio::fs::read_to_string(&self.watch_path).await?;
        let new_config: RokoConfig = toml::from_str(&toml_content)?;
        let config_signal = config_signal(
            &new_config,
            ConfigSource::TomlFile(self.watch_path.clone()),
        );

        // The Graph routes this through Compose -> Verify -> publish
        Ok(vec![config_signal])
    }
}
```

### 5.1 The Config Reload Graph

```toml
# Graph: config hot reload pipeline
[graph]
name = "config-reload"
description = "Watch roko.toml, merge overrides, validate, publish"

[[graph.node]]
id   = "watch"
kind = "block"
block = "config-watch-trigger"

[[graph.node]]
id   = "compose"
kind = "block"
block = "config-compose"

[[graph.node]]
id   = "verify"
kind = "block"
block = "config-verify"

[[graph.edge]]
from = "watch"
to   = "compose"
[[graph.edge.maps]]
from = "out"
to   = "toml_input"

[[graph.edge]]
from = "compose"
to   = "verify"

# After verification, the config Signal is published on the Bus.
# All Cells that depend on config subscribe to "config.reloaded".
```

---

## 6. Schema Versioning and Migration

Config carries two version numbers:

| Field | Purpose | Current |
|---|---|---|
| `config_version` | Layout version for migration tooling | 2 |
| `schema_version` | Semantic version for the parameter set | 2 |

When a breaking change is introduced (renamed field, changed default, removed parameter), bump `schema_version` and add a migration function.

```rust
/// Config migration as a Cell.
///
/// Takes a config Signal at schema version N and produces a config Signal
/// at schema version N+1. Migrations are chained: v1 -> v2 -> v3.
pub struct ConfigMigrateCell {
    migrations: BTreeMap<u32, Box<dyn Fn(&toml::Value) -> Result<toml::Value, MigrationError>>>,
}

impl Cell for ConfigMigrateCell {
    fn name(&self) -> &str { "config-migrate" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let signal = &input[0];
        let mut config_value: toml::Value = extract_toml(&signal)?;
        let mut current_version = extract_schema_version(&config_value);

        while current_version < CURRENT_SCHEMA_VERSION {
            let migrator = self.migrations.get(&current_version)
                .ok_or(CellError::NoMigrationPath {
                    from: current_version,
                    to: CURRENT_SCHEMA_VERSION,
                })?;
            config_value = migrator(&config_value)?;
            current_version += 1;
        }

        // Migrated config is a new Signal with lineage to the old one
        let new_config: RokoConfig = toml::from_str(&config_value.to_string())?;
        let mut result = config_signal(&new_config, ConfigSource::Migrated);
        result.metadata.parent_hashes = vec![signal.hash()];

        Ok(vec![result])
    }
}
```

| Version | Format | Notes |
|---|---|---|
| `config_version = 1` | Legacy Mori format | Warns on load, suggests `roko config migrate` |
| `config_version = 2` | Current unified schema | Default for new workspaces |

---

## 7. Minimal Config

A bare-minimum config Signal:

```toml
config_version = 2
schema_version = 2

[project]
name = "my-project"
```

Every other section uses `#[serde(default)]` defaults. The system is fully functional with just a project name. The config Compose Cell fills in defaults for any field not specified.

---

## 8. Full Section Reference

The canonical source is `crates/roko-core/src/config/schema.rs`. All types derive `Serialize + Deserialize`.

### 8.1 `[project]` -- ProjectConfig

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | String | `"roko-project"` | Workspace name |
| `root` | String | `"."` | Workspace root path |
| `fresh_base_branch` | String | `"main"` | Base branch for worktree operations |
| `default_domain` | Option\<String\> | None | Default task domain |

### 8.2 `[server]` -- ServerConfig

| Field | Type | Default | Description |
|---|---|---|---|
| `bind` | String | `"127.0.0.1"` | Bind address |
| `port` | u16 | `6677` | HTTP port |
| `cors_origins` | Vec\<String\> | `[]` | Allowed CORS origins (empty = permissive) |
| `auth_token` | Option\<String\> | None | Legacy single auth token |

### 8.3 `[serve]` -- ServeConfig

| Field | Type | Default | Description |
|---|---|---|---|
| `port` | Option\<u16\> | None | Override port (falls back to `server.port`) |
| `auto_orchestrate` | bool | `true` | Auto-start orchestration on plan execution |

#### `[serve.auth]` -- ServeAuthConfig

| Field | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Enable authentication middleware |
| `api_key` | String | `""` | Legacy single API key (use `api_keys` instead) |
| `api_keys` | Vec\<ApiKeyEntry\> | `[]` | Named scoped API keys |
| `privy_app_id` | Option\<String\> | None | Privy app ID for JWT validation |

```toml
[[serve.auth.api_keys]]
name = "dashboard"
key_hash = "sha256:..."    # SHA-256 hex of plaintext key
scope = "admin"            # "read" | "agent:write" | "plan:write" | "admin"
created_at = "2026-04-20T00:00:00Z"
expires_at = "2027-04-20T00:00:00Z"  # optional
```

### 8.4 `[agent]` -- AgentConfig

| Field | Type | Default | Description |
|---|---|---|---|
| `default_model` | String | `"claude-sonnet-4-6"` | Default LLM model |
| `default_backend` | String | `"claude"` | Default provider backend |
| `default_effort` | String | `"medium"` | Task effort level |
| `context_limit_k` | u32 | `200` | Context window limit (K tokens) |
| `bare_mode` | bool | `true` | Run agents in bare mode (no MCP) |
| `fallback_model` | Option\<String\> | None | Fallback when primary unavailable |
| `extensions` | Vec\<String\> | `[]` | Default extension chain |
| `domain` | Option\<String\> | None | Default domain profile |
| `mode` | AgentMode | `Ephemeral` | `ephemeral` / `persistent` / `reactive` |

#### `[agent.roles.<name>]` -- per-role overrides

```toml
[agent.roles.reviewer]
model = "claude-haiku-4-5"
effort = "low"
turn_budget_usd = 0.5
```

Available override fields: `model`, `backend`, `effort`, `temperament`, `context_limit_k`, `tools`, `budget`, `thresholds`, `routing_overrides`, `turn_budget_usd`.

#### `[agent.data_llm]` -- DataLlmConfig

| Field | Type | Default | Description |
|---|---|---|---|
| `model` | String | `"claude-haiku-3-5"` | Model for data extraction |
| `max_tokens` | u64 | `4096` | Output token limit |
| `temperature` | f64 | `0.0` | Temperature (0 = deterministic) |
| `strip_tool_calls` | bool | `true` | Remove tool calls from output |
| `sanitize_input` | bool | `true` | Sanitize inputs before sending |

### 8.5 `[[agents]]` -- agent definitions

```toml
[[agents]]
name = "coder-1"
domain = "coding"
prompt = "Implement features and fix bugs"
model = "claude-sonnet-4-6"
enabled = true
```

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | String | *required* | Unique agent name |
| `domain` | String | *required* | `"coding"` / `"research"` / `"chain"` / `"general"` |
| `prompt` | String | `""` | Agent purpose description |
| `model` | Option\<String\> | None | Override model |
| `chain_rpc` | Option\<String\> | None | Chain RPC for chain agents |
| `enabled` | bool | `true` | Enable/disable |

### 8.6 `[providers]` -- LLM provider backends

```toml
[providers.anthropic]
kind = "anthropic_api"
api_key_env = "ANTHROPIC_API_KEY"
max_concurrent = 50

[providers.ollama]
kind = "ollama"
base_url = "http://localhost:11434"
```

| Field | Type | Default | Description |
|---|---|---|---|
| `kind` | ProviderKind | *required* | `anthropic_api` / `claude_cli` / `openai_compat` / `cursor_acp` / `gemini_api` / `perplexity_api` / `ollama` / `codex` / `openai` |
| `base_url` | Option\<String\> | None | API endpoint |
| `api_key_env` | Option\<String\> | None | Env var for API key |
| `command` | Option\<String\> | None | CLI binary (subprocess providers) |
| `timeout_ms` | Option\<u64\> | `120_000` | Request timeout |
| `ttft_timeout_ms` | Option\<u64\> | `15_000` | Time-to-first-token timeout |
| `connect_timeout_ms` | Option\<u64\> | `5_000` | TCP connection timeout |
| `max_concurrent` | Option\<u32\> | None | Concurrency limit |

### 8.7 `[models]` -- model profiles

```toml
[models.claude-sonnet]
provider = "anthropic"
slug = "claude-sonnet-4-6-20250514"
context_window = 200000
supports_tools = true
supports_thinking = true
supports_caching = true
cost_input_per_m = 3.0
cost_output_per_m = 15.0
```

| Field | Type | Default | Description |
|---|---|---|---|
| `provider` | String | *required* | Key into `[providers.*]` |
| `slug` | String | *required* | Model ID for API calls |
| `context_window` | u64 | `128_000` | Max context tokens |
| `max_output` | Option\<u64\> | None | Max output tokens |
| `supports_tools` | bool | `true` | Tool/function calling |
| `supports_thinking` | bool | `false` | Extended reasoning |
| `supports_vision` | bool | `false` | Image inputs |
| `supports_caching` | bool | `false` | Provider-side caching |
| `cost_input_per_m` | Option\<f64\> | None | $/M input tokens |
| `cost_output_per_m` | Option\<f64\> | None | $/M output tokens |

### 8.8 `[routing]` -- model routing

Controls the CascadeRouter (LinUCB bandit) for automatic model selection:

| Field | Type | Default | Description |
|---|---|---|---|
| `mode` | String | `"auto_override"` | Routing mode |
| `algorithm` | String | `"linucb"` | `linucb` / `thompson` |
| `discount_factor` | f64 | `0.99` | Temporal discount |
| `fast_task_model` | String | `"claude-haiku-4-5"` | T0 reflex model |
| `standard_task_model` | String | `"claude-sonnet-4-6"` | T1 reflective model |
| `complex_task_model` | String | `"claude-opus-4-6"` | T2 deliberate model |

#### `[routing.weights]`

| Field | Type | Default | Description |
|---|---|---|---|
| `quality` | f64 | `0.5` | Weight for gate pass rate |
| `cost` | f64 | `0.3` | Weight for cost efficiency |
| `latency` | f64 | `0.2` | Weight for response speed |

Per-complexity overrides: `[routing.weights.mechanical]`, `[routing.weights.focused]`, `[routing.weights.integrative]`, `[routing.weights.architectural]`.

### 8.9 `[gates]` -- gate pipeline

| Field | Type | Default | Description |
|---|---|---|---|
| `clippy_enabled` | bool | `true` | Run clippy gate |
| `skip_tests` | bool | `false` | Skip test gate |
| `max_iterations` | u32 | `3` | Max retry iterations on gate failure |
| `domain_gates` | HashMap | `{}` | Per-domain custom gate lists |

### 8.10 `[pipeline]` -- execution pipeline per complexity

| Tier | strategist | reviewers | reviewer_mode | max_iterations |
|---|---|---|---|---|
| mechanical | false | false | quick | 1 |
| focused | false | false | quick | 2 |
| integrative | true | true | quick | 2 |
| architectural | true | true | full | 3 |

### 8.11 `[budget]` -- cost limits

| Field | Type | Default | Description |
|---|---|---|---|
| `max_plan_usd` | f32 | `25.0` | Max cost per plan execution |
| `max_turn_usd` | f32 | `3.0` | Max cost per agent turn |
| `prompt_token_budget` | usize | `10_000` | Max prompt tokens |

### 8.12 `[conductor]` -- orchestration control

| Field | Type | Default | Description |
|---|---|---|---|
| `max_agents` | usize | `8` | Max concurrent agents |
| `max_parallel_plans` | usize | `1` | Max parallel plan executions |
| `parallel_enabled` | bool | `false` | Enable parallel task execution |
| `express_mode` | bool | `false` | Skip strategist for quick fixes |
| `max_auto_fix_attempts` | u32 | `3` | Auto-fix retries before replan |
| `auto_fix_model` | String | `"claude-haiku-4-5"` | Model for auto-fix attempts |
| `warm_implementers_per_plan` | usize | `1` | Pre-spawned warm agents |

### 8.13 `[learning]` -- learning and feedback

| Field | Type | Default | Description |
|---|---|---|---|
| `auto_playbook_refresh` | bool | `true` | Auto-update playbook rules |
| `knowledge_file_intel` | bool | `true` | Include file intel in context |
| `knowledge_warnings` | bool | `true` | Include warnings in context |
| `knowledge_wave_context` | bool | `true` | Include sibling task context |
| `knowledge_error_patterns` | bool | `true` | Include error patterns in context |
| `file_intel_max_entries` | usize | `15` | Max file intel entries per prompt |
| `warning_max_entries` | usize | `5` | Max warning entries per prompt |
| `replan_on_gate_failure` | bool | `true` | Trigger replan on gate failure |
| `replan_max_per_plan` | u32 | `2` | Max replans per plan |
| `replan_gate_attempts` | u32 | `3` | Gate attempts before replan |

### 8.14 `[demurrage]` -- signal decay

| Field | Type | Default | Description |
|---|---|---|---|
| `rate_per_hour` | f64 | `0.01` | Decay rate per hour |
| `min_balance` | f64 | `0.1` | Minimum signal balance |
| `freeze_threshold` | f64 | `0.05` | Balance below which signal freezes |
| `freeze_before_delete` | bool | `true` | Freeze before garbage collection |

### 8.15 Additional Sections

| Section | Key Fields | Notes |
|---|---|---|
| `[chain]` | `rpc_url`, `chain_id`, `wallet_key`, `agent_registry`, `bounty_market` | Blockchain integration |
| `[relay]` | `url`, `workspace_name`, `heartbeat_interval_secs` | Relay connection |
| `[energy]` | `pool_usd`, `per_task_cap_usd`, `metabolism_rate` | Cognitive energy model |
| `[attention]` | `max_tokens_per_layer`, `utilization_target`, `auction_enabled` | Context budget |
| `[tui]` | `refresh_rate_ms` | Terminal UI |
| `[deploy]` | `backend`, `railway_api_token`, `project_id`, `worker_image` | Cloud deployment |
| `[prd]` | `auto_plan` | PRD lifecycle |
| `[tools]` | `allow`, `deny`, `profiles.<name>` | Tool permissions |
| `[[subscriptions]]` | `template`, `trigger`, `concurrency_limit`, `cooldown_secs` | Event subscriptions |
| `[[scheduler.cron]]` | `name`, `expression`, `signal_kind` | Scheduled events |

---

## 9. Domain Profiles as Cognitive Postures

A domain profile is a **complete cognitive posture** -- not a string label like `"coding"`. It bundles clock configuration, extensions, wakeup events, context weights, gate configuration, and infrastructure settings into a coherent whole.

### 9.1 Profile Schema

```rust
pub struct DomainProfile {
    pub name: String,
    pub version: Version,
    pub description: String,
    pub base: Option<String>,           // extends another profile
    pub clock: ClockConfig,
    pub extensions: ExtensionConfig,
    pub wakeup: WakeupConfig,
    pub context_weights: ContextWeights,
    pub gates: GateConfig,
    pub infrastructure: InfraConfig,
    pub models: ModelConfig,
}

pub struct ClockConfig {
    pub gamma_ms: u64,                  // perception tick
    pub theta_ms: u64,                  // inference tick
    pub delta_ms: u64,                  // reflection tick
    pub regime: Regime,                 // Calm / Normal / Volatile / Crisis
}

pub struct ContextWeights {
    pub neuro: f64,                     // knowledge store context
    pub task: f64,                      // task-specific context
    pub research: f64,                  // research findings
    pub heuristic: f64,                 // learned heuristics
    pub episode: f64,                   // past episodes
    pub pheromone: f64,                 // stigmergic signals
    pub affect: f64,                    // somatic markers
    pub system: f64,                    // system instructions
}
```

### 9.2 Profile Comparison

| Dimension | Coding | Security Audit | Research | Trading |
|---|---|---|---|---|
| **Clock (gamma)** | 200ms | 300ms | 500ms | 100ms |
| **Clock (theta)** | 3000ms | 5000ms | 10000ms | 1000ms |
| **Extensions** | git, compiler, test-runner | vuln-scanner, dep-audit | web-search, citation-check | chain-reader, risk-mgr |
| **Wakeup** | Code changes, PR events | Scheduled scans, vuln feeds | Manual trigger, cron | Price ticks, chain events |
| **Context** | High task (0.4) | High research (0.3) | High research (0.4) | High task (0.4), high neuro (0.3) |
| **Gates** | compile, test, clippy | vuln_scan, llm_judge, diff | fact-check, citation | risk, position-limit |
| **Budget** | $10, ephemeral | $25, persistent | $15, ephemeral | $50, persistent |

### 9.3 Profile Inheritance

Profiles extend other profiles via `base`. Deep merge: arrays are concatenated (extensions), objects are merged (gates config), scalars are overridden (clock values).

```toml
[profile]
name = "defi-security-audit"
base = "security-audit"           # inherits all settings

[profile.extensions]
enabled = ["chain-reader", "slither-analyzer"]   # added on top of base

[profile.gates]
vuln_scan = { severity = "low" }                 # stricter threshold
```

---

## 10. Config Evolution in L4

Configuration is evolvable. In L4 structural adaptation (07-LEARNING.md), the system proposes config changes based on observed outcomes.

```rust
/// L4 config evolution: the system proposes config changes.
///
/// Flow:
/// 1. L4 StructuralAdaptation observes suboptimal parameters.
/// 2. Generates a ConfigProposal Signal.
/// 3. Proposal goes through ConfigVerifyCell.
/// 4. If valid, enters human approval queue.
/// 5. If approved, new Config Signal emitted with source Evolved.
/// 6. Evolved has LOWEST priority -- it is a suggestion, not a mandate.
pub struct ConfigProposal {
    pub changes: BTreeMap<String, serde_json::Value>,
    pub rationale: String,
    pub evidence: Vec<ContentHash>,
    pub expected_improvement: String,
}
```

### 10.1 Config Demurrage

Stale config loses balance. When a config Signal's balance drops below threshold, the system emits a warning Pulse:

```rust
pub fn config_demurrage_check(config_signal: &Signal, now: Instant) -> Option<Signal> {
    let age_days = (now - config_signal.created_at).as_secs_f64() / 86400.0;
    let balance = config_signal.metadata.demurrage_balance;

    if balance < CONFIG_STALE_THRESHOLD && age_days > 30.0 {
        Some(Signal::pulse(
            Kind::Alert,
            topic!("config.stale_warning"),
            ConfigStaleWarning {
                section: extract_section(&config_signal),
                age_days,
                balance,
                recommendation: "Review this config section or re-validate".into(),
            },
        ))
    } else {
        None
    }
}
```

### 10.2 Feedback Loops per Learning Level

| Level | What happens to config |
|---|---|
| **L1** | Adaptive threshold tuning adjusts gate parameters within declared ranges |
| **L2** | CascadeRouter adjusts routing weights based on outcomes; config provides initial values |
| **L3** | Delta consolidation reviews which config values have been overridden most often, proposes permanent changes |
| **L4** | Full config evolution proposals based on KPI trends |

---

## 11. Configuration Hierarchy (Three Layers)

Three layers, deep-merged:

1. **Workspace**: `<workspace>/workspace.toml` or `roko.toml` -- top precedence
2. **User**: `~/.roko/config.toml` -- middle
3. **Built-in defaults** -- bottom

CLI flags override config. Environment variables (`ROKO_*`) override config but are overridden by flags.

### 11.1 Workspace Scoping

Roko supports multi-workspace operation. A single daemon can serve multiple workspaces, each with its own capability grants, knowledge scope, and resource limits.

```toml
# ~/.roko/daemon.toml
[daemon]
port = 6677
workspaces = [
  { path = "/Users/will/dev/nunchi/roko/roko",   name = "roko" },
  { path = "/Users/will/dev/nunchi/dashboard",    name = "dashboard" },
]

[daemon.limits]
max_agents_per_workspace = 20
max_total_agents = 50
max_budget_per_workspace_usd = 100.0
```

### 11.2 Cross-Workspace Knowledge Sharing

Knowledge Signals are scoped to their workspace by default. Cross-workspace sharing is explicit:

```toml
# roko workspace shares coding heuristics
[space.knowledge]
share_with = ["tag:nunchi"]
share_kinds = ["Heuristic", "Insight"]

# dashboard workspace imports from roko
[space.knowledge]
import_from = ["roko"]
import_filter = { min_tier = "Consolidated" }
```

Shared Signals carry their origin workspace tag in CaMeL provenance. The receiving workspace can query but not modify the original.

---

## 12. Secret Management

Secrets are **never stored in roko.toml**. Three mechanisms:

1. **Environment variables**: `api_key_env = "ANTHROPIC_API_KEY"` in provider config
2. **Secrets store**: `roko config secrets set <key> <value>` stores encrypted at `~/.roko/secrets/`
3. **`${VAR}` expansion**: Any string value can reference env vars: `rpc_url = "${ETH_RPC_URL}"`

**Secret rotation**: `roko config secrets rotate <key>` updates the secret and signals roko-serve to reload (hot-swap, no restart required).

---

## 13. Full Working Example

```toml
config_version = 2

[project]
name = "my-workspace"
fresh_base_branch = "main"

[server]
bind = "0.0.0.0"
port = 6677

[serve.auth]
enabled = true
privy_app_id = "cmhw01vut003tjx0d5lmqc8zs"

[agent]
default_model = "claude-sonnet-4-6"
context_limit_k = 200

[routing]
fast_task_model = "claude-haiku-4-5"
standard_task_model = "claude-sonnet-4-6"
complex_task_model = "claude-opus-4-6"

[budget]
max_plan_usd = 25.0
max_turn_usd = 3.0

[conductor]
max_agents = 8
express_mode = false

[learning]
replan_on_gate_failure = true
file_intel_max_entries = 15

[gates]
clippy_enabled = true
skip_tests = false

[[agents]]
name = "coder-1"
domain = "coding"
prompt = "Implement features and fix bugs in Rust"

[[agents]]
name = "pr-reviewer"
domain = "coding"
model = "claude-haiku-4-5"

[[agents]]
name = "researcher"
domain = "research"
```

---

## 14. Acceptance Criteria

| Criterion | Verification |
|---|---|
| Config Signal round-trips through serialize -> parse -> reserialize with content hash preserved | Unit test on representative config |
| ConfigComposeCell merges CLI > Env > TOML > Evolved correctly per field | Multi-source merge test |
| ConfigVerifyCell catches all 7 invariant violations | One negative test per invariant |
| ConfigWatchTrigger fires on roko.toml change, debounces within 500ms | File modification trigger test |
| Config reload Graph: watch -> compose -> verify -> publish pipeline works end-to-end | Integration test with file change |
| Config migration chain: v1 -> v2 preserves all values | Round-trip migration test |
| Schema version mismatch triggers migration automatically | Test with old-version config |
| Malformed TOML refuses to start with line/column error | Parse error test |
| Unknown fields warn but do not fail | Forward compatibility test |
| Missing roko.toml uses defaults and is fully functional | Default config boot test |
| Minimal config (just project.name) produces functional system | Minimal config test |
| Domain profile configures all dimensions simultaneously | Set `profile = "security-audit"`, verify clock + extensions + gates + models |
| Profile inheritance: child overrides parent via deep merge | Profile with `base`, verify merge semantics |
| Environment variable convention: ROKO_SECTION_FIELD maps correctly | Env var resolution test |
| Config demurrage: stale config emits warning after 30 days | Demurrage check test |
| L4 config proposal: evolved values have lowest priority | Compose with Evolved source, verify CLI/env override it |
| Secret management: secrets never appear in config Signal payload | Lineage inspection test |
| Multi-workspace daemon isolates config per workspace | Config isolation test |
