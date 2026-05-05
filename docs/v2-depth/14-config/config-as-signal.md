# Config as Signal

> Depth for [20-configuration-schema.md](../../docs/00-architecture/20-configuration-schema.md). Redesigns configuration as a Signal -- content-addressed, versioned, lineage-tracked, and subject to demurrage. Runtime overrides become a Compose protocol. Schema validation becomes a Verify Cell. Hot reload becomes a Trigger Cell.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Kind, demurrage, content addressing), [02-CELL](../../unified/02-CELL.md) (Compose protocol, Verify protocol, Trigger Cell), [03-GRAPH](../../unified/03-GRAPH.md) (Graph wiring), [06-TRIGGER-SYSTEM](../../unified/06-TRIGGER-SYSTEM.md) (Trigger Cell, file watcher)

---

## 1. Configuration IS a Signal

Configuration is not special. It is data that:
- has a content hash (two identical configs produce the same hash),
- has a version (schema_version tracks evolution),
- has lineage (each config derives from the previous one plus an override source),
- decays (stale config is worse than no config -- a six-month-old routing weight is probably wrong).

In unified terms, configuration is a Signal with `Kind::Config`. It participates in the same Store, Bus, and protocol system as every other Signal. This is not a metaphor -- it means config Signals get content-addressed, they carry lineage, they can be queried by HDC similarity, and they are subject to demurrage.

```rust
/// Configuration as a Signal.
///
/// A config Signal carries the full or partial configuration state.
/// It is content-addressed: the same config values produce the same hash.
/// It is versioned: schema_version tracks breaking changes.
/// It carries lineage: parent_hashes point to the config(s) it was derived from.
///
/// See [01-SIGNAL.md](../../unified/01-SIGNAL.md) SS1 for the Signal envelope.
pub fn config_signal(config: &RokoConfig, source: ConfigSource) -> Signal {
    Signal {
        kind: Kind::Config,
        payload: serde_json::to_value(config).expect("config is serializable"),
        metadata: SignalMetadata {
            schema_version: config.schema_version,
            source: source.to_string(),
            // Content hash computed from canonical serialization
            hash: ContentHash::compute(&canonical_bytes(config)),
            // Lineage: points to the previous config Signal
            parent_hashes: vec![],  // set by the Compose protocol
            // Config Signals start with full balance
            demurrage_balance: 1.0,
            ..Default::default()
        },
    }
}

/// Where a config value came from, in priority order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// Lowest priority: the roko.toml file on disk.
    TomlFile(PathBuf),
    /// Middle priority: environment variable.
    EnvVar(String),
    /// Highest priority: CLI flag.
    CliFlag(String),
    /// Runtime override via HTTP API.
    ApiOverride { principal: String },
    /// L4 evolution: config changed by structural adaptation.
    Evolved { proposal_id: String },
}
```

---

## 2. The 60+ Parameters as Signal Fields

The `RokoConfig` struct has 60+ parameters across 20+ sections. In the unified model, each section is a nested struct within the config Signal's payload. The full parameter catalog from the architecture spec maps directly.

```toml
# roko.toml is the TOML serialization of a Config Signal.
# When loaded, it becomes a Signal with Kind::Config.

config_version = 2
schema_version = 2

[project]
name = "roko"
root = "."
fresh_base_branch = "main"

[agent]
default_model = "claude-sonnet-4-6"
default_backend = "claude"
default_effort = "medium"
context_limit_k = 200

[agent.roles.implementer]
model = "claude-opus-4-6"
effort = "high"

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

[demurrage]
flat_tax_per_day = 0.01
exp_decay_per_day = 0.005
min_balance = 0.0
cited_bonus = 0.05
retrieved_bonus = 0.02
gated_bonus = 0.03
surprised_bonus = 0.15

[learning]
auto_refresh_playbook = true
adaptive_thresholds = true
cascade_router_persistence = true
```

### Interdependencies as Invariants

Some parameters constrain each other. These invariants are checked by the Verify Cell (section 4).

| Invariant | Parameters | Rule |
|---|---|---|
| Budget ordering | `warn_threshold`, `block_threshold` | `warn < block` |
| Reward weight sum | `cost_weight + latency_weight + quality_weight` | `== 1.0` (tolerance 0.01) |
| Agent capacity vs budget | `max_agents * max_task_usd` | `<= max_session_usd` (warning) |
| Gate iterations hierarchy | `gates.max_iterations`, `pipeline.*.max_iterations` | Pipeline overrides gate |
| Provider existence | `agent.default_model` | Provider for this model must exist in `[providers]` |
| Demurrage floor | `demurrage.min_balance` | `>= 0.0` |
| Demurrage bonuses | `demurrage.*_bonus` | `>= 0.0` (negative reinforcement belongs in Verify, not config) |

---

## 3. Runtime Overrides as Compose Protocol

The three override sources (CLI > env > roko.toml) are a **Compose protocol** that merges config Signals by priority. The Compose protocol ([02-CELL.md](../../unified/02-CELL.md) SS4) assembles a composite output from multiple input Signals under budget constraints.

```rust
/// Compose Cell: merges config Signals by priority.
///
/// Input: up to 3 config Signals (toml, env, cli), each partial.
/// Output: 1 merged config Signal with full lineage.
///
/// Priority: CLI > Env > TOML > Default.
/// Each field is resolved independently: the highest-priority source
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
        // Sort inputs by priority (highest first)
        let mut sources = input.into_iter()
            .map(|s| {
                let source: ConfigSource = extract_source(&s)?;
                let priority = source.priority();
                Ok((priority, s))
            })
            .collect::<Result<Vec<_>, CellError>>()?;
        sources.sort_by(|a, b| b.0.cmp(&a.0));

        // Start with defaults
        let mut merged = RokoConfig::default();

        // Apply in reverse priority order (lowest first, highest wins)
        let mut parent_hashes = Vec::new();
        for (_, signal) in sources.iter().rev() {
            apply_partial_config(&mut merged, &signal.payload)?;
            parent_hashes.push(signal.hash());
        }

        // Build the merged config Signal with lineage
        let mut result = config_signal(&merged, ConfigSource::Composed);
        result.metadata.parent_hashes = parent_hashes;

        Ok(vec![result])
    }
}

impl ConfigSource {
    fn priority(&self) -> u8 {
        match self {
            ConfigSource::TomlFile(_) => 1,
            ConfigSource::EnvVar(_) => 2,
            ConfigSource::CliFlag(_) => 3,
            ConfigSource::ApiOverride { .. } => 4,
            ConfigSource::Evolved { .. } => 0, // lowest: evolution is a suggestion
        }
    }
}
```

The key insight: the merged config Signal records all parent hashes. If you need to know where a particular value came from, walk the lineage. If you need to know what changed, diff the current config Signal against its parent. This is the same lineage system used for every other Signal.

### Resolution Algorithm

```
fn resolve_field(field: &str, sources: &[ConfigSignal]) -> Value {
    // Sources are sorted by priority (highest first)
    for source in sources {
        if let Some(value) = source.get_field(field) {
            return value;
        }
    }
    // Fall through to compiled default
    RokoConfig::default().get_field(field)
}
```

### Environment Variable Convention

```
ROKO_CONDUCTOR_MAX_AGENTS=8  ->  conductor.max_agents = 8
ROKO_BUDGET_MAX_PLAN_USD=200 ->  budget.max_plan_usd = 200.0
```

Convention: `ROKO_` prefix, section and field joined by `_`, all uppercase.

---

## 4. Schema Validation as a Verify Cell

Configuration validation is a **Verify protocol Cell** (see [02-CELL.md](../../unified/02-CELL.md) SS3). It takes a config Signal as input and emits a verdict Signal.

```rust
/// Verify Cell: config schema validation.
///
/// Checks:
/// 1. All invariants (budget ordering, weight sum, etc.)
/// 2. Type correctness (ranges, enums)
/// 3. Provider existence for referenced models
/// 4. Schema version compatibility
/// 5. Unknown fields (warn, not fail -- forward compatibility)
pub struct ConfigVerifyCell;

impl Cell for ConfigVerifyCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn name(&self) -> &str { "config-verify" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let config_signal = &input[0];
        let config: RokoConfig = serde_json::from_value(config_signal.payload.clone())?;

        let mut warnings: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        // Invariant: budget ordering
        if config.budget.warn_threshold >= config.budget.block_threshold {
            errors.push(format!(
                "warn_threshold ({}) must be < block_threshold ({})",
                config.budget.warn_threshold, config.budget.block_threshold
            ));
        }

        // Invariant: reward weight sum
        let weight_sum = config.routing.cost_weight
            + config.routing.latency_weight
            + config.routing.quality_weight;
        if (weight_sum - 1.0).abs() > 0.01 {
            errors.push(format!(
                "Routing weights sum to {weight_sum:.3}, expected 1.0"
            ));
        }

        // Invariant: provider exists for default model
        if !config.providers.is_empty() {
            let has_provider = config.providers.values()
                .any(|p| p.models_include(&config.agent.default_model));
            if !has_provider {
                warnings.push(format!(
                    "Default model '{}' has no matching provider",
                    config.agent.default_model
                ));
            }
        }

        // Invariant: demurrage floor
        if config.demurrage.min_balance < 0.0 {
            errors.push("demurrage.min_balance must be >= 0.0".into());
        }

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

### Error Handling

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

## 5. Hot Reload as a Trigger Cell

When roko.toml changes on disk, the system should reload without restart. This is a **Trigger Cell** (see [06-TRIGGER-SYSTEM.md](../../unified/06-TRIGGER-SYSTEM.md)) watching the config file.

```rust
/// Trigger Cell: watches roko.toml for changes and triggers reload.
///
/// When the file changes:
/// 1. Read the new config from disk.
/// 2. Create a new Config Signal with TomlFile source.
/// 3. Run through ConfigComposeCell (merge with env and CLI).
/// 4. Run through ConfigVerifyCell.
/// 5. If verification passes: publish the new config Signal on
///    "config.reloaded" topic.
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
        // Input is the file-change event Signal from the OS watcher
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
        let config_signal = config_signal(&new_config, ConfigSource::TomlFile(
            self.watch_path.clone()
        ));

        // The Graph will route this through Compose -> Verify -> publish
        Ok(vec![config_signal])
    }
}
```

### The Config Reload Graph

```toml
# Graph: config hot reload pipeline
[graph.config-reload]
cells = ["config-watch-trigger", "config-compose", "config-verify"]

[[graph.config-reload.edges]]
from = "config-watch-trigger.out"
to = "config-compose.toml_input"

[[graph.config-reload.edges]]
from = "config-compose.out"
to = "config-verify.in"

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

        // The migrated config is a new Signal with lineage to the old one
        let new_config: RokoConfig = toml::from_str(&config_value.to_string())?;
        let mut result = config_signal(&new_config, ConfigSource::Migrated);
        result.metadata.parent_hashes = vec![signal.hash()];

        Ok(vec![result])
    }
}
```

---

## 7. Config Evolution in L4

Configuration is not just static input. It is an evolvable artifact. In L4 structural adaptation ([10-LEARNING-LOOPS.md](../../unified/10-LEARNING-LOOPS.md) SS6), the system can propose config changes based on observed outcomes.

```rust
/// L4 config evolution: the system proposes config changes.
///
/// Flow:
/// 1. L4 StructuralAdaptation observes that routing weights are suboptimal
///    (e.g., quality_weight is too high, cost_weight too low).
/// 2. It generates a ConfigProposal Signal.
/// 3. The proposal goes through ConfigVerifyCell to ensure validity.
/// 4. If valid, the proposal enters a human approval queue.
/// 5. If approved, a new Config Signal is emitted with source Evolved.
/// 6. The new config has the LOWEST priority in the Compose merge
///    (evolution is a suggestion, not a mandate).
pub struct ConfigProposal {
    /// Which fields to change.
    pub changes: BTreeMap<String, serde_json::Value>,
    /// Why this change is proposed.
    pub rationale: String,
    /// Evidence: the KPI/anti-metric signals that motivated the proposal.
    pub evidence: Vec<ContentHash>,
    /// Expected improvement.
    pub expected_improvement: String,
}
```

This is what config evolution looks like: the system notices that its demurrage parameters are too aggressive (warm tier is emptying too fast, retrieval quality is declining), proposes a reduction in `flat_tax_per_day`, and submits the proposal for human approval. The proposal itself is a Signal with full lineage to the evidence that motivated it.

### Config Demurrage

Stale config is worse than no config. A routing weight that was optimal six months ago is probably wrong today. Config Signals are subject to demurrage, just like every other Signal.

```rust
/// Config demurrage: stale config loses balance.
///
/// When a config Signal's balance drops below a threshold,
/// the system emits a warning Pulse: "This config section has not
/// been reviewed or validated in N days."
///
/// This does NOT automatically change the config. It alerts the
/// operator that the config may need attention.
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

---

## 8. Minimal vs Full Config

A bare-minimum config Signal:

```toml
config_version = 2
schema_version = 2

[project]
name = "my-project"
```

Every other section uses serde defaults. The system is fully functional with just a project name. This is by design -- the defaults should be good enough for a first run. The config Compose Cell fills in defaults for any field not specified.

---

## What This Enables

1. **Config lineage**: every config change is tracked. "Who changed what, when, and why?" is answerable by walking the Signal lineage graph.
2. **Config as first-class data**: config participates in the same Store, Bus, and protocol system as every other Signal. No special-case infrastructure.
3. **Hot reload without restart**: the Trigger Cell watches roko.toml and pushes changes through the Compose -> Verify pipeline.
4. **Config evolution**: L4 can propose config changes based on observed outcomes, with human approval in the loop.
5. **Stale config detection**: demurrage on config Signals alerts when configuration has not been reviewed.
6. **Override transparency**: the Compose protocol records which source won for each field, visible in lineage.

## Feedback Loops

- **L1**: adaptive threshold tuning adjusts gate parameters within their declared ranges. This IS config adjustment at the L1 timescale.
- **L2**: CascadeRouter adjusts routing weights based on outcomes. The routing section of config provides the initial values; L2 refines them at runtime.
- **L3**: Delta consolidation reviews which config values have been overridden most often and proposes a permanent change.
- **L4**: full config evolution proposals based on KPI trends.

## Open Questions

1. **Config conflicts in multi-agent systems**: when multiple agents have different config overrides for the same field, which one wins? The current answer is "per-agent config Signals with agent-scoped merge" but the semantics are not fully specified.
2. **Config rollback**: if a hot-reloaded config causes problems, should the system automatically revert? L1 has auto-rollback for parameter tuning, but config-level rollback is more complex because config changes can be structural.
3. **Config encryption**: some config fields (API keys, secrets) should never appear in Signal lineage. The current approach is to store secrets separately and reference them by name. But the config Signal still carries the secret name, which is metadata leakage.
4. **Config diffing for dashboards**: operators want to see "what changed in this config reload." The lineage system enables this, but a purpose-built diff view is not yet specified.
5. **Config as code vs config as data**: should roko.toml be checked into version control? If so, the config Signal has dual lineage (git history AND Signal lineage). How do these two lineage systems interact?
