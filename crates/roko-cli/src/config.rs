//! `roko.toml` schema — declarative config for the CLI's universal loop.
//!
//! The config picks an agent backend (any CLI that reads prompts on stdin),
//! sets a token budget for prompt composition, and lists the gates to run
//! on the agent's output.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use roko_core::agent::ProviderKind;
use roko_core::config::schema::{
    ModelProfile, ProviderConfig, ProviderRouting, RokoConfig, SubscriptionConfig,
};
use roko_core::config::{ServeConfig, ServeDeployConfig, ServeDeployWebhookConfig};
use roko_daimon::StrategySpaceDefinition;
use roko_orchestrator::ExecutorConfig;

/// The top-level `roko.toml` document.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    /// Agent backend (the CLI that will be invoked via `ExecAgent`).
    pub agent: AgentConfig,
    /// Automatically generate a plan when a PRD is promoted.
    #[serde(default)]
    pub auto_plan: bool,
    /// Automatic dream-cycle settings for daemon mode.
    #[serde(default)]
    pub dreams: DreamsConfig,
    /// Daimon affect-engine configuration.
    #[serde(default)]
    pub daimon: DaimonConfig,
    /// Tool registry preferences.
    #[serde(default)]
    pub tools: ToolsConfig,
    /// Prompt assembly settings.
    #[serde(default)]
    pub prompt: PromptConfig,
    /// Per-repository configuration blocks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<RepoConfig>,
    /// Gates to run on the agent output, in declaration order.
    #[serde(default, rename = "gate")]
    pub gates: Vec<GateConfig>,
    /// Executor runtime settings.
    #[serde(default)]
    pub executor: ExecutorConfig,
    /// Cost budget configuration.
    #[serde(default)]
    pub budget: BudgetConfig,
    /// Provider registry keyed by provider name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub providers: HashMap<String, ProviderConfig>,
    /// Model registry keyed by model name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub models: HashMap<String, ModelProfile>,
    /// API serving options.
    #[serde(default)]
    pub serve: ServeConfig,
    /// Structured log output format for cloud deployments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_format: Option<String>,
    /// HTTP bind address for cloud deployments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind: Option<String>,
    /// Persistent workspace directory for cloud deployments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agent: AgentConfig::default(),
            auto_plan: false,
            dreams: DreamsConfig::default(),
            daimon: DaimonConfig::default(),
            tools: ToolsConfig::default(),
            prompt: PromptConfig::default(),
            repos: Vec::new(),
            gates: vec![GateConfig::default_shell_true()],
            executor: ExecutorConfig::default(),
            budget: BudgetConfig::default(),
            providers: HashMap::new(),
            models: HashMap::new(),
            serve: ServeConfig::default(),
            log_format: None,
            bind: None,
            data_dir: None,
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
    pub fn default_toml_template(cloud: bool) -> Result<String> {
        let mut config = Self::default();
        // Use "claude" as the default agent command for init — not the struct
        // default ("cat") which is a safe no-op for tests.  Users running
        // `roko init` expect a working config out of the box.
        config.agent.command = "claude".into();
        if cloud {
            config.log_format = Some("json".to_string());
            config.bind = Some("0.0.0.0".to_string());
            config.data_dir = Some(PathBuf::from("/data/.roko"));
        }
        let rendered = config.to_toml()?;
        let cloud_deploy = if cloud {
            "\n# Auto-register webhooks after deploy\n\
             [[serve.deploy.webhooks]]\n\
             provider = \"github\"\n\
             owner = \"nunchi\"\n\
             repo = \"roko\"\n\
             \n\
             [[serve.deploy.webhooks]]\n\
             provider = \"github\"\n\
             owner = \"nunchi\"\n\
             repo = \"collaboration\"\n"
        } else {
            ""
        };
        Ok(format!(
            "# REQUIRED_ENV\n\
             # Required environment variables (set in .env or shell):\n\
             # GITHUB_TOKEN       — GitHub personal access token (for MCP GitHub server)\n\
             # GITHUB_WEBHOOK_SECRET — GitHub webhook secret for deploy registration\n\
             # SLACK_BOT_TOKEN    — Slack bot token (for MCP Slack server)\n\
             # SLACK_SIGNING_SECRET — Slack webhook signing secret\n\
             # ANTHROPIC_API_KEY  — Claude API key (for direct API agents, not needed for CLI agents)\n\
             \n\
             {rendered}{cloud_deploy}\n\
             # PRD settings (parsed by `RokoConfig`)\n\
             [prd]\n\
             auto_plan = false\n"
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

/// Automatic dream-cycle settings for daemon mode.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DreamsConfig {
    /// Enable the automatic dream cycle.
    #[serde(default = "DreamsConfig::default_auto_dream")]
    pub auto_dream: bool,
    /// Idle duration threshold, in minutes, before a dream can run.
    #[serde(default = "DreamsConfig::default_idle_threshold_mins")]
    pub idle_threshold_mins: u64,
    /// Minimum number of new episodes required before dreaming.
    #[serde(default = "DreamsConfig::default_min_episodes_for_dream")]
    pub min_episodes_for_dream: usize,
}

impl DreamsConfig {
    const fn default_auto_dream() -> bool {
        true
    }

    const fn default_idle_threshold_mins() -> u64 {
        15
    }

    const fn default_min_episodes_for_dream() -> usize {
        5
    }
}

impl Default for DreamsConfig {
    fn default() -> Self {
        Self {
            auto_dream: Self::default_auto_dream(),
            idle_threshold_mins: Self::default_idle_threshold_mins(),
            min_episodes_for_dream: Self::default_min_episodes_for_dream(),
        }
    }
}

/// Daimon affect-engine configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DaimonConfig {
    /// Domain-specific strategy-space registration for somatic markers.
    #[serde(default)]
    pub strategy_space: StrategySpaceDefinition,
}

impl Default for DaimonConfig {
    fn default() -> Self {
        Self {
            strategy_space: StrategySpaceDefinition::default(),
        }
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

/// Partial `DreamsConfig` — every field optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DreamsLayer {
    /// Enable the automatic dream cycle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_dream: Option<bool>,
    /// Idle threshold in minutes before dreaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_threshold_mins: Option<u64>,
    /// Minimum number of new episodes required before dreaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_episodes_for_dream: Option<usize>,
}

impl DreamsLayer {
    /// Merge another layer on top — `overlay` wins.
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            auto_dream: overlay.auto_dream.or(self.auto_dream),
            idle_threshold_mins: overlay.idle_threshold_mins.or(self.idle_threshold_mins),
            min_episodes_for_dream: overlay
                .min_episodes_for_dream
                .or(self.min_episodes_for_dream),
        }
    }

    /// Resolve into a concrete [`DreamsConfig`] value.
    #[must_use]
    pub fn resolve(self) -> DreamsConfig {
        let defaults = DreamsConfig::default();
        DreamsConfig {
            auto_dream: self.auto_dream.unwrap_or(defaults.auto_dream),
            idle_threshold_mins: self
                .idle_threshold_mins
                .unwrap_or(defaults.idle_threshold_mins),
            min_episodes_for_dream: self
                .min_episodes_for_dream
                .unwrap_or(defaults.min_episodes_for_dream),
        }
    }
}

/// Partial `DaimonConfig` — every field optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DaimonLayer {
    /// Domain-specific strategy-space registration for somatic markers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_space: Option<StrategySpaceLayer>,
}

impl DaimonLayer {
    /// Merge another layer on top — `overlay` wins.
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            strategy_space: match (self.strategy_space, overlay.strategy_space) {
                (Some(base), Some(overlay)) => Some(base.merge(overlay)),
                (None, Some(overlay)) => Some(overlay),
                (Some(base), None) => Some(base),
                (None, None) => None,
            },
        }
    }

    /// Resolve into a concrete [`DaimonConfig`] value.
    pub fn resolve(self) -> Result<DaimonConfig> {
        let defaults = DaimonConfig::default();
        Ok(DaimonConfig {
            strategy_space: match self.strategy_space {
                Some(strategy_space) => strategy_space.resolve()?,
                None => defaults.strategy_space,
            },
        })
    }
}

/// Partial strategy-space registration config.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StrategySpaceLayer {
    /// Domain identifier for this strategy-space mapping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Human-readable labels for the fixed 8 dimensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<Vec<String>>,
}

impl StrategySpaceLayer {
    /// Merge another layer on top — `overlay` wins.
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            domain: overlay.domain.or(self.domain),
            dimensions: overlay.dimensions.or(self.dimensions),
        }
    }

    /// Resolve into a validated [`StrategySpaceDefinition`].
    pub fn resolve(self) -> Result<StrategySpaceDefinition> {
        let defaults = StrategySpaceDefinition::default();
        let domain = self.domain.unwrap_or(defaults.domain);
        let dimensions_vec = self
            .dimensions
            .unwrap_or_else(|| defaults.dimensions.into_iter().collect());
        let dimensions: [String; 8] =
            dimensions_vec.try_into().map_err(|values: Vec<String>| {
                anyhow!(
                    "daimon.strategy_space.dimensions must contain exactly 8 entries, got {}",
                    values.len()
                )
            })?;
        StrategySpaceDefinition { domain, dimensions }.validate()
    }
}

/// Per-repository configuration inside `roko.toml`.
///
/// Repo-specific subscriptions are additive: they sit alongside the global
/// subscription set and can narrow behavior for one checkout.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RepoConfig {
    /// Human-readable repo name.
    pub name: String,
    /// Filesystem path to the repo root.
    pub path: PathBuf,
    /// Branch name tracked for this repo.
    pub branch: String,
    /// Template names active for this repo.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<String>,
    /// Repo-specific subscriptions to load in addition to the global set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscriptions: Vec<SubscriptionConfig>,
}

/// Loaded runtime data for a configured repository.
#[derive(Clone, Debug)]
pub struct RepoEntry {
    /// Declarative repo config from `roko.toml`.
    pub config: RepoConfig,
    /// Canonical repository root.
    pub root: PathBuf,
    /// Optional repo-local `.roko/roko.toml` config.
    pub roko_config: Option<RokoConfig>,
    /// Path to the repo-local config when it exists.
    pub roko_config_path: Option<PathBuf>,
}

/// Runtime registry of configured repositories.
#[derive(Clone, Debug, Default)]
pub struct RepoRegistry {
    repos: Vec<RepoEntry>,
}

impl RepoRegistry {
    /// Load and validate all configured repos.
    pub fn load(config: &Config, workdir: &Path) -> Result<Self> {
        let mut repos = Vec::with_capacity(config.repos.len());
        let mut seen_names = std::collections::HashSet::new();

        for repo in &config.repos {
            if repo.name.trim().is_empty() {
                return Err(anyhow!("configured repo name must not be empty"));
            }
            if !seen_names.insert(repo.name.clone()) {
                return Err(anyhow!("duplicate configured repo name: {}", repo.name));
            }

            let root = Self::resolve_root(repo, workdir)?;
            let (roko_config, roko_config_path) = Self::load_repo_config(&root, &repo.name)?;

            repos.push(RepoEntry {
                config: repo.clone(),
                root,
                roko_config,
                roko_config_path,
            });
        }

        Ok(Self { repos })
    }

    /// Return all loaded repo entries.
    #[must_use]
    pub fn repos(&self) -> &[RepoEntry] {
        &self.repos
    }

    /// True when no repos are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.repos.is_empty()
    }

    /// Find a repo by configured name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&RepoEntry> {
        self.repos.iter().find(|repo| repo.config.name == name)
    }

    /// Find the repo whose name matches a repository full-name from a webhook
    /// signal payload (e.g. `"owner/repo"`). Falls back to matching the bare
    /// repo name portion.
    #[must_use]
    pub fn find_by_full_name(&self, full_name: &str) -> Option<&RepoEntry> {
        // Exact name match first.
        if let Some(entry) = self.get(full_name) {
            return Some(entry);
        }
        // Match bare name (e.g. "my-repo" in "owner/my-repo").
        let bare = full_name.rsplit('/').next().unwrap_or(full_name);
        self.repos.iter().find(|entry| entry.config.name == bare)
    }

    fn resolve_root(repo: &RepoConfig, workdir: &Path) -> Result<PathBuf> {
        let configured = if repo.path.is_absolute() {
            repo.path.clone()
        } else {
            workdir.join(&repo.path)
        };
        let root = configured.canonicalize().with_context(|| {
            format!("resolve repo '{}' path {}", repo.name, configured.display())
        })?;
        if !root.is_dir() {
            return Err(anyhow!(
                "configured repo '{}' path is not a directory: {}",
                repo.name,
                root.display()
            ));
        }
        Ok(root)
    }

    fn load_repo_config(
        root: &Path,
        repo_name: &str,
    ) -> Result<(Option<RokoConfig>, Option<PathBuf>)> {
        let path = root.join(".roko").join("roko.toml");
        if !path.is_file() {
            return Ok((None, None));
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read repo config {}", path.display()))?;
        let config = RokoConfig::from_toml(&text)
            .map_err(|err| anyhow!(err))
            .with_context(|| {
                format!(
                    "parse repo config {} for repo {}",
                    path.display(),
                    repo_name
                )
            })?;
        Ok((Some(config), Some(path)))
    }
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
    /// Value came from `ROKO_CONFIG` or a `ROKO__*` override.
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
    /// Automatically generate a plan when a PRD is promoted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_plan: Option<bool>,
    /// Automatic dream-cycle overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dreams: Option<DreamsLayer>,
    /// Daimon configuration overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daimon: Option<DaimonLayer>,
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
    /// Provider registry overrides keyed by provider name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<HashMap<String, ProviderLayer>>,
    /// Model registry overrides keyed by model name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<HashMap<String, ModelProfileLayer>>,
    /// API serving options overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serve: Option<ServeLayer>,
    /// Per-repository configuration blocks.
    #[serde(default, rename = "repos", skip_serializing_if = "Option::is_none")]
    pub repos: Option<Vec<RepoConfig>>,
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
        if let Some(auto_plan) = overlay.auto_plan {
            self.auto_plan = Some(auto_plan);
        }
        if let Some(dreams) = overlay.dreams {
            self.dreams = Some(match self.dreams {
                Some(base) => base.merge(dreams),
                None => dreams,
            });
        }
        if let Some(daimon) = overlay.daimon {
            self.daimon = Some(match self.daimon {
                Some(base) => base.merge(daimon),
                None => daimon,
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
        if let Some(overlay_providers) = overlay.providers {
            let mut providers = self.providers.unwrap_or_default();
            for (name, layer) in overlay_providers {
                providers
                    .entry(name)
                    .and_modify(|base| *base = base.clone().merge(layer.clone()))
                    .or_insert(layer);
            }
            self.providers = Some(providers);
        }
        if let Some(overlay_models) = overlay.models {
            let mut models = self.models.unwrap_or_default();
            for (name, layer) in overlay_models {
                models
                    .entry(name)
                    .and_modify(|base| *base = base.clone().merge(layer.clone()))
                    .or_insert(layer);
            }
            self.models = Some(models);
        }
        if let Some(s) = overlay.serve {
            self.serve = Some(match self.serve {
                Some(base) => base.merge(s),
                None => s,
            });
        }
        if let Some(repos) = overlay.repos {
            self.repos = Some(repos);
        }
        self
    }

    /// True if this layer has no fields set.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.agent.is_none()
            && self.auto_plan.is_none()
            && self.dreams.is_none()
            && self.daimon.is_none()
            && self.tools.is_none()
            && self.prompt.is_none()
            && self.gates.is_none()
            && self.executor.is_none()
            && self.providers.is_none()
            && self.models.is_none()
            && self.serve.is_none()
            && self.repos.is_none()
    }

    /// Resolve into a concrete [`Config`], filling missing fields with defaults.
    pub fn resolve(self) -> Result<Config> {
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
        let auto_plan = self.auto_plan.unwrap_or(false);
        let dreams = match self.dreams {
            Some(dreams) => dreams.resolve(),
            None => DreamsConfig::default(),
        };
        let daimon = match self.daimon {
            Some(daimon) => daimon.resolve()?,
            None => DaimonConfig::default(),
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
                    use_worktrees: e.use_worktrees.unwrap_or(defaults.use_worktrees),
                    speculative_threshold_multiplier: e
                        .speculative_threshold_multiplier
                        .unwrap_or(defaults.speculative_threshold_multiplier),
                    resource_budget: defaults.resource_budget,
                }
            }
            None => ExecutorConfig::default(),
        };
        let providers = match self.providers {
            Some(providers) => providers
                .into_iter()
                .map(|(name, layer)| {
                    let provider = layer
                        .resolve()
                        .with_context(|| format!("resolve providers.{name}"))?;
                    Ok((name, provider))
                })
                .collect::<Result<HashMap<_, _>>>()?,
            None => HashMap::new(),
        };
        let models = match self.models {
            Some(models) => models
                .into_iter()
                .map(|(name, layer)| {
                    let profile = layer
                        .resolve()
                        .with_context(|| format!("resolve models.{name}"))?;
                    Ok((name, profile))
                })
                .collect::<Result<HashMap<_, _>>>()?,
            None => HashMap::new(),
        };
        let serve = match self.serve {
            Some(s) => {
                let defaults = ServeConfig::default();
                ServeConfig {
                    auto_orchestrate: match s.auto_orchestrate {
                        Some(auto_orchestrate) => auto_orchestrate,
                        None => defaults.auto_orchestrate,
                    },
                    auth: match s.auth {
                        Some(auth) => auth.resolve(defaults.auth),
                        None => defaults.auth,
                    },
                    deploy: match s.deploy {
                        Some(deploy) => deploy.resolve(defaults.deploy),
                        None => defaults.deploy,
                    },
                }
            }
            None => ServeConfig::default(),
        };
        Ok(Config {
            agent,
            auto_plan,
            dreams,
            daimon,
            tools,
            prompt,
            repos: self.repos.unwrap_or_default(),
            gates,
            executor,
            budget: BudgetConfig::default(),
            providers,
            models,
            serve,
            log_format: None,
            bind: None,
            data_dir: None,
        })
    }
}

/// Partial provider config used for layered merges.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ProviderLayer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ProviderKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
}

impl ProviderLayer {
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            kind: overlay.kind.or(self.kind),
            base_url: overlay.base_url.or(self.base_url),
            api_key_env: overlay.api_key_env.or(self.api_key_env),
            command: overlay.command.or(self.command),
            args: overlay.args.or(self.args),
            timeout_ms: overlay.timeout_ms.or(self.timeout_ms),
            ttft_timeout_ms: overlay.ttft_timeout_ms.or(self.ttft_timeout_ms),
            connect_timeout_ms: overlay.connect_timeout_ms.or(self.connect_timeout_ms),
            extra_headers: overlay.extra_headers.or(self.extra_headers),
            max_concurrent: overlay.max_concurrent.or(self.max_concurrent),
        }
    }

    pub fn resolve(self) -> Result<ProviderConfig> {
        Ok(ProviderConfig {
            kind: self.kind.context("missing required field `kind`")?,
            base_url: self.base_url,
            api_key_env: self.api_key_env,
            command: self.command,
            args: self.args,
            timeout_ms: self.timeout_ms.or(Some(120_000)),
            ttft_timeout_ms: self.ttft_timeout_ms.or(Some(15_000)),
            connect_timeout_ms: self.connect_timeout_ms.or(Some(5_000)),
            extra_headers: self.extra_headers,
            max_concurrent: self.max_concurrent,
        })
    }
}

/// Partial OpenRouter routing overrides used for layered merges.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ProviderRoutingLayer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fallbacks: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_parameters: Option<Vec<String>>,
}

impl ProviderRoutingLayer {
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            sort: overlay.sort.or(self.sort),
            order: overlay.order.or(self.order),
            allow_fallbacks: overlay.allow_fallbacks.or(self.allow_fallbacks),
            max_price: overlay.max_price.or(self.max_price),
            require_parameters: overlay.require_parameters.or(self.require_parameters),
        }
    }

    #[must_use]
    pub fn resolve(self) -> ProviderRouting {
        ProviderRouting {
            sort: self.sort,
            order: self.order,
            allow_fallbacks: self.allow_fallbacks,
            max_price: self.max_price,
            require_parameters: self.require_parameters,
        }
    }
}

/// Partial model profile used for layered merges.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ModelProfileLayer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_web_search: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_mcp_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_partial: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_grounding: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_code_execution: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_caching: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_routing: Option<ProviderRoutingLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_input_per_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_output_per_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_input_per_m_high: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_output_per_m_high: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cache_read_per_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cache_write_per_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tools: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_ratio: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_search: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_citations: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_async: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_embedding_model: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_per_request: Option<f64>,
}

impl ModelProfileLayer {
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            provider: overlay.provider.or(self.provider),
            slug: overlay.slug.or(self.slug),
            context_window: overlay.context_window.or(self.context_window),
            max_output: overlay.max_output.or(self.max_output),
            supports_tools: overlay.supports_tools.or(self.supports_tools),
            supports_thinking: overlay.supports_thinking.or(self.supports_thinking),
            supports_vision: overlay.supports_vision.or(self.supports_vision),
            supports_web_search: overlay.supports_web_search.or(self.supports_web_search),
            supports_mcp_tools: overlay.supports_mcp_tools.or(self.supports_mcp_tools),
            supports_partial: overlay.supports_partial.or(self.supports_partial),
            supports_grounding: overlay.supports_grounding.or(self.supports_grounding),
            supports_code_execution: overlay
                .supports_code_execution
                .or(self.supports_code_execution),
            supports_caching: overlay.supports_caching.or(self.supports_caching),
            provider_routing: match (self.provider_routing, overlay.provider_routing) {
                (Some(base), Some(overlay)) => Some(base.merge(overlay)),
                (None, Some(overlay)) => Some(overlay),
                (Some(base), None) => Some(base),
                (None, None) => None,
            },
            tool_format: overlay.tool_format.or(self.tool_format),
            cost_input_per_m: overlay.cost_input_per_m.or(self.cost_input_per_m),
            cost_output_per_m: overlay.cost_output_per_m.or(self.cost_output_per_m),
            cost_input_per_m_high: overlay.cost_input_per_m_high.or(self.cost_input_per_m_high),
            cost_output_per_m_high: overlay
                .cost_output_per_m_high
                .or(self.cost_output_per_m_high),
            cost_cache_read_per_m: overlay.cost_cache_read_per_m.or(self.cost_cache_read_per_m),
            cost_cache_write_per_m: overlay
                .cost_cache_write_per_m
                .or(self.cost_cache_write_per_m),
            thinking_level: overlay.thinking_level.or(self.thinking_level),
            max_tools: overlay.max_tools.or(self.max_tools),
            tokenizer_ratio: overlay.tokenizer_ratio.or(self.tokenizer_ratio),
            supports_search: overlay.supports_search.or(self.supports_search),
            supports_citations: overlay.supports_citations.or(self.supports_citations),
            supports_async: overlay.supports_async.or(self.supports_async),
            is_embedding_model: overlay.is_embedding_model.or(self.is_embedding_model),
            search_context_size: overlay.search_context_size.or(self.search_context_size),
            cost_per_request: overlay.cost_per_request.or(self.cost_per_request),
        }
    }

    pub fn resolve(self) -> Result<ModelProfile> {
        Ok(ModelProfile {
            provider: self.provider.context("missing required field `provider`")?,
            slug: self.slug.context("missing required field `slug`")?,
            context_window: self.context_window.unwrap_or(128_000),
            max_output: self.max_output,
            supports_tools: self.supports_tools.unwrap_or(true),
            supports_thinking: self.supports_thinking.unwrap_or(false),
            supports_vision: self.supports_vision.unwrap_or(false),
            supports_web_search: self.supports_web_search.unwrap_or(false),
            supports_mcp_tools: self.supports_mcp_tools.unwrap_or(false),
            supports_partial: self.supports_partial.unwrap_or(false),
            supports_grounding: self.supports_grounding.unwrap_or(false),
            supports_code_execution: self.supports_code_execution.unwrap_or(false),
            supports_caching: self.supports_caching.unwrap_or(false),
            provider_routing: self.provider_routing.map(ProviderRoutingLayer::resolve),
            tool_format: self
                .tool_format
                .unwrap_or_else(|| "openai_json".to_string()),
            cost_input_per_m: self.cost_input_per_m,
            cost_output_per_m: self.cost_output_per_m,
            cost_input_per_m_high: self.cost_input_per_m_high,
            cost_output_per_m_high: self.cost_output_per_m_high,
            cost_cache_read_per_m: self.cost_cache_read_per_m,
            cost_cache_write_per_m: self.cost_cache_write_per_m,
            thinking_level: self.thinking_level,
            max_tools: self.max_tools,
            tokenizer_ratio: self.tokenizer_ratio,
            supports_search: self.supports_search.unwrap_or(false),
            supports_citations: self.supports_citations.unwrap_or(false),
            supports_async: self.supports_async.unwrap_or(false),
            is_embedding_model: self.is_embedding_model.unwrap_or(false),
            search_context_size: self.search_context_size,
            cost_per_request: self.cost_per_request,
        })
    }
}

fn parse_toml_with_env<T>(text: &str, context: &'static str) -> Result<T>
where
    T: DeserializeOwned,
{
    let mut value: toml::Value = toml::from_str(text).context(context)?;
    interpolate_env_values(&mut value)?;
    value
        .try_into()
        .map_err(|err| anyhow!(err))
        .context(context)
}

pub(crate) fn apply_layer_value(layer: &mut ConfigLayer, key: &str, value: &str) -> Result<()> {
    match key.split('.').collect::<Vec<_>>().as_slice() {
        ["auto_plan"] => {
            layer.auto_plan = Some(value.parse::<bool>().context("parse auto_plan as bool")?);
        }
        ["agent", "command"] => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.command = Some(value.into());
        }
        ["agent", "args"] => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.args = Some(parse_string_list(value, "parse JSON array for agent.args")?);
        }
        ["agent", "model"] => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.model = Some(value.into());
        }
        ["agent", "effort"] => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.effort = Some(value.into());
        }
        ["agent", "bare_mode"] => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.bare_mode = Some(value.parse::<bool>().context("parse bare_mode as bool")?);
        }
        ["agent", "fallback_model"] => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.fallback_model = Some(value.into());
        }
        ["agent", "timeout_ms"] => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.timeout_ms = Some(value.parse().context("parse timeout_ms as u64")?);
        }
        ["agent", "env"] => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.env =
                Some(serde_json::from_str(value).context("parse JSON array for agent.env")?);
        }
        ["agent", "clean_output"] => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.clean_output = Some(
                value
                    .parse::<bool>()
                    .context("parse clean_output as bool")?,
            );
        }
        ["agent", "mcp_config"] => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.mcp_config = Some(PathBuf::from(value));
        }
        ["dreams", "auto_dream"] => {
            let dreams = layer.dreams.get_or_insert_with(DreamsLayer::default);
            dreams.auto_dream = Some(value.parse::<bool>().context("parse auto_dream as bool")?);
        }
        ["dreams", "idle_threshold_mins"] => {
            let dreams = layer.dreams.get_or_insert_with(DreamsLayer::default);
            dreams.idle_threshold_mins = Some(
                value
                    .parse::<u64>()
                    .context("parse idle_threshold_mins as u64")?,
            );
        }
        ["dreams", "min_episodes_for_dream"] => {
            let dreams = layer.dreams.get_or_insert_with(DreamsLayer::default);
            dreams.min_episodes_for_dream = Some(
                value
                    .parse::<usize>()
                    .context("parse min_episodes_for_dream as usize")?,
            );
        }
        ["daimon", "strategy_space", "domain"] => {
            let daimon = layer.daimon.get_or_insert_with(DaimonLayer::default);
            let strategy_space = daimon
                .strategy_space
                .get_or_insert_with(StrategySpaceLayer::default);
            strategy_space.domain = Some(value.into());
        }
        ["daimon", "strategy_space", "dimensions"] => {
            let daimon = layer.daimon.get_or_insert_with(DaimonLayer::default);
            let strategy_space = daimon
                .strategy_space
                .get_or_insert_with(StrategySpaceLayer::default);
            strategy_space.dimensions = Some(parse_string_list(
                value,
                "parse JSON array for daimon.strategy_space.dimensions",
            )?);
        }
        ["tools", "prefer_mcp"] => {
            let tools = layer.tools.get_or_insert_with(ToolsLayer::default);
            tools.prefer_mcp = Some(value.parse::<bool>().context("parse prefer_mcp as bool")?);
        }
        ["tools", "global_denied"] => {
            let tools = layer.tools.get_or_insert_with(ToolsLayer::default);
            tools.global_denied = Some(parse_string_list(
                value,
                "parse JSON array for tools.global_denied",
            )?);
        }
        ["tools", "mcp_timeout_secs"] => {
            let tools = layer.tools.get_or_insert_with(ToolsLayer::default);
            tools.mcp_timeout_secs = Some(
                value
                    .parse::<u64>()
                    .context("parse mcp_timeout_secs as u64")?,
            );
        }
        ["prompt", "token_budget"] => {
            let prompt = layer.prompt.get_or_insert_with(PromptLayer::default);
            prompt.token_budget = Some(
                value
                    .parse::<usize>()
                    .context("parse token_budget as usize")?,
            );
        }
        ["prompt", "role"] => {
            let prompt = layer.prompt.get_or_insert_with(PromptLayer::default);
            prompt.role = Some(value.into());
        }
        ["prompt", "files"] => {
            let prompt = layer.prompt.get_or_insert_with(PromptLayer::default);
            prompt.files =
                Some(serde_json::from_str(value).context("parse JSON array for prompt.files")?);
        }
        ["executor", "max_concurrent_plans"] => {
            let executor = layer.executor.get_or_insert_with(ExecutorLayer::default);
            executor.max_concurrent_plans = Some(
                value
                    .parse::<usize>()
                    .context("parse max_concurrent_plans as usize")?,
            );
        }
        ["executor", "max_concurrent_tasks"] => {
            let executor = layer.executor.get_or_insert_with(ExecutorLayer::default);
            executor.max_concurrent_tasks = Some(
                value
                    .parse::<usize>()
                    .context("parse max_concurrent_tasks as usize")?,
            );
        }
        ["executor", "max_auto_fix_iterations"] => {
            let executor = layer.executor.get_or_insert_with(ExecutorLayer::default);
            executor.max_auto_fix_iterations = Some(
                value
                    .parse::<u32>()
                    .context("parse max_auto_fix_iterations as u32")?,
            );
        }
        ["executor", "max_merge_attempts"] => {
            let executor = layer.executor.get_or_insert_with(ExecutorLayer::default);
            executor.max_merge_attempts = Some(
                value
                    .parse::<u32>()
                    .context("parse max_merge_attempts as u32")?,
            );
        }
        ["executor", "task_timeout_secs"] => {
            let executor = layer.executor.get_or_insert_with(ExecutorLayer::default);
            executor.task_timeout_secs = Some(
                value
                    .parse::<u64>()
                    .context("parse task_timeout_secs as u64")?,
            );
        }
        ["executor", "budget_usd"] => {
            let executor = layer.executor.get_or_insert_with(ExecutorLayer::default);
            executor.budget_usd = Some(value.parse::<f64>().context("parse budget_usd as f64")?);
        }
        ["executor", "auto_replan"] => {
            let executor = layer.executor.get_or_insert_with(ExecutorLayer::default);
            executor.auto_replan =
                Some(value.parse::<bool>().context("parse auto_replan as bool")?);
        }
        ["executor", "use_worktrees"] => {
            let executor = layer.executor.get_or_insert_with(ExecutorLayer::default);
            executor.use_worktrees = Some(
                value
                    .parse::<bool>()
                    .context("parse use_worktrees as bool")?,
            );
        }
        ["providers", name, "kind"] => {
            let provider = provider_layer_mut(layer, name);
            provider.kind = Some(parse_string_enum(value, "parse provider kind")?);
        }
        ["providers", name, "base_url"] => {
            let provider = provider_layer_mut(layer, name);
            provider.base_url = Some(value.into());
        }
        ["providers", name, "api_key_env"] => {
            let provider = provider_layer_mut(layer, name);
            provider.api_key_env = Some(value.into());
        }
        ["providers", name, "command"] => {
            let provider = provider_layer_mut(layer, name);
            provider.command = Some(value.into());
        }
        ["providers", name, "args"] => {
            let provider = provider_layer_mut(layer, name);
            provider.args = Some(parse_string_list(
                value,
                "parse JSON array for provider args",
            )?);
        }
        ["providers", name, "timeout_ms"] => {
            let provider = provider_layer_mut(layer, name);
            provider.timeout_ms = Some(value.parse::<u64>().context("parse timeout_ms as u64")?);
        }
        ["providers", name, "ttft_timeout_ms"] => {
            let provider = provider_layer_mut(layer, name);
            provider.ttft_timeout_ms = Some(
                value
                    .parse::<u64>()
                    .context("parse ttft_timeout_ms as u64")?,
            );
        }
        ["providers", name, "connect_timeout_ms"] => {
            let provider = provider_layer_mut(layer, name);
            provider.connect_timeout_ms = Some(
                value
                    .parse::<u64>()
                    .context("parse connect_timeout_ms as u64")?,
            );
        }
        ["providers", name, "extra_headers"] => {
            let provider = provider_layer_mut(layer, name);
            provider.extra_headers =
                Some(serde_json::from_str(value).context("parse JSON object for extra_headers")?);
        }
        ["providers", name, "max_concurrent"] => {
            let provider = provider_layer_mut(layer, name);
            provider.max_concurrent = Some(
                value
                    .parse::<u32>()
                    .context("parse max_concurrent as u32")?,
            );
        }
        ["models", name, "provider"] => {
            let model = model_layer_mut(layer, name);
            model.provider = Some(value.into());
        }
        ["models", name, "slug"] => {
            let model = model_layer_mut(layer, name);
            model.slug = Some(value.into());
        }
        ["models", name, "context_window"] => {
            let model = model_layer_mut(layer, name);
            model.context_window = Some(
                value
                    .parse::<u64>()
                    .context("parse context_window as u64")?,
            );
        }
        ["models", name, "max_output"] => {
            let model = model_layer_mut(layer, name);
            model.max_output = Some(value.parse::<u64>().context("parse max_output as u64")?);
        }
        ["models", name, "supports_tools"] => {
            let model = model_layer_mut(layer, name);
            model.supports_tools = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_tools as bool")?,
            );
        }
        ["models", name, "supports_thinking"] => {
            let model = model_layer_mut(layer, name);
            model.supports_thinking = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_thinking as bool")?,
            );
        }
        ["models", name, "supports_vision"] => {
            let model = model_layer_mut(layer, name);
            model.supports_vision = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_vision as bool")?,
            );
        }
        ["models", name, "supports_web_search"] => {
            let model = model_layer_mut(layer, name);
            model.supports_web_search = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_web_search as bool")?,
            );
        }
        ["models", name, "supports_mcp_tools"] => {
            let model = model_layer_mut(layer, name);
            model.supports_mcp_tools = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_mcp_tools as bool")?,
            );
        }
        ["models", name, "supports_partial"] => {
            let model = model_layer_mut(layer, name);
            model.supports_partial = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_partial as bool")?,
            );
        }
        ["models", name, "supports_grounding"] => {
            let model = model_layer_mut(layer, name);
            model.supports_grounding = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_grounding as bool")?,
            );
        }
        ["models", name, "supports_code_execution"] => {
            let model = model_layer_mut(layer, name);
            model.supports_code_execution = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_code_execution as bool")?,
            );
        }
        ["models", name, "supports_caching"] => {
            let model = model_layer_mut(layer, name);
            model.supports_caching = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_caching as bool")?,
            );
        }
        ["models", name, "provider_routing", "sort"] => {
            let routing = model_routing_layer_mut(layer, name);
            routing.sort = Some(value.into());
        }
        ["models", name, "provider_routing", "order"] => {
            let routing = model_routing_layer_mut(layer, name);
            routing.order = Some(parse_string_list(
                value,
                "parse JSON array for provider_routing.order",
            )?);
        }
        ["models", name, "provider_routing", "allow_fallbacks"] => {
            let routing = model_routing_layer_mut(layer, name);
            routing.allow_fallbacks = Some(
                value
                    .parse::<bool>()
                    .context("parse allow_fallbacks as bool")?,
            );
        }
        ["models", name, "provider_routing", "max_price"] => {
            let routing = model_routing_layer_mut(layer, name);
            routing.max_price = Some(value.parse::<f64>().context("parse max_price as f64")?);
        }
        ["models", name, "provider_routing", "require_parameters"] => {
            let routing = model_routing_layer_mut(layer, name);
            routing.require_parameters = Some(parse_string_list(
                value,
                "parse JSON array for provider_routing.require_parameters",
            )?);
        }
        ["models", name, "tool_format"] => {
            let model = model_layer_mut(layer, name);
            model.tool_format = Some(value.into());
        }
        ["models", name, "cost_input_per_m"] => {
            let model = model_layer_mut(layer, name);
            model.cost_input_per_m = Some(
                value
                    .parse::<f64>()
                    .context("parse cost_input_per_m as f64")?,
            );
        }
        ["models", name, "cost_output_per_m"] => {
            let model = model_layer_mut(layer, name);
            model.cost_output_per_m = Some(
                value
                    .parse::<f64>()
                    .context("parse cost_output_per_m as f64")?,
            );
        }
        ["models", name, "cost_input_per_m_high"] => {
            let model = model_layer_mut(layer, name);
            model.cost_input_per_m_high = Some(
                value
                    .parse::<f64>()
                    .context("parse cost_input_per_m_high as f64")?,
            );
        }
        ["models", name, "cost_output_per_m_high"] => {
            let model = model_layer_mut(layer, name);
            model.cost_output_per_m_high = Some(
                value
                    .parse::<f64>()
                    .context("parse cost_output_per_m_high as f64")?,
            );
        }
        ["models", name, "cost_cache_read_per_m"] => {
            let model = model_layer_mut(layer, name);
            model.cost_cache_read_per_m = Some(
                value
                    .parse::<f64>()
                    .context("parse cost_cache_read_per_m as f64")?,
            );
        }
        ["models", name, "cost_cache_write_per_m"] => {
            let model = model_layer_mut(layer, name);
            model.cost_cache_write_per_m = Some(
                value
                    .parse::<f64>()
                    .context("parse cost_cache_write_per_m as f64")?,
            );
        }
        ["models", name, "thinking_level"] => {
            let model = model_layer_mut(layer, name);
            model.thinking_level = Some(value.into());
        }
        ["models", name, "max_tools"] => {
            let model = model_layer_mut(layer, name);
            model.max_tools = Some(value.parse::<u32>().context("parse max_tools as u32")?);
        }
        ["models", name, "tokenizer_ratio"] => {
            let model = model_layer_mut(layer, name);
            model.tokenizer_ratio = Some(
                value
                    .parse::<f64>()
                    .context("parse tokenizer_ratio as f64")?,
            );
        }
        ["models", name, "supports_search"] => {
            let model = model_layer_mut(layer, name);
            model.supports_search = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_search as bool")?,
            );
        }
        ["models", name, "supports_citations"] => {
            let model = model_layer_mut(layer, name);
            model.supports_citations = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_citations as bool")?,
            );
        }
        ["models", name, "supports_async"] => {
            let model = model_layer_mut(layer, name);
            model.supports_async = Some(
                value
                    .parse::<bool>()
                    .context("parse supports_async as bool")?,
            );
        }
        ["models", name, "is_embedding_model"] => {
            let model = model_layer_mut(layer, name);
            model.is_embedding_model = Some(
                value
                    .parse::<bool>()
                    .context("parse is_embedding_model as bool")?,
            );
        }
        ["models", name, "search_context_size"] => {
            let model = model_layer_mut(layer, name);
            model.search_context_size = Some(value.into());
        }
        ["models", name, "cost_per_request"] => {
            let model = model_layer_mut(layer, name);
            model.cost_per_request = Some(
                value
                    .parse::<f64>()
                    .context("parse cost_per_request as f64")?,
            );
        }
        ["serve", "auth", "enabled"] => {
            let auth = serve_auth_layer_mut(layer);
            auth.enabled = Some(value.parse::<bool>().context("parse enabled as bool")?);
        }
        ["serve", "auth", "api_key"] => {
            let auth = serve_auth_layer_mut(layer);
            auth.api_key = Some(value.into());
        }
        ["serve", "deploy", "provider"] => {
            let deploy = serve_deploy_layer_mut(layer);
            deploy.provider = Some(value.into());
        }
        ["serve", "deploy", "environment"] => {
            let deploy = serve_deploy_layer_mut(layer);
            deploy.environment = Some(parse_string_list(
                value,
                "parse JSON array for serve.deploy.environment",
            )?);
        }
        ["serve", "deploy", "webhooks"] => {
            let deploy = serve_deploy_layer_mut(layer);
            deploy.webhooks = Some(
                serde_json::from_str(value)
                    .context("parse JSON array for serve.deploy.webhooks")?,
            );
        }
        _ => return Err(anyhow!("unknown key: {key}")),
    }

    Ok(())
}

fn parse_string_list(value: &str, json_context: &'static str) -> Result<Vec<String>> {
    if value.trim_start().starts_with('[') {
        serde_json::from_str(value).context(json_context)
    } else {
        Ok(value.split_whitespace().map(String::from).collect())
    }
}

fn parse_string_enum<T>(value: &str, context: &'static str) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.to_string())).context(context)
}

fn provider_layer_mut<'a>(layer: &'a mut ConfigLayer, name: &str) -> &'a mut ProviderLayer {
    layer
        .providers
        .get_or_insert_with(HashMap::new)
        .entry(name.to_string())
        .or_default()
}

fn model_layer_mut<'a>(layer: &'a mut ConfigLayer, name: &str) -> &'a mut ModelProfileLayer {
    layer
        .models
        .get_or_insert_with(HashMap::new)
        .entry(name.to_string())
        .or_default()
}

fn model_routing_layer_mut<'a>(
    layer: &'a mut ConfigLayer,
    name: &str,
) -> &'a mut ProviderRoutingLayer {
    model_layer_mut(layer, name)
        .provider_routing
        .get_or_insert_with(ProviderRoutingLayer::default)
}

fn serve_auth_layer_mut(layer: &mut ConfigLayer) -> &mut ServeAuthLayer {
    layer
        .serve
        .get_or_insert_with(ServeLayer::default)
        .auth
        .get_or_insert_with(ServeAuthLayer::default)
}

fn serve_deploy_layer_mut(layer: &mut ConfigLayer) -> &mut ServeDeployLayer {
    layer
        .serve
        .get_or_insert_with(ServeLayer::default)
        .deploy
        .get_or_insert_with(ServeDeployLayer::default)
}

fn collect_env_override_layer() -> Result<(ConfigLayer, Vec<String>)> {
    collect_env_override_layer_from(std::env::vars())
}

fn collect_env_override_layer_from<I>(vars: I) -> Result<(ConfigLayer, Vec<String>)>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut layer = ConfigLayer::default();
    let mut paths = Vec::new();

    for (key, value) in vars {
        let Some(path) = env_override_path(&key) else {
            continue;
        };
        apply_layer_value(&mut layer, &path, &value)
            .with_context(|| format!("set {path} from {key}"))?;
        paths.push(path);
    }

    Ok((layer, paths))
}

fn env_override_path(key: &str) -> Option<String> {
    let suffix = key.strip_prefix("ROKO__")?;
    if suffix.is_empty() {
        return None;
    }
    Some(suffix.to_ascii_lowercase().replace("__", "."))
}

fn apply_env_source_overrides(sources: &mut ConfigSources, paths: &[String]) {
    for path in paths {
        match path.as_str() {
            "auto_plan" => sources.auto_plan = Source::Env,
            "agent.command" => sources.agent_command = Source::Env,
            "agent.args" => sources.agent_args = Source::Env,
            "agent.model" => sources.agent_model = Source::Env,
            "agent.effort" => sources.agent_effort = Source::Env,
            "agent.bare_mode" => sources.agent_bare_mode = Source::Env,
            "agent.fallback_model" => sources.agent_fallback_model = Source::Env,
            "agent.timeout_ms" => sources.agent_timeout_ms = Source::Env,
            "tools.prefer_mcp" => sources.tools_prefer_mcp = Source::Env,
            "tools.global_denied" => sources.tools_global_denied = Source::Env,
            "tools.mcp_timeout_secs" => sources.tools_mcp_timeout_secs = Source::Env,
            "prompt.token_budget" => sources.prompt_token_budget = Source::Env,
            "prompt.role" => sources.prompt_role = Source::Env,
            "dreams.auto_dream" => sources.dreams_auto_dream = Source::Env,
            "dreams.idle_threshold_mins" => sources.dreams_idle_threshold_mins = Source::Env,
            "dreams.min_episodes_for_dream" => sources.dreams_min_episodes_for_dream = Source::Env,
            path if path.starts_with("providers.") => sources.providers = Source::Env,
            path if path.starts_with("models.") => sources.models = Source::Env,
            _ => {}
        }
    }
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
    /// Whether to use isolated git worktrees for plan and task execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_worktrees: Option<bool>,
    /// Multiplier applied to expected-minutes before speculative task splits kick in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speculative_threshold_multiplier: Option<f64>,
}

/// Partial `ServeConfig` — every field optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ServeLayer {
    /// Whether serve-side publish events trigger orchestration automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_orchestrate: Option<bool>,
    /// API auth settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<ServeAuthLayer>,
    /// Cloud deployment settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy: Option<ServeDeployLayer>,
}

impl ServeLayer {
    /// Merge another layer on top — `overlay` wins.
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            auto_orchestrate: overlay.auto_orchestrate.or(self.auto_orchestrate),
            auth: match (self.auth, overlay.auth) {
                (Some(base), Some(overlay)) => Some(base.merge(overlay)),
                (_, Some(overlay)) => Some(overlay),
                (base, None) => base,
            },
            deploy: match (self.deploy, overlay.deploy) {
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

/// Partial cloud deployment settings — every field optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ServeDeployLayer {
    /// Deployment provider, e.g. `railway` or `fly`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Environment variables required for deploy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Vec<String>>,
    /// Webhooks to register after deploy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhooks: Option<Vec<ServeDeployWebhookLayer>>,
}

impl ServeDeployLayer {
    /// Merge another layer on top — `overlay` wins.
    #[must_use]
    pub fn merge(self, overlay: Self) -> Self {
        Self {
            provider: overlay.provider.or(self.provider),
            environment: overlay.environment.or(self.environment),
            webhooks: overlay.webhooks.or(self.webhooks),
        }
    }

    /// Resolve into a concrete [`ServeConfig::deploy`] value.
    #[must_use]
    pub fn resolve(self, defaults: ServeDeployConfig) -> ServeDeployConfig {
        ServeDeployConfig {
            provider: self.provider.unwrap_or(defaults.provider),
            environment: self.environment.unwrap_or(defaults.environment),
            webhooks: match self.webhooks {
                Some(webhooks) => webhooks
                    .into_iter()
                    .map(ServeDeployWebhookLayer::resolve)
                    .collect(),
                None => defaults.webhooks,
            },
        }
    }
}

/// Partial webhook registration settings — every field optional.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ServeDeployWebhookLayer {
    /// Webhook provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Repository owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Repository name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

impl ServeDeployWebhookLayer {
    /// Resolve into a concrete [`ServeDeployWebhookConfig`].
    #[must_use]
    pub fn resolve(self) -> ServeDeployWebhookConfig {
        ServeDeployWebhookConfig {
            provider: self.provider.unwrap_or_else(|| "github".to_string()),
            owner: self.owner.unwrap_or_default(),
            repo: self.repo.unwrap_or_default(),
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
            use_worktrees: overlay.use_worktrees.or(self.use_worktrees),
            speculative_threshold_multiplier: overlay
                .speculative_threshold_multiplier
                .or(self.speculative_threshold_multiplier),
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
    /// Loaded runtime repo registry.
    pub repo_registry: RepoRegistry,
    /// Which source supplied each field.
    pub sources: ConfigSources,
    /// Paths consulted during resolution.
    pub paths: ConfigPaths,
}

/// Per-field provenance for [`ResolvedConfig`].
#[derive(Clone, Debug)]
pub struct ConfigSources {
    /// Where `auto_plan` came from.
    pub auto_plan: Source,
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
    /// Where `providers` came from.
    pub providers: Source,
    /// Where `models` came from.
    pub models: Source,
    /// Where `dreams.auto_dream` came from.
    pub dreams_auto_dream: Source,
    /// Where `dreams.idle_threshold_mins` came from.
    pub dreams_idle_threshold_mins: Source,
    /// Where `dreams.min_episodes_for_dream` came from.
    pub dreams_min_episodes_for_dream: Source,
    /// Where `gates` came from.
    pub gates: Source,
}

/// Load global + project configs, merge them, and return a `ResolvedConfig`.
///
/// Precedence (highest first): `ROKO__*` env vars → `ROKO_CONFIG` env var →
/// project → global → defaults.
pub fn load_layered(workdir: &Path) -> Result<ResolvedConfig> {
    let paths = resolve_paths(workdir);
    let (env_layer, env_paths) = collect_env_override_layer()?;

    // If ROKO_CONFIG is set, it alone resolves the file config; field-level
    // ROKO__* env vars still apply on top.
    if let Some(env_path) = &paths.env_override {
        let layer = ConfigLayer::from_file(env_path)?.merge(env_layer);
        let sources = sources_from_layer(&layer, Source::Env, Source::Default);
        let config = layer.resolve()?;
        let repo_registry = RepoRegistry::load(&config, workdir)?;
        return Ok(ResolvedConfig {
            config,
            repo_registry,
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

    let mut sources = compute_sources(&global_layer, &project_layer);
    apply_env_source_overrides(&mut sources, &env_paths);
    let merged = global_layer.merge(project_layer).merge(env_layer);
    let config = merged.resolve()?;
    let repo_registry = RepoRegistry::load(&config, workdir)?;

    Ok(ResolvedConfig {
        config,
        repo_registry,
        sources,
        paths,
    })
}

/// Compute per-field provenance from global + project layers.
fn compute_sources(global: &ConfigLayer, project: &ConfigLayer) -> ConfigSources {
    let g_auto_plan = global.auto_plan.is_some();
    let p_auto_plan = project.auto_plan.is_some();
    let g_agent = global.agent.as_ref();
    let g_tools = global.tools.as_ref();
    let p_agent = project.agent.as_ref();
    let p_tools = project.tools.as_ref();
    let g_prompt = global.prompt.as_ref();
    let p_prompt = project.prompt.as_ref();
    let g_dreams = global.dreams.as_ref();
    let p_dreams = project.dreams.as_ref();

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
        auto_plan: pick(p_auto_plan, g_auto_plan),
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
        providers: pick(project.providers.is_some(), global.providers.is_some()),
        models: pick(project.models.is_some(), global.models.is_some()),
        dreams_auto_dream: pick(
            p_dreams.and_then(|d| d.auto_dream).is_some(),
            g_dreams.and_then(|d| d.auto_dream).is_some(),
        ),
        dreams_idle_threshold_mins: pick(
            p_dreams.and_then(|d| d.idle_threshold_mins).is_some(),
            g_dreams.and_then(|d| d.idle_threshold_mins).is_some(),
        ),
        dreams_min_episodes_for_dream: pick(
            p_dreams.and_then(|d| d.min_episodes_for_dream).is_some(),
            g_dreams.and_then(|d| d.min_episodes_for_dream).is_some(),
        ),
        gates: pick(project.gates.is_some(), global.gates.is_some()),
    }
}

/// Tag every field in a single-layer config as `present` or `fallback`.
fn sources_from_layer(layer: &ConfigLayer, present: Source, fallback: Source) -> ConfigSources {
    let agent = layer.agent.as_ref();
    let tools = layer.tools.as_ref();
    let prompt = layer.prompt.as_ref();
    let dreams = layer.dreams.as_ref();
    let pick = |is_set: bool| -> Source { if is_set { present } else { fallback } };
    ConfigSources {
        auto_plan: pick(layer.auto_plan.is_some()),
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
        providers: pick(layer.providers.is_some()),
        models: pick(layer.models.is_some()),
        dreams_auto_dream: pick(dreams.and_then(|d| d.auto_dream).is_some()),
        dreams_idle_threshold_mins: pick(dreams.and_then(|d| d.idle_threshold_mins).is_some()),
        dreams_min_episodes_for_dream: pick(
            dreams.and_then(|d| d.min_episodes_for_dream).is_some(),
        ),
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
        assert!(cfg.repos.is_empty());
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

[dreams]
auto_dream = false
idle_threshold_mins = 30
min_episodes_for_dream = 8

[tools]
prefer_mcp = true
global_denied = ["write_file", "edit_file"]
mcp_timeout_secs = 45

[prompt]
token_budget = 20000
role = "You are a senior Rust engineer."

[[repos]]
name = "roko"
path = "/Users/will/dev/nunchi/roko/roko"
branch = "main"
templates = ["pr-reviewer", "test-writer", "ci-fixer"]

[[repos.subscriptions]]
template = "code-implementer"
trigger = "github:issues:labeled:implement"

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
        assert!(!cfg.dreams.auto_dream);
        assert_eq!(cfg.dreams.idle_threshold_mins, 30);
        assert_eq!(cfg.dreams.min_episodes_for_dream, 8);
        assert!(cfg.tools.prefer_mcp);
        assert_eq!(
            cfg.tools.global_denied,
            vec!["write_file".to_string(), "edit_file".to_string()]
        );
        assert_eq!(cfg.tools.mcp_timeout_secs, 45);
        assert_eq!(cfg.prompt.token_budget, 20_000);
        assert_eq!(cfg.repos.len(), 1);
        assert_eq!(cfg.repos[0].name, "roko");
        assert_eq!(
            cfg.repos[0].templates,
            vec![
                "pr-reviewer".to_string(),
                "test-writer".to_string(),
                "ci-fixer".to_string()
            ]
        );
        assert_eq!(cfg.repos[0].subscriptions.len(), 1);
        assert_eq!(cfg.repos[0].subscriptions[0].template, "code-implementer");
        assert_eq!(
            cfg.repos[0].subscriptions[0].trigger,
            "github:issues:labeled:implement"
        );
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
use_worktrees = true
"#;
        let cfg = Config::parse_toml(toml).unwrap();
        assert_eq!(cfg.executor.max_concurrent_plans, 8);
        assert_eq!(cfg.executor.max_concurrent_tasks, 12);
        assert_eq!(cfg.executor.max_auto_fix_iterations, 9);
        assert_eq!(cfg.executor.max_merge_attempts, 4);
        assert_eq!(cfg.executor.task_timeout_secs, 42);
        assert_eq!(cfg.executor.budget_usd, Some(1.5));
        assert!(!cfg.executor.auto_replan);
        assert!(cfg.executor.use_worktrees);
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
    fn parses_serve_deploy_section_from_toml() {
        let toml = r#"
[agent]
command = "cat"

[serve.deploy]
provider = "fly"
environment = ["GITHUB_TOKEN", "SLACK_BOT_TOKEN"]

[[serve.deploy.webhooks]]
provider = "github"
owner = "nunchi"
repo = "roko"
"#;
        let cfg = Config::parse_toml(toml).unwrap();
        assert_eq!(cfg.serve.deploy.provider, "fly");
        assert_eq!(
            cfg.serve.deploy.environment,
            vec!["GITHUB_TOKEN".to_string(), "SLACK_BOT_TOKEN".to_string()]
        );
        assert_eq!(cfg.serve.deploy.webhooks.len(), 1);
        assert_eq!(cfg.serve.deploy.webhooks[0].provider, "github");
        assert_eq!(cfg.serve.deploy.webhooks[0].owner, "nunchi");
        assert_eq!(cfg.serve.deploy.webhooks[0].repo, "roko");
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
use_worktrees = true
"#,
        )
        .unwrap();

        let cfg = layer.resolve().unwrap();
        assert_eq!(cfg.executor.max_concurrent_plans, 6);
        assert_eq!(
            cfg.executor.max_concurrent_tasks,
            ExecutorConfig::default().max_concurrent_tasks
        );
        assert_eq!(cfg.executor.task_timeout_secs, 900);
        assert!(!cfg.executor.auto_replan);
        assert!(cfg.executor.use_worktrees);
        assert!(!cfg.serve.auth.enabled);
        assert!(cfg.serve.auth.api_key.is_empty());
        assert_eq!(cfg.serve.deploy.provider, "railway");
        assert_eq!(
            cfg.serve.deploy.environment,
            vec![
                "GITHUB_TOKEN".to_string(),
                "GITHUB_WEBHOOK_SECRET".to_string(),
                "SLACK_BOT_TOKEN".to_string(),
                "SLACK_SIGNING_SECRET".to_string()
            ]
        );
        assert!(cfg.serve.deploy.webhooks.is_empty());
    }

    #[test]
    fn layer_resolve_uses_auto_plan_override() {
        let layer = ConfigLayer::parse_toml(
            r#"
auto_plan = true
"#,
        )
        .unwrap();

        let cfg = layer.resolve().unwrap();
        assert!(cfg.auto_plan);
    }

    #[test]
    fn default_config_roundtrips_through_toml() {
        let cfg = Config::default();
        let text = cfg.to_toml().unwrap();
        let parsed = Config::parse_toml(&text).unwrap();
        assert_eq!(parsed.agent.command, cfg.agent.command);
        assert_eq!(parsed.auto_plan, cfg.auto_plan);
        assert_eq!(parsed.dreams.auto_dream, cfg.dreams.auto_dream);
        assert_eq!(
            parsed.dreams.idle_threshold_mins,
            cfg.dreams.idle_threshold_mins
        );
        assert_eq!(
            parsed.dreams.min_episodes_for_dream,
            cfg.dreams.min_episodes_for_dream
        );
        assert_eq!(parsed.tools.prefer_mcp, cfg.tools.prefer_mcp);
        assert_eq!(parsed.tools.global_denied, cfg.tools.global_denied);
        assert_eq!(parsed.tools.mcp_timeout_secs, cfg.tools.mcp_timeout_secs);
        assert_eq!(parsed.providers, cfg.providers);
        assert_eq!(parsed.models, cfg.models);
        assert_eq!(parsed.repos.len(), cfg.repos.len());
        assert_eq!(parsed.gates.len(), cfg.gates.len());
        assert_eq!(parsed.serve.auth.enabled, cfg.serve.auth.enabled);
        assert_eq!(parsed.serve.auth.api_key, cfg.serve.auth.api_key);
        assert_eq!(parsed.serve.deploy.provider, cfg.serve.deploy.provider);
        assert_eq!(
            parsed.serve.deploy.environment,
            cfg.serve.deploy.environment
        );
        assert_eq!(parsed.serve.deploy.webhooks, cfg.serve.deploy.webhooks);
    }

    #[test]
    fn repo_registry_loads_repo_local_config() {
        use std::fs;

        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().join("repo-a");
        fs::create_dir_all(repo_root.join(".roko")).unwrap();
        fs::write(
            repo_root.join(".roko").join("roko.toml"),
            "schema_version = 2\n",
        )
        .unwrap();

        let mut cfg = Config::default();
        cfg.repos = vec![RepoConfig {
            name: "repo-a".to_string(),
            path: PathBuf::from("repo-a"),
            branch: "main".to_string(),
            templates: Vec::new(),
            subscriptions: Vec::new(),
        }];

        let registry = RepoRegistry::load(&cfg, tmp.path()).unwrap();
        assert_eq!(registry.repos().len(), 1);
        let repo = registry.get("repo-a").unwrap();
        assert!(repo.root.ends_with("repo-a"));
        assert!(repo.roko_config.is_some());
        assert!(
            repo.roko_config_path
                .as_ref()
                .is_some_and(|path| path.ends_with(".roko/roko.toml"))
        );
    }

    #[test]
    fn repo_registry_errors_when_repo_path_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.repos = vec![RepoConfig {
            name: "missing".to_string(),
            path: PathBuf::from("missing"),
            branch: "main".to_string(),
            templates: Vec::new(),
            subscriptions: Vec::new(),
        }];

        let err = RepoRegistry::load(&cfg, tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing"));
        assert!(msg.contains("resolve repo 'missing' path"));
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

        let merged = global.merge(project).resolve().unwrap();
        assert_eq!(merged.agent.command, "mods");
        assert_eq!(
            merged.agent.args,
            vec!["run".to_string(), "llama3".to_string()]
        );
        assert_eq!(merged.agent.timeout_ms, 60_000);
        assert!(merged.tools.prefer_mcp);
        assert_eq!(merged.tools.global_denied, vec!["web_fetch".to_string()]);
        assert_eq!(merged.tools.mcp_timeout_secs, 99);
        assert_eq!(merged.prompt.token_budget, 8000);
        assert_eq!(merged.prompt.role, "global role");
    }

    #[test]
    fn layer_merge_merges_provider_and_model_entries() {
        let global = ConfigLayer::parse_toml(
            r#"
[providers.zai]
kind = "openai_compat"
base_url = "https://global.example"
api_key_env = "GLOBAL_KEY"

[models.glm-5-1]
provider = "zai"
slug = "glm-5.1"
supports_tools = true
"#,
        )
        .unwrap();
        let project = ConfigLayer::parse_toml(
            r#"
[providers.zai]
base_url = "https://project.example"
timeout_ms = 42000

[models.glm-5-1]
supports_thinking = true
max_output = 131072
"#,
        )
        .unwrap();

        let merged = global.merge(project).resolve().unwrap();
        let provider = merged.providers.get("zai").unwrap();
        assert_eq!(provider.kind, ProviderKind::OpenAiCompat);
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://project.example")
        );
        assert_eq!(provider.api_key_env.as_deref(), Some("GLOBAL_KEY"));
        assert_eq!(provider.timeout_ms, Some(42_000));

        let model = merged.models.get("glm-5-1").unwrap();
        assert_eq!(model.provider, "zai");
        assert_eq!(model.slug, "glm-5.1");
        assert!(model.supports_tools);
        assert!(model.supports_thinking);
        assert_eq!(model.max_output, Some(131_072));
    }

    #[test]
    fn apply_layer_value_sets_provider_and_model_entries() {
        let mut layer = ConfigLayer::default();
        apply_layer_value(&mut layer, "providers.zai.kind", "openai_compat").unwrap();
        apply_layer_value(
            &mut layer,
            "providers.zai.base_url",
            "https://api.z.ai/api/paas/v4",
        )
        .unwrap();
        apply_layer_value(&mut layer, "models.glm51.provider", "zai").unwrap();
        apply_layer_value(&mut layer, "models.glm51.slug", "glm-5.1").unwrap();
        apply_layer_value(&mut layer, "models.glm51.supports_thinking", "true").unwrap();

        let cfg = layer.resolve().unwrap();
        let provider = cfg.providers.get("zai").unwrap();
        assert_eq!(provider.kind, ProviderKind::OpenAiCompat);
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://api.z.ai/api/paas/v4")
        );

        let model = cfg.models.get("glm51").unwrap();
        assert_eq!(model.provider, "zai");
        assert_eq!(model.slug, "glm-5.1");
        assert!(model.supports_thinking);
    }

    #[test]
    fn layer_resolve_errors_when_provider_kind_missing() {
        let layer = ConfigLayer::parse_toml(
            r#"
[providers.zai]
base_url = "https://api.z.ai/api/paas/v4"
"#,
        )
        .unwrap();

        let err = layer.resolve().unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("resolve providers.zai"));
        assert!(msg.contains("missing required field `kind`"));
    }

    #[test]
    fn layer_resolve_empty_uses_defaults() {
        let layer = ConfigLayer::default();
        let cfg = layer.resolve().unwrap();
        assert_eq!(cfg.agent.command, "cat");
        assert!(!cfg.tools.prefer_mcp);
        assert!(cfg.tools.global_denied.is_empty());
        assert_eq!(cfg.tools.mcp_timeout_secs, 30);
        assert_eq!(cfg.prompt.token_budget, 10_000);
        assert!(cfg.providers.is_empty());
        assert!(cfg.models.is_empty());
        assert!(cfg.dreams.auto_dream);
        assert_eq!(cfg.dreams.idle_threshold_mins, 15);
        assert_eq!(cfg.dreams.min_episodes_for_dream, 5);
        assert_eq!(cfg.daimon.strategy_space.domain, "coding");
        assert_eq!(cfg.daimon.strategy_space.dimensions[0], "complexity");
        assert!(cfg.gates.is_empty());
        assert!(!cfg.serve.auth.enabled);
        assert!(cfg.serve.auth.api_key.is_empty());
    }

    #[test]
    fn layer_resolve_uses_dreams_override() {
        let layer = ConfigLayer::parse_toml(
            r#"
[dreams]
auto_dream = false
idle_threshold_mins = 22
min_episodes_for_dream = 9
"#,
        )
        .unwrap();

        let cfg = layer.resolve().unwrap();
        assert!(!cfg.dreams.auto_dream);
        assert_eq!(cfg.dreams.idle_threshold_mins, 22);
        assert_eq!(cfg.dreams.min_episodes_for_dream, 9);
    }

    #[test]
    fn layer_resolve_uses_daimon_strategy_space_override() {
        let layer = ConfigLayer::parse_toml(
            r#"
[daimon.strategy_space]
domain = "chain"
dimensions = [
  "volatility",
  "liquidity",
  "correlation",
  "leverage",
  "time_horizon",
  "concentration",
  "counterparty_risk",
  "regulatory_exposure",
]
"#,
        )
        .unwrap();

        let cfg = layer.resolve().unwrap();
        assert_eq!(cfg.daimon.strategy_space.domain, "chain");
        assert_eq!(cfg.daimon.strategy_space.dimensions[0], "volatility");
        assert_eq!(
            cfg.daimon.strategy_space.dimensions[7],
            "regulatory_exposure"
        );
    }

    #[test]
    fn layer_resolve_rejects_non_8d_strategy_space() {
        let layer = ConfigLayer::parse_toml(
            r#"
[daimon.strategy_space]
domain = "chain"
dimensions = ["volatility", "liquidity"]
"#,
        )
        .unwrap();

        let err = layer.resolve().unwrap_err();
        assert!(
            err.to_string()
                .contains("daimon.strategy_space.dimensions must contain exactly 8 entries")
        );
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
        assert_eq!(sources.providers, Source::Default);
        assert_eq!(sources.models, Source::Default);
        assert_eq!(sources.agent_args, Source::Default);
        assert_eq!(sources.dreams_auto_dream, Source::Default);
        assert_eq!(sources.dreams_idle_threshold_mins, Source::Default);
        assert_eq!(sources.dreams_min_episodes_for_dream, Source::Default);
    }

    #[test]
    fn env_override_layer_applies_last_and_marks_sources() {
        let global = ConfigLayer::parse_toml(
            r#"
[agent]
command = "cat"
model = "from-file"

[providers.zai]
kind = "openai_compat"
base_url = "https://file.example"
"#,
        )
        .unwrap();
        let project = ConfigLayer::default();
        let (env_layer, env_paths) = collect_env_override_layer_from(vec![
            ("ROKO__AGENT__MODEL".to_string(), "test".to_string()),
            (
                "ROKO__PROVIDERS__ZAI__BASE_URL".to_string(),
                "https://env.example".to_string(),
            ),
        ])
        .unwrap();

        let mut sources = compute_sources(&global, &project);
        apply_env_source_overrides(&mut sources, &env_paths);
        let resolved = global.merge(project).merge(env_layer).resolve().unwrap();

        assert_eq!(resolved.agent.model.as_deref(), Some("test"));
        assert_eq!(sources.agent_model, Source::Env);
        assert_eq!(
            resolved.providers.get("zai").unwrap().base_url.as_deref(),
            Some("https://env.example")
        );
        assert_eq!(sources.providers, Source::Env);
    }

    #[test]
    fn env_override_layer_applies_daimon_strategy_space() {
        let (env_layer, _) = collect_env_override_layer_from(vec![
            (
                "ROKO__DAIMON__STRATEGY_SPACE__DOMAIN".to_string(),
                "chain".to_string(),
            ),
            (
                "ROKO__DAIMON__STRATEGY_SPACE__DIMENSIONS".to_string(),
                serde_json::json!([
                    "volatility",
                    "liquidity",
                    "correlation",
                    "leverage",
                    "time_horizon",
                    "concentration",
                    "counterparty_risk",
                    "regulatory_exposure"
                ])
                .to_string(),
            ),
        ])
        .unwrap();

        let resolved = ConfigLayer::default().merge(env_layer).resolve().unwrap();
        assert_eq!(resolved.daimon.strategy_space.domain, "chain");
        assert_eq!(resolved.daimon.strategy_space.dimensions[3], "leverage");
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
        let merged = global.merge(project).resolve().unwrap();
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
        let rendered = Config::default_toml_template(false).unwrap();
        assert!(rendered.contains("# REQUIRED_ENV"));
        assert!(rendered.contains("GITHUB_TOKEN"));
        assert!(rendered.contains("GITHUB_WEBHOOK_SECRET"));
        assert!(rendered.contains("SLACK_BOT_TOKEN"));
        assert!(rendered.contains("SLACK_SIGNING_SECRET"));
        assert!(rendered.contains("ANTHROPIC_API_KEY"));
        assert!(rendered.contains("[serve.deploy]"));
        assert!(rendered.contains("[prd]"));
        assert!(rendered.contains("auto_plan = false"));
        assert!(rendered.contains("[dreams]"));
        assert!(rendered.contains("auto_dream = true"));
    }

    #[test]
    fn cloud_default_toml_template_includes_cloud_settings() {
        let rendered = Config::default_toml_template(true).unwrap();
        assert!(rendered.contains(r#"log_format = "json""#));
        assert!(rendered.contains(r#"bind = "0.0.0.0""#));
        assert!(rendered.contains(r#"data_dir = "/data/.roko""#));
        assert!(rendered.contains(r#"provider = "railway""#));
        assert!(rendered.contains("GITHUB_WEBHOOK_SECRET"));
        assert!(rendered.contains("Auto-register webhooks after deploy"));
        assert!(rendered.contains("[[serve.deploy.webhooks]]"));
    }
}
