//! `roko.toml` schema — declarative config for the CLI's universal loop.
//!
//! The config picks an agent backend (any CLI that reads prompts on stdin),
//! sets a token budget for prompt composition, and lists the gates to run
//! on the agent's output.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

use roko_core::config::ServeConfig;
use roko_orchestrator::ExecutorConfig;

/// The top-level `roko.toml` document.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    /// Agent backend (the CLI that will be invoked via `ExecAgent`).
    pub agent: AgentConfig,
    /// Tool registry preferences.
    #[serde(default)]
    pub tools: ToolsConfig,
    /// Prompt assembly settings.
    #[serde(default)]
    pub prompt: PromptConfig,
    /// Gates to run on the agent output, in declaration order.
    #[serde(default, rename = "gate")]
    pub gates: Vec<GateConfig>,
    /// Executor runtime settings.
    #[serde(default)]
    pub executor: ExecutorConfig,
    /// Cost budget configuration.
    #[serde(default)]
    pub budget: BudgetConfig,
    /// API serving options.
    #[serde(default)]
    pub serve: ServeConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agent: AgentConfig::default(),
            tools: ToolsConfig::default(),
            prompt: PromptConfig::default(),
            gates: vec![GateConfig::default_shell_true()],
            executor: ExecutorConfig::default(),
            budget: BudgetConfig::default(),
            serve: ServeConfig::default(),
        }
    }
}

impl Config {
    /// Read a TOML config from `path`.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        Self::parse_toml(&text).with_context(|| format!("parse config {}", path.display()))
    }

    /// Parse a TOML config from a string.
    pub fn parse_toml(text: &str) -> Result<Self> {
        parse_toml_with_env(text, "invalid roko.toml")
    }

    /// Render this config back to a TOML string.
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("serialize roko.toml")
    }

    /// Render the default `roko.toml` template used by `roko init`.
    pub fn default_toml_template() -> Result<String> {
        let rendered = Self::default().to_toml()?;
        Ok(format!(
            "# REQUIRED_ENV\n\
             # Required environment variables (set in .env or shell):\n\
             # GITHUB_TOKEN       — GitHub personal access token (for MCP GitHub server)\n\
             # SLACK_BOT_TOKEN    — Slack bot token (for MCP Slack server)\n\
             # SLACK_SIGNING_SECRET — Slack webhook signing secret\n\
             # ANTHROPIC_API_KEY  — Claude API key (for direct API agents, not needed for CLI agents)\n\
             \n\
             {rendered}"
        ))
    }
}

/// Agent backend — the external CLI invoked via `ExecAgent`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentConfig {
    /// Program name, e.g. `"cat"`, `"ollama"`, `"claude"`.
    pub command: String,
    /// Extra args passed to the program.
    #[serde(default)]
    pub args: Vec<String>,
    /// Preferred model slug for Claude-style CLIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Reasoning effort passed to Claude-style CLIs.
    #[serde(default = "AgentConfig::default_effort")]
    pub effort: String,
    /// Whether to run Claude in `--bare` mode.
    #[serde(default = "AgentConfig::default_bare_mode")]
    pub bare_mode: bool,
    /// Optional fallback model slug for Claude-style CLIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_model: Option<String>,
    /// Timeout in milliseconds (default: `120_000`).
    #[serde(default = "AgentConfig::default_timeout")]
    pub timeout_ms: u64,
    /// Env vars passed to the subprocess. Useful for `OLLAMA_NOPROGRESS=1`,
    /// API keys, `OLLAMA_HOST`, etc.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// Whether to post-process the agent output — strip ANSI escapes and
    /// reasoning-model "thinking" traces. Default: `true` (so reasoning
    /// models like glm-4 / gemma-reasoning work out of the box).
    #[serde(default = "AgentConfig::default_clean")]
    pub clean_output: bool,
    /// Optional path to an MCP config file (`.mcp.json`). When set, this
    /// is passed to Claude via `--mcp-config`. If unset, `ClaudeCliAgent`
    /// auto-discovers by walking up from the working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config: Option<PathBuf>,
    /// Per-tier model mapping. Keys: `mechanical`, `focused`, `integrative`, `architectural`.
    #[serde(default)]
    pub tier_models: std::collections::HashMap<String, String>,
    /// Retry escalation configuration.
    #[serde(default)]
    pub escalation: EscalationConfig,
}

/// Tool registry preferences.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ToolsConfig {
    /// When true, MCP tools win over built-ins on name collisions.
    #[serde(default)]
    pub prefer_mcp: bool,
    /// Tool names that are blocked everywhere, regardless of role.
    #[serde(default)]
    pub global_denied: Vec<String>,
    /// MCP server startup timeout in seconds.
    #[serde(default = "ToolsConfig::default_mcp_timeout_secs")]
    pub mcp_timeout_secs: u64,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            prefer_mcp: false,
            global_denied: Vec::new(),
            mcp_timeout_secs: Self::default_mcp_timeout_secs(),
        }
    }
}

impl ToolsConfig {
    const fn default_mcp_timeout_secs() -> u64 {
        30
    }
}

impl AgentConfig {
    const fn default_timeout() -> u64 {
        120_000
    }

    fn default_effort() -> String {
        "medium".to_string()
    }

    const fn default_bare_mode() -> bool {
        true
    }

    const fn default_clean() -> bool {
        true
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            command: "cat".into(),
            args: Vec::new(),
            model: None,
            effort: Self::default_effort(),
            bare_mode: Self::default_bare_mode(),
            fallback_model: None,
            timeout_ms: Self::default_timeout(),
            env: Vec::new(),
            clean_output: Self::default_clean(),
            mcp_config: None,
            tier_models: std::collections::HashMap::new(),
            escalation: EscalationConfig::default(),
        }
    }
}

/// Retry escalation configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EscalationConfig {
    /// Maximum retries per task before failing.
    #[serde(default = "EscalationConfig::default_max_retries")]
    pub max_retries: u32,
    /// Whether to escalate to a higher-tier model on failure.
    #[serde(default = "EscalationConfig::default_escalate")]
    pub escalate_model: bool,
}

impl EscalationConfig {
    const fn default_max_retries() -> u32 {
        3
    }
    const fn default_escalate() -> bool {
        true
    }
}

impl Default for EscalationConfig {
    fn default() -> Self {
        Self {
            max_retries: Self::default_max_retries(),
            escalate_model: Self::default_escalate(),
        }
    }
}

/// Cost budget configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BudgetConfig {
    /// Maximum USD spend per plan.
    #[serde(default = "BudgetConfig::default_max_plan")]
    pub max_plan_usd: f64,
    /// Maximum USD spend per task.
    #[serde(default = "BudgetConfig::default_max_task")]
    pub max_task_usd: f64,
    /// Maximum USD spend per session (across all plans).
    #[serde(default = "BudgetConfig::default_max_session")]
    pub max_session_usd: f64,
    /// Warn at this percentage of budget consumed.
    #[serde(default = "BudgetConfig::default_warn_pct")]
    pub warn_at_percent: u32,
}

impl BudgetConfig {
    const fn default_max_plan() -> f64 {
        10.0
    }
    const fn default_max_task() -> f64 {
        1.0
    }
    const fn default_max_session() -> f64 {
        50.0
    }
    const fn default_warn_pct() -> u32 {
        80
    }

    /// Return the USD spend at which the plan should start warning.
    #[must_use]
    pub fn warn_threshold_usd(&self) -> f64 {
        if self.max_plan_usd <= 0.0 {
            0.0
        } else {
            self.max_plan_usd * f64::from(self.warn_at_percent) / 100.0
        }
    }
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_plan_usd: Self::default_max_plan(),
            max_task_usd: Self::default_max_task(),
            max_session_usd: Self::default_max_session(),
            warn_at_percent: Self::default_warn_pct(),
        }
    }
}

/// Prompt assembly settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PromptConfig {
    /// Token budget for the composer (approximate — 4 bytes per token).
    #[serde(default = "PromptConfig::default_budget")]
    pub token_budget: usize,
    /// System/role section injected as a Critical prompt section.
    #[serde(default = "PromptConfig::default_role")]
    pub role: String,
    /// Files whose contents should be injected into the prompt as sections.
    #[serde(default, rename = "files")]
    pub files: Vec<PromptFile>,
    /// Per-role prompt budgets (chars per section).
    #[serde(default)]
    pub budgets: std::collections::HashMap<String, RoleBudget>,
    /// Per-tier context budgets (tokens). Overrides the defaults in ContextProvider.
    #[serde(default)]
    pub context_budgets: ContextBudgetConfig,
}

/// Per-tier context token budget overrides for the ContextProvider.
///
/// These control how much context is assembled for each model tier.
/// If unset, defaults are: surgical=4000, focused=12000, full=24000.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextBudgetConfig {
    /// Token budget for surgical tier (Haiku / Ollama / mechanical tasks).
    #[serde(default = "ContextBudgetConfig::default_surgical")]
    pub surgical: usize,
    /// Token budget for focused tier (Sonnet / focused+integrative tasks).
    #[serde(default = "ContextBudgetConfig::default_focused")]
    pub focused: usize,
    /// Token budget for full tier (Opus / architectural tasks).
    #[serde(default = "ContextBudgetConfig::default_full")]
    pub full: usize,
}

impl ContextBudgetConfig {
    const fn default_surgical() -> usize {
        4_000
    }
    const fn default_focused() -> usize {
        12_000
    }
    const fn default_full() -> usize {
        24_000
    }

    /// Convert to the ContextBudgets type used by roko-compose.
    #[must_use]
    pub fn to_context_budgets(&self) -> roko_compose::ContextBudgets {
        roko_compose::ContextBudgets {
            surgical: self.surgical,
            focused: self.focused,
            full: self.full,
        }
    }
}

impl Default for ContextBudgetConfig {
    fn default() -> Self {
        Self {
            surgical: Self::default_surgical(),
            focused: Self::default_focused(),
            full: Self::default_full(),
        }
    }
}

/// Per-role prompt budget (character limits per section).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RoleBudget {
    /// Plan context budget (chars).
    #[serde(default)]
    pub plan: usize,
    /// PRD context budget (chars).
    #[serde(default)]
    pub prd: usize,
    /// Brief context budget (chars).
    #[serde(default)]
    pub brief: usize,
    /// File context budget (chars).
    #[serde(default)]
    pub file_context: usize,
    /// Skills/playbook budget (chars).
    #[serde(default)]
    pub skills: usize,
}

impl PromptConfig {
    const fn default_budget() -> usize {
        10_000
    }

    fn default_role() -> String {
        "implementer".to_string()
    }
}

impl Default for PromptConfig {
    fn default() -> Self {
        Self {
            token_budget: Self::default_budget(),
            role: Self::default_role(),
            files: Vec::new(),
            budgets: std::collections::HashMap::new(),
            context_budgets: ContextBudgetConfig::default(),
        }
    }
}

/// A file whose contents get injected into the prompt as a `PromptSection`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PromptFile {
    /// Path to the file (relative to the workdir).
    pub path: std::path::PathBuf,
    /// Display name for the section header. Defaults to the file name.
    #[serde(default)]
    pub name: Option<String>,
    /// Priority: `"low"`, `"normal"`, `"high"`, or `"critical"`. Default: `"normal"`.
    #[serde(default)]
    pub priority: Option<String>,
    /// Per-file hard cap in tokens — truncates oversized files before inclusion.
    #[serde(default)]
    pub hard_cap: Option<usize>,
}

/// One gate entry in `roko.toml`. Multiple gates run in declaration order.
///
/// The `kind` field selects the gate type. Each variant has its own fields
/// (currently only `Shell` is fully configurable; other kinds accept a
/// `build_system` tag).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GateConfig {
    /// Arbitrary shell command; passes on exit code 0.
    Shell {
        /// Program to invoke.
        program: String,
        /// Args to pass.
        #[serde(default)]
        args: Vec<String>,
        /// Timeout in milliseconds.
        #[serde(default = "default_gate_timeout")]
        timeout_ms: u64,
    },
    /// `cargo check` (or equivalent) run in the working dir.
    Compile {
        /// Build system (cargo, npm, go, python, forge, make).
        #[serde(default = "default_build_system")]
        build_system: String,
        /// Timeout in milliseconds.
        #[serde(default = "default_gate_timeout_long")]
        timeout_ms: u64,
    },
    /// `cargo clippy` (or equivalent lint command).
    Clippy {
        /// Build system.
        #[serde(default = "default_build_system")]
        build_system: String,
        /// Timeout in milliseconds.
        #[serde(default = "default_gate_timeout_long")]
        timeout_ms: u64,
    },
    /// `cargo test` (or equivalent test command).
    Test {
        /// Build system.
        #[serde(default = "default_build_system")]
        build_system: String,
        /// Timeout in milliseconds.
        #[serde(default = "default_gate_timeout_long")]
        timeout_ms: u64,
    },
}

impl GateConfig {
    /// A default `shell` gate that runs `true` (always passes). Useful as a
    /// placeholder in `roko init` output.
    #[must_use]
    pub fn default_shell_true() -> Self {
        Self::Shell {
            program: "true".into(),
            args: Vec::new(),
            timeout_ms: default_gate_timeout(),
        }
    }
}

const fn default_gate_timeout() -> u64 {
    60_000
}

const fn default_gate_timeout_long() -> u64 {
    600_000
}

fn default_build_system() -> String {
    "cargo".into()
}

// -----------------------------------------------------------------------
// Layered config: global (~/.config/roko/config.toml) + project (./roko.toml)
// -----------------------------------------------------------------------

/// Where each field in a [`ResolvedConfig`] came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Value came from the global config file.
    Global,
    /// Value came from the project-local `roko.toml`.
    Project,
    /// Value is the built-in default.
    Default,
    /// Value came from the file pointed at by `ROKO_CONFIG`.
    Env,
}

impl Source {
    /// Short tag printed by `roko config show`.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Global => "[global]",
            Self::Project => "[project]",
            Self::Default => "[default]",
            Self::Env => "[env]",
        }
    }
}

/// Partial config — every field optional. Used for the global/project layers
/// that get merged into a final [`Config`].
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ConfigLayer {
    /// Agent backend overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentLayer>,
    /// Tool registry preference overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsLayer>,
    /// Prompt settings overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<PromptLayer>,
    /// Gate list (replaces rather than merges if present).
    #[serde(default, rename = "gate", skip_serializing_if = "Option::is_none")]
    pub gates: Option<Vec<GateConfig>>,
    /// Executor settings overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<ExecutorLayer>,
    /// API serving options overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serve: Option<ServeLayer>,
}

impl ConfigLayer {
    /// Parse a layer from a TOML string.
    pub fn parse_toml(text: &str) -> Result<Self> {
        parse_toml_with_env(text, "invalid config toml")
    }

    /// Read a layer from a file on disk.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        Self::parse_toml(&text).with_context(|| format!("parse config {}", path.display()))
    }

    /// Merge two layers — `overlay` wins field-by-field.
    #[must_use]
    pub fn merge(mut self, overlay: Self) -> Self {
        if let Some(a) = overlay.agent {
            self.agent = Some(match self.agent {
                Some(base) => base.merge(a),
                None => a,
            });
        }
        if let Some(t) = overlay.tools {
            self.tools = Some(match self.tools {
                Some(base) => base.merge(t),
                None => t,
            });
        }
        if let Some(p) = overlay.prompt {
            self.prompt = Some(match self.prompt {
                Some(base) => base.merge(p),
                None => p,
            });
        }
        if let Some(g) = overlay.gates {
            self.gates = Some(g);
        }
        if let Some(e) = overlay.executor {
            self.executor = Some(match self.executor {
                Some(base) => base.merge(e),
                None => e,
            });
        }
        if let Some(s) = overlay.serve {
            self.serve = Some(match self.serve {
                Some(base) => base.merge(s),
                None => s,
            });
        }
        self
    }

    /// True if this layer has no fields set.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.agent.is_none()
            && self.tools.is_none()
            && self.prompt.is_none()
            && self.gates.is_none()
            && self.executor.is_none()
            && self.serve.is_none()
    }

    /// Resolve into a concrete [`Config`], filling missing fields with defaults.
    #[must_use]
    pub fn resolve(self) -> Config {
        let agent = match self.agent {
            Some(a) => {
                let defaults = AgentConfig::default();
                AgentConfig {
                    command: a.command.unwrap_or(defaults.command),
                    args: a.args.unwrap_or(defaults.args),
                    model: a.model.or(defaults.model),
                    effort: a.effort.unwrap_or(defaults.effort),
                    bare_mode: a.bare_mode.unwrap_or(defaults.bare_mode),
                    fallback_model: a.fallback_model.or(defaults.fallback_model),
                    timeout_ms: a.timeout_ms.unwrap_or(defaults.timeout_ms),
                    env: a.env.unwrap_or(defaults.env),
                    clean_output: a.clean_output.unwrap_or(defaults.clean_output),
                    mcp_config: a.mcp_config.or(defaults.mcp_config),
                    tier_models: defaults.tier_models,
                    escalation: defaults.escalation,
                }
            }
            None => AgentConfig::default(),
        };
        let tools = match self.tools {
            Some(t) => {
                let defaults = ToolsConfig::default();
                ToolsConfig {
                    prefer_mcp: t.prefer_mcp.unwrap_or(defaults.prefer_mcp),
                    global_denied: t.global_denied.unwrap_or(defaults.global_denied),
                    mcp_timeout_secs: t.mcp_timeout_secs.unwrap_or(defaults.mcp_timeout_secs),
                }
            }
            None => ToolsConfig::default(),
        };
        let prompt = match self.prompt {
            Some(p) => {
                let defaults = PromptConfig::default();
                PromptConfig {
                    token_budget: p.token_budget.unwrap_or(defaults.token_budget),
                    role: p.role.unwrap_or(defaults.role),
                    files: p.files.unwrap_or(defaults.files),
                    budgets: defaults.budgets,
                    context_budgets: defaults.context_budgets,
                }
            }
            None => PromptConfig::default(),
        };
        let gates = self.gates.unwrap_or_default();
        let executor = match self.executor {
            Some(e) => {
                let defaults = ExecutorConfig::default();
                ExecutorConfig {
                    max_concurrent_plans: e
                        .max_concurrent_plans
                        .unwrap_or(defaults.max_concurrent_plans),
                    max_concurrent_tasks: e
                        .max_concurrent_tasks
                        .unwrap_or(defaults.max_concurrent_tasks),
                    max_auto_fix_iterations: e
                        .max_auto_fix_iterations
                        .unwrap_or(defaults.max_auto_fix_iterations),
                    max_merge_attempts: e.max_merge_attempts.unwrap_or(defaults.max_merge_attempts),
                    task_timeout_secs: e.task_timeout_secs.unwrap_or(defaults.task_timeout_secs),
                    budget_usd: e.budget_usd.or(defaults.budget_usd),
                    auto_replan: e.auto_replan.unwrap_or(defaults.auto_replan),
                }
            }
            None => ExecutorConfig::default(),
        };
        let serve = match self.serve {
            Some(s) => {
                let defaults = ServeConfig::default();
                ServeConfig {
                    auth: match s.auth {
                        Some(auth) => auth.resolve(defaults.auth),
                        None => defaults.auth,
                    },
                }
            }
            None => ServeConfig::default(),
        };
        Config {
            agent,
            tools,
            prompt,
            gates,
            executor,
            budget: BudgetConfig::default(),
            serve,
        }
    }
}

fn parse_toml_with_env<T>(text: &str, context: &'static str) -> Result<T>
where
    T: DeserializeOwned,
{
    let mut value: toml::Value = toml::from_str(text).context(context)?;
    interpolate_env_values(&mut value)?;
    value.try_into().map_err(|err| anyhow!(err)).context(context)
}

fn interpolate_env_values(value: &mut toml::Value) -> Result<()> {
    match value {
        toml::Value::String(s) => {
            *s = interpolate_env_string(s)?;
        }
        toml::Value::Array(items) => {
            for item in items {
                interpolate_env_values(item)?;
            }
        }
        toml::Value::Table(entries) => {
            for (_, item) in entries.iter_mut() {
                interpolate_env_values(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn interpolate_env_string(input: &str) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            output.push_str(&rest[start..]);
            return Ok(output);
        };
        let var_name = &after[..end];
        if var_name.is_empty()
            || !var_name
                .chars()
                .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        {
            output.push_str("${");
            rest = &rest[start + 2..];
            continue;
        }
        let value = std::env::var(var_name).map_err(|_| {
            anyhow!(
                "Config error: ${{{var_name}}} referenced but {var_name} not set. Set it in .env or environment."
            )
        })?;
        output.push_str(&value);
        rest = &after[end + 1..];
    }

    output.push_str(rest);
    Ok(output)
}

/// Partial `AgentConfig` — every field optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentLayer {
    /// Program to invoke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Extra args.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Preferred model slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Claude effort level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Claude bare-mode toggle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bare_mode: Option<bool>,
    /// Claude fallback model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_model: Option<String>,
    /// Subprocess timeout in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Env vars for the agent subprocess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<(String, String)>>,
    /// Whether to strip ANSI + thinking traces from agent output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clean_output: Option<bool>,
    /// Optional explicit MCP config path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config: Option<PathBuf>,
}

impl AgentLayer {
    /// Merge another layer on top — `overlay` wins.
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            command: overlay.command.or(self.command),
            args: overlay.args.or(self.args),
            model: overlay.model.or(self.model),
            effort: overlay.effort.or(self.effort),
            bare_mode: overlay.bare_mode.or(self.bare_mode),
            fallback_model: overlay.fallback_model.or(self.fallback_model),
            timeout_ms: overlay.timeout_ms.or(self.timeout_ms),
            env: overlay.env.or(self.env),
            clean_output: overlay.clean_output.or(self.clean_output),
            mcp_config: overlay.mcp_config.or(self.mcp_config),
        }
    }
}

/// Partial `ToolsConfig` — every field optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ToolsLayer {
    /// When true, MCP tools win over built-ins on name collisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer_mcp: Option<bool>,
    /// Tool names blocked everywhere, regardless of role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_denied: Option<Vec<String>>,
    /// MCP server startup timeout in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_timeout_secs: Option<u64>,
}

impl ToolsLayer {
    /// Merge another layer on top — `overlay` wins.
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            prefer_mcp: overlay.prefer_mcp.or(self.prefer_mcp),
            global_denied: overlay.global_denied.or(self.global_denied),
            mcp_timeout_secs: overlay.mcp_timeout_secs.or(self.mcp_timeout_secs),
        }
    }
}

/// Partial `PromptConfig` — every field optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PromptLayer {
    /// Token budget for prompt composition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<usize>,
    /// System role / persona text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Files to inject as prompt sections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<PromptFile>>,
}

/// Partial `ExecutorConfig` — every field optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ExecutorLayer {
    /// Maximum number of plans executing concurrently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent_plans: Option<usize>,
    /// Maximum number of tasks executing concurrently within a plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent_tasks: Option<usize>,
    /// Maximum auto-fix iterations before declaring failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_auto_fix_iterations: Option<u32>,
    /// Maximum merge retry attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_merge_attempts: Option<u32>,
    /// Per-task timeout in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_timeout_secs: Option<u64>,
    /// Optional cost cap in USD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
    /// Whether to auto-replan after repeated gate failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_replan: Option<bool>,
}

/// Partial `ServeConfig` — every field optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ServeLayer {
    /// API auth settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<ServeAuthLayer>,
}

impl ServeLayer {
    /// Merge another layer on top — `overlay` wins.
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            auth: match (self.auth, overlay.auth) {
                (Some(base), Some(overlay)) => Some(base.merge(overlay)),
                (_, Some(overlay)) => Some(overlay),
                (base, None) => base,
            },
        }
    }
}

/// Partial API auth settings — every field optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ServeAuthLayer {
    /// Whether `/api/*` requires an `X-Api-Key` header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Shared API key expected in `X-Api-Key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl ServeAuthLayer {
    /// Merge another layer on top — `overlay` wins.
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            enabled: overlay.enabled.or(self.enabled),
            api_key: overlay.api_key.or(self.api_key),
        }
    }

    /// Resolve into a concrete [`ServeConfig::auth`] value.
    #[must_use]
    pub fn resolve(
        self,
        defaults: roko_core::config::ServeAuthConfig,
    ) -> roko_core::config::ServeAuthConfig {
        roko_core::config::ServeAuthConfig {
            enabled: self.enabled.unwrap_or(defaults.enabled),
            api_key: self.api_key.unwrap_or(defaults.api_key),
        }
    }
}

impl ExecutorLayer {
    /// Merge another layer on top — `overlay` wins.
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            max_concurrent_plans: overlay.max_concurrent_plans.or(self.max_concurrent_plans),
            max_concurrent_tasks: overlay.max_concurrent_tasks.or(self.max_concurrent_tasks),
            max_auto_fix_iterations: overlay
                .max_auto_fix_iterations
                .or(self.max_auto_fix_iterations),
            max_merge_attempts: overlay.max_merge_attempts.or(self.max_merge_attempts),
            task_timeout_secs: overlay.task_timeout_secs.or(self.task_timeout_secs),
            budget_usd: overlay.budget_usd.or(self.budget_usd),
            auto_replan: overlay.auto_replan.or(self.auto_replan),
        }
    }
}

impl PromptLayer {
    /// Merge another layer on top — `overlay` wins.
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            token_budget: overlay.token_budget.or(self.token_budget),
            role: overlay.role.or(self.role),
            files: overlay.files.or(self.files),
        }
    }
}

/// Absolute paths to the global and project config files (whether they
/// exist or not).
#[derive(Clone, Debug)]
pub struct ConfigPaths {
    /// Global config path (always set — even if file missing).
    pub global: PathBuf,
    /// Project config path, if discovered. None means no `roko.toml` in
    /// `workdir` or any ancestor.
    pub project: Option<PathBuf>,
    /// Value of `ROKO_CONFIG` env var if set — overrides the merge.
    pub env_override: Option<PathBuf>,
}

/// Resolve the path to the global config file.
///
/// Honors `$XDG_CONFIG_HOME` then falls back to `$HOME/.config`.
#[must_use]
pub fn global_config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("roko").join("config.toml");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".config")
        .join("roko")
        .join("config.toml")
}

/// Walk up from `start` looking for `roko.toml`. Returns the first hit.
#[must_use]
pub fn discover_project_config(start: &Path) -> Option<PathBuf> {
    let mut cur = start
        .canonicalize()
        .ok()
        .unwrap_or_else(|| start.to_path_buf());
    loop {
        let candidate = cur.join("roko.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Compute the paths used to resolve config for `workdir`.
#[must_use]
pub fn resolve_paths(workdir: &Path) -> ConfigPaths {
    ConfigPaths {
        global: global_config_path(),
        project: discover_project_config(workdir),
        env_override: std::env::var_os("ROKO_CONFIG").map(PathBuf::from),
    }
}

/// Fully-loaded config with field-level provenance.
#[derive(Clone, Debug)]
pub struct ResolvedConfig {
    /// Merged, default-filled config.
    pub config: Config,
    /// Which source supplied each field.
    pub sources: ConfigSources,
    /// Paths consulted during resolution.
    pub paths: ConfigPaths,
}

/// Per-field provenance for [`ResolvedConfig`].
#[derive(Clone, Debug)]
pub struct ConfigSources {
    /// Where `agent.command` came from.
    pub agent_command: Source,
    /// Where `agent.args` came from.
    pub agent_args: Source,
    /// Where `agent.model` came from.
    pub agent_model: Source,
    /// Where `agent.effort` came from.
    pub agent_effort: Source,
    /// Where `agent.bare_mode` came from.
    pub agent_bare_mode: Source,
    /// Where `agent.fallback_model` came from.
    pub agent_fallback_model: Source,
    /// Where `agent.timeout_ms` came from.
    pub agent_timeout_ms: Source,
    /// Where `tools.prefer_mcp` came from.
    pub tools_prefer_mcp: Source,
    /// Where `tools.global_denied` came from.
    pub tools_global_denied: Source,
    /// Where `tools.mcp_timeout_secs` came from.
    pub tools_mcp_timeout_secs: Source,
    /// Where `prompt.token_budget` came from.
    pub prompt_token_budget: Source,
    /// Where `prompt.role` came from.
    pub prompt_role: Source,
    /// Where `gates` came from.
    pub gates: Source,
}

/// Load global + project configs, merge them, and return a `ResolvedConfig`.
///
/// Precedence (highest first): `ROKO_CONFIG` env var → project → global → defaults.
pub fn load_layered(workdir: &Path) -> Result<ResolvedConfig> {
    let paths = resolve_paths(workdir);

    // If ROKO_CONFIG is set, it alone resolves the config.
    if let Some(env_path) = &paths.env_override {
        let layer = ConfigLayer::from_file(env_path)?;
        let sources = sources_from_layer(&layer, Source::Env, Source::Default);
        let config = layer.resolve();
        return Ok(ResolvedConfig {
            config,
            sources,
            paths,
        });
    }

    let global_layer = if paths.global.is_file() {
        ConfigLayer::from_file(&paths.global)?
    } else {
        ConfigLayer::default()
    };
    let project_layer = match &paths.project {
        Some(p) => ConfigLayer::from_file(p)?,
        None => ConfigLayer::default(),
    };

    let sources = compute_sources(&global_layer, &project_layer);
    let merged = global_layer.merge(project_layer);
    let config = merged.resolve();

    Ok(ResolvedConfig {
        config,
        sources,
        paths,
    })
}

/// Compute per-field provenance from global + project layers.
fn compute_sources(global: &ConfigLayer, project: &ConfigLayer) -> ConfigSources {
    let g_agent = global.agent.as_ref();
    let g_tools = global.tools.as_ref();
    let p_agent = project.agent.as_ref();
    let p_tools = project.tools.as_ref();
    let g_prompt = global.prompt.as_ref();
    let p_prompt = project.prompt.as_ref();

    let pick = |in_project: bool, in_global: bool| -> Source {
        if in_project {
            Source::Project
        } else if in_global {
            Source::Global
        } else {
            Source::Default
        }
    };

    ConfigSources {
        agent_command: pick(
            p_agent.and_then(|a| a.command.as_ref()).is_some(),
            g_agent.and_then(|a| a.command.as_ref()).is_some(),
        ),
        agent_args: pick(
            p_agent.and_then(|a| a.args.as_ref()).is_some(),
            g_agent.and_then(|a| a.args.as_ref()).is_some(),
        ),
        agent_model: pick(
            p_agent.and_then(|a| a.model.as_ref()).is_some(),
            g_agent.and_then(|a| a.model.as_ref()).is_some(),
        ),
        agent_effort: pick(
            p_agent.and_then(|a| a.effort.as_ref()).is_some(),
            g_agent.and_then(|a| a.effort.as_ref()).is_some(),
        ),
        agent_bare_mode: pick(
            p_agent.and_then(|a| a.bare_mode).is_some(),
            g_agent.and_then(|a| a.bare_mode).is_some(),
        ),
        agent_fallback_model: pick(
            p_agent.and_then(|a| a.fallback_model.as_ref()).is_some(),
            g_agent.and_then(|a| a.fallback_model.as_ref()).is_some(),
        ),
        agent_timeout_ms: pick(
            p_agent.and_then(|a| a.timeout_ms).is_some(),
            g_agent.and_then(|a| a.timeout_ms).is_some(),
        ),
        tools_prefer_mcp: pick(
            p_tools.and_then(|t| t.prefer_mcp).is_some(),
            g_tools.and_then(|t| t.prefer_mcp).is_some(),
        ),
        tools_global_denied: pick(
            p_tools.and_then(|t| t.global_denied.as_ref()).is_some(),
            g_tools.and_then(|t| t.global_denied.as_ref()).is_some(),
        ),
        tools_mcp_timeout_secs: pick(
            p_tools.and_then(|t| t.mcp_timeout_secs).is_some(),
            g_tools.and_then(|t| t.mcp_timeout_secs).is_some(),
        ),
        prompt_token_budget: pick(
            p_prompt.and_then(|p| p.token_budget).is_some(),
            g_prompt.and_then(|p| p.token_budget).is_some(),
        ),
        prompt_role: pick(
            p_prompt.and_then(|p| p.role.as_ref()).is_some(),
            g_prompt.and_then(|p| p.role.as_ref()).is_some(),
        ),
        gates: pick(project.gates.is_some(), global.gates.is_some()),
    }
}

/// Tag every field in a single-layer config as `present` or `fallback`.
fn sources_from_layer(layer: &ConfigLayer, present: Source, fallback: Source) -> ConfigSources {
    let agent = layer.agent.as_ref();
    let tools = layer.tools.as_ref();
    let prompt = layer.prompt.as_ref();
    let pick = |is_set: bool| -> Source { if is_set { present } else { fallback } };
    ConfigSources {
        agent_command: pick(agent.and_then(|a| a.command.as_ref()).is_some()),
        agent_args: pick(agent.and_then(|a| a.args.as_ref()).is_some()),
        agent_model: pick(agent.and_then(|a| a.model.as_ref()).is_some()),
        agent_effort: pick(agent.and_then(|a| a.effort.as_ref()).is_some()),
        agent_bare_mode: pick(agent.and_then(|a| a.bare_mode).is_some()),
        agent_fallback_model: pick(agent.and_then(|a| a.fallback_model.as_ref()).is_some()),
        agent_timeout_ms: pick(agent.and_then(|a| a.timeout_ms).is_some()),
        tools_prefer_mcp: pick(tools.and_then(|t| t.prefer_mcp).is_some()),
        tools_global_denied: pick(tools.and_then(|t| t.global_denied.as_ref()).is_some()),
        tools_mcp_timeout_secs: pick(tools.and_then(|t| t.mcp_timeout_secs).is_some()),
        prompt_token_budget: pick(prompt.and_then(|p| p.token_budget).is_some()),
        prompt_role: pick(prompt.and_then(|p| p.role.as_ref()).is_some()),
        gates: pick(layer.gates.is_some()),
    }
}

// -----------------------------------------------------------------------
// LLM CLI detection (for the `roko config init` wizard)
// -----------------------------------------------------------------------

/// A locally-installed LLM CLI that the wizard can offer as an agent backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedCli {
    /// The command name (`ollama`, `mods`, `llm`, `claude`, `aichat`).
    pub command: String,
    /// Default args the wizard will suggest (e.g. `["run", "<model>"]` for
    /// ollama — filled in from the model picker).
    pub default_args: Vec<String>,
    /// Human-readable description for the wizard prompt.
    pub description: String,
}

/// Candidates the wizard asks about, in order of preference.
#[must_use]
pub fn candidate_clis() -> Vec<DetectedCli> {
    vec![
        DetectedCli {
            command: "claude".into(),
            default_args: vec![],
            description: "Claude CLI (anthropic)".into(),
        },
        DetectedCli {
            command: "ollama".into(),
            default_args: vec!["run".into()],
            description: "Ollama (local models)".into(),
        },
        DetectedCli {
            command: "mods".into(),
            default_args: vec![],
            description: "charmbracelet/mods".into(),
        },
        DetectedCli {
            command: "llm".into(),
            default_args: vec![],
            description: "simonw/llm".into(),
        },
        DetectedCli {
            command: "aichat".into(),
            default_args: vec![],
            description: "aichat CLI".into(),
        },
        DetectedCli {
            command: "cat".into(),
            default_args: vec![],
            description: "cat (echo; smoke tests only)".into(),
        },
    ]
}

/// Return the subset of [`candidate_clis`] actually on the user's `PATH`.
#[must_use]
pub fn detect_clis() -> Vec<DetectedCli> {
    candidate_clis()
        .into_iter()
        .filter(|c| command_on_path(&c.command))
        .collect()
}

/// Cheap `which` — scan `$PATH` for an executable named `cmd`.
#[must_use]
pub fn command_on_path(cmd: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(cmd);
        if let Ok(meta) = std::fs::metadata(&candidate) {
            if meta.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if meta.permissions().mode() & 0o111 != 0 {
                        return true;
                    }
                }
                #[cfg(not(unix))]
                {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let toml = r#"
[agent]
command = "cat"
"#;
        let cfg = Config::parse_toml(toml).unwrap();
        assert_eq!(cfg.agent.command, "cat");
        assert_eq!(cfg.agent.timeout_ms, 120_000);
        assert!(!cfg.tools.prefer_mcp);
        assert!(cfg.tools.global_denied.is_empty());
        assert_eq!(cfg.tools.mcp_timeout_secs, 30);
        assert_eq!(cfg.prompt.token_budget, 10_000);
    }

    #[test]
    fn parses_full_config() {
        let toml = r#"
[agent]
command = "ollama"
args = ["run", "llama3"]
timeout_ms = 30000

[budget]
warn_at_percent = 90

[tools]
prefer_mcp = true
global_denied = ["write_file", "edit_file"]
mcp_timeout_secs = 45

[prompt]
token_budget = 20000
role = "You are a senior Rust engineer."

[[gate]]
kind = "shell"
program = "echo"
args = ["ok"]

[[gate]]
kind = "compile"
build_system = "cargo"
"#;
        let cfg = Config::parse_toml(toml).unwrap();
        assert_eq!(cfg.agent.command, "ollama");
        assert_eq!(
            cfg.agent.args,
            vec!["run".to_string(), "llama3".to_string()]
        );
        assert_eq!(cfg.agent.timeout_ms, 30_000);
        assert_eq!(cfg.budget.warn_at_percent, 90);
        assert!(cfg.tools.prefer_mcp);
        assert_eq!(
            cfg.tools.global_denied,
            vec!["write_file".to_string(), "edit_file".to_string()]
        );
        assert_eq!(cfg.tools.mcp_timeout_secs, 45);
        assert_eq!(cfg.prompt.token_budget, 20_000);
        assert_eq!(cfg.gates.len(), 2);
    }

    #[test]
    fn parses_executor_section_from_toml() {
        let toml = r#"
[agent]
command = "cat"

[tools]
prefer_mcp = false
global_denied = ["bash"]
mcp_timeout_secs = 15

[executor]
max_concurrent_plans = 8
max_concurrent_tasks = 12
max_auto_fix_iterations = 9
max_merge_attempts = 4
task_timeout_secs = 42
budget_usd = 1.5
auto_replan = false
"#;
        let cfg = Config::parse_toml(toml).unwrap();
        assert_eq!(cfg.executor.max_concurrent_plans, 8);
        assert_eq!(cfg.executor.max_concurrent_tasks, 12);
        assert_eq!(cfg.executor.max_auto_fix_iterations, 9);
        assert_eq!(cfg.executor.max_merge_attempts, 4);
        assert_eq!(cfg.executor.task_timeout_secs, 42);
        assert_eq!(cfg.executor.budget_usd, Some(1.5));
        assert!(!cfg.executor.auto_replan);
        assert!(!cfg.tools.prefer_mcp);
        assert_eq!(cfg.tools.global_denied, vec!["bash".to_string()]);
        assert_eq!(cfg.tools.mcp_timeout_secs, 15);
    }

    #[test]
    fn parses_serve_auth_section_from_toml() {
        let toml = r#"
[agent]
command = "cat"

[serve.auth]
enabled = true
api_key = "secret"
"#;
        let cfg = Config::parse_toml(toml).unwrap();
        assert!(cfg.serve.auth.enabled);
        assert_eq!(cfg.serve.auth.api_key, "secret");
    }

    #[test]
    fn interpolates_env_vars_in_string_values() {
        let path = std::env::var("PATH").expect("PATH must be set for tests");
        let toml = r#"
[agent]
command = "${PATH}"
args = ["--token=${PATH}"]

[prompt]
role = "prefix-${PATH}-suffix"
"#;
        let cfg = Config::parse_toml(toml).unwrap();
        assert_eq!(cfg.agent.command, path);
        assert_eq!(cfg.agent.args, vec![format!("--token={path}")]);
        assert_eq!(cfg.prompt.role, format!("prefix-{path}-suffix"));
    }

    #[test]
    fn missing_env_var_reports_clear_error() {
        let toml = r#"
[agent]
command = "${ROKO_TEST_MISSING_SECRET_9B1C}"
"#;
        let err = Config::parse_toml(toml).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(
            "Config error: ${ROKO_TEST_MISSING_SECRET_9B1C} referenced but ROKO_TEST_MISSING_SECRET_9B1C not set. Set it in .env or environment."
        ));
    }

    #[test]
    fn budget_warn_threshold_defaults_to_eighty_percent() {
        let budget = BudgetConfig::default();
        assert_eq!(budget.warn_at_percent, 80);
        assert!((budget.warn_threshold_usd() - 8.0).abs() < f64::EPSILON);
    }

    #[test]
    fn layer_resolve_uses_executor_overrides() {
        let layer = ConfigLayer::parse_toml(
            r#"
[executor]
max_concurrent_plans = 6
task_timeout_secs = 900
auto_replan = false
"#,
        )
        .unwrap();

        let cfg = layer.resolve();
        assert_eq!(cfg.executor.max_concurrent_plans, 6);
        assert_eq!(
            cfg.executor.max_concurrent_tasks,
            ExecutorConfig::default().max_concurrent_tasks
        );
        assert_eq!(cfg.executor.task_timeout_secs, 900);
        assert!(!cfg.executor.auto_replan);
        assert!(!cfg.serve.auth.enabled);
        assert!(cfg.serve.auth.api_key.is_empty());
    }

    #[test]
    fn default_config_roundtrips_through_toml() {
        let cfg = Config::default();
        let text = cfg.to_toml().unwrap();
        let parsed = Config::parse_toml(&text).unwrap();
        assert_eq!(parsed.agent.command, cfg.agent.command);
        assert_eq!(parsed.tools.prefer_mcp, cfg.tools.prefer_mcp);
        assert_eq!(parsed.tools.global_denied, cfg.tools.global_denied);
        assert_eq!(parsed.tools.mcp_timeout_secs, cfg.tools.mcp_timeout_secs);
        assert_eq!(parsed.gates.len(), cfg.gates.len());
        assert_eq!(parsed.serve.auth.enabled, cfg.serve.auth.enabled);
        assert_eq!(parsed.serve.auth.api_key, cfg.serve.auth.api_key);
    }

    #[test]
    fn layer_merge_project_overrides_global() {
        let global = ConfigLayer::parse_toml(
            r#"
[agent]
command = "ollama"
args = ["run", "llama3"]
timeout_ms = 60000

[tools]
prefer_mcp = true
global_denied = ["web_fetch"]
mcp_timeout_secs = 99

[prompt]
token_budget = 4000
role = "global role"
"#,
        )
        .unwrap();
        let project = ConfigLayer::parse_toml(
            r#"
[agent]
command = "mods"

[prompt]
token_budget = 8000
"#,
        )
        .unwrap();

        let merged = global.merge(project).resolve();
        assert_eq!(merged.agent.command, "mods");
        assert_eq!(
            merged.agent.args,
            vec!["run".to_string(), "llama3".to_string()]
        );
        assert_eq!(merged.agent.timeout_ms, 60_000);
        assert!(!merged.tools.prefer_mcp);
        assert_eq!(merged.tools.global_denied, vec!["web_fetch".to_string()]);
        assert_eq!(merged.tools.mcp_timeout_secs, 99);
        assert_eq!(merged.prompt.token_budget, 8000);
        assert_eq!(merged.prompt.role, "global role");
    }

    #[test]
    fn layer_resolve_empty_uses_defaults() {
        let layer = ConfigLayer::default();
        let cfg = layer.resolve();
        assert_eq!(cfg.agent.command, "cat");
        assert!(!cfg.tools.prefer_mcp);
        assert!(cfg.tools.global_denied.is_empty());
        assert_eq!(cfg.tools.mcp_timeout_secs, 30);
        assert_eq!(cfg.prompt.token_budget, 10_000);
        assert!(cfg.gates.is_empty());
        assert!(!cfg.serve.auth.enabled);
        assert!(cfg.serve.auth.api_key.is_empty());
    }

    #[test]
    fn sources_track_provenance() {
        let global = ConfigLayer::parse_toml(
            r#"
[agent]
command = "ollama"
timeout_ms = 60000
"#,
        )
        .unwrap();
        let project = ConfigLayer::parse_toml(
            r#"
[agent]
command = "mods"

[prompt]
token_budget = 8000
"#,
        )
        .unwrap();

        let sources = compute_sources(&global, &project);
        assert_eq!(sources.agent_command, Source::Project);
        assert_eq!(sources.agent_timeout_ms, Source::Global);
        assert_eq!(sources.tools_prefer_mcp, Source::Default);
        assert_eq!(sources.tools_global_denied, Source::Default);
        assert_eq!(sources.tools_mcp_timeout_secs, Source::Default);
        assert_eq!(sources.prompt_token_budget, Source::Project);
        assert_eq!(sources.prompt_role, Source::Default);
        assert_eq!(sources.agent_args, Source::Default);
    }

    #[test]
    fn gates_replace_rather_than_merge() {
        let global = ConfigLayer::parse_toml(
            r#"
[[gate]]
kind = "compile"
build_system = "cargo"
"#,
        )
        .unwrap();
        let project = ConfigLayer::parse_toml(
            r#"
[[gate]]
kind = "shell"
program = "echo"
"#,
        )
        .unwrap();
        let merged = global.merge(project).resolve();
        assert_eq!(merged.gates.len(), 1);
        assert!(matches!(&merged.gates[0], GateConfig::Shell { program, .. } if program == "echo"));
    }

    #[test]
    fn global_path_ends_in_roko_config_toml() {
        let path = global_config_path();
        assert!(path.ends_with("roko/config.toml"));
    }

    #[test]
    fn discover_project_config_walks_upward() {
        use std::fs;
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        fs::create_dir_all(&nested).unwrap();
        fs::write(tmp.path().join("roko.toml"), "").unwrap();
        let found = discover_project_config(&nested).unwrap();
        assert_eq!(
            found.canonicalize().unwrap(),
            tmp.path().join("roko.toml").canonicalize().unwrap()
        );
    }

    #[test]
    fn detect_clis_does_not_panic() {
        let _ = detect_clis();
    }

    #[test]
    fn default_toml_template_includes_required_env_section() {
        let rendered = Config::default_toml_template().unwrap();
        assert!(rendered.contains("# REQUIRED_ENV"));
        assert!(rendered.contains("GITHUB_TOKEN"));
        assert!(rendered.contains("SLACK_BOT_TOKEN"));
        assert!(rendered.contains("SLACK_SIGNING_SECRET"));
        assert!(rendered.contains("ANTHROPIC_API_KEY"));
    }
}
