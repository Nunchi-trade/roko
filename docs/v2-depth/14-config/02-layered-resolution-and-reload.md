# Layered Resolution and Hot Reload

> Depth for [04-configuration-layered-resolution.md](../../docs/12-interfaces/04-configuration-layered-resolution.md). Redesigns layered config resolution as a Signal merge Pipeline. Four layers are four Store sources merged by priority into a resolved config Signal. Hot-reload via Trigger Cell. Auto-detection as Score Cells. Domain profiles as Rack macros.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, content addressing, lineage), [02-CELL](../../unified/02-CELL.md) (Compose protocol, Score protocol, Trigger Cell, Verify Cell), [03-GRAPH](../../unified/03-GRAPH.md) (Pipeline pattern, Rack specialization), [06-TRIGGER-SYSTEM](../../unified/06-TRIGGER-SYSTEM.md) (file watcher Trigger), [config-as-signal.md](config-as-signal.md) (Config Signal, ConfigSource, schema validation)

---

## 1. Resolution as a Signal Merge Pipeline

Config resolution is a **Pipeline pattern** where four source Cells emit partial config Signals, and a Compose Cell merges them by priority into a single resolved config Signal. This is not metaphor -- it is the literal runtime Graph that produces the active configuration.

```
[DefaultsCell] ──(priority: 0)──┐
                                 │
[TomlLoaderCell] ──(priority: 1)─┤
                                 ├──▶ [ConfigComposeCell] ──▶ [ConfigVerifyCell] ──▶ resolved config Signal
[EnvLoaderCell] ──(priority: 2)──┤
                                 │
[CliLoaderCell] ──(priority: 3)──┘
```

Each source Cell emits a **partial config Signal** -- a Signal of `Kind::Config` whose payload contains only the fields that source specifies. The ConfigComposeCell (documented in [config-as-signal.md](config-as-signal.md)) merges them field-by-field, highest priority wins.

### The Four Sources

| Priority | Source Cell | What it reads | When it fires |
|---|---|---|---|
| 0 (lowest) | `DefaultsCell` | Compiled-in `RokoConfig::default()` | Once at startup |
| 1 | `TomlLoaderCell` | `roko.toml` from project root | Startup + hot reload |
| 2 | `EnvLoaderCell` | `ROKO_*` environment variables | Startup only (env is static) |
| 3 (highest) | `CliLoaderCell` | CLI flags passed to the current invocation | Startup only |

### Why This Design

The Pipeline makes resolution **auditable**. The resolved config Signal carries `parent_hashes` pointing to all four source Signals. To answer "where did this value come from?", walk the lineage and check which source Signal contributed it. This is the same lineage system used for every other Signal in the system.

---

## 2. Source Cell Implementations

### DefaultsCell

```rust
/// Emits the compiled-in defaults as a partial config Signal.
/// This fires exactly once at startup and produces a Signal with every field populated.
/// It serves as the "floor" -- any field not overridden by higher layers gets this value.
pub struct DefaultsCell;

impl Cell for DefaultsCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }
    fn name(&self) -> &str { "config-defaults" }

    async fn execute(&self, _input: Vec<Signal>, _ctx: &CellContext) -> Result<Vec<Signal>> {
        let config = RokoConfig::default();
        Ok(vec![config_signal(&config, ConfigSource::Defaults)])
    }
}
```

### TomlLoaderCell

```rust
/// Reads roko.toml from the project root and emits a partial config Signal.
/// Only fields present in the TOML file appear in the output Signal.
/// Missing fields are NOT defaulted here -- that is the DefaultsCell's job.
pub struct TomlLoaderCell {
    path: PathBuf,  // usually: {project_root}/roko.toml
}

impl Cell for TomlLoaderCell {
    fn name(&self) -> &str { "config-toml-loader" }

    async fn execute(&self, _input: Vec<Signal>, _ctx: &CellContext) -> Result<Vec<Signal>> {
        let content = match tokio::fs::read_to_string(&self.path).await {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // No roko.toml -- emit empty partial (defaults will cover everything)
                return Ok(vec![]);
            }
            Err(e) => return Err(e.into()),
        };

        let partial: PartialConfig = toml::from_str(&content)?;
        Ok(vec![partial_config_signal(&partial, ConfigSource::TomlFile(self.path.clone()))])
    }
}
```

### EnvLoaderCell

```rust
/// Scans environment for ROKO_* variables and emits a partial config Signal.
/// Convention: ROKO_{SECTION}_{FIELD} maps to [section] field in TOML.
pub struct EnvLoaderCell;

impl Cell for EnvLoaderCell {
    fn name(&self) -> &str { "config-env-loader" }

    async fn execute(&self, _input: Vec<Signal>, _ctx: &CellContext) -> Result<Vec<Signal>> {
        let mut partial = PartialConfig::empty();

        for (key, value) in std::env::vars() {
            if let Some(path) = key.strip_prefix("ROKO_") {
                let config_path = path.to_lowercase().replace('_', ".");
                partial.set_field(&config_path, &value)?;
            }
        }

        if partial.is_empty() {
            return Ok(vec![]);
        }

        Ok(vec![partial_config_signal(&partial, ConfigSource::EnvVar("ROKO_*".into()))])
    }
}
```

### CliLoaderCell

```rust
/// Extracts config-relevant CLI flags and emits a partial config Signal.
/// Only flags that override config (--model, --backend, --effort, etc.) appear here.
pub struct CliLoaderCell {
    overrides: Vec<(String, String)>,  // parsed from CLI args
}

impl Cell for CliLoaderCell {
    fn name(&self) -> &str { "config-cli-loader" }

    async fn execute(&self, _input: Vec<Signal>, _ctx: &CellContext) -> Result<Vec<Signal>> {
        if self.overrides.is_empty() {
            return Ok(vec![]);
        }

        let mut partial = PartialConfig::empty();
        for (path, value) in &self.overrides {
            partial.set_field(path, value)?;
        }

        Ok(vec![partial_config_signal(&partial, ConfigSource::CliFlag("--flag".into()))])
    }
}
```

---

## 3. Environment Variable Convention

Every `roko.toml` field maps to an environment variable following a deterministic convention:

```
ROKO_{SECTION}_{FIELD} = value
```

### Rules

1. Prefix is always `ROKO_`.
2. Section and field names are uppercased.
3. Nested sections join with `_`.
4. Array indices are not supported via env vars (use TOML for arrays).
5. Boolean values: `true`, `1`, `yes` are truthy; everything else is falsy.

### Examples

| Environment variable | TOML equivalent | Type |
|---|---|---|
| `ROKO_AGENT_MODEL` | `[agent] model = "..."` | string |
| `ROKO_AGENT_DEFAULT_BACKEND` | `[agent] default_backend = "..."` | string |
| `ROKO_GATES_CLIPPY` | `[gates] clippy = true` | bool |
| `ROKO_BUDGET_MAX_PLAN_USD` | `[budget] max_plan_usd = 200.0` | float |
| `ROKO_SERVER_PORT` | `[server] port = 9090` | integer |
| `ROKO_ROUTING_EXPLORATION_RATE` | `[routing] exploration_rate = 0.1` | float |
| `ROKO_DAIMON_ENABLED` | `[daimon] enabled = true` | bool |
| `ROKO_NEURO_ENABLED` | `[neuro] enabled = true` | bool |
| `ROKO_DREAMS_SCHEDULE` | `[dreams] schedule = "idle"` | string |
| `ROKO_OBSERVABILITY_LOG_FORMAT` | `[observability] log_format = "json"` | string |

### Secret Variables

Secret-bearing env vars follow the convention `{PROVIDER}_API_KEY`:

| Variable | What it provides |
|---|---|
| `ANTHROPIC_API_KEY` | Anthropic provider authentication |
| `OPENROUTER_API_KEY` | OpenRouter provider authentication |
| `GEMINI_API_KEY` | Google Gemini provider authentication |
| `PERPLEXITY_API_KEY` | Perplexity research backend |

These are NOT prefixed with `ROKO_` because they are standard provider conventions. The config system reads them via `providers.{name}.api_key_env` indirection.

---

## 4. Hot Reload via Trigger Cell

When `roko.toml` changes on disk, the system reloads configuration without restart. This is a **Trigger Cell** that watches the config file and re-fires the resolution Pipeline.

### The Hot Reload Graph

```toml
# Hot reload Pipeline: Trigger -> Loader -> Compose -> Verify -> Publish
[graph.config-hot-reload]
trigger = "file-watch"

[[graph.config-hot-reload.cells]]
name = "config-file-trigger"
type = "trigger"
watch = "roko.toml"
debounce_ms = 500

[[graph.config-hot-reload.cells]]
name = "config-toml-loader"
type = "store"

[[graph.config-hot-reload.cells]]
name = "config-compose"
type = "compose"

[[graph.config-hot-reload.cells]]
name = "config-verify"
type = "verify"

[[graph.config-hot-reload.edges]]
from = "config-file-trigger.out"
to = "config-toml-loader.trigger"

[[graph.config-hot-reload.edges]]
from = "config-toml-loader.out"
to = "config-compose.toml_input"

[[graph.config-hot-reload.edges]]
from = "config-compose.out"
to = "config-verify.in"
```

### Reload Semantics

| Step | What happens | Failure mode |
|---|---|---|
| 1. File change detected | `inotify` (Linux) or `kqueue` (macOS) fires | None -- OS-level |
| 2. Debounce (500ms) | Ignore rapid successive saves | -- |
| 3. Read new TOML | Parse file content | Parse error: publish `config.reload_failed` Pulse, keep old config |
| 4. Compose with env + CLI | Merge by priority (env and CLI unchanged) | -- |
| 5. Verify | Schema validation, invariant checks | Validation error: publish `config.reload_failed`, keep old config |
| 6. Publish | Emit new config Signal on `config.reloaded` topic | -- |

### What Does NOT Hot-Reload

Some config changes require restart because they affect process-level state:

| Section | Hot-reloadable? | Why |
|---|---|---|
| `[agent]` model, backend | Yes | Affects next dispatch, not current |
| `[gates]` pipeline, thresholds | Yes | Gate runner reads config per invocation |
| `[routing]` weights | Yes | CascadeRouter re-reads on next route |
| `[budget]` limits | Yes | Budget checked per-task |
| `[server]` port, bind | **No** | Listener is bound at startup |
| `[profile]` shape selection | **No** | Substrate/Bus backends bound at startup |
| `[providers]` API endpoints | Partial | New providers appear; removed providers drain |

### Bus Topics for Config Lifecycle

| Topic | When published | Payload |
|---|---|---|
| `config.loaded` | Startup, after initial resolution | Full resolved config Signal hash |
| `config.reloaded` | After successful hot reload | New resolved config Signal hash |
| `config.reload_failed` | After failed hot reload attempt | Error details, old config remains active |
| `config.stale_warning` | When config demurrage balance drops below threshold | Section, age, recommendation |

---

## 5. Auto-Detection as Score Cells

When `roko init` runs without a `--template` flag, the system scans the project to detect the development environment. In unified terms, this is a set of **Score Cells** that rate the fitness of each language profile against the project directory.

### Architecture

```
[RustScoreCell] ──(score: 0.95)──┐
                                  │
[NodeScoreCell] ──(score: 0.0)───┤
                                  ├──▶ [ProfileRouteCell] ──▶ selected profile config Signal
[GoScoreCell] ────(score: 0.0)───┤
                                  │
[PythonScoreCell]─(score: 0.0)───┘
```

Each Score Cell examines the project directory and returns a confidence score (0.0 to 1.0). The ProfileRouteCell selects the highest-scoring profile.

### Score Cell Logic

```rust
/// Score Cell: rates fitness of the Rust profile for this project.
pub struct RustScoreCell;

impl Cell for RustScoreCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn name(&self) -> &str { "detect-rust" }

    async fn execute(&self, _input: Vec<Signal>, ctx: &CellContext) -> Result<Vec<Signal>> {
        let root = ctx.project_root();
        let mut score = 0.0;

        // Primary signal: Cargo.toml exists
        if root.join("Cargo.toml").exists() {
            score += 0.7;
        }

        // Secondary: workspace members
        if root.join("Cargo.lock").exists() {
            score += 0.1;
        }

        // Tertiary: src/ directory
        if root.join("src").is_dir() || root.join("crates").is_dir() {
            score += 0.1;
        }

        // Quaternary: .rs files exist
        if has_files_with_extension(root, "rs") {
            score += 0.1;
        }

        Ok(vec![score_signal("rust", score)])
    }
}
```

### Detection Results

| Language | Files checked | Generated gate config |
|---|---|---|
| Rust | `Cargo.toml`, `Cargo.lock`, `src/`, `crates/`, `*.rs` | compile: `cargo build`, test: `cargo test`, clippy: `cargo clippy --no-deps -- -D warnings` |
| Node.js | `package.json`, `node_modules/`, `tsconfig.json`, `*.ts` | compile: `npm run build` (if script exists), test: `npm test` |
| Go | `go.mod`, `go.sum`, `*.go` | compile: `go build ./...`, test: `go test ./...`, vet: `go vet ./...` |
| Python | `pyproject.toml`, `setup.py`, `requirements.txt`, `*.py` | test: `pytest`, lint: `ruff check .` (if ruff available) |

### Multi-Language Projects

If multiple Score Cells return scores above 0.5, the system generates a composite profile with gates for each detected language. The ProfileRouteCell does not pick one winner -- it combines all high-confidence detections.

---

## 6. Domain Profiles as Rack Macros

A **Rack** is a Graph with parameterizable slots (knobs and jacks). Domain profiles are Rack macros -- pre-configured parameter bundles that fill Rack slots for common deployment patterns.

### Profile Selection

```toml
# roko.toml: select a deployment profile
profile = "single-server"
```

The `profile` field selects a Rack macro. The macro provides defaults for all deployment-sensitive config:

### Built-in Profiles

| Profile | Substrate | Bus | Auth | Observability | Use case |
|---|---|---|---|---|---|
| `laptop` | SQLite (local file) | in-memory | none | human logs, optional metrics | Solo development |
| `single-server` | SQLite (persistent path) | in-memory | basic (API key) | JSON logs, `/metrics` | Self-hosted team server |
| `container` | SQLite or Postgres (env var) | in-memory or NATS | basic or OIDC | JSON stdout, OTLP | Docker/Kubernetes |
| `clustered` | Postgres (connection pool) | NATS (distributed) | OIDC | JSON stdout, OTLP, shared traces | Multi-node production |
| `edge` | in-memory (volatile) | none | header-based | compact logs, sampled | CDN workers, IoT |

### Profile as Config Signal

A profile is itself a partial config Signal at priority 0.5 (between defaults and TOML). The resolution Pipeline becomes:

```
[DefaultsCell] ──(priority: 0)──────┐
                                     │
[ProfileCell] ──(priority: 0.5)─────┤
                                     ├──▶ [ConfigComposeCell] ──▶ resolved
[TomlLoaderCell] ──(priority: 1)────┤
                                     │
[EnvLoaderCell] ──(priority: 2)─────┤
                                     │
[CliLoaderCell] ──(priority: 3)─────┘
```

This means: the profile fills in deployment-specific defaults, but explicit TOML values override the profile. A user can select `profile = "clustered"` and still override `substrate.kind = "sqlite"` for local testing.

### Custom Profiles

Users can define custom profiles in `roko.toml`:

```toml
profile = "my-team"

[profile.my-team]
listen = "0.0.0.0:8080"
auth = "oidc"
substrate = { kind = "postgres", url = "${DATABASE_URL}" }
bus = { kind = "nats", url = "${NATS_URL}" }
observability = { log_format = "json", metrics_bind = "0.0.0.0:9090" }
```

---

## 7. Resolution Algorithm Summary

The complete resolution for a single config field:

```
fn resolve_field(field: &str) -> Value {
    // Priority 3: CLI flag (highest)
    if let Some(v) = cli_overrides.get(field) { return v; }

    // Priority 2: Environment variable
    if let Some(v) = env_var_for(field) { return v; }

    // Priority 1: roko.toml explicit value
    if let Some(v) = toml_file.get(field) { return v; }

    // Priority 0.5: Selected profile defaults
    if let Some(v) = profile_defaults.get(field) { return v; }

    // Priority 0: Compiled defaults (lowest)
    RokoConfig::default().get(field)
}
```

Every step produces a Signal. The final resolved config Signal carries the lineage of all contributing sources. This is auditable: `roko config show --provenance` walks the lineage and annotates each field with its source.

---

## What This Enables

1. **Zero-config startup**: `roko run "fix bug"` works with no `roko.toml`. Defaults and auto-detection handle everything.
2. **Progressive complexity**: Start with `[agent] model = "claude-sonnet-4-6"`. Add sections as needs grow. Never required to specify what you do not need to change.
3. **Hot reload without restart**: Edit `roko.toml` during a plan run. Routing weights, gate thresholds, and budget limits update immediately. No restart, no state loss.
4. **Deployment portability**: Change `profile = "laptop"` to `profile = "clustered"` and override 2-3 backend URLs. Same binary, same features, different operational posture.
5. **Provenance tracking**: "Why is my model set to opus?" is answered by `roko config show --provenance agent.model`, which reports exactly which source (CLI, env, TOML, profile, or default) contributed the value.

## Feedback Loops

- **L1 (per-turn)**: Adaptive thresholds adjust gate parameters within the ranges declared by config. This is runtime tuning, not config change -- but it reads the config-declared bounds.
- **L2 (per-session)**: CascadeRouter refines routing weights. The config `[routing]` section provides initial values; L2 overrides them at runtime. On next session, the persisted router state loads alongside config.
- **L3 (cross-session)**: Dream consolidation can observe which config fields are most frequently overridden and propose permanent changes (see [config-as-signal.md](config-as-signal.md) section 7, L4 evolution).
- **Hot reload loop**: Config change -> Trigger -> Compose -> Verify -> publish `config.reloaded` -> all Cells that read config receive updated values. If verify fails, old config persists and `config.reload_failed` alerts the operator.

## Open Questions

1. **Atomic multi-field updates**: If a TOML edit changes both `budget.warn_threshold` and `budget.block_threshold`, the hot reload must apply both atomically. Current Pipeline processes the full file as a unit, so this works -- but what about env var changes that arrive one at a time?
2. **Profile inheritance**: Should `profile = "my-team"` be able to `extends = "clustered"`? Inheritance chains add complexity but reduce duplication for teams with many similar profiles.
3. **Config in multi-agent setups**: When multiple agents share a workspace, should they share one resolved config or each maintain agent-scoped overrides? The current model assumes one config per workspace.
4. **Env var arrays and maps**: The `ROKO_*` convention handles scalar values cleanly. Representing arrays (`gates.pipeline = ["compile", "test"]`) or maps (`providers.anthropic.base_url`) via env vars requires a sub-convention (comma-separated? JSON? `ROKO_GATES_PIPELINE_0`?).
5. **Config drift detection**: In clustered deployments, different nodes may have different TOML files. Should the system detect config drift across nodes and alert? This is a distributed consistency problem.

## Implementation Tasks

| Task | Crate | Effort | Priority |
|---|---|---|---|
| Implement `PartialConfig` type with field-level presence tracking | `roko-core` | M | High |
| Wire `EnvLoaderCell` into startup (currently ad hoc in `config.rs`) | `roko-cli` | S | High |
| Implement `ConfigWatchTrigger` using existing `notify::RecommendedWatcher` | `roko-cli` | M | High |
| Add hot-reload Pipeline Graph (Trigger -> Compose -> Verify -> publish) | `roko-cli` | M | High |
| Add `roko config show --provenance` that walks lineage to annotate sources | `roko-cli` | S | Medium |
| Implement profile selection with built-in 5 profiles | `roko-cli` | M | Medium |
| Wire auto-detection Score Cells into `roko init` | `roko-cli` | S | Already partial |
| Add `config.reloaded` / `config.reload_failed` Bus topic publishing | `roko-core` | S | Medium |
| Document env var mapping for all 60+ config fields | docs | S | Medium |
| Add multi-language composite detection for mixed projects | `roko-cli` | S | Low |
