//! Config field metadata registry for the interactive config editor.
//!
//! Defines every editable config field with its label, description, type,
//! and valid options. The `build_flat_items()` function produces the
//! flattened display list consumed by the config view renderer.

use std::collections::HashMap;
use std::path::Path;

use crate::tui::state::model_context_limit;

/// What kind of value a config field holds, and how to edit it.
#[derive(Debug, Clone)]
pub enum ConfigFieldKind {
    /// Boolean toggle.
    Bool,
    /// Enumerated set of valid string values.
    Enum(Vec<&'static str>),
    /// Integer with optional bounds and preset values.
    Int {
        /// Minimum allowed value.
        min: Option<i64>,
        /// Maximum allowed value.
        max: Option<i64>,
        /// Preset values for h/l cycling.
        presets: Vec<i64>,
    },
    /// Floating-point with optional bounds.
    Float {
        /// Minimum allowed value.
        min: Option<f64>,
        /// Maximum allowed value.
        max: Option<f64>,
    },
    /// Free-form string.
    Str,
    /// Read-only display value (not editable).
    ReadOnly,
}

/// Where a config value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// From `roko.toml`.
    File,
    /// From environment variable override.
    Env,
    /// Default value (not explicitly set).
    Default,
}

impl ConfigSource {
    /// Short label for display (e.g. `"file"`, `"env"`, `"default"`).
    pub const fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Env => "env",
            Self::Default => "default",
        }
    }
}

/// Static metadata for one config field.
#[derive(Debug, Clone)]
pub struct ConfigFieldMeta {
    /// Dotted config key path (e.g. `"agent.default_model"`).
    pub key: &'static str,
    /// Human-readable label shown in the UI.
    pub label: &'static str,
    /// One-line description shown below the selected field.
    pub description: &'static str,
    /// Field type determining edit behaviour.
    pub kind: ConfigFieldKind,
    /// Group/section name for grouping in the UI.
    pub group: &'static str,
}

/// One item in the flattened config display list.
#[derive(Debug, Clone)]
pub enum ConfigItem {
    /// Section header (group name).
    Header(String),
    /// Editable or read-only field.
    Field {
        /// Field metadata (label, kind, description).
        meta: ConfigFieldMeta,
        /// Current display value.
        value: String,
        /// Where this value came from.
        source: ConfigSource,
    },
    /// The Apply & Save button.
    SaveButton,
}

/// Known ROKO_* env vars and the config field they override.
const ENV_OVERRIDES: &[(&str, &str)] = &[
    ("ROKO_MODEL", "agent.default_model"),
    ("ROKO_BACKEND", "agent.default_backend"),
    ("ROKO_EFFORT", "agent.default_effort"),
    ("ROKO_CONTEXT_LIMIT_K", "agent.context_limit_k"),
    ("ROKO_MAX_AGENTS", "conductor.max_agents"),
    ("ROKO_BUDGET_USD", "budget.max_plan_usd"),
    ("ROKO_PARALLEL", "conductor.parallel_enabled"),
    ("ROKO_EXPRESS", "conductor.express_mode"),
    ("ROKO_SKIP_TESTS", "gates.skip_tests"),
    ("ROKO_CLIPPY", "gates.clippy_enabled"),
];

/// Build the set of env-overridden keys (dot-path -> env var name).
pub fn active_env_overrides() -> HashMap<String, String> {
    let mut map = HashMap::new();
    for &(env_var, key) in ENV_OVERRIDES {
        if std::env::var(env_var).is_ok() {
            map.insert(key.to_string(), env_var.to_string());
        }
    }
    map
}

/// All field definitions in display order.
#[allow(clippy::too_many_lines)]
pub fn all_fields() -> Vec<ConfigFieldMeta> {
    vec![
        // ── Agent ──
        ConfigFieldMeta {
            key: "agent.default_model",
            label: "Default Model",
            description: "Model used for agent tasks unless overridden per-role",
            kind: ConfigFieldKind::Enum(vec![
                "claude-sonnet-4-6",
                "claude-opus-4-6",
                "claude-haiku-4-5",
            ]),
            group: "Agent",
        },
        ConfigFieldMeta {
            key: "agent.default_backend",
            label: "Default Backend",
            description: "Backend used to spawn agents (claude, codex, cursor, ollama)",
            kind: ConfigFieldKind::Enum(vec![
                "claude",
                "codex",
                "cursor",
                "openai",
                "ollama",
                "perplexity",
            ]),
            group: "Agent",
        },
        ConfigFieldMeta {
            key: "agent.default_effort",
            label: "Default Effort",
            description: "Reasoning effort level for agent tasks",
            kind: ConfigFieldKind::Enum(vec!["low", "medium", "high", "max"]),
            group: "Agent",
        },
        ConfigFieldMeta {
            key: "agent.context_limit_k",
            label: "Context Limit (K)",
            description: "Context window limit in thousands of tokens",
            kind: ConfigFieldKind::Int {
                min: Some(8),
                max: Some(1000),
                presets: vec![32, 64, 128, 200, 500, 1000],
            },
            group: "Agent",
        },
        ConfigFieldMeta {
            key: "agent.bare_mode",
            label: "Bare Mode",
            description: "Skip built-in system prompt when spawning agents",
            kind: ConfigFieldKind::Bool,
            group: "Agent",
        },
        ConfigFieldMeta {
            key: "agent.fallback_model",
            label: "Fallback Model",
            description: "Model to retry with if an agent spawn fails",
            kind: ConfigFieldKind::Str,
            group: "Agent",
        },
        ConfigFieldMeta {
            key: "agent.timeout_ms",
            label: "Timeout (ms)",
            description: "Subprocess timeout in milliseconds",
            kind: ConfigFieldKind::Int {
                min: Some(5000),
                max: Some(600_000),
                presets: vec![30_000, 60_000, 120_000, 300_000],
            },
            group: "Agent",
        },
        // ── Budget ──
        ConfigFieldMeta {
            key: "budget.max_plan_usd",
            label: "Max Plan USD",
            description: "Maximum dollars to spend per plan execution",
            kind: ConfigFieldKind::Float {
                min: Some(0.0),
                max: Some(500.0),
            },
            group: "Budget",
        },
        ConfigFieldMeta {
            key: "budget.max_turn_usd",
            label: "Max Turn USD",
            description: "Maximum dollars per single agent turn",
            kind: ConfigFieldKind::Float {
                min: Some(0.0),
                max: Some(50.0),
            },
            group: "Budget",
        },
        ConfigFieldMeta {
            key: "budget.prompt_token_budget",
            label: "Prompt Token Budget",
            description: "Token budget for prompt composition",
            kind: ConfigFieldKind::Int {
                min: Some(1000),
                max: Some(model_context_limit("gemini-pro") as i64),
                presets: vec![5000, 10_000, 20_000, 50_000],
            },
            group: "Budget",
        },
        // ── Gates ──
        ConfigFieldMeta {
            key: "gates.clippy_enabled",
            label: "Clippy Enabled",
            description: "Run clippy/lint gate after task implementation",
            kind: ConfigFieldKind::Bool,
            group: "Gates",
        },
        ConfigFieldMeta {
            key: "gates.skip_tests",
            label: "Skip Tests",
            description: "Skip the test gate entirely",
            kind: ConfigFieldKind::Bool,
            group: "Gates",
        },
        ConfigFieldMeta {
            key: "gates.max_iterations",
            label: "Max Iterations",
            description: "Max gate retry iterations before giving up",
            kind: ConfigFieldKind::Int {
                min: Some(1),
                max: Some(10),
                presets: vec![1, 2, 3, 5],
            },
            group: "Gates",
        },
        // ── Routing ──
        ConfigFieldMeta {
            key: "routing.mode",
            label: "Mode",
            description: "Routing mode for model selection",
            kind: ConfigFieldKind::Enum(vec!["auto_override", "manual", "round_robin"]),
            group: "Routing",
        },
        ConfigFieldMeta {
            key: "routing.algorithm",
            label: "Algorithm",
            description: "Online learning algorithm for the router",
            kind: ConfigFieldKind::Enum(vec!["linucb", "thompson"]),
            group: "Routing",
        },
        ConfigFieldMeta {
            key: "routing.fast_task_model",
            label: "Fast Task Model",
            description: "Model for low-complexity tasks",
            kind: ConfigFieldKind::Str,
            group: "Routing",
        },
        ConfigFieldMeta {
            key: "routing.standard_task_model",
            label: "Standard Task Model",
            description: "Model for standard-complexity tasks",
            kind: ConfigFieldKind::Str,
            group: "Routing",
        },
        ConfigFieldMeta {
            key: "routing.complex_task_model",
            label: "Complex Task Model",
            description: "Model for high-complexity / retry tasks",
            kind: ConfigFieldKind::Str,
            group: "Routing",
        },
        ConfigFieldMeta {
            key: "routing.context_strategy",
            label: "Context Strategy",
            description: "How context is assembled for agents",
            kind: ConfigFieldKind::Enum(vec!["mcp_first", "hybrid", "inline_heavy"]),
            group: "Routing",
        },
        ConfigFieldMeta {
            key: "routing.weights.quality",
            label: "Weight: Quality",
            description: "Relative weight for quality/success in routing reward",
            kind: ConfigFieldKind::Float {
                min: Some(0.0),
                max: Some(1.0),
            },
            group: "Routing",
        },
        ConfigFieldMeta {
            key: "routing.weights.cost",
            label: "Weight: Cost",
            description: "Relative weight for low cost in routing reward",
            kind: ConfigFieldKind::Float {
                min: Some(0.0),
                max: Some(1.0),
            },
            group: "Routing",
        },
        ConfigFieldMeta {
            key: "routing.weights.latency",
            label: "Weight: Latency",
            description: "Relative weight for low latency in routing reward",
            kind: ConfigFieldKind::Float {
                min: Some(0.0),
                max: Some(1.0),
            },
            group: "Routing",
        },
        // ── Conductor ──
        ConfigFieldMeta {
            key: "conductor.max_agents",
            label: "Max Agents",
            description: "Maximum concurrently running agents",
            kind: ConfigFieldKind::Int {
                min: Some(1),
                max: Some(32),
                presets: vec![1, 2, 4, 8, 16],
            },
            group: "Conductor",
        },
        ConfigFieldMeta {
            key: "conductor.max_parallel_plans",
            label: "Max Parallel Plans",
            description: "Maximum plans executing in parallel",
            kind: ConfigFieldKind::Int {
                min: Some(1),
                max: Some(16),
                presets: vec![1, 2, 4, 8],
            },
            group: "Conductor",
        },
        ConfigFieldMeta {
            key: "conductor.parallel_enabled",
            label: "Parallel Enabled",
            description: "Enable parallel execution mode",
            kind: ConfigFieldKind::Bool,
            group: "Conductor",
        },
        ConfigFieldMeta {
            key: "conductor.express_mode",
            label: "Express Mode",
            description: "Single implementer, no reviews, auto-fix on gate failure",
            kind: ConfigFieldKind::Bool,
            group: "Conductor",
        },
        ConfigFieldMeta {
            key: "conductor.auto_advance_batch",
            label: "Auto Advance Batch",
            description: "Auto-advance to the next plan on batch completion",
            kind: ConfigFieldKind::Bool,
            group: "Conductor",
        },
        ConfigFieldMeta {
            key: "conductor.auto_merge_on_complete",
            label: "Auto Merge on Complete",
            description: "Auto-merge plans to batch on review completion",
            kind: ConfigFieldKind::Bool,
            group: "Conductor",
        },
        ConfigFieldMeta {
            key: "conductor.max_auto_fix_attempts",
            label: "Max Auto-Fix Attempts",
            description: "Max auto-fix attempts before failing (express mode)",
            kind: ConfigFieldKind::Int {
                min: Some(0),
                max: Some(10),
                presets: vec![0, 1, 3, 5],
            },
            group: "Conductor",
        },
        ConfigFieldMeta {
            key: "conductor.enabled_roles.architect",
            label: "Role: Architect",
            description: "Enable the architect agent role",
            kind: ConfigFieldKind::Bool,
            group: "Conductor",
        },
        ConfigFieldMeta {
            key: "conductor.enabled_roles.auditor",
            label: "Role: Auditor",
            description: "Enable the auditor agent role",
            kind: ConfigFieldKind::Bool,
            group: "Conductor",
        },
        ConfigFieldMeta {
            key: "conductor.enabled_roles.scribe",
            label: "Role: Scribe",
            description: "Enable the scribe agent role",
            kind: ConfigFieldKind::Bool,
            group: "Conductor",
        },
        ConfigFieldMeta {
            key: "conductor.enabled_roles.critic",
            label: "Role: Critic",
            description: "Enable the critic agent role",
            kind: ConfigFieldKind::Bool,
            group: "Conductor",
        },
        // ── Pipeline ──
        ConfigFieldMeta {
            key: "pipeline.mechanical.strategist",
            label: "Mechanical: Strategist",
            description: "Run strategist before implementation for mechanical tasks",
            kind: ConfigFieldKind::Bool,
            group: "Pipeline",
        },
        ConfigFieldMeta {
            key: "pipeline.mechanical.reviewers",
            label: "Mechanical: Reviewers",
            description: "Run reviewers after implementation for mechanical tasks",
            kind: ConfigFieldKind::Bool,
            group: "Pipeline",
        },
        ConfigFieldMeta {
            key: "pipeline.mechanical.max_iterations",
            label: "Mechanical: Max Iterations",
            description: "Max implementation-review iterations for mechanical tasks",
            kind: ConfigFieldKind::Int {
                min: Some(1),
                max: Some(10),
                presets: vec![1, 2, 3],
            },
            group: "Pipeline",
        },
        ConfigFieldMeta {
            key: "pipeline.focused.strategist",
            label: "Focused: Strategist",
            description: "Run strategist before implementation for focused tasks",
            kind: ConfigFieldKind::Bool,
            group: "Pipeline",
        },
        ConfigFieldMeta {
            key: "pipeline.focused.reviewers",
            label: "Focused: Reviewers",
            description: "Run reviewers after implementation for focused tasks",
            kind: ConfigFieldKind::Bool,
            group: "Pipeline",
        },
        ConfigFieldMeta {
            key: "pipeline.focused.max_iterations",
            label: "Focused: Max Iterations",
            description: "Max implementation-review iterations for focused tasks",
            kind: ConfigFieldKind::Int {
                min: Some(1),
                max: Some(10),
                presets: vec![1, 2, 3],
            },
            group: "Pipeline",
        },
        ConfigFieldMeta {
            key: "pipeline.integrative.strategist",
            label: "Integrative: Strategist",
            description: "Run strategist before implementation for integrative tasks",
            kind: ConfigFieldKind::Bool,
            group: "Pipeline",
        },
        ConfigFieldMeta {
            key: "pipeline.integrative.reviewers",
            label: "Integrative: Reviewers",
            description: "Run reviewers after implementation for integrative tasks",
            kind: ConfigFieldKind::Bool,
            group: "Pipeline",
        },
        ConfigFieldMeta {
            key: "pipeline.integrative.max_iterations",
            label: "Integrative: Max Iterations",
            description: "Max implementation-review iterations for integrative tasks",
            kind: ConfigFieldKind::Int {
                min: Some(1),
                max: Some(10),
                presets: vec![1, 2, 3, 5],
            },
            group: "Pipeline",
        },
        ConfigFieldMeta {
            key: "pipeline.architectural.strategist",
            label: "Architectural: Strategist",
            description: "Run strategist before implementation for architectural tasks",
            kind: ConfigFieldKind::Bool,
            group: "Pipeline",
        },
        ConfigFieldMeta {
            key: "pipeline.architectural.reviewers",
            label: "Architectural: Reviewers",
            description: "Run reviewers after implementation for architectural tasks",
            kind: ConfigFieldKind::Bool,
            group: "Pipeline",
        },
        ConfigFieldMeta {
            key: "pipeline.architectural.max_iterations",
            label: "Architectural: Max Iterations",
            description: "Max implementation-review iterations for architectural tasks",
            kind: ConfigFieldKind::Int {
                min: Some(1),
                max: Some(10),
                presets: vec![1, 2, 3, 5],
            },
            group: "Pipeline",
        },
        // ── Learning ──
        ConfigFieldMeta {
            key: "learning.auto_playbook_refresh",
            label: "Auto Playbook Refresh",
            description: "Auto-refresh playbook rules after successful tasks",
            kind: ConfigFieldKind::Bool,
            group: "Learning",
        },
        ConfigFieldMeta {
            key: "learning.knowledge_file_intel",
            label: "Knowledge File Intel",
            description: "Inject file difficulty profiles into agent context",
            kind: ConfigFieldKind::Bool,
            group: "Learning",
        },
        ConfigFieldMeta {
            key: "learning.knowledge_warnings",
            label: "Knowledge Warnings",
            description: "Inject warnings into agent context",
            kind: ConfigFieldKind::Bool,
            group: "Learning",
        },
        ConfigFieldMeta {
            key: "learning.learning_min_occurrences",
            label: "Min Occurrences",
            description: "Min occurrences before promoting learned rules",
            kind: ConfigFieldKind::Int {
                min: Some(1),
                max: Some(20),
                presets: vec![1, 2, 3, 5],
            },
            group: "Learning",
        },
        // ── PRD ──
        ConfigFieldMeta {
            key: "prd.auto_plan",
            label: "Auto Plan",
            description: "Automatically generate a plan when a PRD is promoted",
            kind: ConfigFieldKind::Bool,
            group: "PRD",
        },
        // ── Project ──
        ConfigFieldMeta {
            key: "project.name",
            label: "Name",
            description: "Human-readable project name",
            kind: ConfigFieldKind::Str,
            group: "Project",
        },
        ConfigFieldMeta {
            key: "project.root",
            label: "Root",
            description: "Project root directory (relative or absolute)",
            kind: ConfigFieldKind::Str,
            group: "Project",
        },
        ConfigFieldMeta {
            key: "project.fresh_base_branch",
            label: "Fresh Base Branch",
            description: "Git branch used as the base for fresh batch/worktree creation",
            kind: ConfigFieldKind::Str,
            group: "Project",
        },
        // ── TUI ──
        ConfigFieldMeta {
            key: "tui.refresh_rate_ms",
            label: "Refresh Rate (ms)",
            description: "TUI refresh interval in milliseconds",
            kind: ConfigFieldKind::Int {
                min: Some(50),
                max: Some(5000),
                presets: vec![100, 250, 500, 1000],
            },
            group: "TUI",
        },
        ConfigFieldMeta {
            key: "tui.effects.screen_postfx",
            label: "Screen PostFX",
            description: "Enable full-screen post-processing and modal glow effects",
            kind: ConfigFieldKind::Bool,
            group: "TUI",
        },
        // ── Server ──
        ConfigFieldMeta {
            key: "server.bind",
            label: "Bind Address",
            description: "Address to bind the HTTP server to",
            kind: ConfigFieldKind::Str,
            group: "Server",
        },
        ConfigFieldMeta {
            key: "server.port",
            label: "Port",
            description: "Port number for the HTTP server",
            kind: ConfigFieldKind::Int {
                min: Some(1),
                max: Some(65535),
                presets: vec![8080, 9090, 3000],
            },
            group: "Server",
        },
    ]
}

/// Group display order.
const GROUP_ORDER: &[&str] = &[
    "Agent",
    "Budget",
    "Gates",
    "Routing",
    "Conductor",
    "Pipeline",
    "Learning",
    "PRD",
    "Project",
    "TUI",
    "Server",
];

/// Resolve the current value of a config field from the TOML tree.
fn resolve_value(key: &str, toml_root: Option<&toml::Value>) -> Option<String> {
    let root = toml_root?.as_table()?;
    let parts: Vec<&str> = key.split('.').collect();
    let mut current: &toml::Value = root.get(parts[0])?;
    for part in &parts[1..] {
        current = current.as_table()?.get(*part)?;
    }
    Some(format_toml_value(current))
}

/// Resolve the default value of a config field from the default config.
fn resolve_default(key: &str, defaults: &toml::Value) -> Option<String> {
    resolve_value(key, Some(defaults)).or_else(|| match key {
        "tui.effects.screen_postfx" => Some("false".to_string()),
        _ => None,
    })
}

/// Determine the source of a field's value.
fn determine_source(
    key: &str,
    value: &str,
    default_value: Option<&str>,
    env_overrides: &HashMap<String, String>,
) -> ConfigSource {
    if env_overrides.contains_key(key) {
        return ConfigSource::Env;
    }
    if let Some(default_val) = default_value {
        if value != default_val {
            return ConfigSource::File;
        }
    }
    ConfigSource::Default
}

/// Build the flattened display list from metadata + current config + pending edits.
#[allow(clippy::implicit_hasher)]
pub fn build_flat_items(root: &Path, pending: &HashMap<String, String>) -> Vec<ConfigItem> {
    let fields = all_fields();
    let env_overrides = active_env_overrides();

    let config_path = root.join("roko.toml");
    let toml_config: Option<toml::Value> = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| s.parse().ok());

    let default_cfg = roko_core::config::RokoConfig::default();
    let default_toml: toml::Value = default_cfg
        .to_toml()
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()));

    let mut items = Vec::new();
    let mut current_group: Option<&str> = None;

    for group_name in GROUP_ORDER {
        let group_fields: Vec<&ConfigFieldMeta> =
            fields.iter().filter(|f| f.group == *group_name).collect();
        if group_fields.is_empty() {
            continue;
        }

        if current_group != Some(group_name) {
            items.push(ConfigItem::Header(group_name.to_string()));
            current_group = Some(group_name);
        }

        for meta in group_fields {
            // Use pending edit if present, otherwise resolve from TOML, then default
            let value = pending
                .get(meta.key)
                .cloned()
                .or_else(|| resolve_value(meta.key, toml_config.as_ref()))
                .or_else(|| resolve_default(meta.key, &default_toml))
                .unwrap_or_default();

            let default_str = resolve_default(meta.key, &default_toml);
            let source = if pending.contains_key(meta.key) {
                // Pending edit -- treat as file-sourced (will be saved)
                ConfigSource::File
            } else {
                determine_source(meta.key, &value, default_str.as_deref(), &env_overrides)
            };

            items.push(ConfigItem::Field {
                meta: meta.clone(),
                value,
                source,
            });
        }
    }

    items.push(ConfigItem::SaveButton);
    items
}

/// Format a TOML value for display.
pub fn format_toml_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => {
            if s.is_empty() {
                "(empty)".to_string()
            } else {
                s.clone()
            }
        }
        toml::Value::Integer(n) => n.to_string(),
        toml::Value::Float(f) => {
            if (*f - f.floor()).abs() < f64::EPSILON && f.abs() < 1_000_000.0 {
                format!("{f:.1}")
            } else {
                format!("{f}")
            }
        }
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(format_toml_value).collect();
            format!("[{}]", items.join(", "))
        }
        toml::Value::Table(_) => "{...}".to_string(),
        toml::Value::Datetime(dt) => dt.to_string(),
    }
}

/// Save pending edits into roko.toml.
///
/// Reads the existing file as a TOML value tree, patches the changed keys,
/// and writes the result back. Uses the `toml` crate (already a dependency)
/// rather than `toml_edit` to avoid adding a new dep.
#[allow(clippy::implicit_hasher)]
pub fn save_pending_edits(root: &Path, pending: &HashMap<String, String>) -> Result<(), String> {
    if pending.is_empty() {
        return Ok(());
    }

    let config_path = root.join("roko.toml");
    let content =
        std::fs::read_to_string(&config_path).map_err(|e| format!("read roko.toml: {e}"))?;

    let mut root_val: toml::Value = content
        .parse()
        .map_err(|e| format!("parse roko.toml: {e}"))?;

    let fields = all_fields();

    for (key, value) in pending {
        let meta = fields.iter().find(|f| f.key == key);
        let toml_val = coerce_to_toml(value, meta.map(|m| &m.kind));
        set_toml_path(&mut root_val, key, toml_val)?;
    }

    // Re-serialize via RokoConfig for consistent formatting
    let toml_str =
        toml::to_string_pretty(&root_val).map_err(|e| format!("serialize roko.toml: {e}"))?;

    std::fs::write(&config_path, toml_str).map_err(|e| format!("write roko.toml: {e}"))?;

    Ok(())
}

/// Coerce a string value to the appropriate TOML type based on field metadata.
fn coerce_to_toml(value: &str, kind: Option<&ConfigFieldKind>) -> toml::Value {
    match kind {
        Some(ConfigFieldKind::Bool) => toml::Value::Boolean(value == "true"),
        Some(ConfigFieldKind::Int { .. }) => value.parse::<i64>().map_or_else(
            |_| toml::Value::String(value.to_string()),
            toml::Value::Integer,
        ),
        Some(ConfigFieldKind::Float { .. }) => value.parse::<f64>().map_or_else(
            |_| toml::Value::String(value.to_string()),
            toml::Value::Float,
        ),
        _ => toml::Value::String(value.to_string()),
    }
}

/// Set a dotted key path in a TOML value tree, creating intermediate tables
/// as needed.
fn set_toml_path(root: &mut toml::Value, key: &str, val: toml::Value) -> Result<(), String> {
    let parts: Vec<&str> = key.split('.').collect();
    let mut current = root;

    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Leaf: set the value
            if let Some(table) = current.as_table_mut() {
                table.insert(part.to_string(), val);
                return Ok(());
            }
            return Err(format!("path '{key}': parent is not a table"));
        }
        // Intermediate: ensure table exists and descend
        if !current.as_table().is_some_and(|t| t.contains_key(*part)) {
            if let Some(table) = current.as_table_mut() {
                table.insert(part.to_string(), toml::Value::Table(toml::map::Map::new()));
            }
        }
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .ok_or_else(|| format!("path '{key}': cannot descend into '{part}'"))?;
    }

    Err("empty key path".to_string())
}

/// Truncate a string to `max` chars, appending `...` if needed.
pub fn truncate(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        return s.to_string();
    }
    if max <= 3 {
        return ".".repeat(max);
    }

    let keep = max - 3;
    let truncated = s.chars().take(keep).collect::<String>();
    format!("{truncated}...")
}

/// Format a token count with K/M suffix.
#[allow(clippy::cast_precision_loss)]
pub fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncate_preserves_unicode_boundaries() {
        let input = "── Agent ─────────────────────────────────────────────────────────────────────────────────";
        assert_eq!(truncate(input, 12), "── Agent ...");
    }

    #[test]
    fn truncate_handles_tiny_widths() {
        assert_eq!(truncate("abcdef", 3), "...");
        assert_eq!(truncate("abcdef", 2), "..");
    }
}
