//! Unified `RokoConfig` schema with hierarchical sections.
//!
//! Every section is a separate struct so callers can destructure just the
//! slice they need. All fields carry serde defaults so a bare `schema_version = 2`
//! produces a fully-populated config.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// Current schema version. Bump on incompatible changes.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// A non-fatal configuration warning produced by [`RokoConfig::validate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigWarning {
    /// Human-readable description of the issue.
    pub message: String,
}

// ---- top-level -----------------------------------------------------------

/// Root configuration for the Roko runtime.
///
/// ```toml
/// schema_version = 2
///
/// [project]
/// name = "my-dapp"
///
/// [agent]
/// default_model = "claude-sonnet-4-6"
///
/// [agent.roles.implementer]
/// model = "claude-opus-4-6"
/// ```
#[allow(clippy::derive_partial_eq_without_eq)] // contains f32 via BudgetConfig
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RokoConfig {
    /// Schema version for migration tooling.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    /// Project metadata.
    #[serde(default)]
    pub project: ProjectConfig,

    /// Agent / model settings (including per-role overrides).
    #[serde(default)]
    pub agent: AgentConfig,

    /// Verification gates.
    #[serde(default)]
    pub gates: GatesConfig,

    /// Model routing configuration.
    #[serde(default)]
    pub routing: RoutingConfig,

    /// Spend / token budgets.
    #[serde(default)]
    pub budget: BudgetConfig,

    /// Conductor (meta-orchestrator) settings.
    #[serde(default)]
    pub conductor: ConductorConfig,

    /// Learning subsystem toggles.
    #[serde(default)]
    pub learning: LearningConfig,

    /// Terminal UI preferences.
    #[serde(default)]
    pub tui: TuiConfig,

    /// HTTP server / gateway settings.
    #[serde(default)]
    pub server: ServerConfig,
}

const fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

impl Default for RokoConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            project: ProjectConfig::default(),
            agent: AgentConfig::default(),
            gates: GatesConfig::default(),
            routing: RoutingConfig::default(),
            budget: BudgetConfig::default(),
            conductor: ConductorConfig::default(),
            learning: LearningConfig::default(),
            tui: TuiConfig::default(),
            server: ServerConfig::default(),
        }
    }
}

impl RokoConfig {
    /// Parse from a TOML string.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Render to a TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }

    /// Render to a pretty-printed TOML string (for config files / examples).
    pub fn to_toml_pretty(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Returns `true` when this config was written with an older schema version.
    #[must_use]
    pub const fn is_stale(&self) -> bool {
        self.schema_version < CURRENT_SCHEMA_VERSION
    }

    /// Apply environment variable overrides.
    ///
    /// Recognized variables:
    /// - `ROKO_MODEL` -- sets `agent.default_model`
    /// - `ROKO_BACKEND` -- sets `agent.default_backend`
    /// - `ROKO_EFFORT` -- sets `agent.default_effort`
    /// - `ROKO_CONTEXT_LIMIT_K` -- sets `agent.context_limit_k`
    /// - `ROKO_MAX_AGENTS` -- sets `conductor.max_agents`
    /// - `ROKO_BUDGET_USD` -- sets `budget.max_plan_usd`
    /// - `ROKO_PARALLEL` -- sets `conductor.parallel_enabled`
    /// - `ROKO_EXPRESS` -- sets `conductor.express_mode`
    /// - `ROKO_SKIP_TESTS` -- sets `gates.skip_tests`
    /// - `ROKO_CLIPPY` -- sets `gates.clippy_enabled`
    pub fn apply_env(&mut self, env_fn: &dyn Fn(&str) -> Option<String>) {
        if let Some(v) = env_fn("ROKO_MODEL") {
            self.agent.default_model = v;
        }
        if let Some(v) = env_fn("ROKO_BACKEND") {
            self.agent.default_backend = v;
        }
        if let Some(v) = env_fn("ROKO_EFFORT") {
            self.agent.default_effort = v;
        }
        if let Some(v) = env_fn("ROKO_CONTEXT_LIMIT_K") {
            if let Ok(n) = v.parse::<u32>() {
                self.agent.context_limit_k = n;
            }
        }
        if let Some(v) = env_fn("ROKO_MAX_AGENTS") {
            if let Ok(n) = v.parse::<usize>() {
                self.conductor.max_agents = n;
            }
        }
        if let Some(v) = env_fn("ROKO_BUDGET_USD") {
            if let Ok(n) = v.parse::<f32>() {
                self.budget.max_plan_usd = n;
            }
        }
        if let Some(v) = env_fn("ROKO_PARALLEL") {
            self.conductor.parallel_enabled = parse_bool_env(&v);
        }
        if let Some(v) = env_fn("ROKO_EXPRESS") {
            self.conductor.express_mode = parse_bool_env(&v);
        }
        if let Some(v) = env_fn("ROKO_SKIP_TESTS") {
            self.gates.skip_tests = parse_bool_env(&v);
        }
        if let Some(v) = env_fn("ROKO_CLIPPY") {
            self.gates.clippy_enabled = parse_bool_env(&v);
        }
    }

    /// Convenience: apply overrides from the real process environment.
    pub fn apply_process_env(&mut self) {
        self.apply_env(&|key| std::env::var(key).ok());
    }

    /// Validate the configuration and return any warnings.
    ///
    /// This is a non-fatal check: warnings indicate values that are likely
    /// misconfigured but don't prevent the runtime from starting.
    #[must_use]
    pub fn validate(&self) -> Vec<ConfigWarning> {
        let mut warnings = Vec::new();

        if self.budget.max_plan_usd <= 0.0 {
            warnings.push(ConfigWarning {
                message: "budget.max_plan_usd must be > 0".into(),
            });
        }
        if self.budget.max_turn_usd <= 0.0 {
            warnings.push(ConfigWarning {
                message: "budget.max_turn_usd must be > 0".into(),
            });
        }
        if self.conductor.max_agents < 1 {
            warnings.push(ConfigWarning {
                message: "conductor.max_agents must be >= 1".into(),
            });
        }
        if self.conductor.max_parallel_plans < 1 {
            warnings.push(ConfigWarning {
                message: "conductor.max_parallel_plans must be >= 1".into(),
            });
        }
        if self.gates.max_iterations < 1 {
            warnings.push(ConfigWarning {
                message: "gates.max_iterations must be >= 1".into(),
            });
        }
        if self.agent.context_limit_k < 50 {
            warnings.push(ConfigWarning {
                message: "agent.context_limit_k must be >= 50".into(),
            });
        }

        warnings
    }

    /// Generate an example config string showing every field with doc comments.
    #[must_use]
    pub fn example_toml() -> String {
        let cfg = Self::default();
        let mut out = String::with_capacity(4096);

        // Infallible writes to String -- unwrap is safe.
        let _ = writeln!(out, "# Roko configuration -- all fields shown with defaults.");
        let _ = writeln!(out, "# Delete any section you don't need; defaults apply.\n");
        let _ = writeln!(out, "schema_version = {CURRENT_SCHEMA_VERSION}\n");

        let _ = writeln!(out, "# -- Project metadata --");
        let _ = writeln!(out, "[project]");
        let _ = writeln!(out, "name = \"{}\"", cfg.project.name);
        let _ = writeln!(out, "root = \"{}\"", cfg.project.root);
        let _ = writeln!(out, "fresh_base_branch = \"{}\"\n", cfg.project.fresh_base_branch);

        let _ = writeln!(out, "# -- Agent / model settings --");
        let _ = writeln!(out, "[agent]");
        let _ = writeln!(out, "default_model = \"{}\"", cfg.agent.default_model);
        let _ = writeln!(out, "default_backend = \"{}\"", cfg.agent.default_backend);
        let _ = writeln!(out, "default_effort = \"{}\"", cfg.agent.default_effort);
        let _ = writeln!(out, "context_limit_k = {}", cfg.agent.context_limit_k);
        let _ = writeln!(out, "bare_mode = {}\n", cfg.agent.bare_mode);

        let _ = writeln!(out, "# Per-role overrides (repeat for each role):");
        let _ = writeln!(out, "# [agent.roles.implementer]");
        let _ = writeln!(out, "# model = \"claude-opus-4-6\"");
        let _ = writeln!(out, "# effort = \"high\"");
        let _ = writeln!(out, "# context_limit_k = 200\n");

        let _ = writeln!(out, "# -- Verification gates --");
        let _ = writeln!(out, "[gates]");
        let _ = writeln!(out, "clippy_enabled = {}", cfg.gates.clippy_enabled);
        let _ = writeln!(out, "skip_tests = {}", cfg.gates.skip_tests);
        let _ = writeln!(out, "max_iterations = {}\n", cfg.gates.max_iterations);

        let _ = writeln!(out, "# -- Model routing --");
        let _ = writeln!(out, "[routing]");
        let _ = writeln!(out, "mode = \"{}\"", cfg.routing.mode);
        let _ = writeln!(out, "fast_task_model = \"{}\"", cfg.routing.fast_task_model);
        let _ = writeln!(out, "standard_task_model = \"{}\"", cfg.routing.standard_task_model);
        let _ = writeln!(out, "complex_task_model = \"{}\"\n", cfg.routing.complex_task_model);

        let _ = writeln!(out, "# -- Spend / token budgets --");
        let _ = writeln!(out, "[budget]");
        let _ = writeln!(out, "max_plan_usd = {:.1}", cfg.budget.max_plan_usd);
        let _ = writeln!(out, "max_turn_usd = {:.1}", cfg.budget.max_turn_usd);
        let _ = writeln!(out, "prompt_token_budget = {}\n", cfg.budget.prompt_token_budget);

        let _ = writeln!(out, "# -- Conductor (meta-orchestrator) --");
        let _ = writeln!(out, "[conductor]");
        let _ = writeln!(out, "max_agents = {}", cfg.conductor.max_agents);
        let _ = writeln!(out, "max_parallel_plans = {}", cfg.conductor.max_parallel_plans);
        let _ = writeln!(out, "parallel_enabled = {}", cfg.conductor.parallel_enabled);
        let _ = writeln!(out, "express_mode = {}", cfg.conductor.express_mode);
        let _ = writeln!(out, "auto_advance_batch = {}", cfg.conductor.auto_advance_batch);
        let _ = writeln!(out, "auto_merge_on_complete = {}", cfg.conductor.auto_merge_on_complete);
        let _ = writeln!(out, "pre_plan = {}", cfg.conductor.pre_plan);
        let _ = writeln!(out, "max_auto_fix_attempts = {}\n", cfg.conductor.max_auto_fix_attempts);

        let _ = writeln!(out, "# -- Learning subsystem --");
        let _ = writeln!(out, "[learning]");
        let _ = writeln!(out, "auto_playbook_refresh = {}", cfg.learning.auto_playbook_refresh);
        let _ = writeln!(out, "knowledge_warnings = {}", cfg.learning.knowledge_warnings);
        let _ = writeln!(out, "learning_min_occurrences = {}\n", cfg.learning.learning_min_occurrences);

        let _ = writeln!(out, "# -- TUI preferences --");
        let _ = writeln!(out, "[tui]");
        let _ = writeln!(out, "refresh_rate_ms = {}\n", cfg.tui.refresh_rate_ms);

        let _ = writeln!(out, "# -- HTTP server / gateway --");
        let _ = writeln!(out, "[server]");
        let _ = writeln!(out, "bind = \"{}\"", cfg.server.bind);
        let _ = writeln!(out, "port = {}", cfg.server.port);

        out
    }
}

fn parse_bool_env(s: &str) -> bool {
    matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

// ---- [project] -----------------------------------------------------------

/// Project-level metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Human-readable project name.
    #[serde(default = "default_project_name")]
    pub name: String,
    /// Project root directory (relative or absolute).
    #[serde(default = "default_dot")]
    pub root: String,
    /// Git branch used as the base for fresh batch/worktree creation.
    #[serde(default = "default_fresh_base_branch")]
    pub fresh_base_branch: String,
}

fn default_project_name() -> String {
    "roko-project".into()
}

fn default_dot() -> String {
    ".".into()
}

fn default_fresh_base_branch() -> String {
    "main".into()
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            name: default_project_name(),
            root: default_dot(),
            fresh_base_branch: default_fresh_base_branch(),
        }
    }
}

// ---- [agent] -------------------------------------------------------------

/// Agent / model configuration, including per-role overrides.
#[allow(clippy::derive_partial_eq_without_eq)] // contains f32 via RoleOverride
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Default model slug (e.g. `"claude-sonnet-4-6"`).
    #[serde(default = "default_model")]
    pub default_model: String,
    /// Default backend (e.g. `"claude"`, `"codex"`, `"cursor"`).
    #[serde(default = "default_backend")]
    pub default_backend: String,
    /// Default reasoning effort (`"low"`, `"medium"`, `"high"`, `"max"`).
    #[serde(default = "default_effort")]
    pub default_effort: String,
    /// Context window limit in thousands of tokens.
    #[serde(default = "default_context_limit_k")]
    pub context_limit_k: u32,
    /// When true, agents use `--bare` (skip built-in system prompt).
    #[serde(default = "default_true")]
    pub bare_mode: bool,
    /// Global fallback model: if an agent spawn fails, retry with this.
    #[serde(default)]
    pub fallback_model: Option<String>,
    /// Per-role overrides keyed by role label (e.g. `"implementer"`, `"architect"`).
    #[serde(default)]
    pub roles: HashMap<String, RoleOverride>,
}

fn default_model() -> String {
    "claude-sonnet-4-6".into()
}

fn default_backend() -> String {
    "claude".into()
}

fn default_effort() -> String {
    "medium".into()
}

const fn default_context_limit_k() -> u32 {
    200
}

const fn default_true() -> bool {
    true
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            default_model: default_model(),
            default_backend: default_backend(),
            default_effort: default_effort(),
            context_limit_k: default_context_limit_k(),
            bare_mode: default_true(),
            fallback_model: None,
            roles: HashMap::new(),
        }
    }
}

/// Per-role override under `[agent.roles.<role>]`.
///
/// Every field is optional; absent means "use the agent-level default".
#[allow(clippy::derive_partial_eq_without_eq)] // contains f32
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleOverride {
    /// Model slug override for this role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Backend override for this role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// Reasoning effort override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Context window override (in thousands of tokens).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit_k: Option<u32>,
    /// Turn budget override (USD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_budget_usd: Option<f32>,
}

// ---- [gates] -------------------------------------------------------------

/// Gate (verification) settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatesConfig {
    /// Enable clippy / lint gate.
    #[serde(default = "default_true")]
    pub clippy_enabled: bool,
    /// Skip test gate entirely.
    #[serde(default)]
    pub skip_tests: bool,
    /// Max gate retry iterations before giving up.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
}

const fn default_max_iterations() -> u32 {
    3
}

impl Default for GatesConfig {
    fn default() -> Self {
        Self {
            clippy_enabled: default_true(),
            skip_tests: false,
            max_iterations: default_max_iterations(),
        }
    }
}

// ---- [routing] -----------------------------------------------------------

/// Model routing configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Routing mode (`"auto_override"`).
    #[serde(default = "default_routing_mode")]
    pub mode: String,
    /// Model for low-complexity tasks.
    #[serde(default = "default_fast_model")]
    pub fast_task_model: String,
    /// Model for standard-complexity tasks.
    #[serde(default = "default_standard_model")]
    pub standard_task_model: String,
    /// Model for high-complexity / retry tasks.
    #[serde(default = "default_complex_model")]
    pub complex_task_model: String,
    /// Context strategy (`"mcp_first"`, `"hybrid"`, `"inline_heavy"`).
    #[serde(default = "default_context_strategy")]
    pub context_strategy: String,
}

fn default_routing_mode() -> String {
    "auto_override".into()
}

fn default_fast_model() -> String {
    "claude-haiku-4-5".into()
}

fn default_standard_model() -> String {
    "claude-sonnet-4-6".into()
}

fn default_complex_model() -> String {
    "claude-opus-4-6".into()
}

fn default_context_strategy() -> String {
    "mcp_first".into()
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            mode: default_routing_mode(),
            fast_task_model: default_fast_model(),
            standard_task_model: default_standard_model(),
            complex_task_model: default_complex_model(),
            context_strategy: default_context_strategy(),
        }
    }
}

// ---- [budget] ------------------------------------------------------------

/// Spend / token budget settings.
#[allow(clippy::derive_partial_eq_without_eq)] // contains f32
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Max dollars to spend per plan.
    #[serde(default = "default_max_plan_usd")]
    pub max_plan_usd: f32,
    /// Max dollars per single agent turn.
    #[serde(default = "default_max_turn_usd")]
    pub max_turn_usd: f32,
    /// Token budget for prompt composition.
    #[serde(default = "default_prompt_token_budget")]
    pub prompt_token_budget: usize,
}

const fn default_max_plan_usd() -> f32 {
    25.0
}

const fn default_max_turn_usd() -> f32 {
    3.0
}

const fn default_prompt_token_budget() -> usize {
    10_000
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_plan_usd: default_max_plan_usd(),
            max_turn_usd: default_max_turn_usd(),
            prompt_token_budget: default_prompt_token_budget(),
        }
    }
}

// ---- [conductor] ---------------------------------------------------------

/// Conductor (meta-orchestrator) settings.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConductorConfig {
    /// Max concurrently running agents.
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,
    /// Max plans executing in parallel.
    #[serde(default = "default_max_parallel_plans")]
    pub max_parallel_plans: usize,
    /// Enable parallel execution mode.
    #[serde(default)]
    pub parallel_enabled: bool,
    /// Express mode: single implementer, no reviews, auto-fix on gate failure.
    #[serde(default)]
    pub express_mode: bool,
    /// Auto-advance to the next plan on batch completion.
    #[serde(default = "default_true")]
    pub auto_advance_batch: bool,
    /// Auto-merge plans to batch on review completion.
    #[serde(default)]
    pub auto_merge_on_complete: bool,
    /// Enable the pre-planning phase.
    #[serde(default)]
    pub pre_plan: bool,
    /// Max auto-fix attempts before failing (express mode).
    #[serde(default = "default_max_auto_fix")]
    pub max_auto_fix_attempts: u32,
    /// Model for the auto-fixer agent.
    #[serde(default = "default_auto_fix_model")]
    pub auto_fix_model: String,
    /// Conductor-specific model override.
    #[serde(default)]
    pub conductor_model: Option<String>,
    /// Warm implementers to keep per active plan.
    #[serde(default = "default_warm_impl")]
    pub warm_implementers_per_plan: usize,
    /// Enable individual agent roles.
    #[serde(default)]
    pub enabled_roles: AgentRoleToggles,
}

const fn default_max_agents() -> usize {
    8
}

const fn default_max_parallel_plans() -> usize {
    1
}

const fn default_max_auto_fix() -> u32 {
    3
}

fn default_auto_fix_model() -> String {
    "claude-haiku-4-5".into()
}

const fn default_warm_impl() -> usize {
    1
}

impl Default for ConductorConfig {
    fn default() -> Self {
        Self {
            max_agents: default_max_agents(),
            max_parallel_plans: default_max_parallel_plans(),
            parallel_enabled: false,
            express_mode: false,
            auto_advance_batch: true,
            auto_merge_on_complete: false,
            pre_plan: false,
            max_auto_fix_attempts: default_max_auto_fix(),
            auto_fix_model: default_auto_fix_model(),
            conductor_model: None,
            warm_implementers_per_plan: default_warm_impl(),
            enabled_roles: AgentRoleToggles::default(),
        }
    }
}

/// Toggles for optional agent roles.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRoleToggles {
    /// Enable the architect role.
    #[serde(default = "default_true")]
    pub architect: bool,
    /// Enable the auditor role.
    #[serde(default = "default_true")]
    pub auditor: bool,
    /// Enable the scribe role.
    #[serde(default = "default_true")]
    pub scribe: bool,
    /// Enable the critic role.
    #[serde(default = "default_true")]
    pub critic: bool,
}

impl Default for AgentRoleToggles {
    fn default() -> Self {
        Self {
            architect: true,
            auditor: true,
            scribe: true,
            critic: true,
        }
    }
}

// ---- [learning] ----------------------------------------------------------

/// Learning subsystem configuration.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningConfig {
    /// Auto-refresh playbook rules after successful tasks.
    #[serde(default = "default_true")]
    pub auto_playbook_refresh: bool,
    /// Inject file difficulty profiles into agent context.
    #[serde(default = "default_true")]
    pub knowledge_file_intel: bool,
    /// Inject grimoire warnings into agent context.
    #[serde(default = "default_true")]
    pub knowledge_warnings: bool,
    /// Enable cross-task wave context propagation.
    #[serde(default = "default_true")]
    pub knowledge_wave_context: bool,
    /// Enable error signature pattern matching.
    #[serde(default = "default_true")]
    pub knowledge_error_patterns: bool,
    /// Min occurrences before promoting learned rules.
    #[serde(default = "default_learning_min_occ")]
    pub learning_min_occurrences: usize,
    /// Max file-intel entries to inject per task.
    #[serde(default = "default_file_intel_max")]
    pub file_intel_max_entries: usize,
    /// Max warning entries to inject per task.
    #[serde(default = "default_warning_max")]
    pub warning_max_entries: usize,
}

const fn default_learning_min_occ() -> usize {
    2
}

const fn default_file_intel_max() -> usize {
    15
}

const fn default_warning_max() -> usize {
    5
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            auto_playbook_refresh: true,
            knowledge_file_intel: true,
            knowledge_warnings: true,
            knowledge_wave_context: true,
            knowledge_error_patterns: true,
            learning_min_occurrences: default_learning_min_occ(),
            file_intel_max_entries: default_file_intel_max(),
            warning_max_entries: default_warning_max(),
        }
    }
}

// ---- [tui] ---------------------------------------------------------------

/// Terminal UI preferences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiConfig {
    /// Refresh interval in milliseconds.
    #[serde(default = "default_refresh_rate")]
    pub refresh_rate_ms: u64,
}

const fn default_refresh_rate() -> u64 {
    250
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            refresh_rate_ms: default_refresh_rate(),
        }
    }
}

// ---- [server] ------------------------------------------------------------

/// HTTP server / gateway settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Address to bind to.
    #[serde(default = "default_bind")]
    pub bind: String,
    /// Port number.
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_bind() -> String {
    "127.0.0.1".into()
}

const fn default_port() -> u16 {
    9090
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
        }
    }
}

// ---- tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrips_through_toml() {
        let cfg = RokoConfig::default();
        let text = cfg.to_toml().expect("serialize");
        let back = RokoConfig::from_toml(&text).expect("deserialize");
        assert_eq!(cfg, back);
    }

    #[test]
    fn empty_toml_uses_all_defaults() {
        let cfg = RokoConfig::from_toml("").expect("parse empty");
        assert_eq!(cfg, RokoConfig::default());
    }

    #[test]
    fn schema_version_defaults_to_current() {
        let cfg = RokoConfig::from_toml("").expect("parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn is_stale_detects_old_version() {
        let mut cfg = RokoConfig::default();
        cfg.schema_version = 1;
        assert!(cfg.is_stale());
    }

    #[test]
    fn is_stale_returns_false_for_current() {
        let cfg = RokoConfig::default();
        assert!(!cfg.is_stale());
    }

    #[test]
    fn project_section_parses() {
        let toml = r#"
[project]
name = "my-dapp"
root = "/home/user/code"
fresh_base_branch = "develop"
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.project.name, "my-dapp");
        assert_eq!(cfg.project.root, "/home/user/code");
        assert_eq!(cfg.project.fresh_base_branch, "develop");
    }

    #[test]
    fn agent_section_with_role_overrides() {
        let toml = r#"
[agent]
default_model = "claude-opus-4-6"
default_effort = "high"

[agent.roles.implementer]
model = "claude-sonnet-4-6"
effort = "max"
context_limit_k = 300

[agent.roles.architect]
model = "claude-opus-4-6"
turn_budget_usd = 5.0
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.agent.default_model, "claude-opus-4-6");
        assert_eq!(cfg.agent.default_effort, "high");

        let imp = cfg.agent.roles.get("implementer").expect("implementer");
        assert_eq!(imp.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(imp.effort.as_deref(), Some("max"));
        assert_eq!(imp.context_limit_k, Some(300));

        let arch = cfg.agent.roles.get("architect").expect("architect");
        assert_eq!(arch.model.as_deref(), Some("claude-opus-4-6"));
        assert!((arch.turn_budget_usd.expect("budget") - 5.0).abs() < f32::EPSILON);
    }

    #[test]
    fn gates_section_parses() {
        let toml = r#"
[gates]
clippy_enabled = false
skip_tests = true
max_iterations = 5
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert!(!cfg.gates.clippy_enabled);
        assert!(cfg.gates.skip_tests);
        assert_eq!(cfg.gates.max_iterations, 5);
    }

    #[test]
    fn routing_section_parses() {
        let toml = r#"
[routing]
mode = "manual"
fast_task_model = "haiku"
standard_task_model = "sonnet"
complex_task_model = "opus"
context_strategy = "inline_heavy"
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.routing.mode, "manual");
        assert_eq!(cfg.routing.context_strategy, "inline_heavy");
    }

    #[test]
    fn budget_section_parses() {
        let toml = r#"
[budget]
max_plan_usd = 100.0
max_turn_usd = 10.0
prompt_token_budget = 20000
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert!((cfg.budget.max_plan_usd - 100.0).abs() < f32::EPSILON);
        assert!((cfg.budget.max_turn_usd - 10.0).abs() < f32::EPSILON);
        assert_eq!(cfg.budget.prompt_token_budget, 20_000);
    }

    #[test]
    fn conductor_section_parses() {
        let toml = r#"
[conductor]
max_agents = 16
parallel_enabled = true
express_mode = true
max_auto_fix_attempts = 5

[conductor.enabled_roles]
architect = false
critic = false
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.conductor.max_agents, 16);
        assert!(cfg.conductor.parallel_enabled);
        assert!(cfg.conductor.express_mode);
        assert_eq!(cfg.conductor.max_auto_fix_attempts, 5);
        assert!(!cfg.conductor.enabled_roles.architect);
        assert!(!cfg.conductor.enabled_roles.critic);
        // defaults preserved
        assert!(cfg.conductor.enabled_roles.auditor);
        assert!(cfg.conductor.enabled_roles.scribe);
    }

    #[test]
    fn learning_section_parses() {
        let toml = r#"
[learning]
auto_playbook_refresh = false
learning_min_occurrences = 5
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert!(!cfg.learning.auto_playbook_refresh);
        assert_eq!(cfg.learning.learning_min_occurrences, 5);
    }

    #[test]
    fn tui_section_parses() {
        let toml = r#"
[tui]
refresh_rate_ms = 500
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.tui.refresh_rate_ms, 500);
    }

    #[test]
    fn server_section_parses() {
        let toml = r#"
[server]
bind = "0.0.0.0"
port = 8080
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.server.bind, "0.0.0.0");
        assert_eq!(cfg.server.port, 8080);
    }

    #[test]
    fn env_overrides_apply() {
        let mut cfg = RokoConfig::default();
        let env = |key: &str| -> Option<String> {
            match key {
                "ROKO_MODEL" => Some("claude-opus-4-6".into()),
                "ROKO_BACKEND" => Some("codex".into()),
                "ROKO_EFFORT" => Some("max".into()),
                "ROKO_CONTEXT_LIMIT_K" => Some("300".into()),
                "ROKO_MAX_AGENTS" => Some("16".into()),
                "ROKO_BUDGET_USD" => Some("50.0".into()),
                "ROKO_PARALLEL" => Some("true".into()),
                "ROKO_EXPRESS" => Some("1".into()),
                "ROKO_SKIP_TESTS" => Some("yes".into()),
                "ROKO_CLIPPY" => Some("false".into()),
                _ => None,
            }
        };
        cfg.apply_env(&env);
        assert_eq!(cfg.agent.default_model, "claude-opus-4-6");
        assert_eq!(cfg.agent.default_backend, "codex");
        assert_eq!(cfg.agent.default_effort, "max");
        assert_eq!(cfg.agent.context_limit_k, 300);
        assert_eq!(cfg.conductor.max_agents, 16);
        assert!((cfg.budget.max_plan_usd - 50.0).abs() < f32::EPSILON);
        assert!(cfg.conductor.parallel_enabled);
        assert!(cfg.conductor.express_mode);
        assert!(cfg.gates.skip_tests);
        assert!(!cfg.gates.clippy_enabled);
    }

    #[test]
    fn env_override_bad_int_ignored() {
        let mut cfg = RokoConfig::default();
        let env = |key: &str| -> Option<String> {
            match key {
                "ROKO_CONTEXT_LIMIT_K" => Some("not_a_number".into()),
                _ => None,
            }
        };
        cfg.apply_env(&env);
        // Unchanged from default.
        assert_eq!(cfg.agent.context_limit_k, default_context_limit_k());
    }

    #[test]
    fn full_roundtrip_with_roles() {
        let toml = r#"
schema_version = 2

[project]
name = "test"

[agent]
default_model = "claude-opus-4-6"

[agent.roles.implementer]
model = "claude-sonnet-4-6"
effort = "high"

[agent.roles.conductor]
context_limit_k = 50

[gates]
clippy_enabled = false

[routing]
fast_task_model = "haiku"

[budget]
max_plan_usd = 50.0

[conductor]
max_agents = 4
express_mode = true

[learning]
auto_playbook_refresh = false

[tui]
refresh_rate_ms = 100

[server]
port = 3000
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        let text = cfg.to_toml().expect("serialize");
        let back = RokoConfig::from_toml(&text).expect("re-parse");
        assert_eq!(cfg, back);
    }

    #[test]
    fn example_toml_contains_all_sections() {
        let example = RokoConfig::example_toml();
        assert!(example.contains("[project]"));
        assert!(example.contains("[agent]"));
        assert!(example.contains("[gates]"));
        assert!(example.contains("[routing]"));
        assert!(example.contains("[budget]"));
        assert!(example.contains("[conductor]"));
        assert!(example.contains("[learning]"));
        assert!(example.contains("[tui]"));
        assert!(example.contains("[server]"));
    }

    #[test]
    fn example_toml_is_valid_toml() {
        let example = RokoConfig::example_toml();
        // Strip comment-only lines and parse.
        let cfg = RokoConfig::from_toml(&example).expect("example should parse as valid TOML");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn role_override_absent_fields_are_none() {
        let toml = r#"
[agent.roles.implementer]
model = "opus"
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        let imp = cfg.agent.roles.get("implementer").expect("role");
        assert_eq!(imp.model.as_deref(), Some("opus"));
        assert!(imp.effort.is_none());
        assert!(imp.backend.is_none());
        assert!(imp.context_limit_k.is_none());
        assert!(imp.turn_budget_usd.is_none());
    }

    #[test]
    fn parse_bool_env_variants() {
        assert!(parse_bool_env("true"));
        assert!(parse_bool_env("1"));
        assert!(parse_bool_env("yes"));
        assert!(parse_bool_env("on"));
        assert!(parse_bool_env("TRUE"));
        assert!(parse_bool_env("  Yes  "));
        assert!(!parse_bool_env("false"));
        assert!(!parse_bool_env("0"));
        assert!(!parse_bool_env("no"));
        assert!(!parse_bool_env("off"));
        assert!(!parse_bool_env(""));
    }

    #[test]
    fn validate_warns_on_zero_budget() {
        let mut cfg = RokoConfig::default();
        cfg.budget.max_plan_usd = 0.0;
        let warnings = cfg.validate();
        assert!(
            warnings.iter().any(|w| w.message.contains("max_plan_usd")),
            "expected warning about max_plan_usd, got: {warnings:?}"
        );
    }

    #[test]
    fn validate_default_config_has_no_warnings() {
        let cfg = RokoConfig::default();
        let warnings = cfg.validate();
        assert!(warnings.is_empty(), "default config should be valid: {warnings:?}");
    }

    #[test]
    fn validate_catches_all_bad_values() {
        let mut cfg = RokoConfig::default();
        cfg.budget.max_plan_usd = 0.0;
        cfg.budget.max_turn_usd = -1.0;
        cfg.conductor.max_agents = 0;
        cfg.conductor.max_parallel_plans = 0;
        cfg.gates.max_iterations = 0;
        cfg.agent.context_limit_k = 10;
        let warnings = cfg.validate();
        assert_eq!(warnings.len(), 6, "expected 6 warnings, got: {warnings:?}");
    }
}
