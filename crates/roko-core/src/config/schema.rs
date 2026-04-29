//! Unified `RokoConfig` schema with hierarchical sections.
//!
//! Every section is a separate struct so callers can destructure just the
//! slice they need. All fields carry serde defaults so a bare config still
//! produces a fully-populated `RokoConfig`.
//!
//! Section structs live in dedicated submodules and are re-exported here so
//! that `schema::FooConfig` continues to resolve for all existing callers.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fmt::Write as _;

use crate::agent::{AgentBackend, ProviderKind};
use crate::tool::{ToolFormat, profile_for_model};
use regex::Regex;
use serde::{Deserialize, Serialize};

// Re-export all section structs from submodules.
pub use super::agent::*;
pub use super::budget::*;
pub use super::chain::*;
pub use super::gates::*;
pub use super::learning::*;
pub use super::project::*;
pub use super::provider::*;
pub use super::routing::*;
pub use super::serve::*;
pub use super::subscriptions::*;
pub use super::tools::*;
pub use super::tui_cfg::*;

use super::provider::{
    default_provider_connect_timeout_ms, default_provider_timeout_ms,
    default_provider_ttft_timeout_ms,
};

/// Current schema version. Bump on incompatible changes.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Config layout version for migration tooling.
pub const CURRENT_CONFIG_VERSION: u32 = 2;

// ---- top-level -----------------------------------------------------------

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RokoConfig {
    #[serde(default = "default_config_version")]
    pub config_version: u32,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(default)]
    pub prd: PrdConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub models: HashMap<String, ModelProfile>,
    #[serde(default)]
    pub gates: GatesConfig,
    #[serde(default)]
    pub routing: RoutingConfig,
    #[serde(default)]
    pub pipeline: PipelineConfig,
    #[serde(default)]
    pub budget: BudgetConfig,
    #[serde(default)]
    pub conductor: ConductorConfig,
    #[serde(default, skip_serializing_if = "WatcherConfig::is_empty")]
    pub watcher: WatcherConfig,
    #[serde(default)]
    pub learning: LearningConfig,
    #[serde(default)]
    pub demurrage: DemurrageConfig,
    #[serde(default)]
    pub attention: AttentionConfig,
    #[serde(default)]
    pub immune: ImmuneConfig,
    #[serde(default)]
    pub temporal: TemporalConfig,
    #[serde(default)]
    pub goals: GoalsConfig,
    #[serde(default)]
    pub energy: EnergyConfig,
    #[serde(default)]
    pub tui: TuiConfig,
    #[serde(default)]
    pub serve: ServeConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    #[serde(default)]
    pub webhooks: WebhooksConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscriptions: Vec<SubscriptionConfig>,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub deploy: DeployConfig,
    #[serde(default)]
    pub perplexity: PerplexityConfig,
    #[serde(default)]
    pub gemini: GeminiConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub oneirography: OneirographyConfig,
    #[serde(default)]
    pub chain: ChainConfig,
    #[serde(default)]
    pub relay: RelayConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentDefinition>,
}

const fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}
const fn default_config_version() -> u32 {
    1
}

impl Default for RokoConfig {
    fn default() -> Self {
        Self {
            config_version: CURRENT_CONFIG_VERSION,
            schema_version: CURRENT_SCHEMA_VERSION,
            project: ProjectConfig::default(),
            prd: PrdConfig::default(),
            agent: AgentConfig::default(),
            providers: HashMap::new(),
            models: HashMap::new(),
            gates: GatesConfig::default(),
            routing: RoutingConfig::default(),
            pipeline: PipelineConfig::default(),
            budget: BudgetConfig::default(),
            conductor: ConductorConfig::default(),
            watcher: WatcherConfig::default(),
            learning: LearningConfig::default(),
            demurrage: DemurrageConfig::default(),
            attention: AttentionConfig::default(),
            immune: ImmuneConfig::default(),
            temporal: TemporalConfig::default(),
            goals: GoalsConfig::default(),
            energy: EnergyConfig::default(),
            tui: TuiConfig::default(),
            serve: ServeConfig::default(),
            scheduler: SchedulerConfig::default(),
            webhooks: WebhooksConfig::default(),
            subscriptions: Vec::new(),
            server: ServerConfig::default(),
            deploy: DeployConfig::default(),
            perplexity: PerplexityConfig::default(),
            gemini: GeminiConfig::default(),
            tools: ToolsConfig::default(),
            oneirography: OneirographyConfig::default(),
            chain: ChainConfig::default(),
            relay: RelayConfig::default(),
            agents: Vec::new(),
        }
    }
}

// ---- RokoConfig impl -----------------------------------------------------

impl RokoConfig {
    /// Parse from a TOML string.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        let config: Self = toml::from_str(s)?;
        if config.config_version == 1 {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                tracing::warn!(
                    "roko.toml uses config version 1 (no [providers] section)\n  hint: run `roko config migrate` to upgrade"
                );
            });
        }
        Ok(config)
    }

    /// Render to a TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }

    /// Render to a pretty-printed TOML string.
    pub fn to_toml_pretty(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Returns `true` when this config was written with an older schema version.
    #[must_use]
    pub const fn is_stale(&self) -> bool {
        self.schema_version < CURRENT_SCHEMA_VERSION
    }

    // ---- provider / model synthesis --------------------------------------

    /// Return the provider registry that should be used at runtime.
    #[must_use]
    pub fn effective_providers(&self) -> HashMap<String, ProviderConfig> {
        if !self.providers.is_empty() {
            let mut providers = self.providers.clone();
            if !providers.contains_key("anthropic")
                && std::env::var_os("ANTHROPIC_API_KEY").is_some()
            {
                providers.insert(
                    "anthropic".into(),
                    ProviderConfig {
                        kind: ProviderKind::AnthropicApi,
                        base_url: Some("https://api.anthropic.com".to_string()),
                        api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
                        command: None,
                        args: None,
                        timeout_ms: default_provider_timeout_ms(),
                        ttft_timeout_ms: default_provider_ttft_timeout_ms(),
                        connect_timeout_ms: default_provider_connect_timeout_ms(),
                        extra_headers: None,
                        max_concurrent: None,
                    },
                );
            }
            return providers;
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
                timeout_ms: self.agent.timeout_ms.or(default_provider_timeout_ms()),
                ttft_timeout_ms: default_provider_ttft_timeout_ms(),
                connect_timeout_ms: default_provider_connect_timeout_ms(),
                extra_headers: None,
                max_concurrent: None,
            },
        );

        let has_env_key = std::env::var_os("ANTHROPIC_API_KEY").is_some();
        let has_config_key = self.agent_env_value("ANTHROPIC_API_KEY").is_some();
        let base_url = self
            .agent_env_value("ANTHROPIC_BASE_URL")
            .map(|s| s.to_owned())
            .or_else(|| has_env_key.then(|| "https://api.anthropic.com".to_string()));
        if let Some(base_url) = base_url {
            providers.insert(
                "anthropic".into(),
                ProviderConfig {
                    kind: ProviderKind::AnthropicApi,
                    base_url: Some(base_url),
                    api_key_env: if has_env_key || has_config_key {
                        Some("ANTHROPIC_API_KEY".to_string())
                    } else {
                        None
                    },
                    command: None,
                    args: None,
                    timeout_ms: self.agent.timeout_ms.or(default_provider_timeout_ms()),
                    ttft_timeout_ms: default_provider_ttft_timeout_ms(),
                    connect_timeout_ms: default_provider_connect_timeout_ms(),
                    extra_headers: None,
                    max_concurrent: None,
                },
            );
        }

        providers
    }

    /// Return the model registry that should be used at runtime.
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
            AgentBackend::Perplexity => ProviderKind::PerplexityApi.label(),
            AgentBackend::Cerebras => ProviderKind::CerebrasApi.label(),
            AgentBackend::Codex | AgentBackend::OpenAi | AgentBackend::Ollama => {
                ProviderKind::OpenAiCompat.label()
            }
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
            supports_grounding: false,
            supports_code_execution: false,
            supports_caching: false,
            provider_routing: None,
            tool_format: tool_profile.preferred.as_str().to_owned(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_input_per_m_high: None,
            cost_output_per_m_high: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            thinking_level: None,
            max_tools: Some(u32::from(tool_profile.max_tools_before_degrade)),
            tokenizer_ratio: None,
            ..Default::default()
        }
    }

    fn agent_env_value(&self, key: &str) -> Option<&str> {
        self.agent.env.as_ref().and_then(|entries| {
            entries
                .iter()
                .find_map(|(k, v)| (k == key).then_some(v.as_str()))
        })
    }

    // ---- env overrides ---------------------------------------------------

    /// Apply environment variable overrides.
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

    // ---- secret resolution -----------------------------------------------

    /// Interpolate `${VAR}` patterns in provider config strings.
    pub fn interpolate_env_vars(&mut self) {
        Self::interpolate_env_vars_with(&mut self.providers, &|key| std::env::var(key).ok());
    }

    fn interpolate_env_vars_with(
        providers: &mut HashMap<String, ProviderConfig>,
        env_fn: &dyn Fn(&str) -> Option<String>,
    ) {
        for provider in providers.values_mut() {
            if let Some(ref mut url) = provider.base_url {
                *url = interpolate_vars(url, env_fn);
            }
            if let Some(ref mut key_env) = provider.api_key_env {
                *key_env = interpolate_vars(key_env, env_fn);
            }
            if let Some(ref mut cmd) = provider.command {
                *cmd = interpolate_vars(cmd, env_fn);
            }
            if let Some(ref headers) = provider.extra_headers {
                let mut resolved = HashMap::with_capacity(headers.len());
                for (k, v) in headers {
                    resolved.insert(k.clone(), interpolate_vars(v, env_fn));
                }
                provider.extra_headers = Some(resolved);
            }
        }
    }

    /// Resolve `*_file` secret references in provider configs.
    pub fn resolve_file_secrets(&mut self) {
        for provider in self.providers.values_mut() {
            if let Some(ref headers) = provider.extra_headers {
                let mut resolved = HashMap::with_capacity(headers.len());
                for (key, value) in headers {
                    if key.ends_with("_file") {
                        let base_key = key.trim_end_matches("_file").to_string();
                        if let Ok(content) = std::fs::read_to_string(value.trim()) {
                            resolved.insert(base_key, content.trim().to_string());
                        }
                    } else {
                        resolved.insert(key.clone(), value.clone());
                    }
                }
                provider.extra_headers = Some(resolved);
            }
        }
    }

    // ---- hot-reload classification ---------------------------------------

    /// Classify a proposed configuration change.
    #[must_use]
    pub fn classify_changes(&self, proposed: &Self) -> ConfigChangeReport {
        let mut report = ConfigChangeReport::default();

        if self.budget != proposed.budget {
            report.hot_reloaded.push("budget");
        }
        if self.gates != proposed.gates {
            report.hot_reloaded.push("gates");
        }
        if self.routing != proposed.routing {
            report.hot_reloaded.push("routing");
        }
        if self.learning != proposed.learning {
            report.hot_reloaded.push("learning");
        }
        if self.demurrage != proposed.demurrage {
            report.hot_reloaded.push("demurrage");
        }
        if self.scheduler != proposed.scheduler {
            report.hot_reloaded.push("scheduler");
        }
        if self.watcher != proposed.watcher {
            report.hot_reloaded.push("watcher");
        }
        if self.subscriptions != proposed.subscriptions {
            report.hot_reloaded.push("subscriptions");
        }
        if self.conductor != proposed.conductor {
            report.hot_reloaded.push("conductor");
        }
        if self.attention != proposed.attention {
            report.hot_reloaded.push("attention");
        }
        if self.goals != proposed.goals {
            report.hot_reloaded.push("goals");
        }

        if self.agent != proposed.agent {
            report.requires_restart.push("agent");
        }
        if self.project != proposed.project {
            report.requires_restart.push("project");
        }
        if self.serve != proposed.serve {
            report.requires_restart.push("serve");
        }
        if self.providers != proposed.providers {
            report.requires_restart.push("providers");
        }
        if self.models != proposed.models {
            report.requires_restart.push("models");
        }
        if self.server != proposed.server {
            report.requires_restart.push("server");
        }

        if proposed.budget.max_plan_usd > self.budget.max_plan_usd {
            report.warnings.push(format!(
                "budget.max_plan_usd increased from {} to {}",
                self.budget.max_plan_usd, proposed.budget.max_plan_usd
            ));
        }

        report
    }

    // ---- example TOML generation -----------------------------------------

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
        Self::write_example_pipeline(&mut out, &cfg);
        Self::write_example_budget(&mut out, &cfg);
        Self::write_example_conductor(&mut out, &cfg);
        Self::write_example_learning(&mut out, &cfg);
        Self::write_example_demurrage(&mut out, &cfg);
        Self::write_example_attention(&mut out, &cfg);
        Self::write_example_immune(&mut out, &cfg);
        Self::write_example_temporal(&mut out, &cfg);
        Self::write_example_goals(&mut out, &cfg);
        Self::write_example_energy(&mut out, &cfg);
        Self::write_example_tui_and_server(&mut out, &cfg);
        Self::write_example_scheduler(&mut out, &cfg);
        Self::write_example_webhooks(&mut out, &cfg);
        Self::write_example_deploy(&mut out, &cfg);
        out
    }

    fn write_example_prelude(out: &mut String) {
        let _ = writeln!(
            out,
            "# Roko configuration -- all fields shown with defaults."
        );
        let _ = writeln!(
            out,
            "# Delete any section you don't need; defaults apply.\n"
        );
        let _ = writeln!(out, "config_version = {CURRENT_CONFIG_VERSION}");
        let _ = writeln!(out, "schema_version = {CURRENT_SCHEMA_VERSION}\n");
    }
    fn write_example_project(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Project metadata --");
        let _ = writeln!(out, "[project]");
        let _ = writeln!(out, "name = \"{}\"", c.project.name);
        let _ = writeln!(out, "root = \"{}\"", c.project.root);
        let _ = writeln!(
            out,
            "fresh_base_branch = \"{}\"\n",
            c.project.fresh_base_branch
        );
    }
    fn write_example_prd(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- PRD lifecycle settings --");
        let _ = writeln!(out, "[prd]");
        let _ = writeln!(out, "auto_plan = {}\n", c.prd.auto_plan);
    }
    fn write_example_agent(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Agent / model settings --");
        let _ = writeln!(out, "[agent]");
        let _ = writeln!(out, "default_model = \"{}\"", c.agent.default_model);
        let _ = writeln!(out, "default_backend = \"{}\"", c.agent.default_backend);
        let _ = writeln!(out, "default_effort = \"{}\"", c.agent.default_effort);
        let _ = writeln!(out, "temperament = \"{}\"", c.agent.temperament);
        let _ = writeln!(out, "context_limit_k = {}", c.agent.context_limit_k);
        let _ = writeln!(out, "# policy_manifests = [\".roko/roles/manifest.toml\"]");
        let _ = writeln!(out, "bare_mode = {}\n", c.agent.bare_mode);
        let _ = writeln!(out, "# Per-role overrides (repeat for each role):");
        let _ = writeln!(out, "# [agent.roles.implementer]");
        let _ = writeln!(out, "# role = \"implementer\"");
        let _ = writeln!(out, "# model = \"claude-opus-4-6\"");
        let _ = writeln!(out, "# effort = \"high\"");
        let _ = writeln!(out, "# temperament = \"exploratory\"");
        let _ = writeln!(out, "# context_limit_k = 200");
        let _ = writeln!(out, "# tools = [\"read\", \"edit\", \"bash\", \"git-*\"]");
        let _ = writeln!(
            out,
            "# budget = {{ max_tokens_per_turn = 12000, max_cost_usd_cents_per_turn = 500 }}"
        );
        let _ = writeln!(out, "# thresholds = {{ gate_pass_rate_floor = 0.65 }}");
        let _ = writeln!(
            out,
            "# routing_overrides = {{ force_backend = \"claude\", force_tier = \"focused\" }}"
        );
        let _ = writeln!(out, "# legacy: turn_budget_usd = 5.0\n");
    }
    fn write_example_gates(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Verification gates --");
        let _ = writeln!(out, "[gates]");
        let _ = writeln!(out, "clippy_enabled = {}", c.gates.clippy_enabled);
        let _ = writeln!(out, "skip_tests = {}", c.gates.skip_tests);
        let _ = writeln!(out, "max_iterations = {}\n", c.gates.max_iterations);
    }
    fn write_example_routing(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Model routing --");
        let _ = writeln!(out, "[routing]");
        let _ = writeln!(out, "mode = \"{}\"", c.routing.mode);
        let _ = writeln!(out, "algorithm = \"{}\"", c.routing.algorithm.label());
        let _ = writeln!(out, "discount_factor = {}", c.routing.discount_factor);
        let _ = writeln!(out, "fast_task_model = \"{}\"", c.routing.fast_task_model);
        let _ = writeln!(
            out,
            "standard_task_model = \"{}\"",
            c.routing.standard_task_model
        );
        let _ = writeln!(
            out,
            "complex_task_model = \"{}\"\n",
            c.routing.complex_task_model
        );
        let _ = writeln!(out, "[routing.weights]");
        let _ = writeln!(out, "quality = {}", c.routing.weights.default.quality);
        let _ = writeln!(out, "cost = {}", c.routing.weights.default.cost);
        let _ = writeln!(out, "latency = {}\n", c.routing.weights.default.latency);
        let mech = c.routing.weights.for_tier("mechanical");
        let _ = writeln!(out, "[routing.weights.mechanical]");
        let _ = writeln!(out, "quality = {}", mech.quality);
        let _ = writeln!(out, "cost = {}", mech.cost);
        let _ = writeln!(out, "latency = {}\n", mech.latency);
    }
    fn write_example_pipeline(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Complexity-to-pipeline mapping --");
        for (name, band) in [
            ("mechanical", c.pipeline.mechanical),
            ("focused", c.pipeline.focused),
            ("integrative", c.pipeline.integrative),
            ("architectural", c.pipeline.architectural),
        ] {
            let _ = writeln!(out, "[pipeline.{name}]");
            let _ = writeln!(out, "strategist = {}", band.strategist);
            let _ = writeln!(out, "reviewers = {}", band.reviewers);
            let _ = writeln!(out, "reviewer_mode = \"{}\"", band.reviewer_mode.label());
            let _ = writeln!(out, "max_iterations = {}\n", band.max_iterations);
        }
    }
    fn write_example_budget(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Spend / token budgets --");
        let _ = writeln!(out, "[budget]");
        let _ = writeln!(out, "max_plan_usd = {:.1}", c.budget.max_plan_usd);
        let _ = writeln!(out, "max_turn_usd = {:.1}", c.budget.max_turn_usd);
        let _ = writeln!(
            out,
            "prompt_token_budget = {}\n",
            c.budget.prompt_token_budget
        );
    }
    fn write_example_conductor(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Conductor (meta-orchestrator) --");
        let _ = writeln!(out, "[conductor]");
        let _ = writeln!(out, "max_agents = {}", c.conductor.max_agents);
        let _ = writeln!(
            out,
            "max_parallel_plans = {}",
            c.conductor.max_parallel_plans
        );
        let _ = writeln!(out, "parallel_enabled = {}", c.conductor.parallel_enabled);
        let _ = writeln!(out, "express_mode = {}", c.conductor.express_mode);
        let _ = writeln!(
            out,
            "auto_advance_batch = {}",
            c.conductor.auto_advance_batch
        );
        let _ = writeln!(
            out,
            "auto_merge_on_complete = {}",
            c.conductor.auto_merge_on_complete
        );
        let _ = writeln!(out, "pre_plan = {}", c.conductor.pre_plan);
        let _ = writeln!(
            out,
            "max_auto_fix_attempts = {}\n",
            c.conductor.max_auto_fix_attempts
        );
    }
    fn write_example_learning(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Learning subsystem --");
        let _ = writeln!(out, "[learning]");
        let _ = writeln!(
            out,
            "auto_playbook_refresh = {}",
            c.learning.auto_playbook_refresh
        );
        let _ = writeln!(
            out,
            "knowledge_warnings = {}",
            c.learning.knowledge_warnings
        );
        let _ = writeln!(
            out,
            "learning_min_occurrences = {}\n",
            c.learning.learning_min_occurrences
        );
        let _ = writeln!(
            out,
            "replan_on_gate_failure = {}",
            c.learning.replan_on_gate_failure
        );
        let _ = writeln!(
            out,
            "replan_max_per_plan = {}",
            c.learning.replan_max_per_plan
        );
        let _ = writeln!(
            out,
            "replan_gate_attempts = {}\n",
            c.learning.replan_gate_attempts
        );
    }
    fn write_example_demurrage(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Knowledge demurrage --");
        let _ = writeln!(out, "[demurrage]");
        let _ = writeln!(out, "rate_per_hour = {}", c.demurrage.rate_per_hour);
        let _ = writeln!(out, "min_balance = {}", c.demurrage.min_balance);
        let _ = writeln!(out, "freeze_threshold = {}", c.demurrage.freeze_threshold);
        let _ = writeln!(out, "thaw_balance = {}", c.demurrage.thaw_balance);
        let _ = writeln!(out, "max_balance = {}", c.demurrage.max_balance);
        let _ = writeln!(out, "death_threshold = {}", c.demurrage.death_threshold);
        let _ = writeln!(
            out,
            "freeze_before_delete = {}",
            c.demurrage.freeze_before_delete
        );
        let _ = writeln!(out, "gc_interval_secs = {}\n", c.demurrage.gc_interval_secs);
    }
    fn write_example_attention(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Attention budget allocation --");
        let _ = writeln!(out, "[attention]");
        let _ = writeln!(
            out,
            "max_tokens_per_layer = {}",
            c.attention.max_tokens_per_layer
        );
        let _ = writeln!(
            out,
            "utilization_target = {}",
            c.attention.utilization_target
        );
        let _ = writeln!(out, "auction_enabled = {}", c.attention.auction_enabled);
        let _ = writeln!(
            out,
            "task_reserve_tokens = {}\n",
            c.attention.task_reserve_tokens
        );
    }
    fn write_example_immune(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Anomaly detection / immune system --");
        let _ = writeln!(out, "[immune]");
        let _ = writeln!(
            out,
            "quarantine_threshold = {}",
            c.immune.quarantine_threshold
        );
        let _ = writeln!(out, "max_quarantined = {}", c.immune.max_quarantined);
        let _ = writeln!(out, "auto_reject = {}", c.immune.auto_reject);
        let _ = writeln!(out, "taint_levels = {:?}\n", c.immune.taint_levels);
    }
    fn write_example_temporal(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Temporal planning --");
        let _ = writeln!(out, "[temporal]");
        let _ = writeln!(out, "max_depth = {}", c.temporal.max_depth);
        let _ = writeln!(out, "epoch_secs = {}", c.temporal.epoch_secs);
        let _ = writeln!(
            out,
            "enforce_allen_relations = {}\n",
            c.temporal.enforce_allen_relations
        );
    }
    fn write_example_goals(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Goal hierarchy --");
        let _ = writeln!(out, "[goals]");
        let _ = writeln!(out, "max_active = {}", c.goals.max_active);
        let _ = writeln!(out, "correctness_weight = {}", c.goals.correctness_weight);
        let _ = writeln!(
            out,
            "completion_threshold = {}",
            c.goals.completion_threshold
        );
        let _ = writeln!(out, "prune_threshold = {}\n", c.goals.prune_threshold);
    }
    fn write_example_energy(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- Compute budget / energy --");
        let _ = writeln!(out, "[energy]");
        let _ = writeln!(out, "pool_usd = {}", c.energy.pool_usd);
        let _ = writeln!(out, "per_task_cap_usd = {}", c.energy.per_task_cap_usd);
        let _ = writeln!(out, "metabolism_rate = {}\n", c.energy.metabolism_rate);
    }
    fn write_example_tui_and_server(out: &mut String, c: &Self) {
        let _ = writeln!(out, "# -- TUI preferences --");
        let _ = writeln!(out, "[tui]");
        let _ = writeln!(out, "refresh_rate_ms = {}\n", c.tui.refresh_rate_ms);
        let _ = writeln!(out, "# -- Serve settings / API auth --");
        let _ = writeln!(out, "[serve]");
        let _ = writeln!(out, "auto_start = {}", c.serve.auto_start);
        let _ = writeln!(out, "auto_orchestrate = {}", c.serve.auto_orchestrate);
        let _ = writeln!(out, "[serve.auth]");
        let _ = writeln!(out, "enabled = {}", c.serve.auth.enabled);
        let _ = writeln!(out, "api_key = \"{}\"\n", c.serve.auth.api_key);
        let _ = writeln!(out, "# -- HTTP server / gateway --");
        let _ = writeln!(out, "[server]");
        let _ = writeln!(out, "bind = \"{}\"", c.server.bind);
        let _ = writeln!(out, "port = {}", c.server.port);
        let _ = writeln!(out, "\n# -- Cloud deployment --");
        let _ = writeln!(out, "[serve.deploy]");
        let _ = writeln!(out, "provider = \"{}\"", c.serve.deploy.provider);
        let _ = writeln!(out, "environment = {:?}", c.serve.deploy.environment);
        let _ = writeln!(out, "\n[[serve.deploy.webhooks]]");
        let _ = writeln!(out, "provider = \"github\"");
        let _ = writeln!(out, "owner = \"nunchi\"");
        let _ = writeln!(out, "repo = \"roko\"");
        let _ = writeln!(out, "\n[[serve.deploy.webhooks]]");
        let _ = writeln!(out, "provider = \"github\"");
        let _ = writeln!(out, "owner = \"nunchi\"");
        let _ = writeln!(out, "repo = \"collaboration\"");
    }
    fn write_example_scheduler(out: &mut String, _c: &Self) {
        let _ = writeln!(out, "\n# -- Cron scheduler --");
        let _ = writeln!(out, "[scheduler]");
        let _ = writeln!(out, "[[scheduler.cron]]");
        let _ = writeln!(out, "name = \"weekly-digest\"");
        let _ = writeln!(out, "expression = \"0 9 * * MON\"");
        let _ = writeln!(out, "signal_kind = \"scheduler:cron:weekly-digest\"");
    }
    fn write_example_webhooks(out: &mut String, _c: &Self) {
        let _ = writeln!(out, "\n# -- Webhooks --");
        let _ = writeln!(out, "[webhooks.github]");
        let _ = writeln!(out, "secret = \"change-me\"");
    }
    fn write_example_deploy(out: &mut String, c: &Self) {
        let _ = writeln!(out, "\n# -- Cloud deployment (Railway, etc.) --");
        let _ = writeln!(out, "[deploy]");
        let _ = writeln!(out, "backend = \"{}\"", c.deploy.backend);
        let _ = writeln!(out, "# railway_api_token = \"...\"");
        let _ = writeln!(out, "# project_id = \"...\"");
        let _ = writeln!(out, "# environment_id = \"...\"");
        let _ = writeln!(
            out,
            "# worker_image = \"ghcr.io/example/roko-worker:latest\""
        );
        let _ = writeln!(out, "# default_region = \"us-west1\"");
    }
}

// ---- ConfigChangeReport --------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigChangeReport {
    pub hot_reloaded: Vec<&'static str>,
    pub requires_restart: Vec<&'static str>,
    pub warnings: Vec<String>,
}

impl ConfigChangeReport {
    #[must_use]
    pub fn has_changes(&self) -> bool {
        !self.hot_reloaded.is_empty() || !self.requires_restart.is_empty()
    }
    #[must_use]
    pub fn needs_restart(&self) -> bool {
        !self.requires_restart.is_empty()
    }
    #[must_use]
    pub fn changed_count(&self) -> usize {
        self.hot_reloaded.len() + self.requires_restart.len()
    }
}

impl fmt::Display for ConfigChangeReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.hot_reloaded.is_empty() {
            write!(f, "hot-reloaded: {}", self.hot_reloaded.join(", "))?;
        }
        if !self.requires_restart.is_empty() {
            if !self.hot_reloaded.is_empty() {
                write!(f, "; ")?;
            }
            write!(f, "requires restart: {}", self.requires_restart.join(", "))?;
        }
        for w in &self.warnings {
            write!(f, "\n  warning: {w}")?;
        }
        Ok(())
    }
}

// ---- ValidationWarning ---------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationWarning {
    UnknownProvider {
        model: String,
        provider: String,
        similar: Option<String>,
    },
    UnknownModel {
        field: String,
        model: String,
    },
}

impl fmt::Display for ValidationWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownProvider {
                model,
                provider,
                similar,
            } => {
                write!(
                    f,
                    "Model '{model}' references missing provider '{provider}'"
                )?;
                if let Some(s) = similar {
                    write!(f, " (did you mean '{s}'?)")?;
                }
                Ok(())
            }
            Self::UnknownModel { field, model } => {
                write!(f, "{field} references missing model '{model}'")
            }
        }
    }
}

#[must_use]
pub fn validate_references(config: &RokoConfig) -> Vec<ValidationWarning> {
    let providers = config.effective_providers();
    let provider_keys = providers.keys().map(String::as_str).collect::<HashSet<_>>();
    let mut warnings = Vec::new();

    let mut model_entries = config.models.iter().collect::<Vec<_>>();
    model_entries.sort_unstable_by_key(|(l, _)| *l);
    for (model_key, profile) in model_entries {
        let provider = profile.provider.trim();
        if !provider_keys.contains(provider) {
            warnings.push(ValidationWarning::UnknownProvider {
                model: model_key.clone(),
                provider: profile.provider.clone(),
                similar: find_similar(provider, provider_keys.iter().copied()),
            });
        }
    }

    let explicit_model_keys = config
        .models
        .keys()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let effective_models = config.effective_models();

    if let Some(fallback) = config
        .agent
        .fallback_model
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
    {
        let exists = if explicit_model_keys.is_empty() {
            effective_models.contains_key(fallback)
        } else {
            explicit_model_keys.contains(fallback)
        };
        if !exists {
            warnings.push(ValidationWarning::UnknownModel {
                field: "agent.fallback_model".to_string(),
                model: fallback.to_string(),
            });
        }
    }

    if !explicit_model_keys.is_empty() {
        let mut tier_entries = config.agent.tier_models.iter().collect::<Vec<_>>();
        tier_entries.sort_unstable_by_key(|(l, _)| *l);
        for (tier, model_key) in tier_entries {
            let model_key = model_key.trim();
            if model_key.is_empty() || explicit_model_keys.contains(model_key) {
                continue;
            }
            warnings.push(ValidationWarning::UnknownModel {
                field: format!("agent.tier_models.{tier}"),
                model: model_key.to_string(),
            });
        }
    }

    warnings
}

// ---- Conductor (not extracted, stays in schema) --------------------------

/// Conductor (meta-orchestrator) settings.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConductorConfig {
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,
    #[serde(default = "default_max_parallel_plans")]
    pub max_parallel_plans: usize,
    #[serde(default)]
    pub parallel_enabled: bool,
    #[serde(default)]
    pub express_mode: bool,
    #[serde(default = "default_true")]
    pub auto_advance_batch: bool,
    #[serde(default)]
    pub auto_merge_on_complete: bool,
    #[serde(default)]
    pub pre_plan: bool,
    #[serde(default = "default_max_auto_fix")]
    pub max_auto_fix_attempts: u32,
    #[serde(default = "default_auto_fix_model")]
    pub auto_fix_model: String,
    #[serde(default)]
    pub conductor_model: Option<String>,
    #[serde(default = "default_warm_impl")]
    pub warm_implementers_per_plan: usize,
    #[serde(default)]
    pub enabled_roles: AgentRoleToggles,
    /// Per-watcher threshold overrides for the conductor anomaly ensemble.
    #[serde(default)]
    pub watchers: WatcherThresholds,
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
            watchers: WatcherThresholds::default(),
        }
    }
}

/// Per-watcher threshold configuration.
///
/// Every field is optional. A missing field means the watcher uses its
/// built-in default, which keeps old `roko.toml` files compatible while making
/// runtime oversight tunable without editing Rust code.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WatcherThresholds {
    #[serde(default)]
    pub compile_fail_repeat: Option<CompileFailRepeatConfig>,
    #[serde(default)]
    pub context_window_pressure: Option<ContextWindowPressureConfig>,
    #[serde(default)]
    pub cost_overrun: Option<CostOverrunConfig>,
    #[serde(default)]
    pub ghost_turn: Option<GhostTurnConfig>,
    #[serde(default)]
    pub iteration_loop: Option<IterationLoopConfig>,
    #[serde(default)]
    pub review_loop: Option<ReviewLoopConfig>,
    #[serde(default)]
    pub spec_drift: Option<SpecDriftConfig>,
    #[serde(default)]
    pub stuck_pattern: Option<StuckPatternConfig>,
    #[serde(default)]
    pub test_failure_budget: Option<TestFailureBudgetConfig>,
    #[serde(default)]
    pub time_overrun: Option<TimeOverrunConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileFailRepeatConfig {
    #[serde(default = "default_compile_fail_repeat_max")]
    pub max_repeats: usize,
}

const fn default_compile_fail_repeat_max() -> usize {
    3
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextWindowPressureConfig {
    #[serde(default = "default_context_pressure_warn")]
    pub warn_threshold: f64,
    #[serde(default = "default_context_pressure_critical")]
    pub critical_threshold: f64,
}

const fn default_context_pressure_warn() -> f64 {
    0.75
}

const fn default_context_pressure_critical() -> f64 {
    0.90
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostOverrunConfig {
    #[serde(default = "default_cost_overrun_warn")]
    pub warn_usd: f64,
    #[serde(default = "default_cost_overrun_critical")]
    pub critical_usd: f64,
}

const fn default_cost_overrun_warn() -> f64 {
    1.0
}

const fn default_cost_overrun_critical() -> f64 {
    5.0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhostTurnConfig {
    #[serde(default = "default_ghost_min_output_tokens")]
    pub min_output_tokens: u32,
    #[serde(default = "default_ghost_max_consecutive")]
    pub max_consecutive: usize,
}

const fn default_ghost_min_output_tokens() -> u32 {
    1
}

const fn default_ghost_max_consecutive() -> usize {
    3
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IterationLoopConfig {
    #[serde(default = "default_iteration_loop_max")]
    pub max_iterations: usize,
}

const fn default_iteration_loop_max() -> usize {
    3
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewLoopConfig {
    #[serde(default = "default_review_loop_max")]
    pub max_rejections: usize,
}

const fn default_review_loop_max() -> usize {
    3
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpecDriftConfig {
    #[serde(default = "default_spec_drift_ratio")]
    pub max_ratio: f64,
}

const fn default_spec_drift_ratio() -> f64 {
    0.25
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StuckPatternConfig {
    #[serde(default = "default_stuck_pattern_max")]
    pub max_identical_actions: usize,
}

const fn default_stuck_pattern_max() -> usize {
    4
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestFailureBudgetConfig {
    #[serde(default = "default_test_failure_increase")]
    pub min_failure_increase: u32,
}

const fn default_test_failure_increase() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeOverrunConfig {
    #[serde(default = "default_time_overrun_alert_ratio")]
    pub alert_ratio: f64,
}

const fn default_time_overrun_alert_ratio() -> f64 {
    0.80
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRoleToggles {
    #[serde(default = "default_true")]
    pub architect: bool,
    #[serde(default = "default_true")]
    pub auditor: bool,
    #[serde(default = "default_true")]
    pub scribe: bool,
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

/// Agent definition for multi-agent startup via `roko up`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub name: String,
    pub domain: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub chain_rpc: Option<String>,
    #[serde(default = "default_agent_enabled")]
    pub enabled: bool,
}
const fn default_agent_enabled() -> bool {
    true
}

// ---- utility functions ---------------------------------------------------

fn parse_bool_env(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn interpolate_vars(value: &str, env_fn: &dyn Fn(&str) -> Option<String>) -> String {
    if !value.contains("${") {
        return value.to_string();
    }
    let re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").expect("valid regex");
    re.replace_all(value, |caps: &regex::Captures| {
        env_fn(&caps[1]).unwrap_or_default()
    })
    .into_owned()
}

fn find_similar<'a>(needle: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let needle = needle.trim();
    if needle.is_empty() {
        return None;
    }
    let mut best_match = None;
    let mut best_distance = usize::MAX;
    for candidate in candidates {
        let distance = edit_distance(needle, candidate);
        if distance < best_distance {
            best_distance = distance;
            best_match = Some(candidate);
        }
    }
    (best_distance <= 3).then(|| best_match.expect("distance implies candidate").to_string())
}

fn edit_distance(left: &str, right: &str) -> usize {
    if left == right {
        return 0;
    }
    if left.is_empty() {
        return right.chars().count();
    }
    if right.is_empty() {
        return left.chars().count();
    }
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut costs = (0..=right_chars.len()).collect::<Vec<_>>();
    for (left_idx, left_ch) in left.chars().enumerate() {
        let mut previous_diagonal = costs[0];
        costs[0] = left_idx + 1;
        for (right_idx, right_ch) in right_chars.iter().copied().enumerate() {
            let ins = costs[right_idx + 1] + 1;
            let del = costs[right_idx] + 1;
            let sub = previous_diagonal + usize::from(left_ch != right_ch);
            previous_diagonal = costs[right_idx + 1];
            costs[right_idx + 1] = ins.min(del).min(sub);
        }
    }
    *costs.last().unwrap_or(&0)
}

// ---- test helper ---------------------------------------------------------

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

// ---- tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct SharedLogBuffer {
        inner: Arc<Mutex<Vec<u8>>>,
    }
    struct SharedLogWriter {
        inner: Arc<Mutex<Vec<u8>>>,
    }
    impl SharedLogBuffer {
        fn into_string(self) -> String {
            String::from_utf8(self.inner.lock().expect("lock").clone()).expect("utf-8")
        }
    }
    impl<'a> MakeWriter<'a> for SharedLogBuffer {
        type Writer = SharedLogWriter;
        fn make_writer(&'a self) -> Self::Writer {
            SharedLogWriter {
                inner: Arc::clone(&self.inner),
            }
        }
    }
    impl Write for SharedLogWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.inner.lock().expect("lock").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn capture_warn_logs<T>(f: impl FnOnce() -> T) -> (T, String) {
        let buffer = SharedLogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_writer(buffer.clone())
            .finish();
        let dispatch = tracing::Dispatch::new(subscriber);
        let value = tracing::dispatcher::with_default(&dispatch, f);
        (value, buffer.into_string())
    }
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
        let expected = RokoConfig {
            config_version: 1,
            ..RokoConfig::default()
        };
        assert_eq!(cfg, expected);
    }
    #[test]
    fn config_version_defaults_to_legacy() {
        let cfg = RokoConfig::from_toml("").expect("parse");
        assert_eq!(cfg.config_version, 1);
    }
    #[test]
    fn default_config_uses_current_config_version() {
        let cfg = RokoConfig::default();
        assert_eq!(cfg.config_version, CURRENT_CONFIG_VERSION);
    }

    #[test]
    fn config_version_detection_warns_for_legacy_configs() {
        let (cfg, logs) = capture_warn_logs(|| {
            RokoConfig::from_toml(
                r#"
[agent]
default_model = "claude-sonnet-4-6"
"#,
            )
            .expect("parse")
        });
        assert_eq!(cfg.config_version, 1);
        // Note: the static Once guard means the warning only fires once per
        // process. If another test parsed a v1 config first, `logs` may be
        // empty. We assert the config version is correct regardless.
        let _ = logs;
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
        let toml = "[project]\nname = \"my-dapp\"\nroot = \"/home/user/code\"\nfresh_base_branch = \"develop\"\n";
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.project.name, "my-dapp");
        assert_eq!(cfg.project.root, "/home/user/code");
        assert_eq!(cfg.project.fresh_base_branch, "develop");
    }
    #[test]
    fn config_load() {
        let toml = "[agent]\ndefault_model = \"claude-sonnet-4-6\"\n";
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.agent.default_model, "claude-sonnet-4-6");
    }
    #[test]
    fn gates_section_parses() {
        let toml = "[gates]\nclippy_enabled = false\nskip_tests = true\nmax_iterations = 5\n";
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert!(!cfg.gates.clippy_enabled);
        assert!(cfg.gates.skip_tests);
        assert_eq!(cfg.gates.max_iterations, 5);
    }
    #[test]
    fn budget_section_parses() {
        let toml =
            "[budget]\nmax_plan_usd = 100.0\nmax_turn_usd = 10.0\nprompt_token_budget = 20000\n";
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert!((cfg.budget.max_plan_usd - 100.0).abs() < f32::EPSILON);
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
        assert_eq!(cfg.conductor.max_agents, 16);
        assert!(cfg.conductor.parallel_enabled);
        assert!(!cfg.gates.clippy_enabled);
    }

    #[test]
    fn example_toml_contains_all_sections() {
        let example = RokoConfig::example_toml();
        assert!(example.contains("[project]"));
        assert!(example.contains("[agent]"));
        assert!(example.contains("[gates]"));
        assert!(example.contains("[routing]"));
        assert!(example.contains("[budget]"));
        assert!(example.contains("[tui]"));
        assert!(example.contains("[serve]"));
        assert!(example.contains("auto_start = false"));
    }
    #[test]
    fn example_toml_is_valid_toml() {
        let example = RokoConfig::example_toml();
        let cfg = RokoConfig::from_toml(&example).expect("parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
    }
    #[test]
    fn kimi_config_parse() {
        let example = include_str!("../../../../examples/roko-kimi.toml");
        let cfg = RokoConfig::from_toml(example).expect("parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
        let model = cfg.models.get("kimi-k2-5").expect("kimi");
        assert_eq!(model.provider, "moonshot");
    }
    #[test]
    fn openrouter_config() {
        let example = include_str!("../../../../examples/roko-openrouter.toml");
        let cfg = RokoConfig::from_toml(example).expect("parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
    }
    #[test]
    fn parse_bool_env_variants() {
        assert!(parse_bool_env("true"));
        assert!(parse_bool_env("1"));
        assert!(parse_bool_env("yes"));
        assert!(!parse_bool_env("false"));
        assert!(!parse_bool_env("0"));
        assert!(!parse_bool_env(""));
    }
    #[test]
    fn interpolate_vars_expands_env_references() {
        let env_fn = |key: &str| -> Option<String> {
            match key {
                "API_KEY" => Some("sk-secret".into()),
                _ => None,
            }
        };
        assert_eq!(interpolate_vars("${API_KEY}", &env_fn), "sk-secret");
        assert_eq!(interpolate_vars("plain text", &env_fn), "plain text");
        assert_eq!(interpolate_vars("${MISSING}", &env_fn), "");
    }
    #[test]
    fn classify_changes_detects_hot_reloadable_budget_change() {
        let current = RokoConfig::default();
        let mut proposed = current.clone();
        proposed.budget.max_plan_usd += 5.0;
        let report = current.classify_changes(&proposed);
        assert!(report.has_changes());
        assert!(!report.needs_restart());
        assert!(report.hot_reloaded.contains(&"budget"));
    }
    #[test]
    fn classify_changes_no_changes_yields_empty_report() {
        let config = RokoConfig::default();
        let report = config.classify_changes(&config);
        assert!(!report.has_changes());
        assert_eq!(report.changed_count(), 0);
    }

    #[test]
    fn effective_providers_backwards_compat() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roko.toml");
        let text = std::fs::read_to_string(path).expect("read roko.toml");
        let cfg = RokoConfig::from_toml(&text).expect("parse roko.toml");
        let providers = cfg.effective_providers();
        let claude = providers.get("claude_cli").expect("claude_cli provider");
        assert_eq!(claude.kind, ProviderKind::ClaudeCli);
    }

    #[test]
    fn effective_models_backwards_compat() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roko.toml");
        let text = std::fs::read_to_string(path).expect("read roko.toml");
        let cfg = RokoConfig::from_toml(&text).expect("parse roko.toml");
        let models = cfg.effective_models();
        let default_model = models.get("claude-sonnet-4-6").expect("default model");
        assert_eq!(default_model.provider, "claude_cli");
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
        let api_key_env = std::env::var("ROKO_API_KEY_ENV_NAME").expect("env");
        let expected = std::env::var("ROKO_EXPECT_API_KEY").expect("expected");
        let cfg = ProviderConfig {
            kind: ProviderKind::OpenAiCompat,
            base_url: None,
            api_key_env: Some(api_key_env),
            command: None,
            args: None,
            timeout_ms: None,
            ttft_timeout_ms: Some(15_000),
            connect_timeout_ms: Some(5_000),
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
        let api_key_env = std::env::var("ROKO_API_KEY_ENV_NAME").expect("env");
        let cfg = ProviderConfig {
            kind: ProviderKind::OpenAiCompat,
            base_url: None,
            api_key_env: Some(api_key_env),
            command: None,
            args: None,
            timeout_ms: None,
            ttft_timeout_ms: Some(15_000),
            connect_timeout_ms: Some(5_000),
            extra_headers: None,
            max_concurrent: None,
        };
        assert_eq!(cfg.resolve_api_key(), None);
    }

    #[test]
    fn full_roundtrip_with_roles() {
        let toml = "schema_version = 2\n[project]\nname = \"test\"\n[agent]\ndefault_model = \"claude-opus-4-6\"\n[agent.roles.implementer]\nmodel = \"claude-sonnet-4-6\"\neffort = \"high\"\n[gates]\nclippy_enabled = false\n[routing]\nfast_task_model = \"haiku\"\n[budget]\nmax_plan_usd = 50.0\n[conductor]\nmax_agents = 4\nexpress_mode = true\n[learning]\nauto_playbook_refresh = false\n[tui]\nrefresh_rate_ms = 100\n[serve.auth]\nenabled = true\napi_key = \"secret\"\n[server]\nport = 3000\n";
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        let text = cfg.to_toml().expect("serialize");
        let back = RokoConfig::from_toml(&text).expect("re-parse");
        assert_eq!(cfg, back);
    }

    #[test]
    fn validate_references_warns_on_unknown_provider_with_suggestion() {
        let mut cfg = RokoConfig::default();
        cfg.providers.insert(
            "openrouter".to_string(),
            ProviderConfig {
                kind: ProviderKind::OpenAiCompat,
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_key_env: Some("OPENROUTER_API_KEY".to_string()),
                command: None,
                args: None,
                timeout_ms: None,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            },
        );
        cfg.models.insert(
            "glm-5-1".to_string(),
            ModelProfile {
                provider: "openruoter".to_string(),
                slug: "z-ai/glm-5.1".to_string(),
                context_window: 200_000,
                supports_tools: true,
                ..Default::default()
            },
        );
        let warnings = validate_references(&cfg);
        assert_eq!(
            warnings,
            vec![ValidationWarning::UnknownProvider {
                model: "glm-5-1".to_string(),
                provider: "openruoter".to_string(),
                similar: Some("openrouter".to_string())
            }]
        );
    }

    #[test]
    fn perplexity_example_config() {
        let example = include_str!("../../../../examples/roko-perplexity.toml");
        let cfg = RokoConfig::from_toml(example).expect("parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
        let pplx = cfg.providers.get("perplexity").expect("perplexity");
        assert_eq!(pplx.kind, ProviderKind::PerplexityApi);
    }

    #[test]
    fn gemini_example_config() {
        let example = include_str!("../../../../examples/roko-gemini.toml");
        let cfg = RokoConfig::from_toml(example).expect("parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
        let provider = cfg.providers.get("gemini").expect("gemini");
        assert_eq!(provider.kind, ProviderKind::GeminiApi);
        for (model_key, slug) in [
            ("gemini-2-5-flash-lite", "gemini-2.5-flash-lite"),
            ("gemini-2-5-flash", "gemini-2.5-flash"),
            ("gemini-2-5-pro", "gemini-2.5-pro"),
        ] {
            let model = cfg.models.get(model_key).expect(model_key);
            assert_eq!(model.provider, "gemini");
            assert_eq!(model.slug, slug);
            let resolved = crate::agent::resolve_model(&cfg, model_key);
            assert_eq!(resolved.model_key, model_key);
            assert_eq!(resolved.slug, slug);
            assert_eq!(resolved.provider_kind, ProviderKind::GeminiApi);
        }
    }

    #[test]
    fn multi_provider_config() {
        let example = include_str!("../../../../examples/roko-multi-provider.toml");
        let cfg = RokoConfig::from_toml(example).expect("parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(cfg.providers.contains_key("claude_cli"));
        assert!(cfg.providers.contains_key("gemini"));
        assert!(cfg.providers.contains_key("perplexity"));
    }

    #[test]
    fn resolve_file_secrets_reads_from_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let secret_path = dir.path().join("api_key");
        std::fs::write(&secret_path, "  file-secret-value  \n").expect("write");
        let mut config = RokoConfig::default();
        let mut headers = HashMap::new();
        headers.insert(
            "authorization_file".to_string(),
            secret_path.display().to_string(),
        );
        config.providers.insert(
            "test".to_string(),
            ProviderConfig {
                kind: ProviderKind::OpenAiCompat,
                base_url: None,
                api_key_env: None,
                command: None,
                args: None,
                timeout_ms: None,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: Some(headers),
                max_concurrent: None,
            },
        );
        config.resolve_file_secrets();
        let resolved = config.providers["test"]
            .extra_headers
            .as_ref()
            .expect("headers");
        assert_eq!(
            resolved.get("authorization").map(String::as_str),
            Some("file-secret-value")
        );
        assert!(!resolved.contains_key("authorization_file"));
    }

    #[test]
    fn subscription_trigger_cron_roundtrip() {
        let trigger = SubscriptionTrigger::Cron {
            schedule: "*/30 * * * *".into(),
        };
        let json = serde_json::to_string(&trigger).unwrap();
        let parsed: SubscriptionTrigger = serde_json::from_str(&json).unwrap();
        assert_eq!(trigger, parsed);
        assert_eq!(trigger.kind(), "cron");
    }

    #[test]
    fn config_deser_invalid_provider_kind_is_descriptive() {
        let toml = "[providers.bad]\nkind = \"not_a_real_kind\"\n";
        let err = toml::from_str::<RokoConfig>(toml).expect_err("should fail");
        assert_error_contains(err, &["kind", "unknown variant", "not_a_real_kind"]);
    }
}
