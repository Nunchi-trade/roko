//! Unified `RokoConfig` schema with hierarchical sections.
//!
//! Every section is a separate struct so callers can destructure just the
//! slice they need. All fields carry serde defaults so a bare `schema_version = 2`
//! produces a fully-populated config.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use crate::agent::{AgentBackend, ProviderKind};
use crate::tool::{ToolFormat, profile_for_model};
use serde::{Deserialize, Serialize};

/// Current schema version. Bump on incompatible changes.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

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

    /// PRD lifecycle settings.
    #[serde(default)]
    pub prd: PrdConfig,

    /// Agent / model settings (including per-role overrides).
    #[serde(default)]
    pub agent: AgentConfig,

    /// Provider registry keyed by provider name.
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,

    /// Model registry keyed by model name.
    #[serde(default)]
    pub models: HashMap<String, ModelProfile>,

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

    /// File-system watcher settings.
    #[serde(default, skip_serializing_if = "WatcherConfig::is_empty")]
    pub watcher: WatcherConfig,

    /// Learning subsystem toggles.
    #[serde(default)]
    pub learning: LearningConfig,

    /// Terminal UI preferences.
    #[serde(default)]
    pub tui: TuiConfig,

    /// HTTP API serving options.
    #[serde(default)]
    pub serve: ServeConfig,

    /// Cron scheduler settings.
    #[serde(default)]
    pub scheduler: SchedulerConfig,

    /// Webhook ingress configuration.
    #[serde(default)]
    pub webhooks: WebhooksConfig,

    /// Event subscriptions loaded at server startup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscriptions: Vec<SubscriptionConfig>,

    /// HTTP server / gateway settings.
    #[serde(default)]
    pub server: ServerConfig,

    /// Cloud deployment settings (Railway, etc.).
    #[serde(default)]
    pub deploy: DeployConfig,
}

const fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

impl Default for RokoConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            project: ProjectConfig::default(),
            prd: PrdConfig::default(),
            agent: AgentConfig::default(),
            providers: HashMap::new(),
            models: HashMap::new(),
            gates: GatesConfig::default(),
            routing: RoutingConfig::default(),
            budget: BudgetConfig::default(),
            conductor: ConductorConfig::default(),
            watcher: WatcherConfig::default(),
            learning: LearningConfig::default(),
            tui: TuiConfig::default(),
            serve: ServeConfig::default(),
            scheduler: SchedulerConfig::default(),
            webhooks: WebhooksConfig::default(),
            subscriptions: Vec::new(),
            server: ServerConfig::default(),
            deploy: DeployConfig::default(),
        }
    }
}

impl RokoConfig {
    fn write_example_prelude(out: &mut String) {
        let _ = writeln!(
            out,
            "# Roko configuration -- all fields shown with defaults."
        );
        let _ = writeln!(
            out,
            "# Delete any section you don't need; defaults apply.\n"
        );
        let _ = writeln!(out, "schema_version = {CURRENT_SCHEMA_VERSION}\n");
    }

    fn write_example_project(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Project metadata --");
        let _ = writeln!(out, "[project]");
        let _ = writeln!(out, "name = \"{}\"", cfg.project.name);
        let _ = writeln!(out, "root = \"{}\"", cfg.project.root);
        let _ = writeln!(
            out,
            "fresh_base_branch = \"{}\"\n",
            cfg.project.fresh_base_branch
        );
    }

    fn write_example_prd(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- PRD lifecycle settings --");
        let _ = writeln!(out, "[prd]");
        let _ = writeln!(out, "auto_plan = {}\n", cfg.prd.auto_plan);
    }

    fn write_example_agent(out: &mut String, cfg: &Self) {
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
    }

    fn write_example_gates(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Verification gates --");
        let _ = writeln!(out, "[gates]");
        let _ = writeln!(out, "clippy_enabled = {}", cfg.gates.clippy_enabled);
        let _ = writeln!(out, "skip_tests = {}", cfg.gates.skip_tests);
        let _ = writeln!(out, "max_iterations = {}\n", cfg.gates.max_iterations);
    }

    fn write_example_routing(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Model routing --");
        let _ = writeln!(out, "[routing]");
        let _ = writeln!(out, "mode = \"{}\"", cfg.routing.mode);
        let _ = writeln!(out, "fast_task_model = \"{}\"", cfg.routing.fast_task_model);
        let _ = writeln!(
            out,
            "standard_task_model = \"{}\"",
            cfg.routing.standard_task_model
        );
        let _ = writeln!(
            out,
            "complex_task_model = \"{}\"\n",
            cfg.routing.complex_task_model
        );
    }

    fn write_example_budget(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Spend / token budgets --");
        let _ = writeln!(out, "[budget]");
        let _ = writeln!(out, "max_plan_usd = {:.1}", cfg.budget.max_plan_usd);
        let _ = writeln!(out, "max_turn_usd = {:.1}", cfg.budget.max_turn_usd);
        let _ = writeln!(
            out,
            "prompt_token_budget = {}\n",
            cfg.budget.prompt_token_budget
        );
    }

    fn write_example_conductor(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Conductor (meta-orchestrator) --");
        let _ = writeln!(out, "[conductor]");
        let _ = writeln!(out, "max_agents = {}", cfg.conductor.max_agents);
        let _ = writeln!(
            out,
            "max_parallel_plans = {}",
            cfg.conductor.max_parallel_plans
        );
        let _ = writeln!(out, "parallel_enabled = {}", cfg.conductor.parallel_enabled);
        let _ = writeln!(out, "express_mode = {}", cfg.conductor.express_mode);
        let _ = writeln!(
            out,
            "auto_advance_batch = {}",
            cfg.conductor.auto_advance_batch
        );
        let _ = writeln!(
            out,
            "auto_merge_on_complete = {}",
            cfg.conductor.auto_merge_on_complete
        );
        let _ = writeln!(out, "pre_plan = {}", cfg.conductor.pre_plan);
        let _ = writeln!(
            out,
            "max_auto_fix_attempts = {}\n",
            cfg.conductor.max_auto_fix_attempts
        );
    }

    fn write_example_learning(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Learning subsystem --");
        let _ = writeln!(out, "[learning]");
        let _ = writeln!(
            out,
            "auto_playbook_refresh = {}",
            cfg.learning.auto_playbook_refresh
        );
        let _ = writeln!(
            out,
            "knowledge_warnings = {}",
            cfg.learning.knowledge_warnings
        );
        let _ = writeln!(
            out,
            "learning_min_occurrences = {}\n",
            cfg.learning.learning_min_occurrences
        );
    }

    fn write_example_tui_and_server(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- TUI preferences --");
        let _ = writeln!(out, "[tui]");
        let _ = writeln!(out, "refresh_rate_ms = {}\n", cfg.tui.refresh_rate_ms);

        let _ = writeln!(out, "# -- API auth --");
        let _ = writeln!(out, "[serve.auth]");
        let _ = writeln!(out, "enabled = {}", cfg.serve.auth.enabled);
        let _ = writeln!(out, "api_key = \"{}\"\n", cfg.serve.auth.api_key);

        let _ = writeln!(out, "# -- HTTP server / gateway --");
        let _ = writeln!(out, "[server]");
        let _ = writeln!(out, "bind = \"{}\"", cfg.server.bind);
        let _ = writeln!(out, "port = {}", cfg.server.port);

        let _ = writeln!(out, "\n# -- Cloud deployment --");
        let _ = writeln!(out, "[serve.deploy]");
        let _ = writeln!(out, "provider = \"{}\"", cfg.serve.deploy.provider);
        let _ = writeln!(out, "environment = {:?}", cfg.serve.deploy.environment);
        let _ = writeln!(out, "\n[[serve.deploy.webhooks]]");
        let _ = writeln!(out, "provider = \"github\"");
        let _ = writeln!(out, "owner = \"nunchi\"");
        let _ = writeln!(out, "repo = \"roko\"");
        let _ = writeln!(out, "\n[[serve.deploy.webhooks]]");
        let _ = writeln!(out, "provider = \"github\"");
        let _ = writeln!(out, "owner = \"nunchi\"");
        let _ = writeln!(out, "repo = \"collaboration\"");
    }

    fn write_example_scheduler(out: &mut String, _cfg: &Self) {
        let _ = writeln!(out, "\n# -- Cron scheduler --");
        let _ = writeln!(out, "[scheduler]");
        let _ = writeln!(out, "[[scheduler.cron]]");
        let _ = writeln!(out, "name = \"weekly-digest\"");
        let _ = writeln!(out, "expression = \"0 9 * * MON\"");
        let _ = writeln!(out, "signal_kind = \"scheduler:cron:weekly-digest\"");
    }

    fn write_example_webhooks(out: &mut String, _cfg: &Self) {
        let _ = writeln!(out, "\n# -- Webhooks --");
        let _ = writeln!(out, "[webhooks.github]");
        let _ = writeln!(out, "secret = \"change-me\"");
    }

    fn write_example_deploy(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "\n# -- Cloud deployment (Railway, etc.) --");
        let _ = writeln!(out, "[deploy]");
        let _ = writeln!(out, "backend = \"{}\"", cfg.deploy.backend);
        let _ = writeln!(out, "# railway_api_token = \"...\"");
        let _ = writeln!(out, "# project_id = \"...\"");
        let _ = writeln!(out, "# environment_id = \"...\"");
        let _ = writeln!(
            out,
            "# worker_image = \"ghcr.io/example/roko-worker:latest\""
        );
        let _ = writeln!(out, "# default_region = \"us-west1\"");
    }

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

    /// Return the provider registry that should be used at runtime.
    ///
    /// New-style configs use `[providers.*]` directly. Older configs only had
    /// a legacy `[agent]` section, so we synthesize the minimum provider
    /// registry needed for backwards compatibility.
    #[must_use]
    pub fn effective_providers(&self) -> HashMap<String, ProviderConfig> {
        if !self.providers.is_empty() {
            return self.providers.clone();
        }

        let mut providers = HashMap::new();

        let claude_command = self
            .agent
            .command
            .clone()
            .unwrap_or_else(|| "claude".to_string());

        providers.insert(
            "claude_cli".into(),
            ProviderConfig {
                kind: ProviderKind::ClaudeCli,
                base_url: None,
                api_key_env: None,
                command: Some(claude_command),
                args: self.agent.args.clone(),
                timeout_ms: self.agent.timeout_ms,
                extra_headers: None,
                max_concurrent: None,
            },
        );

        if let Some(base_url) = self.agent_env_value("ANTHROPIC_BASE_URL") {
            providers.insert(
                "anthropic".into(),
                ProviderConfig {
                    kind: ProviderKind::AnthropicApi,
                    base_url: Some(base_url.to_owned()),
                    api_key_env: self
                        .agent_env_value("ANTHROPIC_API_KEY")
                        .map(|_| "ANTHROPIC_API_KEY".to_string()),
                    command: None,
                    args: None,
                    timeout_ms: self.agent.timeout_ms,
                    extra_headers: None,
                    max_concurrent: None,
                },
            );
        }

        providers
    }

    /// Return the model registry that should be used at runtime.
    ///
    /// New-style configs use `[models.*]` directly. Older configs only had
    /// `[agent.tier_models]` and a default model, so we synthesize the
    /// minimum model registry needed for backwards compatibility.
    #[must_use]
    pub fn effective_models(&self) -> HashMap<String, ModelProfile> {
        let mut models = HashMap::new();

        for slug in self.agent.tier_models.values() {
            let slug = slug.trim();
            if slug.is_empty() {
                continue;
            }

            models
                .entry(slug.to_owned())
                .or_insert_with(|| self.synthesized_model_profile(slug));
        }

        let default_model = self.agent.default_model.trim();
        if !default_model.is_empty() {
            models
                .entry(default_model.to_owned())
                .or_insert_with(|| self.synthesized_model_profile(default_model));
        }

        for (model_key, profile) in &self.models {
            models.insert(model_key.clone(), profile.clone());
        }

        models
    }

    fn synthesized_model_profile(&self, slug: &str) -> ModelProfile {
        let tool_profile = profile_for_model(slug);
        let backend = AgentBackend::from_model(slug);
        let provider = match backend {
            AgentBackend::Claude => ProviderKind::ClaudeCli.label(),
            AgentBackend::Cursor => ProviderKind::CursorAcp.label(),
            AgentBackend::Codex | AgentBackend::OpenAi | AgentBackend::Ollama => {
                ProviderKind::OpenAiCompat.label()
            }
            AgentBackend::Perplexity => ProviderKind::PerplexityApi.label(),
        };

        let context_window = match tool_profile.preferred {
            ToolFormat::AnthropicBlocks => 200_000,
            _ => default_context_window(),
        };

        ModelProfile {
            provider: provider.to_owned(),
            slug: slug.to_owned(),
            context_window,
            max_output: None,
            supports_tools: tool_profile.supports_tools,
            supports_thinking: false,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            provider_routing: None,
            tool_format: tool_profile.preferred.as_str().to_owned(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            max_tools: Some(u32::from(tool_profile.max_tools_before_degrade)),
            tokenizer_ratio: None,
        }
    }

    fn agent_env_value(&self, key: &str) -> Option<&str> {
        self.agent.env.as_ref().and_then(|entries| {
            entries.iter().find_map(|(entry_key, entry_value)| {
                (entry_key == key).then_some(entry_value.as_str())
            })
        })
    }

    /// Apply environment variable overrides.
    ///
    /// Recognized variables:
    /// - `ROKO_MODEL` -- sets `agent.default_model`
    /// - `ROKO_BACKEND` -- sets `agent.default_backend`
    /// - `ROKO_EFFORT` -- sets `agent.default_effort`
    /// - `ROKO_PROVIDER` -- overrides the provider for `agent.default_model`
    /// - `ROKO_MODEL_SLUG` -- overrides the slug sent to the API
    /// - `ROKO_CONTEXT_LIMIT_K` -- sets `agent.context_limit_k`
    /// - `ROKO_MAX_AGENTS` -- sets `conductor.max_agents`
    /// - `ROKO_BUDGET_USD` -- sets `budget.max_plan_usd`
    /// - `ROKO_PARALLEL` -- sets `conductor.parallel_enabled`
    /// - `ROKO_EXPRESS` -- sets `conductor.express_mode`
    /// - `ROKO_SKIP_TESTS` -- sets `gates.skip_tests`
    /// - `ROKO_CLIPPY` -- sets `gates.clippy_enabled`
    pub fn apply_env(&mut self, env_fn: &dyn Fn(&str) -> Option<String>) {
        let provider_override = env_fn("ROKO_PROVIDER");
        let model_slug_override = env_fn("ROKO_MODEL_SLUG");

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

        if provider_override.is_some() || model_slug_override.is_some() {
            let default_model = self.agent.default_model.trim();
            if !default_model.is_empty() {
                let synthesized = self.synthesized_model_profile(default_model);
                let entry = self
                    .models
                    .entry(default_model.to_owned())
                    .or_insert(synthesized);

                if let Some(v) = provider_override {
                    entry.provider = v;
                }
                if let Some(v) = model_slug_override {
                    entry.slug = v;
                }
            }
        }
    }

    /// Convenience: apply overrides from the real process environment.
    pub fn apply_process_env(&mut self) {
        self.apply_env(&|key| std::env::var(key).ok());
    }

    /// Generate an example config string showing every field with doc comments.
    #[must_use]
    pub fn example_toml() -> String {
        let cfg = Self::default();
        let mut out = String::with_capacity(4096);

        Self::write_example_prelude(&mut out);
        Self::write_example_project(&mut out, &cfg);
        Self::write_example_prd(&mut out, &cfg);
        Self::write_example_agent(&mut out, &cfg);
        Self::write_example_gates(&mut out, &cfg);
        Self::write_example_routing(&mut out, &cfg);
        Self::write_example_budget(&mut out, &cfg);
        Self::write_example_conductor(&mut out, &cfg);
        Self::write_example_learning(&mut out, &cfg);
        Self::write_example_tui_and_server(&mut out, &cfg);
        Self::write_example_scheduler(&mut out, &cfg);
        Self::write_example_webhooks(&mut out, &cfg);
        Self::write_example_deploy(&mut out, &cfg);

        out
    }
}

fn parse_bool_env(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
fn run_resolve_api_key_child(test_name: &str, api_key_env: &str, expected: Option<&str>) {
    let exe = std::env::current_exe().expect("current exe");
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(test_name)
        .arg("--exact")
        .arg("--nocapture")
        .env("ROKO_RESOLVE_API_KEY_CHILD", "1")
        .env("ROKO_API_KEY_ENV_NAME", api_key_env);

    if let Some(value) = expected {
        cmd.env(api_key_env, value)
            .env("ROKO_EXPECT_API_KEY", value);
    } else {
        cmd.env_remove(api_key_env);
    }

    let status = cmd.status().expect("spawn child test");
    assert!(status.success(), "child test {test_name} failed");
}

fn deserialize_glob_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    use serde::de::{SeqAccess, Visitor};
    use std::fmt;

    struct GlobListVisitor;

    impl<'de> Visitor<'de> for GlobListVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a string or a list of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(vec![value.to_owned()])
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(vec![value])
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut items = Vec::new();
            while let Some(value) = seq.next_element::<String>()? {
                items.push(value);
            }
            Ok(items)
        }
    }

    deserializer.deserialize_any(GlobListVisitor)
}

// ---- [project] -----------------------------------------------------------

/// Provider registry entry for `[providers.<name>]`.
///
/// A provider describes where requests go and how the runtime talks to that
/// endpoint. Use it to capture auth, transport, and provider-specific limits
/// without hardcoding them into Rust.
///
/// Fields and defaults:
/// - `kind` (required): protocol family, such as `anthropic_api`,
///   `claude_cli`, `openai_compat`, or `cursor_acp`
/// - `base_url`: optional HTTP endpoint for HTTP-based providers
/// - `api_key_env`: optional environment variable name that holds the API key
/// - `command`: optional CLI binary name for subprocess providers
/// - `args`: optional CLI arguments for subprocess providers
/// - `timeout_ms`: optional request or subprocess timeout
/// - `extra_headers`: optional HTTP headers to inject on outbound requests
/// - `max_concurrent`: optional concurrency limit for this provider
///
/// Defaults:
/// - `kind`: no default, must be set explicitly
/// - `base_url`: `None`
/// - `api_key_env`: `None`
/// - `command`: `None`
/// - `args`: `None`
/// - `timeout_ms`: `None`
/// - `extra_headers`: `None`
/// - `max_concurrent`: `None`
///
/// Examples:
/// ```toml
/// [providers.anthropic]
/// kind = "anthropic_api"
/// base_url = "https://api.anthropic.com"
/// api_key_env = "ANTHROPIC_API_KEY"
/// timeout_ms = 120000
///
/// [providers.claude_cli]
/// kind = "claude_cli"
/// command = "claude"
/// args = ["--print", "--output-format", "stream-json"]
/// ```
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Protocol family used to talk to the provider.
    pub kind: ProviderKind,
    /// Base URL for HTTP providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Environment variable name holding the API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Command to spawn for CLI providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments passed to the CLI command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Request timeout in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Extra headers to inject on outbound requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<HashMap<String, String>>,
    /// Maximum concurrent requests allowed for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
}

impl ProviderConfig {
    /// Resolve the API key from the environment variable named in `api_key_env`.
    #[must_use]
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key_env
            .as_ref()
            .and_then(|env_name| std::env::var(env_name).ok())
    }
}

/// Model registry entry for `[models.<name>]`.
///
/// A model binds a logical model name to a provider entry and the concrete
/// API slug that gets sent on the wire. This is the layer that carries model
/// capabilities and cost metadata, while `provider` points to the transport.
///
/// Fields and defaults:
/// - `provider` (required): key into `[providers.*]`
/// - `slug` (required): model ID sent to the provider API
/// - `context_window`: token window size, defaults to `128_000`
/// - `max_output`: optional output-token cap
/// - `supports_tools`: defaults to `true`
/// - `supports_thinking`: defaults to `false`
/// - `supports_vision`: defaults to `false`
/// - `supports_web_search`: defaults to `false`
/// - `supports_mcp_tools`: defaults to `false`
/// - `supports_partial`: defaults to `false`
/// - `tool_format`: tool wire format, defaults to `"openai_json"`
/// - `cost_input_per_m`: optional input-token cost per million tokens
/// - `cost_output_per_m`: optional output-token cost per million tokens
/// - `cost_cache_read_per_m`: optional cache-read cost per million tokens
/// - `cost_cache_write_per_m`: optional cache-write cost per million tokens
/// - `max_tools`: optional cap before tool behavior degrades
/// - `tokenizer_ratio`: optional ratio versus OpenAI `o200k_base`
///
/// Examples:
/// ```toml
/// [models.glm-5-1]
/// provider = "zai"
/// slug = "glm-5.1"
/// context_window = 200000
/// max_output = 131072
/// supports_thinking = true
/// supports_web_search = true
/// tool_format = "openai_json"
/// cost_input_per_m = 1.40
/// cost_output_per_m = 4.40
///
/// [models.claude-opus]
/// provider = "anthropic"
/// slug = "claude-opus-4-6"
/// context_window = 200000
/// supports_tools = true
/// tool_format = "anthropic_blocks"
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderRouting {
    /// OpenRouter sort mode (`price`, `throughput`, `latency`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    /// Explicit provider order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
    /// Whether OpenRouter may fall back to alternate providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fallbacks: Option<bool>,
    /// Maximum cost per token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price: Option<f64>,
    /// Required provider parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_parameters: Option<Vec<String>>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelProfile {
    /// Key into the `[providers.*]` table.
    pub provider: String,
    /// Model ID sent to the API.
    pub slug: String,
    /// Context window in tokens.
    #[serde(default = "default_context_window")]
    pub context_window: u64,
    /// Maximum output tokens, if the provider/model sets one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
    /// Whether the model supports tool calls.
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    /// Whether the model supports thinking/reasoning output.
    #[serde(default)]
    pub supports_thinking: bool,
    /// Whether the model supports vision inputs.
    #[serde(default)]
    pub supports_vision: bool,
    /// Whether the model supports web search.
    #[serde(default)]
    pub supports_web_search: bool,
    /// Whether the model supports MCP tools.
    #[serde(default)]
    pub supports_mcp_tools: bool,
    /// Whether the model supports partial continuation.
    #[serde(default)]
    pub supports_partial: bool,
    /// OpenRouter-specific routing overrides for this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_routing: Option<ProviderRouting>,
    /// Wire format used for tools.
    #[serde(default = "default_tool_format")]
    pub tool_format: String,
    /// Input token cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_input_per_m: Option<f64>,
    /// Output token cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_output_per_m: Option<f64>,
    /// Cache read cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cache_read_per_m: Option<f64>,
    /// Cache write cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cache_write_per_m: Option<f64>,
    /// Maximum number of tools before behavior degrades.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tools: Option<u32>,
    /// Tokenizer ratio vs OpenAI `o200k_base`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_ratio: Option<f64>,
}

/// PRD lifecycle settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrdConfig {
    /// Automatically generate a plan when a PRD is promoted.
    #[serde(default)]
    pub auto_plan: bool,
}

impl Default for PrdConfig {
    fn default() -> Self {
        Self { auto_plan: false }
    }
}

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
    #[serde(default = "default_model", alias = "model")]
    pub default_model: String,
    /// Default backend (e.g. `"claude"`, `"codex"`, `"cursor"`).
    #[serde(default = "default_backend")]
    pub default_backend: String,
    /// Default reasoning effort (`"low"`, `"medium"`, `"high"`, `"max"`).
    #[serde(default = "default_effort", alias = "effort")]
    pub default_effort: String,
    /// Context window limit in thousands of tokens.
    #[serde(default = "default_context_limit_k")]
    pub context_limit_k: u32,
    /// When true, agents use `--bare` (skip built-in system prompt).
    #[serde(default = "default_true")]
    pub bare_mode: bool,
    /// Legacy agent command used when no provider registry is configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Legacy CLI args used for the Claude subprocess path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Legacy subprocess timeout in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Legacy subprocess environment variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<(String, String)>>,
    /// Legacy per-tier model mapping used before `[models.*]` existed.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tier_models: HashMap<String, String>,
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

const fn default_context_window() -> u64 {
    128_000
}

const fn default_true() -> bool {
    true
}

fn default_tool_format() -> String {
    "openai_json".into()
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            default_model: default_model(),
            default_backend: default_backend(),
            default_effort: default_effort(),
            context_limit_k: default_context_limit_k(),
            bare_mode: default_true(),
            command: None,
            args: None,
            timeout_ms: None,
            env: None,
            tier_models: HashMap::new(),
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

// ---- [serve] -------------------------------------------------------------

/// API serving options.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeConfig {
    /// Authentication settings for `/api/*`.
    #[serde(default)]
    pub auth: ServeAuthConfig,
    /// Cloud deployment settings.
    #[serde(default)]
    pub deploy: ServeDeployConfig,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            auth: ServeAuthConfig::default(),
            deploy: ServeDeployConfig::default(),
        }
    }
}

/// Cron scheduler configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// Cron jobs configured at startup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cron: Vec<SchedulerCronConfig>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { cron: Vec::new() }
    }
}

impl SchedulerConfig {
    /// Returns `true` when no cron jobs are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cron.is_empty()
    }
}

/// One cron job configuration entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerCronConfig {
    /// Human-readable schedule name.
    pub name: String,
    /// Standard cron expression.
    pub expression: String,
    /// Signal kind emitted when the schedule fires.
    pub signal_kind: String,
    /// Extra structured metadata included in the emitted signal body.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl Default for SchedulerCronConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            expression: String::new(),
            signal_kind: String::new(),
            metadata: serde_json::Value::Null,
        }
    }
}

/// Webhook ingress configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhooksConfig {
    /// GitHub webhook configuration.
    #[serde(default)]
    pub github: GithubWebhookConfig,
}

impl Default for WebhooksConfig {
    fn default() -> Self {
        Self {
            github: GithubWebhookConfig::default(),
        }
    }
}

/// GitHub webhook configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubWebhookConfig {
    /// Shared secret used to verify `X-Hub-Signature-256`.
    #[serde(default)]
    pub secret: String,
}

impl Default for GithubWebhookConfig {
    fn default() -> Self {
        Self {
            secret: String::new(),
        }
    }
}

/// Subscription configuration loaded from `roko.toml` and `.roko/subscriptions/*.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionConfig {
    /// Agent template name associated with this subscription.
    pub template: String,
    /// Signal kind glob used to match webhook signals.
    pub trigger: String,
    /// Optional repo / branch / path filters.
    #[serde(default, skip_serializing_if = "SubscriptionFilterConfig::is_empty")]
    pub filter: SubscriptionFilterConfig,
    /// Maximum number of concurrent dispatches for this subscription.
    #[serde(default = "default_subscription_concurrency_limit")]
    pub concurrency_limit: usize,
    /// Minimum interval between dispatches, in seconds.
    #[serde(default)]
    pub cooldown_secs: u64,
    /// Whether the subscription is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for SubscriptionConfig {
    fn default() -> Self {
        Self {
            template: String::new(),
            trigger: String::new(),
            filter: SubscriptionFilterConfig::default(),
            concurrency_limit: default_subscription_concurrency_limit(),
            cooldown_secs: 0,
            enabled: default_true(),
        }
    }
}

fn default_subscription_concurrency_limit() -> usize {
    1
}

/// Optional filter applied after the trigger pattern matches.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionFilterConfig {
    /// Repo glob(s) to match against webhook payload repository fields.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty",
        alias = "repos"
    )]
    pub repo: Vec<String>,
    /// Branch glob(s) to match against webhook payload branch/ref fields.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty",
        alias = "branches"
    )]
    pub branch: Vec<String>,
    /// Path glob(s) to match against changed file paths.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty",
        alias = "paths"
    )]
    pub path: Vec<String>,
    /// Label names to match against webhook payload label fields.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty",
        alias = "labels"
    )]
    pub label: Vec<String>,
    /// Author logins to match against webhook payload author fields.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty",
        alias = "authors"
    )]
    pub author: Vec<String>,
}

impl SubscriptionFilterConfig {
    /// Returns `true` when no filter criteria are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.repo.is_empty()
            && self.branch.is_empty()
            && self.path.is_empty()
            && self.label.is_empty()
            && self.author.is_empty()
    }
}

/// File-system watcher configuration.
///
/// Each watch path can narrow the observed file set with include/exclude
/// glob patterns.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatcherConfig {
    /// Watch roots configured by the user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<WatcherPathConfig>,
}

impl WatcherConfig {
    /// Returns `true` when no watch paths are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// One watched directory and its path filters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatcherPathConfig {
    /// Directory to watch recursively.
    pub directory: PathBuf,
    /// Glob patterns that opt paths into emission.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub include: Vec<String>,
    /// Glob patterns that suppress paths even if they match `include`.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub exclude: Vec<String>,
}

impl Default for WatcherPathConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("."),
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
}

impl WatcherPathConfig {
    /// Returns `true` when no include/exclude filters are configured.
    #[must_use]
    pub fn filters_are_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }
}

/// Authentication settings for the HTTP API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeAuthConfig {
    /// Whether `/api/*` routes require an `X-Api-Key` header.
    #[serde(default)]
    pub enabled: bool,
    /// Shared API key expected in `X-Api-Key`.
    #[serde(default)]
    pub api_key: String,
}

impl Default for ServeAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: String::new(),
        }
    }
}

/// Cloud deployment settings attached to the API server configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeDeployConfig {
    /// Deployment provider, e.g. `"railway"` or `"fly"`.
    #[serde(default = "default_serve_deploy_provider")]
    pub provider: String,
    /// Environment variables that must be present for deployment.
    #[serde(default = "default_serve_deploy_environment")]
    pub environment: Vec<String>,
    /// Webhooks that should be registered after deploy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub webhooks: Vec<ServeDeployWebhookConfig>,
}

fn default_serve_deploy_provider() -> String {
    "railway".into()
}

fn default_serve_deploy_environment() -> Vec<String> {
    vec![
        "GITHUB_TOKEN".into(),
        "GITHUB_WEBHOOK_SECRET".into(),
        "SLACK_BOT_TOKEN".into(),
        "SLACK_SIGNING_SECRET".into(),
    ]
}

impl Default for ServeDeployConfig {
    fn default() -> Self {
        Self {
            provider: default_serve_deploy_provider(),
            environment: default_serve_deploy_environment(),
            webhooks: Vec::new(),
        }
    }
}

/// A webhook registration entry to run after deployment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeDeployWebhookConfig {
    /// Webhook provider.
    #[serde(default = "default_serve_deploy_webhook_provider")]
    pub provider: String,
    /// Repository owner.
    #[serde(default)]
    pub owner: String,
    /// Repository name.
    #[serde(default)]
    pub repo: String,
}

fn default_serve_deploy_webhook_provider() -> String {
    "github".into()
}

impl Default for ServeDeployWebhookConfig {
    fn default() -> Self {
        Self {
            provider: default_serve_deploy_webhook_provider(),
            owner: String::new(),
            repo: String::new(),
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
    /// Allowed CORS origins. Empty = permissive.
    #[serde(default)]
    pub cors_origins: Vec<String>,
    /// Optional bearer token for API authentication.
    #[serde(default)]
    pub auth_token: Option<String>,
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
            cors_origins: Vec::new(),
            auth_token: None,
        }
    }
}

// ---- deploy ---------------------------------------------------------------

/// Cloud deployment configuration.
///
/// ```toml
/// [deploy]
/// backend = "railway-api"
/// railway_api_token = "..."
/// project_id = "..."
/// environment_id = "..."
/// worker_image = "ghcr.io/example/roko-worker:latest"
/// default_region = "us-west1"
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeployConfig {
    /// Which deploy backend to use: `"railway-api"`, `"railway-cli"`, `"manual"`.
    #[serde(default = "default_deploy_backend")]
    pub backend: String,

    /// Railway API token (for the `railway-api` backend).
    #[serde(default)]
    pub railway_api_token: Option<String>,

    /// Railway project ID.
    #[serde(default)]
    pub project_id: Option<String>,

    /// Railway environment ID.
    #[serde(default)]
    pub environment_id: Option<String>,

    /// Docker image for worker containers.
    #[serde(default)]
    pub worker_image: Option<String>,

    /// Default region for deployments.
    #[serde(default)]
    pub default_region: Option<String>,
}

fn default_deploy_backend() -> String {
    "manual".into()
}

impl Default for DeployConfig {
    fn default() -> Self {
        Self {
            backend: default_deploy_backend(),
            railway_api_token: None,
            project_id: None,
            environment_id: None,
            worker_image: None,
            default_region: None,
        }
    }
}

// ---- tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_error_contains(err: toml::de::Error, expected: &[&str]) {
        let message = err.to_string();
        for needle in expected {
            assert!(
                message.contains(needle),
                "expected error `{message}` to contain `{needle}`"
            );
        }
    }

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
    fn config_load() {
        let toml = r#"
[agent]
default_model = "claude-sonnet-4-6"
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.agent.default_model, "claude-sonnet-4-6");
        assert!(cfg.providers.is_empty());
        assert!(cfg.models.is_empty());
    }

    #[test]
    fn config_deser_full_config_with_providers_and_models() {
        let toml = r#"
schema_version = 2

[agent]
default_model = "glm-5-1"
fallback_model = "kimi-k2-5"
bare_mode = true

[providers.zai]
kind = "openai_compat"
base_url = "https://api.z.ai/api/paas/v4"
api_key_env = "ZAI_API_KEY"
timeout_ms = 180000
extra_headers = { "HTTP-Referer" = "roko-agent" }

[providers.moonshot]
kind = "openai_compat"
base_url = "https://api.moonshot.ai/v1"
api_key_env = "MOONSHOT_API_KEY"

[models.glm-5-1]
provider = "zai"
slug = "glm-5.1"
context_window = 200000
max_output = 131072
supports_tools = true
supports_thinking = true
supports_web_search = true
supports_mcp_tools = true
tool_format = "openai_json"
cost_input_per_m = 1.40
cost_output_per_m = 4.40

[models.kimi-k2-5]
provider = "moonshot"
slug = "kimi-k2.5"
context_window = 256000
max_output = 65535
supports_tools = true
supports_thinking = true
supports_vision = true
supports_partial = true
tool_format = "openai_json"
cost_input_per_m = 0.60
cost_output_per_m = 3.00
cost_cache_read_per_m = 0.10
"#;
        let cfg = toml::from_str::<RokoConfig>(toml).expect("parse");

        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(cfg.agent.default_model, "glm-5-1");
        assert_eq!(cfg.agent.fallback_model.as_deref(), Some("kimi-k2-5"));
        assert!(cfg.agent.bare_mode);

        let zai = cfg.providers.get("zai").expect("zai provider");
        assert_eq!(zai.kind, ProviderKind::OpenAiCompat);
        assert_eq!(
            zai.base_url.as_deref(),
            Some("https://api.z.ai/api/paas/v4")
        );
        assert_eq!(zai.api_key_env.as_deref(), Some("ZAI_API_KEY"));
        assert_eq!(zai.timeout_ms, Some(180_000));
        assert_eq!(
            zai.extra_headers
                .as_ref()
                .expect("extra headers")
                .get("HTTP-Referer")
                .map(String::as_str),
            Some("roko-agent")
        );

        let moonshot = cfg.providers.get("moonshot").expect("moonshot provider");
        assert_eq!(moonshot.kind, ProviderKind::OpenAiCompat);
        assert_eq!(
            moonshot.base_url.as_deref(),
            Some("https://api.moonshot.ai/v1")
        );
        assert_eq!(moonshot.api_key_env.as_deref(), Some("MOONSHOT_API_KEY"));

        let glm = cfg.models.get("glm-5-1").expect("glm model");
        assert_eq!(glm.provider, "zai");
        assert_eq!(glm.slug, "glm-5.1");
        assert_eq!(glm.context_window, 200_000);
        assert_eq!(glm.max_output, Some(131_072));
        assert!(glm.supports_tools);
        assert!(glm.supports_thinking);
        assert!(glm.supports_web_search);
        assert!(glm.supports_mcp_tools);
        assert_eq!(glm.tool_format, "openai_json");
        assert_eq!(glm.cost_input_per_m, Some(1.40));
        assert_eq!(glm.cost_output_per_m, Some(4.40));

        let kimi = cfg.models.get("kimi-k2-5").expect("kimi model");
        assert_eq!(kimi.provider, "moonshot");
        assert_eq!(kimi.slug, "kimi-k2.5");
        assert_eq!(kimi.context_window, 256_000);
        assert_eq!(kimi.max_output, Some(65_535));
        assert!(kimi.supports_tools);
        assert!(kimi.supports_thinking);
        assert!(kimi.supports_vision);
        assert!(kimi.supports_partial);
        assert_eq!(kimi.tool_format, "openai_json");
        assert_eq!(kimi.cost_input_per_m, Some(0.60));
        assert_eq!(kimi.cost_output_per_m, Some(3.00));
        assert_eq!(kimi.cost_cache_read_per_m, Some(0.10));
    }

    #[test]
    fn config_deser_minimal_config_only_agent_section() {
        let toml = r#"
[agent]
default_model = "claude-sonnet-4-6"
"#;
        let cfg = toml::from_str::<RokoConfig>(toml).expect("parse");

        assert_eq!(cfg.agent.default_model, "claude-sonnet-4-6");
        assert!(cfg.providers.is_empty());
        assert!(cfg.models.is_empty());
    }

    #[test]
    fn config_deser_mixed_config_with_providers_but_no_models() {
        let toml = r#"
[agent]
default_model = "claude-sonnet-4-6"

[providers.claude]
kind = "claude_cli"
command = "claude"
args = ["--print", "--output-format", "stream-json"]
timeout_ms = 120000

[providers.ollama]
kind = "openai_compat"
base_url = "http://localhost:11434"
"#;
        let cfg = toml::from_str::<RokoConfig>(toml).expect("parse");

        assert_eq!(cfg.agent.default_model, "claude-sonnet-4-6");
        assert_eq!(cfg.providers.len(), 2);
        assert!(cfg.models.is_empty());

        let claude = cfg.providers.get("claude").expect("claude provider");
        assert_eq!(claude.kind, ProviderKind::ClaudeCli);
        assert_eq!(claude.command.as_deref(), Some("claude"));
        assert_eq!(
            claude.args.as_ref().expect("claude args"),
            &vec![
                "--print".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
            ]
        );
        assert_eq!(claude.timeout_ms, Some(120_000));

        let ollama = cfg.providers.get("ollama").expect("ollama provider");
        assert_eq!(ollama.kind, ProviderKind::OpenAiCompat);
        assert_eq!(ollama.base_url.as_deref(), Some("http://localhost:11434"));
    }

    #[test]
    fn config_deser_invalid_provider_kind_is_descriptive() {
        let toml = r#"
[providers.bad]
kind = "not_a_real_kind"
"#;
        let err = toml::from_str::<RokoConfig>(toml).expect_err("should fail");
        assert_error_contains(
            err,
            &[
                "kind",
                "unknown variant",
                "not_a_real_kind",
                "anthropic_api",
                "claude_cli",
                "openai_compat",
                "cursor_acp",
            ],
        );
    }

    #[test]
    fn config_deser_missing_required_fields_is_descriptive() {
        let toml = r#"
[providers.zai]
base_url = "https://api.z.ai/api/paas/v4"
"#;
        let err = toml::from_str::<RokoConfig>(toml).expect_err("should fail");
        assert_error_contains(err, &["providers.zai", "kind", "missing field"]);
    }

    #[test]
    fn config_deser_api_key_env_is_parsed_as_string_reference() {
        let toml = r#"
[providers.zai]
kind = "openai_compat"
api_key_env = "ZAI_API_KEY"
"#;
        let cfg = toml::from_str::<RokoConfig>(toml).expect("parse");
        let provider = cfg.providers.get("zai").expect("provider");
        assert_eq!(provider.api_key_env.as_deref(), Some("ZAI_API_KEY"));
    }

    #[test]
    fn effective_providers_backwards_compat() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roko.toml");
        let text = std::fs::read_to_string(path).expect("read roko.toml");
        let cfg = RokoConfig::from_toml(&text).expect("parse roko.toml");
        let providers = cfg.effective_providers();

        let claude = providers.get("claude_cli").expect("claude_cli provider");
        assert_eq!(claude.kind, ProviderKind::ClaudeCli);
        assert_eq!(claude.command.as_deref(), Some("claude"));
        assert_eq!(
            claude.args.as_ref().expect("claude args"),
            &vec![
                "--print".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--dangerously-skip-permissions".to_string(),
            ]
        );
        assert_eq!(claude.timeout_ms, Some(300_000));

        let anthropic = providers.get("anthropic").expect("anthropic provider");
        assert_eq!(anthropic.kind, ProviderKind::AnthropicApi);
        assert_eq!(anthropic.base_url.as_deref(), Some("http://127.0.0.1:4000"));
        assert_eq!(anthropic.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn config_providers() {
        let toml = r#"
[providers.zai]
kind = "openai_compat"
base_url = "https://api.z.ai/api/paas/v4"
api_key_env = "ZAI_API_KEY"

[models.glm-5-1]
provider = "zai"
slug = "glm-5.1"
supports_thinking = true
tool_format = "openai_json"
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");

        let provider = cfg.providers.get("zai").expect("provider");
        assert_eq!(provider.kind, ProviderKind::OpenAiCompat);
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://api.z.ai/api/paas/v4")
        );
        assert_eq!(provider.api_key_env.as_deref(), Some("ZAI_API_KEY"));

        let model = cfg.models.get("glm-5-1").expect("model");
        assert_eq!(model.provider, "zai");
        assert_eq!(model.slug, "glm-5.1");
        assert!(model.supports_thinking);
        assert_eq!(model.tool_format, "openai_json");
    }

    #[test]
    fn effective_models_backwards_compat() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roko.toml");
        let text = std::fs::read_to_string(path).expect("read roko.toml");
        let cfg = RokoConfig::from_toml(&text).expect("parse roko.toml");
        let models = cfg.effective_models();

        let mechanical = models.get("claude-haiku-4-5").expect("mechanical model");
        assert_eq!(mechanical.provider, "claude_cli");
        assert_eq!(mechanical.slug, "claude-haiku-4-5");
        assert!(mechanical.supports_tools);
        assert_eq!(mechanical.tool_format, "anthropic_blocks");

        let default_model = models.get("claude-sonnet-4-6").expect("default model");
        assert_eq!(default_model.provider, "claude_cli");
        assert_eq!(default_model.slug, "claude-sonnet-4-6");
    }

    #[test]
    fn provider_config_parses_and_roundtrips() {
        let toml = r#"
kind = "openai_compat"
base_url = "https://api.example.com"
api_key_env = "EXAMPLE_API_KEY"
command = "claude"
args = ["--print", "--output-format", "stream-json"]
timeout_ms = 120000
extra_headers = { "HTTP-Referer" = "roko-agent" }
max_concurrent = 4
"#;
        let cfg = toml::from_str::<ProviderConfig>(toml).expect("parse");
        assert_eq!(cfg.kind, ProviderKind::OpenAiCompat);
        assert_eq!(cfg.base_url.as_deref(), Some("https://api.example.com"));
        assert_eq!(cfg.api_key_env.as_deref(), Some("EXAMPLE_API_KEY"));
        assert_eq!(cfg.command.as_deref(), Some("claude"));
        assert_eq!(
            cfg.args.as_ref().expect("args"),
            &vec![
                "--print".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string()
            ]
        );
        assert_eq!(cfg.timeout_ms, Some(120000));
        assert_eq!(
            cfg.extra_headers
                .as_ref()
                .expect("extra_headers")
                .get("HTTP-Referer")
                .map(String::as_str),
            Some("roko-agent")
        );
        assert_eq!(cfg.max_concurrent, Some(4));

        let text = toml::to_string(&cfg).expect("serialize");
        let back = toml::from_str::<ProviderConfig>(&text).expect("reparse");
        assert_eq!(cfg, back);
    }

    #[test]
    fn resolve_api_key_returns_env_value() {
        run_resolve_api_key_child(
            "resolve_api_key_child_present",
            "ZAI_API_KEY",
            Some("test123"),
        );
    }

    #[test]
    fn resolve_api_key_returns_none_when_env_missing() {
        run_resolve_api_key_child("resolve_api_key_child_missing", "ZAI_API_KEY", None);
    }

    #[test]
    fn resolve_api_key_child_present() {
        if std::env::var_os("ROKO_RESOLVE_API_KEY_CHILD").is_none() {
            return;
        }

        let api_key_env = std::env::var("ROKO_API_KEY_ENV_NAME").expect("api key env name");
        let expected = std::env::var("ROKO_EXPECT_API_KEY").expect("expected api key");
        let cfg = ProviderConfig {
            kind: ProviderKind::OpenAiCompat,
            base_url: None,
            api_key_env: Some(api_key_env),
            command: None,
            args: None,
            timeout_ms: None,
            extra_headers: None,
            max_concurrent: None,
        };

        assert_eq!(cfg.resolve_api_key().as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn resolve_api_key_child_missing() {
        if std::env::var_os("ROKO_RESOLVE_API_KEY_CHILD").is_none() {
            return;
        }

        let api_key_env = std::env::var("ROKO_API_KEY_ENV_NAME").expect("api key env name");
        let cfg = ProviderConfig {
            kind: ProviderKind::OpenAiCompat,
            base_url: None,
            api_key_env: Some(api_key_env),
            command: None,
            args: None,
            timeout_ms: None,
            extra_headers: None,
            max_concurrent: None,
        };

        assert_eq!(cfg.resolve_api_key(), None);
    }

    #[test]
    fn model_profile_parses_with_defaults() {
        let toml = r#"
provider = "anthropic"
slug = "claude-opus-4-6"
"#;
        let cfg = toml::from_str::<ModelProfile>(toml).expect("parse");
        assert_eq!(cfg.provider, "anthropic");
        assert_eq!(cfg.slug, "claude-opus-4-6");
        assert_eq!(cfg.context_window, 128_000);
        assert_eq!(cfg.max_output, None);
        assert!(cfg.supports_tools);
        assert!(!cfg.supports_thinking);
        assert!(!cfg.supports_vision);
        assert!(!cfg.supports_web_search);
        assert!(!cfg.supports_mcp_tools);
        assert!(!cfg.supports_partial);
        assert_eq!(cfg.tool_format, "openai_json");
        assert_eq!(cfg.cost_input_per_m, None);
        assert_eq!(cfg.cost_output_per_m, None);
        assert_eq!(cfg.cost_cache_read_per_m, None);
        assert_eq!(cfg.cost_cache_write_per_m, None);
        assert_eq!(cfg.max_tools, None);
        assert_eq!(cfg.tokenizer_ratio, None);
        assert_eq!(cfg.provider_routing, None);

        let text = toml::to_string(&cfg).expect("serialize");
        let back = toml::from_str::<ModelProfile>(&text).expect("reparse");
        assert_eq!(cfg, back);
    }

    #[test]
    fn provider_routing_serializes_to_expected_json() {
        let routing = ProviderRouting {
            sort: Some("price".to_string()),
            order: Some(vec!["anthropic".to_string(), "openai".to_string()]),
            allow_fallbacks: Some(true),
            max_price: Some(0.42),
            require_parameters: Some(vec!["thinking".to_string()]),
        };

        let json = serde_json::to_value(&routing).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({
                "sort": "price",
                "order": ["anthropic", "openai"],
                "allow_fallbacks": true,
                "max_price": 0.42,
                "require_parameters": ["thinking"]
            })
        );

        let sparse = ProviderRouting {
            sort: Some("latency".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_value(&sparse).expect("serialize sparse");
        assert_eq!(json, serde_json::json!({ "sort": "latency" }));
    }

    #[test]
    fn watcher_section_parses_include_and_exclude_globs() {
        let toml = r#"
[watcher]
[[watcher.paths]]
directory = "call-notes"
include = ["*.md", "*.txt"]
exclude = [".git/**", "**/*.swp"]
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.watcher.paths.len(), 1);
        let path = &cfg.watcher.paths[0];
        assert_eq!(path.directory, std::path::PathBuf::from("call-notes"));
        assert_eq!(path.include, vec!["*.md".to_string(), "*.txt".to_string()]);
        assert_eq!(
            path.exclude,
            vec![".git/**".to_string(), "**/*.swp".to_string()]
        );
    }

    #[test]
    fn scheduler_section_parses_cron_jobs() {
        let toml = r#"
[scheduler]
[[scheduler.cron]]
name = "weekly-digest"
expression = "0 9 * * MON"
signal_kind = "scheduler:cron:weekly-digest"
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.scheduler.cron.len(), 1);
        let cron = &cfg.scheduler.cron[0];
        assert_eq!(cron.name, "weekly-digest");
        assert_eq!(cron.expression, "0 9 * * MON");
        assert_eq!(cron.signal_kind, "scheduler:cron:weekly-digest");
    }

    #[test]
    fn prd_section_parses() {
        let toml = r#"
[prd]
auto_plan = true
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert!(cfg.prd.auto_plan);
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
    fn serve_auth_section_parses() {
        let toml = r#"
[serve.auth]
enabled = true
api_key = "secret"
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert!(cfg.serve.auth.enabled);
        assert_eq!(cfg.serve.auth.api_key, "secret");
    }

    #[test]
    fn serve_deploy_section_parses() {
        let toml = r#"
[serve.deploy]
provider = "fly"
environment = ["GITHUB_TOKEN", "SLACK_BOT_TOKEN"]

[[serve.deploy.webhooks]]
provider = "github"
owner = "nunchi"
repo = "roko"
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
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
    fn env_override_provider() {
        let mut cfg = RokoConfig::default();
        let env = |key: &str| -> Option<String> {
            match key {
                "ROKO_PROVIDER" => Some("openrouter".into()),
                "ROKO_MODEL_SLUG" => Some("z-ai/glm-5.1".into()),
                _ => None,
            }
        };
        cfg.apply_env(&env);

        let models = cfg.effective_models();
        let default_model = models.get(&cfg.agent.default_model).expect("default model");
        assert_eq!(default_model.provider, "openrouter");
        assert_eq!(default_model.slug, "z-ai/glm-5.1");
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

[serve.auth]
enabled = true
api_key = "secret"

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
        assert!(example.contains("[prd]"));
        assert!(example.contains("[agent]"));
        assert!(example.contains("[gates]"));
        assert!(example.contains("[routing]"));
        assert!(example.contains("[budget]"));
        assert!(example.contains("[conductor]"));
        assert!(example.contains("[learning]"));
        assert!(example.contains("[tui]"));
        assert!(example.contains("[serve.auth]"));
        assert!(example.contains("[serve.deploy]"));
        assert!(example.contains("[[serve.deploy.webhooks]]"));
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
    fn kimi_config_parse() {
        let example = include_str!("../../../../examples/roko-kimi.toml");
        let cfg = RokoConfig::from_toml(example).expect("roko-kimi example should parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);

        let model = cfg.models.get("kimi-k2-5").expect("kimi model profile");
        assert_eq!(model.provider, "moonshot");
        assert_eq!(model.slug, "kimi-k2.5");
        assert_eq!(model.max_tools, Some(128));
    }

    #[test]
    fn openrouter_config() {
        let example = include_str!("../../../../examples/roko-openrouter.toml");
        let cfg = RokoConfig::from_toml(example).expect("roko-openrouter example should parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);

        let provider = cfg
            .providers
            .get("openrouter")
            .expect("openrouter provider");
        assert_eq!(provider.kind, ProviderKind::OpenAiCompat);
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(provider.api_key_env.as_deref(), Some("OPENROUTER_API_KEY"));
        assert_eq!(
            provider
                .extra_headers
                .as_ref()
                .expect("extra headers")
                .get("HTTP-Referer")
                .map(String::as_str),
            Some("https://github.com/nunchi/roko")
        );

        for model_key in ["glm-5-1-or", "kimi-k2-5-or", "claude-opus-or"] {
            let model = cfg.models.get(model_key).expect("openrouter model");
            assert_eq!(model.provider, "openrouter");
        }
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
}
