//! Unified `RokoConfig` schema with hierarchical sections.
//!
//! Every section is a separate struct so callers can destructure just the
//! slice they need. All fields carry serde defaults so a bare config still
//! produces a fully-populated `RokoConfig`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fmt::Write as _;
use std::path::PathBuf;

use crate::agent::{AgentBackend, ProviderKind};
use crate::task::TaskDomain;
use crate::temperament::Temperament;
use crate::tool::{ToolFormat, profile_for_model};
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};

/// Current schema version. Bump on incompatible changes.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

// ---- top-level -----------------------------------------------------------

/// Root configuration for the Roko runtime.
///
/// ```toml
/// config_version = 2
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
/// tools = ["read", "edit", "bash", "git-*"]
/// ```
pub const CURRENT_CONFIG_VERSION: u32 = 2;

#[allow(clippy::derive_partial_eq_without_eq)] // contains f32 via BudgetConfig
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RokoConfig {
    /// Config layout version for migration tooling.
    #[serde(default = "default_config_version")]
    pub config_version: u32,

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

    /// Complexity-to-pipeline mapping for orchestration stages.
    #[serde(default)]
    pub pipeline: PipelineConfig,

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

    /// Knowledge demurrage (Gesellian decay) settings.
    #[serde(default)]
    pub demurrage: DemurrageConfig,

    /// Attention token budget allocation and context window management.
    #[serde(default)]
    pub attention: AttentionConfig,

    /// Anomaly detection thresholds and quarantine settings.
    #[serde(default)]
    pub immune: ImmuneConfig,

    /// Time horizon preferences and planning depth.
    #[serde(default)]
    pub temporal: TemporalConfig,

    /// Goal hierarchy, priority weights, and completion criteria.
    #[serde(default)]
    pub goals: GoalsConfig,

    /// Compute budget, cost caps per tier.
    #[serde(default)]
    pub energy: EnergyConfig,

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

    /// Perplexity-specific settings (search recency, domain filters, etc.).
    #[serde(default)]
    pub perplexity: PerplexityConfig,

    /// Gemini-specific settings (model defaults, thinking, safety, caching).
    #[serde(default)]
    pub gemini: GeminiConfig,

    /// Tool profile configuration (TOOL-03).
    #[serde(default)]
    pub tools: ToolsConfig,

    /// Oneirography (dream art) pipeline settings (DREAM-13).
    ///
    /// Disabled by default. Opt-in via `[oneirography]` section in roko.toml:
    /// ```toml
    /// [oneirography]
    /// enabled = true
    /// provider = "dall-e-3"
    /// variants = 3
    /// ```
    #[serde(default)]
    pub oneirography: OneirographyConfig,

    /// EVM chain connection settings for chain-domain tools.
    #[serde(default)]
    pub chain: ChainConfig,
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
        let _ = writeln!(out, "config_version = {CURRENT_CONFIG_VERSION}");
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
        let _ = writeln!(out, "temperament = \"{}\"", cfg.agent.temperament);
        let _ = writeln!(out, "context_limit_k = {}", cfg.agent.context_limit_k);
        let _ = writeln!(out, "bare_mode = {}\n", cfg.agent.bare_mode);

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
        let _ = writeln!(out, "algorithm = \"{}\"", cfg.routing.algorithm.label());
        let _ = writeln!(out, "discount_factor = {}", cfg.routing.discount_factor);
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
        let _ = writeln!(out, "[routing.weights]");
        let _ = writeln!(out, "quality = {}", cfg.routing.weights.default.quality);
        let _ = writeln!(out, "cost = {}", cfg.routing.weights.default.cost);
        let _ = writeln!(out, "latency = {}\n", cfg.routing.weights.default.latency);
        let mechanical = cfg.routing.weights.for_tier("mechanical");
        let _ = writeln!(out, "[routing.weights.mechanical]");
        let _ = writeln!(out, "quality = {}", mechanical.quality);
        let _ = writeln!(out, "cost = {}", mechanical.cost);
        let _ = writeln!(out, "latency = {}\n", mechanical.latency);
    }

    fn write_example_pipeline(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Complexity-to-pipeline mapping --");

        let mechanical = cfg.pipeline.mechanical;
        let _ = writeln!(out, "[pipeline.mechanical]");
        let _ = writeln!(out, "strategist = {}", mechanical.strategist);
        let _ = writeln!(out, "reviewers = {}", mechanical.reviewers);
        let _ = writeln!(
            out,
            "reviewer_mode = \"{}\"",
            mechanical.reviewer_mode.label()
        );
        let _ = writeln!(out, "max_iterations = {}\n", mechanical.max_iterations);

        let focused = cfg.pipeline.focused;
        let _ = writeln!(out, "[pipeline.focused]");
        let _ = writeln!(out, "strategist = {}", focused.strategist);
        let _ = writeln!(out, "reviewers = {}", focused.reviewers);
        let _ = writeln!(out, "reviewer_mode = \"{}\"", focused.reviewer_mode.label());
        let _ = writeln!(out, "max_iterations = {}\n", focused.max_iterations);

        let integrative = cfg.pipeline.integrative;
        let _ = writeln!(out, "[pipeline.integrative]");
        let _ = writeln!(out, "strategist = {}", integrative.strategist);
        let _ = writeln!(out, "reviewers = {}", integrative.reviewers);
        let _ = writeln!(
            out,
            "reviewer_mode = \"{}\"",
            integrative.reviewer_mode.label()
        );
        let _ = writeln!(out, "max_iterations = {}\n", integrative.max_iterations);

        let architectural = cfg.pipeline.architectural;
        let _ = writeln!(out, "[pipeline.architectural]");
        let _ = writeln!(out, "strategist = {}", architectural.strategist);
        let _ = writeln!(out, "reviewers = {}", architectural.reviewers);
        let _ = writeln!(
            out,
            "reviewer_mode = \"{}\"",
            architectural.reviewer_mode.label()
        );
        let _ = writeln!(out, "max_iterations = {}\n", architectural.max_iterations);
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
        let _ = writeln!(
            out,
            "replan_on_gate_failure = {}",
            cfg.learning.replan_on_gate_failure
        );
        let _ = writeln!(
            out,
            "replan_max_per_plan = {}",
            cfg.learning.replan_max_per_plan
        );
        let _ = writeln!(
            out,
            "replan_gate_attempts = {}\n",
            cfg.learning.replan_gate_attempts
        );
    }

    fn write_example_demurrage(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Knowledge demurrage --");
        let _ = writeln!(out, "[demurrage]");
        let _ = writeln!(out, "rate_per_hour = {}", cfg.demurrage.rate_per_hour);
        let _ = writeln!(out, "min_balance = {}", cfg.demurrage.min_balance);
        let _ = writeln!(out, "freeze_threshold = {}", cfg.demurrage.freeze_threshold);
        let _ = writeln!(out, "thaw_balance = {}", cfg.demurrage.thaw_balance);
        let _ = writeln!(out, "max_balance = {}", cfg.demurrage.max_balance);
        let _ = writeln!(out, "death_threshold = {}", cfg.demurrage.death_threshold);
        let _ = writeln!(
            out,
            "freeze_before_delete = {}",
            cfg.demurrage.freeze_before_delete
        );
        let _ = writeln!(
            out,
            "gc_interval_secs = {}\n",
            cfg.demurrage.gc_interval_secs
        );
    }

    fn write_example_attention(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Attention budget allocation --");
        let _ = writeln!(out, "[attention]");
        let _ = writeln!(
            out,
            "max_tokens_per_layer = {}",
            cfg.attention.max_tokens_per_layer
        );
        let _ = writeln!(
            out,
            "utilization_target = {}",
            cfg.attention.utilization_target
        );
        let _ = writeln!(out, "auction_enabled = {}", cfg.attention.auction_enabled);
        let _ = writeln!(
            out,
            "task_reserve_tokens = {}\n",
            cfg.attention.task_reserve_tokens
        );
    }

    fn write_example_immune(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Anomaly detection / immune system --");
        let _ = writeln!(out, "[immune]");
        let _ = writeln!(
            out,
            "quarantine_threshold = {}",
            cfg.immune.quarantine_threshold
        );
        let _ = writeln!(out, "max_quarantined = {}", cfg.immune.max_quarantined);
        let _ = writeln!(out, "auto_reject = {}", cfg.immune.auto_reject);
        let _ = writeln!(out, "taint_levels = {:?}\n", cfg.immune.taint_levels);
    }

    fn write_example_temporal(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Temporal planning --");
        let _ = writeln!(out, "[temporal]");
        let _ = writeln!(out, "max_depth = {}", cfg.temporal.max_depth);
        let _ = writeln!(out, "epoch_secs = {}", cfg.temporal.epoch_secs);
        let _ = writeln!(
            out,
            "enforce_allen_relations = {}\n",
            cfg.temporal.enforce_allen_relations
        );
    }

    fn write_example_goals(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Goal hierarchy --");
        let _ = writeln!(out, "[goals]");
        let _ = writeln!(out, "max_active = {}", cfg.goals.max_active);
        let _ = writeln!(out, "correctness_weight = {}", cfg.goals.correctness_weight);
        let _ = writeln!(
            out,
            "completion_threshold = {}",
            cfg.goals.completion_threshold
        );
        let _ = writeln!(out, "prune_threshold = {}\n", cfg.goals.prune_threshold);
    }

    fn write_example_energy(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- Compute budget / energy --");
        let _ = writeln!(out, "[energy]");
        let _ = writeln!(out, "pool_usd = {}", cfg.energy.pool_usd);
        let _ = writeln!(out, "per_task_cap_usd = {}", cfg.energy.per_task_cap_usd);
        let _ = writeln!(out, "metabolism_rate = {}\n", cfg.energy.metabolism_rate);
    }

    fn write_example_tui_and_server(out: &mut String, cfg: &Self) {
        let _ = writeln!(out, "# -- TUI preferences --");
        let _ = writeln!(out, "[tui]");
        let _ = writeln!(out, "refresh_rate_ms = {}\n", cfg.tui.refresh_rate_ms);

        let _ = writeln!(out, "# -- API auth --");
        let _ = writeln!(out, "[serve]");
        let _ = writeln!(out, "auto_orchestrate = {}", cfg.serve.auto_orchestrate);
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
        let config: Self = toml::from_str(s)?;
        if config.config_version == 1 {
            use std::sync::Once;
            static WARN_ONCE: Once = Once::new();
            WARN_ONCE.call_once(|| {
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
                timeout_ms: self.agent.timeout_ms.or(default_provider_timeout_ms()),
                ttft_timeout_ms: default_provider_ttft_timeout_ms(),
                connect_timeout_ms: default_provider_connect_timeout_ms(),
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

    /// Interpolate `${VAR}` patterns in string values of this config.
    ///
    /// Any occurrence of `${SOME_VAR}` is replaced with the contents of the
    /// environment variable `SOME_VAR`. Missing variables expand to the empty
    /// string. This runs after TOML parsing so that secrets can be injected
    /// from the environment without being hardcoded in `roko.toml`:
    ///
    /// ```toml
    /// [providers.anthropic]
    /// api_key_env = "${ANTHROPIC_API_KEY}"
    /// ```
    pub fn interpolate_env_vars(&mut self) {
        Self::interpolate_env_vars_with(&mut self.providers, &|key| std::env::var(key).ok());
    }

    /// Interpolate `${VAR}` patterns in provider config strings using a custom
    /// environment resolver (testable).
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
    ///
    /// Any `ProviderConfig` whose `api_key_env` field ends with `_file` has its
    /// value treated as a file path: the file contents (trimmed) become the
    /// resolved secret. This is the standard Docker/K8s secret mounting pattern:
    ///
    /// ```toml
    /// [providers.anthropic]
    /// api_key_env = "/run/secrets/anthropic_key"
    /// ```
    ///
    /// Note: for the `_file` pattern to work, the config must use a dedicated
    /// `api_key_file` field or the `api_key_env` field must point to a path.
    /// This method checks `extra_headers` for any key ending in `_file` as well.
    pub fn resolve_file_secrets(&mut self) {
        for provider in self.providers.values_mut() {
            // Check extra_headers for _file references.
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

    /// Classify a proposed configuration change into hot-reloadable fields,
    /// fields that require restart, and fields that emit warnings.
    ///
    /// Returns a [`ConfigChangeReport`] indicating which sections changed and
    /// whether the changes can be applied without a process restart.
    ///
    /// Hot-reloadable sections: `[budget]`, `[gates]`, `[routing]`, `[learning]`,
    /// `[demurrage]`, `[scheduler]`, `[watcher]`, `subscriptions`.
    ///
    /// Restart-required sections: `[agent]` (model/backend changes), `[project]`,
    /// `[serve]` (port changes), `providers`.
    #[must_use]
    pub fn classify_changes(&self, proposed: &Self) -> ConfigChangeReport {
        let mut report = ConfigChangeReport::default();

        // Hot-reloadable sections.
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

        // Restart-required sections.
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

        // Warnings for sensitive changes.
        if proposed.budget.max_plan_usd > self.budget.max_plan_usd {
            report.warnings.push(format!(
                "budget.max_plan_usd increased from {} to {}",
                self.budget.max_plan_usd, proposed.budget.max_plan_usd
            ));
        }

        report
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
}

/// Report produced by [`RokoConfig::classify_changes`].
///
/// Groups changed config sections into hot-reloadable (no restart needed) and
/// restart-required buckets, plus optional warnings for sensitive changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigChangeReport {
    /// Sections that changed and can be applied without restart.
    pub hot_reloaded: Vec<&'static str>,
    /// Sections that changed but require a process restart.
    pub requires_restart: Vec<&'static str>,
    /// Operator-facing warnings about sensitive changes.
    pub warnings: Vec<String>,
}

impl ConfigChangeReport {
    /// Whether any changes were detected at all.
    #[must_use]
    pub fn has_changes(&self) -> bool {
        !self.hot_reloaded.is_empty() || !self.requires_restart.is_empty()
    }

    /// Whether a restart is needed to fully apply the changes.
    #[must_use]
    pub fn needs_restart(&self) -> bool {
        !self.requires_restart.is_empty()
    }

    /// Total number of changed sections.
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

/// Non-fatal config warnings emitted by semantic reference validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationWarning {
    /// A model points at a provider key that does not exist.
    UnknownProvider {
        /// Model registry key that contains the bad reference.
        model: String,
        /// Unknown provider reference from the model profile.
        provider: String,
        /// Closest matching provider key, if one is close enough to suggest.
        similar: Option<String>,
    },
    /// A field points at a model key that does not exist.
    UnknownModel {
        /// Fully-qualified config field name.
        field: String,
        /// Unknown model key referenced by that field.
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
                if let Some(similar) = similar {
                    write!(f, " (did you mean '{similar}'?)")?;
                }
                Ok(())
            }
            Self::UnknownModel { field, model } => {
                write!(f, "{field} references missing model '{model}'")
            }
        }
    }
}

/// Validate cross-reference integrity for provider and model keys.
#[must_use]
pub fn validate_references(config: &RokoConfig) -> Vec<ValidationWarning> {
    let providers = config.effective_providers();
    let provider_keys = providers.keys().map(String::as_str).collect::<HashSet<_>>();

    let mut warnings = Vec::new();

    let mut model_entries = config.models.iter().collect::<Vec<_>>();
    model_entries.sort_unstable_by_key(|(left, _)| *left);
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

    if let Some(fallback_model) = config
        .agent
        .fallback_model
        .as_deref()
        .map(str::trim)
        .filter(|fallback_model| !fallback_model.is_empty())
    {
        let model_exists = if explicit_model_keys.is_empty() {
            effective_models.contains_key(fallback_model)
        } else {
            explicit_model_keys.contains(fallback_model)
        };
        if !model_exists {
            warnings.push(ValidationWarning::UnknownModel {
                field: "agent.fallback_model".to_string(),
                model: fallback_model.to_string(),
            });
        }
    }

    if !explicit_model_keys.is_empty() {
        let mut tier_entries = config.agent.tier_models.iter().collect::<Vec<_>>();
        tier_entries.sort_unstable_by_key(|(left, _)| *left);
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

fn parse_bool_env(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Expand `${VAR_NAME}` placeholders in a string using the given resolver.
///
/// Missing variables expand to the empty string, matching shell behavior for
/// `${UNSET:-}`. The regex matches `${A_Z09_UPPER_SNAKE}` identifiers only.
fn interpolate_vars(value: &str, env_fn: &dyn Fn(&str) -> Option<String>) -> String {
    // Fast path: no `${` means no expansion needed.
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
            let insertion = costs[right_idx + 1] + 1;
            let deletion = costs[right_idx] + 1;
            let substitution = previous_diagonal + usize::from(left_ch != right_ch);
            previous_diagonal = costs[right_idx + 1];
            costs[right_idx + 1] = insertion.min(deletion).min(substitution);
        }
    }

    *costs.last().unwrap_or(&0)
}

// ─── Tool profile configuration (TOOL-03) ──────────────────────────────────

/// Tool profile configuration section.
///
/// Parsed from the `[tools]` section in `roko.toml`:
///
/// ```toml
/// [tools]
/// # Extra tools allowed beyond the role profile.
/// allow = ["bash", "web_fetch"]
/// # Tools to deny regardless of role profile.
/// deny = ["write_file"]
///
/// [tools.profiles.coding]
/// extra_tools = ["bash", "edit_file", "write_file"]
/// excluded_tools = []
///
/// [tools.profiles.research]
/// extra_tools = ["web_search", "web_fetch"]
/// excluded_tools = ["write_file", "edit_file"]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    /// Global tool allowlist — these tools are always available regardless of
    /// role or domain profile. Additive with profile-specific tools.
    #[serde(default)]
    pub allow: Vec<String>,

    /// Global tool denylist — these tools are never available regardless of
    /// role or domain profile. Takes precedence over `allow`.
    #[serde(default)]
    pub deny: Vec<String>,

    /// Named domain profiles keyed by domain label (e.g., "coding", "research", "chain").
    #[serde(default)]
    pub profiles: HashMap<String, ToolProfileConfig>,
}

/// Configuration for the oneirography (dream art) pipeline (DREAM-13).
///
/// Disabled by default. Opt-in via `[oneirography]` in roko.toml:
/// ```toml
/// [oneirography]
/// enabled = true
/// provider = "dall-e-3"
/// variants = 3
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OneirographyConfig {
    /// Whether dream art generation is enabled (default `false`).
    pub enabled: bool,
    /// Image generation provider identifier (e.g., `"dall-e-3"`, `"stable-diffusion"`).
    pub provider: String,
    /// Number of image variants to generate per dream cycle.
    pub variants: usize,
    /// Base reserve price for affect-reactive auctions.
    pub base_reserve: f64,
    /// Base auction duration in seconds.
    pub base_duration_seconds: u64,
}

impl Default for OneirographyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "disabled".to_string(),
            variants: 3,
            base_reserve: 0.01,
            base_duration_seconds: 3600,
        }
    }
}

/// Chain connection settings used by the `chain.*` tool domain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ChainConfig {
    /// HTTP JSON-RPC endpoint (e.g. `https://mirage-devnet.up.railway.app`).
    #[serde(default)]
    pub rpc_url: Option<String>,
    /// Chain ID. Must match the endpoint. Mirage uses 1.
    #[serde(default)]
    pub chain_id: Option<u64>,
    /// Hex-encoded private key (0x-prefixed or bare). Used to sign txs.
    #[serde(default)]
    pub wallet_key: Option<String>,
    /// ERC-8004 IdentityRegistry contract address.
    #[serde(default)]
    pub identity_registry: Option<String>,
    /// ERC-8004 ReputationRegistry contract address.
    #[serde(default)]
    pub reputation_registry: Option<String>,
    /// ERC-8004 ValidationRegistry contract address.
    #[serde(default)]
    pub validation_registry: Option<String>,
    /// Deployer / funder address.
    #[serde(default)]
    pub deployer: Option<String>,
}

/// A single named tool profile with extra/excluded tool lists.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolProfileConfig {
    /// Tools added for this domain/profile.
    #[serde(default)]
    pub extra_tools: Vec<String>,

    /// Tools excluded for this domain/profile.
    #[serde(default)]
    pub excluded_tools: Vec<String>,
}

impl ToolsConfig {
    /// Compute the effective tool set for a given domain.
    ///
    /// The result is: `(base_tools + extra_tools + global_allow) - excluded_tools - global_deny`.
    pub fn effective_tools_for_domain(&self, domain: &str, base_tools: &[String]) -> Vec<String> {
        let mut tools: std::collections::HashSet<String> = base_tools.iter().cloned().collect();

        // Add global allows.
        for tool in &self.allow {
            tools.insert(tool.clone());
        }

        // Add domain-specific extras.
        if let Some(profile) = self.profiles.get(domain) {
            for tool in &profile.extra_tools {
                tools.insert(tool.clone());
            }
            // Remove domain-specific exclusions.
            for tool in &profile.excluded_tools {
                tools.remove(tool);
            }
        }

        // Remove global denies (highest priority).
        for tool in &self.deny {
            tools.remove(tool);
        }

        let mut result: Vec<String> = tools.into_iter().collect();
        result.sort();
        result
    }

    /// Returns `true` if a specific tool is allowed for a domain, considering
    /// all profile layers.
    pub fn is_tool_allowed(&self, domain: &str, tool_name: &str) -> bool {
        // Global deny takes precedence.
        if self.deny.iter().any(|t| t == tool_name) {
            return false;
        }

        // Check domain-specific exclusion.
        if let Some(profile) = self.profiles.get(domain) {
            if profile.excluded_tools.iter().any(|t| t == tool_name) {
                return false;
            }
        }

        true
    }
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
/// - `timeout_ms`: optional hard request or subprocess timeout
/// - `ttft_timeout_ms`: optional time-to-first-token timeout
/// - `connect_timeout_ms`: optional TCP connection timeout
/// - `extra_headers`: optional HTTP headers to inject on outbound requests
/// - `max_concurrent`: optional concurrency limit for this provider
///
/// Defaults:
/// - `kind`: no default, must be set explicitly
/// - `base_url`: `None`
/// - `api_key_env`: `None`
/// - `command`: `None`
/// - `args`: `None`
/// - `timeout_ms`: `120_000`
/// - `ttft_timeout_ms`: `15_000`
/// - `connect_timeout_ms`: `5_000`
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
/// ttft_timeout_ms = 15000
/// connect_timeout_ms = 5000
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
    /// Hard request or subprocess timeout in milliseconds.
    #[serde(
        default = "default_provider_timeout_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout_ms: Option<u64>,
    /// Time-to-first-token timeout in milliseconds.
    #[serde(
        default = "default_provider_ttft_timeout_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub ttft_timeout_ms: Option<u64>,
    /// TCP connection timeout in milliseconds.
    #[serde(
        default = "default_provider_connect_timeout_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub connect_timeout_ms: Option<u64>,
    /// Extra headers to inject on outbound requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<HashMap<String, String>>,
    /// Maximum concurrent requests allowed for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
}

const fn default_provider_timeout_ms() -> Option<u64> {
    Some(120_000)
}

const fn default_provider_ttft_timeout_ms() -> Option<u64> {
    Some(15_000)
}

const fn default_provider_connect_timeout_ms() -> Option<u64> {
    Some(5_000)
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// Whether the model supports Google Search grounding.
    #[serde(default)]
    pub supports_grounding: bool,
    /// Whether the model supports built-in code execution.
    #[serde(default)]
    pub supports_code_execution: bool,
    /// Whether the model supports provider-side context caching.
    #[serde(default)]
    pub supports_caching: bool,
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
    /// Input token cost per million tokens for the high-context pricing tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_input_per_m_high: Option<f64>,
    /// Output token cost per million tokens for the high-context pricing tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_output_per_m_high: Option<f64>,
    /// Cache read cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cache_read_per_m: Option<f64>,
    /// Cache write cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cache_write_per_m: Option<f64>,
    /// Provider-specific reasoning depth label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    /// Maximum number of tools before behavior degrades.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tools: Option<u32>,
    /// Tokenizer ratio vs OpenAI `o200k_base`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_ratio: Option<f64>,
    /// Whether the model supports web-grounded search (Perplexity Sonar).
    #[serde(default)]
    pub supports_search: bool,
    /// Whether the model returns citations in responses (Perplexity Sonar).
    #[serde(default)]
    pub supports_citations: bool,
    /// Whether the model supports the async job API (Perplexity deep research).
    #[serde(default)]
    pub supports_async: bool,
    /// Whether this is an embedding model rather than a chat model.
    #[serde(default)]
    pub is_embedding_model: bool,
    /// Search context size hint: "low", "medium", or "high" (Perplexity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<String>,
    /// Per-request fee in USD on top of token costs (Perplexity pricing model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_per_request: Option<f64>,
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
    /// Default work domain for tasks that don't declare one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_domain: Option<TaskDomain>,
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
            default_domain: None,
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
    /// Default agent temperament for roles without a local override.
    #[serde(default)]
    pub temperament: Temperament,
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

    /// Configuration for the Data LLM used in CaMeL dual-LLM isolation.
    ///
    /// When configured, content tagged with `Taint::ExternalFetch` or
    /// `Taint::ThirdPartyPlugin` is routed through this model with tool
    /// calls stripped. The Data LLM processes untrusted content and returns
    /// schema-constrained structured output that is safe for the Control
    /// LLM to consume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_llm: Option<DataLlmConfig>,
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
            temperament: Temperament::default(),
            context_limit_k: default_context_limit_k(),
            bare_mode: default_true(),
            command: None,
            args: None,
            timeout_ms: None,
            env: None,
            tier_models: HashMap::new(),
            fallback_model: None,
            roles: HashMap::new(),
            data_llm: None,
        }
    }
}

/// Per-role spend and token caps under `[agent.roles.<role>]`.
#[allow(clippy::derive_partial_eq_without_eq)] // contains f64 via derived helpers
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentBudget {
    /// Estimated token ceiling for a single turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_turn: Option<u32>,
    /// Estimated spend ceiling for a single turn, expressed in USD cents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd_cents_per_turn: Option<u32>,
}

impl AgentBudget {
    /// Convert the configured USD-cent ceiling into a USD float.
    #[must_use]
    pub fn max_cost_usd_per_turn(&self) -> Option<f64> {
        self.max_cost_usd_cents_per_turn
            .map(|cents| f64::from(cents) / 100.0)
    }
}

/// Per-role adaptive-threshold overrides under `[agent.roles.<role>]`.
#[allow(clippy::derive_partial_eq_without_eq)] // contains f64
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentThresholds {
    /// Minimum pass-rate floor applied over adaptive gate thresholds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_pass_rate_floor: Option<f64>,
}

/// Per-role routing overrides under `[agent.roles.<role>]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingOverrides {
    /// Force routing to a specific backend/provider family when possible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_backend: Option<String>,
    /// Force routing to the configured model tier when possible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_tier: Option<String>,
}

// ---- CaMeL dual-LLM configuration (SAFE-07) ────────────────────────────

/// Configuration for the Data LLM in the CaMeL dual-LLM architecture.
///
/// The Data LLM processes untrusted external content (web fetches, plugin
/// output, user-provided files) with tool-call capability stripped. It
/// receives the untrusted content plus a schema for valid outputs, and
/// returns a structured extraction that is safe for the Control LLM.
///
/// Three defense layers:
/// 1. Input sanitization (strip known injection patterns)
/// 2. Data LLM isolation (no tools, schema-constrained output)
/// 3. Output validation (schema check + anomaly detection)
///
/// ```toml
/// [agent.data_llm]
/// model = "claude-haiku-3-5"
/// max_tokens = 4096
/// temperature = 0.0
/// strip_tool_calls = true
/// ```
#[allow(clippy::derive_partial_eq_without_eq)] // contains f64
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataLlmConfig {
    /// Model slug for the Data LLM (can be a smaller/cheaper model).
    #[serde(default = "default_data_llm_model")]
    pub model: String,

    /// Maximum tokens the Data LLM is allowed to generate.
    #[serde(default = "default_data_llm_max_tokens")]
    pub max_tokens: u64,

    /// Sampling temperature (0.0 for deterministic extraction).
    #[serde(default)]
    pub temperature: f64,

    /// Whether to strip tool-call capability from the Data LLM dispatch.
    ///
    /// When `true` (the default), the Data LLM cannot generate tool calls
    /// and can only produce text/JSON output.
    #[serde(default = "default_true")]
    pub strip_tool_calls: bool,

    /// Optional JSON Schema that the Data LLM output must conform to.
    ///
    /// When set, the router validates the Data LLM response against this
    /// schema before passing it to the Control LLM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,

    /// Whether to sanitize the input before sending to the Data LLM.
    ///
    /// When `true`, known prompt injection patterns are stripped from the
    /// untrusted content before it reaches the Data LLM.
    #[serde(default = "default_true")]
    pub sanitize_input: bool,
}

fn default_data_llm_model() -> String {
    "claude-haiku-3-5".into()
}

const fn default_data_llm_max_tokens() -> u64 {
    4096
}

impl Default for DataLlmConfig {
    fn default() -> Self {
        Self {
            model: default_data_llm_model(),
            max_tokens: default_data_llm_max_tokens(),
            temperature: 0.0,
            strip_tool_calls: true,
            output_schema: None,
            sanitize_input: true,
        }
    }
}

/// Per-role override under `[agent.roles.<role>]`.
///
/// Every field is optional; absent means "use the agent-level default".
#[allow(clippy::derive_partial_eq_without_eq)] // contains f32/f64 via nested fields
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleOverride {
    /// Explicit runtime role label override; defaults to the section name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Model slug override for this role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Backend override for this role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// Reasoning effort override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Temperament override for this role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperament: Option<Temperament>,
    /// Context window override (in thousands of tokens).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit_k: Option<u32>,
    /// Role-local tool whitelist; absent means no additional restriction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// Per-turn token and cost caps for this role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<AgentBudget>,
    /// Per-role adaptive-threshold overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<AgentThresholds>,
    /// Per-role routing overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_overrides: Option<RoutingOverrides>,
    /// Turn budget override (USD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_budget_usd: Option<f32>,
}

impl RoleOverride {
    /// Resolve the effective runtime role label for this config section.
    #[must_use]
    pub fn resolved_role_name<'a>(&'a self, section_name: &'a str) -> &'a str {
        self.role
            .as_deref()
            .map(str::trim)
            .filter(|role| !role.is_empty())
            .unwrap_or(section_name)
    }

    /// Return the effective per-turn budget, folding the legacy
    /// `turn_budget_usd` field into the nested `budget` block.
    #[must_use]
    pub fn effective_budget(&self) -> Option<AgentBudget> {
        let mut budget = self.budget.clone().unwrap_or_default();
        if budget.max_cost_usd_cents_per_turn.is_none() {
            budget.max_cost_usd_cents_per_turn =
                self.turn_budget_usd.and_then(usd_to_cents_per_turn);
        }
        (budget.max_tokens_per_turn.is_some() || budget.max_cost_usd_cents_per_turn.is_some())
            .then_some(budget)
    }

    /// Resolve the effective temperament for this role override.
    #[must_use]
    pub fn resolved_temperament(&self, default: Temperament) -> Temperament {
        self.temperament.unwrap_or(default)
    }
}

impl AgentConfig {
    /// Resolve the effective temperament for `role_label`.
    #[must_use]
    pub fn temperament_for_role(&self, role_label: &str) -> Temperament {
        self.roles
            .get(role_label)
            .map_or(self.temperament, |override_cfg| {
                override_cfg.resolved_temperament(self.temperament)
            })
    }
}

fn usd_to_cents_per_turn(usd: f32) -> Option<u32> {
    let usd = f64::from(usd);
    if !usd.is_finite() || usd.is_sign_negative() {
        return None;
    }
    let cents = (usd * 100.0).round();
    if cents > f64::from(u32::MAX) {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        Some(cents as u32)
    }
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
    /// Per-domain gate overrides. Keys are domain labels (e.g. "research", "docs"),
    /// values are shell commands to run as gates (e.g. `["shell:true"]`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub domain_gates: HashMap<String, Vec<String>>,
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
            domain_gates: HashMap::new(),
        }
    }
}

// ---- [pipeline] ---------------------------------------------------------

/// Reviewer composition for a pipeline band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PipelineReviewerMode {
    /// Single quick-pass reviewer.
    Quick,
    /// Full review suite (architect, auditor, scribe).
    Full,
}

impl PipelineReviewerMode {
    /// Stable config label used in TOML.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Full => "full",
        }
    }
}

impl Default for PipelineReviewerMode {
    fn default() -> Self {
        Self::Quick
    }
}

/// Effective pipeline settings for one complexity band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineBandConfig {
    /// Whether the strategist stage runs before implementation.
    #[serde(default)]
    pub strategist: bool,
    /// Whether reviewer agents run after implementation.
    #[serde(default)]
    pub reviewers: bool,
    /// Which reviewer composition to use when reviewers are enabled.
    #[serde(default)]
    pub reviewer_mode: PipelineReviewerMode,
    /// Maximum implementation-review iterations before stopping.
    #[serde(default = "default_pipeline_band_iterations")]
    pub max_iterations: u32,
}

const fn default_pipeline_band_iterations() -> u32 {
    1
}

impl PipelineBandConfig {
    /// Defaults for the `mechanical` tier.
    #[must_use]
    pub const fn mechanical() -> Self {
        Self {
            strategist: false,
            reviewers: false,
            reviewer_mode: PipelineReviewerMode::Quick,
            max_iterations: 1,
        }
    }

    /// Defaults for the `focused` tier.
    #[must_use]
    pub const fn focused() -> Self {
        Self {
            strategist: false,
            reviewers: false,
            reviewer_mode: PipelineReviewerMode::Quick,
            max_iterations: 2,
        }
    }

    /// Defaults for the `integrative` tier.
    #[must_use]
    pub const fn integrative() -> Self {
        Self {
            strategist: true,
            reviewers: true,
            reviewer_mode: PipelineReviewerMode::Quick,
            max_iterations: 2,
        }
    }

    /// Defaults for the `architectural` tier.
    #[must_use]
    pub const fn architectural() -> Self {
        Self {
            strategist: true,
            reviewers: true,
            reviewer_mode: PipelineReviewerMode::Full,
            max_iterations: 3,
        }
    }
}

impl Default for PipelineBandConfig {
    fn default() -> Self {
        Self::mechanical()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
struct PipelineBandConfigOverride {
    #[serde(default)]
    strategist: Option<bool>,
    #[serde(default)]
    reviewers: Option<bool>,
    #[serde(default)]
    reviewer_mode: Option<PipelineReviewerMode>,
    #[serde(default)]
    max_iterations: Option<u32>,
}

impl PipelineBandConfigOverride {
    fn resolve(self, defaults: PipelineBandConfig) -> PipelineBandConfig {
        PipelineBandConfig {
            strategist: self.strategist.unwrap_or(defaults.strategist),
            reviewers: self.reviewers.unwrap_or(defaults.reviewers),
            reviewer_mode: self.reviewer_mode.unwrap_or(defaults.reviewer_mode),
            max_iterations: self.max_iterations.unwrap_or(defaults.max_iterations),
        }
    }
}

fn deserialize_pipeline_band_with_defaults<'de, D>(
    deserializer: D,
    defaults: PipelineBandConfig,
) -> Result<PipelineBandConfig, D::Error>
where
    D: Deserializer<'de>,
{
    let override_cfg = PipelineBandConfigOverride::deserialize(deserializer)?;
    Ok(override_cfg.resolve(defaults))
}

fn default_mechanical_pipeline() -> PipelineBandConfig {
    PipelineBandConfig::mechanical()
}

fn deserialize_mechanical_pipeline<'de, D>(deserializer: D) -> Result<PipelineBandConfig, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_pipeline_band_with_defaults(deserializer, PipelineBandConfig::mechanical())
}

fn default_focused_pipeline() -> PipelineBandConfig {
    PipelineBandConfig::focused()
}

fn deserialize_focused_pipeline<'de, D>(deserializer: D) -> Result<PipelineBandConfig, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_pipeline_band_with_defaults(deserializer, PipelineBandConfig::focused())
}

fn default_integrative_pipeline() -> PipelineBandConfig {
    PipelineBandConfig::integrative()
}

fn deserialize_integrative_pipeline<'de, D>(deserializer: D) -> Result<PipelineBandConfig, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_pipeline_band_with_defaults(deserializer, PipelineBandConfig::integrative())
}

fn default_architectural_pipeline() -> PipelineBandConfig {
    PipelineBandConfig::architectural()
}

fn deserialize_architectural_pipeline<'de, D>(
    deserializer: D,
) -> Result<PipelineBandConfig, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_pipeline_band_with_defaults(deserializer, PipelineBandConfig::architectural())
}

/// Complexity-to-pipeline mapping.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Mechanical tasks: skip strategist and reviewers.
    #[serde(
        default = "default_mechanical_pipeline",
        deserialize_with = "deserialize_mechanical_pipeline"
    )]
    pub mechanical: PipelineBandConfig,
    /// Focused tasks: implement directly, allow one extra loop.
    #[serde(
        default = "default_focused_pipeline",
        deserialize_with = "deserialize_focused_pipeline"
    )]
    pub focused: PipelineBandConfig,
    /// Integrative tasks: strategist plus a quick reviewer.
    #[serde(
        default = "default_integrative_pipeline",
        deserialize_with = "deserialize_integrative_pipeline"
    )]
    pub integrative: PipelineBandConfig,
    /// Architectural tasks: strategist plus the full reviewer suite.
    #[serde(
        default = "default_architectural_pipeline",
        deserialize_with = "deserialize_architectural_pipeline"
    )]
    pub architectural: PipelineBandConfig,
}

impl PipelineConfig {
    /// Resolve the pipeline settings for a named complexity tier.
    #[must_use]
    pub fn for_tier(&self, tier: &str) -> PipelineBandConfig {
        match tier {
            "mechanical" => self.mechanical,
            "focused" => self.focused,
            "integrative" => self.integrative,
            "architectural" => self.architectural,
            _ => self.focused,
        }
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            mechanical: PipelineBandConfig::mechanical(),
            focused: PipelineBandConfig::focused(),
            integrative: PipelineBandConfig::integrative(),
            architectural: PipelineBandConfig::architectural(),
        }
    }
}

// ---- [routing] -----------------------------------------------------------

/// Routing algorithm for model selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingAlgorithm {
    /// Contextual bandit using upper-confidence bounds.
    LinUcb,
    /// Discounted Thompson sampling for non-stationary routing.
    Thompson,
}

impl RoutingAlgorithm {
    /// Stable config label used in TOML.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LinUcb => "linucb",
            Self::Thompson => "thompson",
        }
    }
}

impl Default for RoutingAlgorithm {
    fn default() -> Self {
        Self::LinUcb
    }
}

/// Reward weights used to scalarize quality, cost, and latency signals.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RewardWeights {
    /// Relative weight for quality / success.
    #[serde(default = "default_reward_weight_quality")]
    pub quality: f64,
    /// Relative weight for low cost.
    #[serde(default = "default_reward_weight_cost")]
    pub cost: f64,
    /// Relative weight for low latency.
    #[serde(default = "default_reward_weight_latency")]
    pub latency: f64,
}

const fn default_reward_weight_quality() -> f64 {
    0.5
}

const fn default_reward_weight_cost() -> f64 {
    0.3
}

const fn default_reward_weight_latency() -> f64 {
    0.2
}

impl Default for RewardWeights {
    fn default() -> Self {
        Self {
            quality: default_reward_weight_quality(),
            cost: default_reward_weight_cost(),
            latency: default_reward_weight_latency(),
        }
    }
}

/// Per-tier reward-weight overrides for routing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingRewardWeightsConfig {
    /// Default weights used when a tier has no explicit override.
    #[serde(flatten)]
    pub default: RewardWeights,
    /// Optional override for mechanical tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mechanical: Option<RewardWeights>,
    /// Optional override for focused tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused: Option<RewardWeights>,
    /// Optional override for integrative tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrative: Option<RewardWeights>,
    /// Optional override for architectural tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architectural: Option<RewardWeights>,
}

impl RoutingRewardWeightsConfig {
    /// Resolve the effective weights for a task tier.
    #[must_use]
    pub fn for_tier(&self, tier: &str) -> RewardWeights {
        match tier {
            "mechanical" => self.mechanical.unwrap_or(self.default),
            "focused" => self.focused.unwrap_or(self.default),
            "integrative" => self.integrative.unwrap_or(self.default),
            "architectural" => self.architectural.unwrap_or(self.default),
            _ => self.default,
        }
    }
}

impl Default for RoutingRewardWeightsConfig {
    fn default() -> Self {
        Self {
            default: RewardWeights::default(),
            mechanical: None,
            focused: None,
            integrative: None,
            architectural: None,
        }
    }
}

/// Model routing configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Routing mode (`"auto_override"`).
    #[serde(default = "default_routing_mode")]
    pub mode: String,
    /// Online learning algorithm used by the router.
    #[serde(default)]
    pub algorithm: RoutingAlgorithm,
    /// Discount factor for Thompson sampling in non-stationary environments.
    #[serde(default = "default_routing_discount_factor")]
    pub discount_factor: f64,
    /// Model for low-complexity tasks.
    #[serde(default = "default_fast_model")]
    pub fast_task_model: String,
    /// Model for standard-complexity tasks.
    #[serde(default = "default_standard_model")]
    pub standard_task_model: String,
    /// Model for high-complexity / retry tasks.
    #[serde(default = "default_complex_model")]
    pub complex_task_model: String,
    /// Reward scalarization weights with optional per-tier overrides.
    #[serde(default)]
    pub weights: RoutingRewardWeightsConfig,
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

const fn default_routing_discount_factor() -> f64 {
    0.99
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            mode: default_routing_mode(),
            algorithm: RoutingAlgorithm::default(),
            discount_factor: default_routing_discount_factor(),
            fast_task_model: default_fast_model(),
            standard_task_model: default_standard_model(),
            complex_task_model: default_complex_model(),
            weights: RoutingRewardWeightsConfig::default(),
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
    /// Whether repeated gate failures should trigger a plan revision.
    #[serde(default = "default_true")]
    pub replan_on_gate_failure: bool,
    /// Maximum number of gate-failure-triggered plan revisions per plan.
    #[serde(default = "default_replan_max_per_plan")]
    pub replan_max_per_plan: u32,
    /// Consecutive gate failures required before emitting a plan revision.
    #[serde(default = "default_replan_gate_attempts")]
    pub replan_gate_attempts: u32,
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

const fn default_replan_max_per_plan() -> u32 {
    2
}

const fn default_replan_gate_attempts() -> u32 {
    3
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
            replan_on_gate_failure: true,
            replan_max_per_plan: default_replan_max_per_plan(),
            replan_gate_attempts: default_replan_gate_attempts(),
        }
    }
}

// ---- [demurrage] ---------------------------------------------------------

/// Knowledge demurrage configuration.
///
/// Controls the Gesellian decay applied to playbook rules and knowledge
/// entries so that stale, unvalidated heuristics naturally fade.
#[allow(clippy::derive_partial_eq_without_eq)] // contains f64
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DemurrageConfig {
    /// Exponential decay rate per hour applied to knowledge entry balances.
    #[serde(default = "default_demurrage_rate_per_hour")]
    pub rate_per_hour: f64,
    /// Entries with balance below this threshold are deprioritized in retrieval.
    #[serde(default = "default_demurrage_min_balance")]
    pub min_balance: f64,
    /// Balance below which entries are frozen into cold storage.
    #[serde(default = "default_demurrage_freeze_threshold")]
    pub freeze_threshold: f64,
    /// Starting balance for thawed (resurrected) entries.
    #[serde(default = "default_demurrage_thaw_balance")]
    pub thaw_balance: f64,
    /// Maximum balance an entry can accumulate from reinforcement.
    #[serde(default = "default_demurrage_max_balance")]
    pub max_balance: f64,
    /// How often to run demurrage GC (in seconds, 0 = manual only).
    #[serde(default)]
    pub gc_interval_secs: u64,
    /// Per-kind rate multipliers (e.g., Warnings decay faster).
    /// Keys are knowledge kind strings ("warning", "insight", etc.).
    #[serde(default)]
    pub kind_rate_multipliers: std::collections::HashMap<String, f64>,
    /// Whether to freeze entries before deleting (true = preserve for resurrection).
    #[serde(default = "default_true")]
    pub freeze_before_delete: bool,
    /// Death threshold: entries with recency factor below this are considered dead.
    #[serde(default = "default_demurrage_death_threshold")]
    pub death_threshold: f64,
}

const fn default_demurrage_rate_per_hour() -> f64 {
    0.01
}

const fn default_demurrage_min_balance() -> f64 {
    0.1
}

const fn default_demurrage_freeze_threshold() -> f64 {
    0.05
}

const fn default_demurrage_thaw_balance() -> f64 {
    0.6
}

const fn default_demurrage_max_balance() -> f64 {
    5.0
}

const fn default_demurrage_death_threshold() -> f64 {
    0.01
}

impl Default for DemurrageConfig {
    fn default() -> Self {
        Self {
            rate_per_hour: default_demurrage_rate_per_hour(),
            min_balance: default_demurrage_min_balance(),
            freeze_threshold: default_demurrage_freeze_threshold(),
            thaw_balance: default_demurrage_thaw_balance(),
            max_balance: default_demurrage_max_balance(),
            gc_interval_secs: 0,
            kind_rate_multipliers: std::collections::HashMap::new(),
            freeze_before_delete: true,
            death_threshold: default_demurrage_death_threshold(),
        }
    }
}

// ---- [attention] ---------------------------------------------------------

/// Attention token budget allocation and context window management.
///
/// Controls how the runtime distributes token budget across prompt layers
/// and manages context window pressure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttentionConfig {
    /// Maximum tokens to allocate per prompt layer (0 = unlimited).
    #[serde(default = "default_attention_max_tokens_per_layer")]
    pub max_tokens_per_layer: usize,
    /// Context window utilization target as a fraction in `[0.0, 1.0]`.
    #[serde(default = "default_attention_utilization_target")]
    pub utilization_target: f64,
    /// Enable attention auction where layers bid for token budget.
    #[serde(default)]
    pub auction_enabled: bool,
    /// Minimum tokens reserved for task context regardless of auction.
    #[serde(default = "default_attention_task_reserve")]
    pub task_reserve_tokens: usize,
}

const fn default_attention_max_tokens_per_layer() -> usize {
    4096
}

const fn default_attention_utilization_target() -> f64 {
    0.85
}

const fn default_attention_task_reserve() -> usize {
    512
}

impl Default for AttentionConfig {
    fn default() -> Self {
        Self {
            max_tokens_per_layer: default_attention_max_tokens_per_layer(),
            utilization_target: default_attention_utilization_target(),
            auction_enabled: false,
            task_reserve_tokens: default_attention_task_reserve(),
        }
    }
}

// ---- [immune] ------------------------------------------------------------

/// Anomaly detection thresholds and quarantine settings.
///
/// Configures the cognitive immune system that detects anomalous outputs,
/// quarantines suspect results, and classifies taint levels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImmuneConfig {
    /// Anomaly score threshold above which outputs are quarantined.
    #[serde(default = "default_immune_quarantine_threshold")]
    pub quarantine_threshold: f64,
    /// Maximum number of quarantined items before triggering escalation.
    #[serde(default = "default_immune_max_quarantined")]
    pub max_quarantined: usize,
    /// Whether to auto-reject quarantined outputs or hold for review.
    #[serde(default)]
    pub auto_reject: bool,
    /// Taint classification levels: low, medium, high.
    #[serde(default = "default_immune_taint_levels")]
    pub taint_levels: Vec<String>,
}

const fn default_immune_quarantine_threshold() -> f64 {
    0.8
}

const fn default_immune_max_quarantined() -> usize {
    50
}

fn default_immune_taint_levels() -> Vec<String> {
    vec!["low".to_string(), "medium".to_string(), "high".to_string()]
}

impl Default for ImmuneConfig {
    fn default() -> Self {
        Self {
            quarantine_threshold: default_immune_quarantine_threshold(),
            max_quarantined: default_immune_max_quarantined(),
            auto_reject: false,
            taint_levels: default_immune_taint_levels(),
        }
    }
}

// ---- [temporal] ----------------------------------------------------------

/// Time horizon preferences and planning depth configuration.
///
/// Controls how deep the planner looks ahead and how temporal relations
/// between tasks are evaluated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalConfig {
    /// Maximum planning depth (number of future task levels to consider).
    #[serde(default = "default_temporal_max_depth")]
    pub max_depth: usize,
    /// Default epoch duration in seconds for batching temporal events.
    #[serde(default = "default_temporal_epoch_secs")]
    pub epoch_secs: u64,
    /// Whether to enforce Allen temporal relations between dependent tasks.
    #[serde(default = "default_true")]
    pub enforce_allen_relations: bool,
}

const fn default_temporal_max_depth() -> usize {
    5
}

const fn default_temporal_epoch_secs() -> u64 {
    3600
}

impl Default for TemporalConfig {
    fn default() -> Self {
        Self {
            max_depth: default_temporal_max_depth(),
            epoch_secs: default_temporal_epoch_secs(),
            enforce_allen_relations: true,
        }
    }
}

// ---- [goals] -------------------------------------------------------------

/// Goal hierarchy configuration with priority weights and completion criteria.
///
/// Controls how goals are ranked, pruned, and when they are considered complete.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoalsConfig {
    /// Maximum number of active goals at any level of the hierarchy.
    #[serde(default = "default_goals_max_active")]
    pub max_active: usize,
    /// Priority weight for correctness vs. speed tradeoff in `[0.0, 1.0]`.
    /// Higher values favor correctness.
    #[serde(default = "default_goals_correctness_weight")]
    pub correctness_weight: f64,
    /// Minimum completion ratio in `[0.0, 1.0]` for a goal to be considered done.
    #[serde(default = "default_goals_completion_threshold")]
    pub completion_threshold: f64,
    /// Prune goals with priority below this value.
    #[serde(default = "default_goals_prune_threshold")]
    pub prune_threshold: f64,
}

const fn default_goals_max_active() -> usize {
    10
}

const fn default_goals_correctness_weight() -> f64 {
    0.7
}

const fn default_goals_completion_threshold() -> f64 {
    0.95
}

const fn default_goals_prune_threshold() -> f64 {
    0.1
}

impl Default for GoalsConfig {
    fn default() -> Self {
        Self {
            max_active: default_goals_max_active(),
            correctness_weight: default_goals_correctness_weight(),
            completion_threshold: default_goals_completion_threshold(),
            prune_threshold: default_goals_prune_threshold(),
        }
    }
}

// ---- [energy] ------------------------------------------------------------

/// Compute budget and cost caps per model tier.
///
/// Controls how much compute budget is available and how costs are capped
/// across different model tiers (cheap, standard, premium).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnergyConfig {
    /// Total compute budget pool in USD.
    #[serde(default = "default_energy_pool_usd")]
    pub pool_usd: f64,
    /// Per-task cost cap in USD (0.0 = no cap).
    #[serde(default)]
    pub per_task_cap_usd: f64,
    /// Per-tier cost multipliers keyed by tier name (e.g., "cheap": 0.5).
    #[serde(default)]
    pub tier_caps: HashMap<String, f64>,
    /// Metabolism rate: fraction of budget replenished per hour.
    #[serde(default = "default_energy_metabolism_rate")]
    pub metabolism_rate: f64,
}

const fn default_energy_pool_usd() -> f64 {
    50.0
}

const fn default_energy_metabolism_rate() -> f64 {
    0.1
}

impl Default for EnergyConfig {
    fn default() -> Self {
        Self {
            pool_usd: default_energy_pool_usd(),
            per_task_cap_usd: 0.0,
            tier_caps: HashMap::new(),
            metabolism_rate: default_energy_metabolism_rate(),
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
    /// Automatically orchestrate follow-up work when publish events arrive.
    #[serde(default = "default_true")]
    pub auto_orchestrate: bool,
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
            auto_orchestrate: true,
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
    /// Engram kind emitted when the schedule fires.
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
    /// Engram kind glob used to match webhook signals.
    pub trigger: String,
    /// Typed trigger configuration (cron schedule, file-watch paths, or webhook URL).
    ///
    /// When set, this takes precedence over the plain `trigger` string for
    /// determining how the subscription fires. The `trigger` field is still
    /// used for signal matching in the dispatch loop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_config: Option<SubscriptionTrigger>,
    /// Optional repo / branch / path filters.
    #[serde(default, skip_serializing_if = "SubscriptionFilterConfig::is_empty")]
    pub filter: SubscriptionFilterConfig,
    /// Maximum number of concurrent dispatches for this subscription.
    #[serde(default = "default_subscription_concurrency_limit")]
    pub concurrency_limit: usize,
    /// Minimum interval between dispatches, in seconds.
    #[serde(default)]
    pub cooldown_secs: u64,
    /// Debounce window in milliseconds. Events arriving within this window
    /// after the first event are coalesced into a single dispatch.
    #[serde(default)]
    pub debounce_ms: u64,
    /// Whether the subscription is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for SubscriptionConfig {
    fn default() -> Self {
        Self {
            template: String::new(),
            trigger: String::new(),
            trigger_config: None,
            filter: SubscriptionFilterConfig::default(),
            concurrency_limit: default_subscription_concurrency_limit(),
            cooldown_secs: 0,
            debounce_ms: 0,
            enabled: default_true(),
        }
    }
}

fn default_subscription_concurrency_limit() -> usize {
    1
}

/// Typed trigger configuration for subscriptions.
///
/// Each variant corresponds to a distinct firing mechanism:
/// - `Cron` fires on a cron schedule (e.g., `*/30 * * * *`).
/// - `FileWatch` fires when watched paths change on disk.
/// - `Webhook` fires when a matching webhook payload arrives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubscriptionTrigger {
    /// Cron-schedule trigger.
    Cron {
        /// Standard cron expression (5 or 6 fields).
        schedule: String,
    },
    /// File-system watch trigger.
    FileWatch {
        /// Directories or file globs to watch.
        paths: Vec<String>,
        /// File-extension filter (e.g., `["rs", "toml"]`). Empty means all.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extensions: Vec<String>,
        /// Whether to watch recursively (default `true`).
        #[serde(default = "default_true")]
        recursive: bool,
    },
    /// Webhook trigger (matched against incoming webhook signals).
    Webhook {
        /// URL pattern or event type glob to match.
        event: String,
    },
}

impl SubscriptionTrigger {
    /// Return the trigger type as a string label.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Cron { .. } => "cron",
            Self::FileWatch { .. } => "file_watch",
            Self::Webhook { .. } => "webhook",
        }
    }
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

// ---- Gemini config -------------------------------------------------------

fn default_thinking_medium() -> String {
    "medium".to_string()
}

/// Gemini-specific model and request settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeminiConfig {
    /// Default model for standard Gemini chat requests.
    pub default_model: Option<String>,
    /// Default model for Gemini grounding requests.
    pub grounding_model: Option<String>,
    /// Default model for Gemini code execution requests.
    pub code_exec_model: Option<String>,
    /// Default Gemini embedding model.
    pub embed_model: Option<String>,
    /// Prefer the standard-tier free models when available.
    #[serde(default)]
    pub use_free_tier: bool,
    /// Gemini native thinking depth: "minimal", "low", "medium", or "high".
    #[serde(default = "default_thinking_medium")]
    pub thinking_level: String,
    /// Enable provider-side context caching when supported.
    #[serde(default)]
    pub enable_context_caching: bool,
    /// Per-category Gemini safety thresholds.
    #[serde(default)]
    pub safety_settings: Vec<SafetySetting>,
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            default_model: None,
            grounding_model: None,
            code_exec_model: None,
            embed_model: None,
            use_free_tier: false,
            thinking_level: default_thinking_medium(),
            enable_context_caching: false,
            safety_settings: Vec::new(),
        }
    }
}

/// Gemini native safety configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetySetting {
    /// Gemini harm category, e.g. `HARM_CATEGORY_HATE_SPEECH`.
    pub category: String,
    /// Gemini blocking threshold, e.g. `BLOCK_NONE`.
    pub threshold: String,
}

// ---- Perplexity config ---------------------------------------------------

fn default_recency() -> String {
    "year".to_string()
}

/// Perplexity-specific search and model settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerplexityConfig {
    /// Default model for search-grounded queries.
    pub default_search_model: Option<String>,
    /// Default model for deep research tasks.
    pub default_research_model: Option<String>,
    /// Default model for reasoning tasks.
    pub default_reasoning_model: Option<String>,
    /// Default model for embeddings.
    pub default_embed_model: Option<String>,
    /// Recency filter applied to web search: "hour"/"day"/"week"/"month"/"year".
    #[serde(default = "default_recency")]
    pub search_recency_filter: String,
    /// Restrict results to academic sources.
    #[serde(default)]
    pub academic_mode: bool,
    /// Global domain allowlist for web search.
    #[serde(default)]
    pub search_domain_filter: Vec<String>,
    /// Include images in search results.
    #[serde(default)]
    pub return_images: bool,
    /// Include related questions in search results.
    #[serde(default = "default_true")]
    pub return_related_questions: bool,
}

impl Default for PerplexityConfig {
    fn default() -> Self {
        Self {
            default_search_model: None,
            default_research_model: None,
            default_reasoning_model: None,
            default_embed_model: None,
            search_recency_filter: default_recency(),
            academic_mode: false,
            search_domain_filter: Vec::new(),
            return_images: false,
            return_related_questions: true,
        }
    }
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
            String::from_utf8(self.inner.lock().expect("lock log buffer").clone())
                .expect("log output should be utf-8")
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
            self.inner
                .lock()
                .expect("lock log writer")
                .extend_from_slice(buf);
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
        assert!(logs.contains("roko.toml uses config version 1 (no [providers] section)"));
        assert!(logs.contains("hint: run `roko config migrate` to upgrade"));
    }

    #[test]
    fn config_version_detection_is_silent_for_current_configs() {
        let (cfg, logs) = capture_warn_logs(|| {
            RokoConfig::from_toml(
                r#"
config_version = 2
schema_version = 2

[agent]
default_model = "glm-5-1"

[providers.zai]
kind = "openai_compat"
base_url = "https://api.z.ai/api/paas/v4"
api_key_env = "ZAI_API_KEY"

[models.glm-5-1]
provider = "zai"
slug = "glm-5.1"
"#,
            )
            .expect("parse")
        });

        assert_eq!(cfg.config_version, 2);
        assert!(
            logs.trim().is_empty(),
            "expected no deprecation warning, got `{logs}`"
        );
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
        assert_eq!(zai.ttft_timeout_ms, Some(15_000));
        assert_eq!(zai.connect_timeout_ms, Some(5_000));
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
        assert_eq!(moonshot.timeout_ms, Some(120_000));
        assert_eq!(moonshot.ttft_timeout_ms, Some(15_000));
        assert_eq!(moonshot.connect_timeout_ms, Some(5_000));

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
        assert_eq!(claude.ttft_timeout_ms, Some(15_000));
        assert_eq!(claude.connect_timeout_ms, Some(5_000));

        let ollama = cfg.providers.get("ollama").expect("ollama provider");
        assert_eq!(ollama.kind, ProviderKind::OpenAiCompat);
        assert_eq!(ollama.base_url.as_deref(), Some("http://localhost:11434"));
        assert_eq!(ollama.timeout_ms, Some(120_000));
        assert_eq!(ollama.ttft_timeout_ms, Some(15_000));
        assert_eq!(ollama.connect_timeout_ms, Some(5_000));
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
        // When roko.toml has explicit [providers.*] entries, effective_providers
        // returns them directly rather than synthesizing from [agent].
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../roko.toml");
        let text = std::fs::read_to_string(path).expect("read roko.toml");
        let cfg = RokoConfig::from_toml(&text).expect("parse roko.toml");
        let providers = cfg.effective_providers();

        let claude = providers.get("claude_cli").expect("claude_cli provider");
        assert_eq!(claude.kind, ProviderKind::ClaudeCli);
        assert_eq!(claude.command.as_deref(), Some("claude"));
    }

    #[test]
    fn effective_providers_synthesized_from_agent_section() {
        // When no explicit [providers] exist, effective_providers synthesizes
        // from the legacy [agent] section.
        let toml = r#"
[agent]
command = "claude"
args = ["--print", "--output-format", "stream-json", "--dangerously-skip-permissions"]
timeout_ms = 300000
env = [
  ["ANTHROPIC_BASE_URL", "http://127.0.0.1:4000"],
  ["ANTHROPIC_API_KEY", "mori-local-gateway"],
]
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
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
        assert_eq!(claude.ttft_timeout_ms, Some(15_000));
        assert_eq!(claude.connect_timeout_ms, Some(5_000));

        let anthropic = providers.get("anthropic").expect("anthropic provider");
        assert_eq!(anthropic.kind, ProviderKind::AnthropicApi);
        assert_eq!(anthropic.base_url.as_deref(), Some("http://127.0.0.1:4000"));
        assert_eq!(anthropic.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
        assert_eq!(anthropic.timeout_ms, Some(300_000));
        assert_eq!(anthropic.ttft_timeout_ms, Some(15_000));
        assert_eq!(anthropic.connect_timeout_ms, Some(5_000));
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
ttft_timeout_ms = 15000
connect_timeout_ms = 5000
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
        assert_eq!(cfg.ttft_timeout_ms, Some(15000));
        assert_eq!(cfg.connect_timeout_ms, Some(5000));
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

        let api_key_env = std::env::var("ROKO_API_KEY_ENV_NAME").expect("api key env name");
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
    fn provider_timeouts_default_when_omitted() {
        let toml = r#"
kind = "openai_compat"
"#;
        let cfg = toml::from_str::<ProviderConfig>(toml).expect("parse");

        assert_eq!(cfg.timeout_ms, Some(120_000));
        assert_eq!(cfg.ttft_timeout_ms, Some(15_000));
        assert_eq!(cfg.connect_timeout_ms, Some(5_000));
    }

    #[test]
    fn provider_timeouts_allow_per_provider_overrides() {
        let toml = r#"
kind = "openai_compat"
timeout_ms = 240000
ttft_timeout_ms = 25000
connect_timeout_ms = 8000
"#;
        let cfg = toml::from_str::<ProviderConfig>(toml).expect("parse");

        assert_eq!(cfg.timeout_ms, Some(240_000));
        assert_eq!(cfg.ttft_timeout_ms, Some(25_000));
        assert_eq!(cfg.connect_timeout_ms, Some(8_000));
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
        assert!(!cfg.supports_grounding);
        assert!(!cfg.supports_code_execution);
        assert!(!cfg.supports_caching);
        assert_eq!(cfg.tool_format, "openai_json");
        assert_eq!(cfg.cost_input_per_m, None);
        assert_eq!(cfg.cost_output_per_m, None);
        assert_eq!(cfg.cost_input_per_m_high, None);
        assert_eq!(cfg.cost_output_per_m_high, None);
        assert_eq!(cfg.cost_cache_read_per_m, None);
        assert_eq!(cfg.cost_cache_write_per_m, None);
        assert_eq!(cfg.thinking_level, None);
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
temperament = "balanced"

[agent.roles.implementer]
role = "code_implementer"
model = "claude-sonnet-4-6"
effort = "max"
temperament = "exploratory"
tools = ["read_file", "git-*"]
context_limit_k = 300
budget = { max_tokens_per_turn = 12000, max_cost_usd_cents_per_turn = 550 }
thresholds = { gate_pass_rate_floor = 0.72 }
routing_overrides = { force_backend = "claude", force_tier = "focused" }

[agent.roles.architect]
model = "claude-opus-4-6"
turn_budget_usd = 5.0
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(cfg.agent.default_model, "claude-opus-4-6");
        assert_eq!(cfg.agent.default_effort, "high");
        assert_eq!(cfg.agent.temperament, Temperament::Balanced);

        let imp = cfg.agent.roles.get("implementer").expect("implementer");
        assert_eq!(imp.role.as_deref(), Some("code_implementer"));
        assert_eq!(imp.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(imp.effort.as_deref(), Some("max"));
        assert_eq!(imp.temperament, Some(Temperament::Exploratory));
        assert_eq!(
            imp.tools.as_deref(),
            Some(&["read_file".to_string(), "git-*".to_string()][..])
        );
        assert_eq!(imp.context_limit_k, Some(300));
        assert_eq!(
            imp.effective_budget(),
            Some(AgentBudget {
                max_tokens_per_turn: Some(12_000),
                max_cost_usd_cents_per_turn: Some(550),
            })
        );
        assert_eq!(
            imp.thresholds
                .as_ref()
                .and_then(|thresholds| thresholds.gate_pass_rate_floor),
            Some(0.72)
        );
        assert_eq!(
            imp.routing_overrides
                .as_ref()
                .and_then(|routing| routing.force_backend.as_deref()),
            Some("claude")
        );
        assert_eq!(
            cfg.agent.temperament_for_role("implementer"),
            Temperament::Exploratory
        );

        let arch = cfg.agent.roles.get("architect").expect("architect");
        assert_eq!(arch.model.as_deref(), Some("claude-opus-4-6"));
        assert!((arch.turn_budget_usd.expect("budget") - 5.0).abs() < f32::EPSILON);
        assert_eq!(
            cfg.agent.temperament_for_role("architect"),
            Temperament::Balanced
        );
        assert_eq!(
            arch.effective_budget(),
            Some(AgentBudget {
                max_tokens_per_turn: None,
                max_cost_usd_cents_per_turn: Some(500),
            })
        );
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
    fn routing_algorithm_config() {
        let default_cfg = RokoConfig::from_toml("").expect("parse defaults");
        assert_eq!(default_cfg.routing.algorithm, RoutingAlgorithm::LinUcb);
        assert!((default_cfg.routing.discount_factor - 0.99).abs() < f64::EPSILON);
        assert_eq!(
            default_cfg.routing.weights.default,
            RewardWeights::default()
        );

        let thompson_toml = r#"
[routing]
algorithm = "thompson"
discount_factor = 0.95
"#;
        let thompson_cfg = RokoConfig::from_toml(thompson_toml).expect("parse");
        assert_eq!(thompson_cfg.routing.algorithm, RoutingAlgorithm::Thompson);
        assert!((thompson_cfg.routing.discount_factor - 0.95).abs() < f64::EPSILON);

        let linucb_toml = r#"
[routing]
algorithm = "linucb"
"#;
        let linucb_cfg = RokoConfig::from_toml(linucb_toml).expect("parse");
        assert_eq!(linucb_cfg.routing.algorithm, RoutingAlgorithm::LinUcb);
    }

    #[test]
    fn pipeline_config_parses_complexity_mapping() {
        let default_cfg = RokoConfig::from_toml("").expect("parse defaults");
        assert_eq!(
            default_cfg.pipeline.mechanical,
            PipelineBandConfig::mechanical()
        );
        assert_eq!(default_cfg.pipeline.focused, PipelineBandConfig::focused());
        assert_eq!(
            default_cfg.pipeline.integrative,
            PipelineBandConfig::integrative()
        );
        assert_eq!(
            default_cfg.pipeline.architectural,
            PipelineBandConfig::architectural()
        );

        let toml = r#"
[pipeline.mechanical]
strategist = false
reviewers = false
max_iterations = 1

[pipeline.focused]
strategist = false
reviewers = false
max_iterations = 2

[pipeline.integrative]
strategist = true
reviewers = true
reviewer_mode = "quick"
max_iterations = 2

[pipeline.architectural]
strategist = true
reviewers = true
reviewer_mode = "full"
max_iterations = 3
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");

        assert_eq!(cfg.pipeline.mechanical, PipelineBandConfig::mechanical());
        assert_eq!(cfg.pipeline.focused, PipelineBandConfig::focused());
        assert_eq!(cfg.pipeline.integrative, PipelineBandConfig::integrative());
        assert_eq!(
            cfg.pipeline.architectural,
            PipelineBandConfig::architectural()
        );

        let mechanical = cfg.pipeline.for_tier("mechanical");
        assert!(!mechanical.strategist);
        assert!(!mechanical.reviewers);
        assert_eq!(mechanical.max_iterations, 1);
    }

    #[test]
    fn pipeline_config_partial_override_keeps_band_defaults() {
        let toml = r#"
[pipeline.architectural]
max_iterations = 4
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");

        assert_eq!(
            cfg.pipeline.architectural,
            PipelineBandConfig {
                strategist: true,
                reviewers: true,
                reviewer_mode: PipelineReviewerMode::Full,
                max_iterations: 4,
            }
        );
    }

    #[test]
    fn routing_reward_weights_config() {
        let toml = r#"
[routing.weights]
quality = 0.5
cost = 0.3
latency = 0.2

[routing.weights.mechanical]
quality = 0.3
cost = 0.6
latency = 0.1

[routing.weights.architectural]
quality = 0.8
cost = 0.1
latency = 0.1
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");

        assert_eq!(
            cfg.routing.weights.default,
            RewardWeights {
                quality: 0.5,
                cost: 0.3,
                latency: 0.2,
            }
        );
        assert_eq!(
            cfg.routing.weights.for_tier("mechanical"),
            RewardWeights {
                quality: 0.3,
                cost: 0.6,
                latency: 0.1,
            }
        );
        assert_eq!(
            cfg.routing.weights.for_tier("architectural"),
            RewardWeights {
                quality: 0.8,
                cost: 0.1,
                latency: 0.1,
            }
        );
        assert_eq!(
            cfg.routing.weights.for_tier("focused"),
            RewardWeights::default()
        );
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
replan_on_gate_failure = false
replan_max_per_plan = 4
replan_gate_attempts = 6
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert!(!cfg.learning.auto_playbook_refresh);
        assert_eq!(cfg.learning.learning_min_occurrences, 5);
        assert!(!cfg.learning.replan_on_gate_failure);
        assert_eq!(cfg.learning.replan_max_per_plan, 4);
        assert_eq!(cfg.learning.replan_gate_attempts, 6);
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
[serve]
auto_orchestrate = false

[serve.auth]
enabled = true
api_key = "secret"
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert!(!cfg.serve.auto_orchestrate);
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
        assert!(example.contains("[pipeline.mechanical]"));
        assert!(example.contains("[budget]"));
        assert!(example.contains("[conductor]"));
        assert!(example.contains("[learning]"));
        assert!(example.contains("[demurrage]"));
        assert!(example.contains("[attention]"));
        assert!(example.contains("[immune]"));
        assert!(example.contains("[temporal]"));
        assert!(example.contains("[goals]"));
        assert!(example.contains("[energy]"));
        assert!(example.contains("[tui]"));
        assert!(example.contains("[serve]"));
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
                max_output: None,
                supports_tools: true,
                supports_thinking: true,
                supports_vision: false,
                supports_web_search: false,
                supports_mcp_tools: false,
                supports_partial: false,
                provider_routing: None,
                tool_format: "openai_json".to_string(),
                cost_input_per_m: None,
                cost_output_per_m: None,
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                max_tools: None,
                tokenizer_ratio: None,
                ..Default::default()
            },
        );

        let warnings = validate_references(&cfg);

        assert_eq!(
            warnings,
            vec![ValidationWarning::UnknownProvider {
                model: "glm-5-1".to_string(),
                provider: "openruoter".to_string(),
                similar: Some("openrouter".to_string()),
            }]
        );
        assert_eq!(
            warnings[0].to_string(),
            "Model 'glm-5-1' references missing provider 'openruoter' (did you mean 'openrouter'?)"
        );
    }

    #[test]
    fn validate_references_warns_on_unknown_fallback_model() {
        let mut cfg = RokoConfig::default();
        cfg.models.insert(
            "glm-5-1".to_string(),
            ModelProfile {
                provider: "claude_cli".to_string(),
                slug: "glm-5.1".to_string(),
                context_window: 200_000,
                max_output: None,
                supports_tools: true,
                supports_thinking: false,
                supports_vision: false,
                supports_web_search: false,
                supports_mcp_tools: false,
                supports_partial: false,
                provider_routing: None,
                tool_format: "openai_json".to_string(),
                cost_input_per_m: None,
                cost_output_per_m: None,
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                max_tools: None,
                tokenizer_ratio: None,
                ..Default::default()
            },
        );
        cfg.agent.fallback_model = Some("missing-model".to_string());

        let warnings = validate_references(&cfg);

        assert!(warnings.contains(&ValidationWarning::UnknownModel {
            field: "agent.fallback_model".to_string(),
            model: "missing-model".to_string(),
        }));
    }

    #[test]
    fn validate_references_allows_legacy_fallback_model() {
        let mut cfg = RokoConfig::default();
        cfg.agent.default_model = "claude-sonnet-4-6".to_string();
        cfg.agent
            .tier_models
            .insert("mechanical".to_string(), "claude-haiku-4-5".to_string());
        cfg.agent.fallback_model = Some("claude-haiku-4-5".to_string());

        let warnings = validate_references(&cfg);

        assert!(
            warnings.is_empty(),
            "expected no warnings, got {warnings:?}"
        );
    }

    #[test]
    fn role_override_absent_fields_are_none() {
        let toml = r#"
[agent.roles.implementer]
model = "opus"
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        let imp = cfg.agent.roles.get("implementer").expect("role");
        assert!(imp.role.is_none());
        assert_eq!(imp.model.as_deref(), Some("opus"));
        assert!(imp.effort.is_none());
        assert!(imp.backend.is_none());
        assert!(imp.context_limit_k.is_none());
        assert!(imp.budget.is_none());
        assert!(imp.thresholds.is_none());
        assert!(imp.routing_overrides.is_none());
        assert!(imp.turn_budget_usd.is_none());
    }

    #[test]
    fn perplexity_config_defaults() {
        let cfg = RokoConfig::from_toml("").expect("parse empty");
        assert_eq!(cfg.perplexity.search_recency_filter, "year");
        assert!(!cfg.perplexity.academic_mode);
        assert!(cfg.perplexity.search_domain_filter.is_empty());
        assert!(!cfg.perplexity.return_images);
        assert!(cfg.perplexity.return_related_questions);
        assert!(cfg.perplexity.default_search_model.is_none());
    }

    #[test]
    fn perplexity_config_section_parses() {
        let toml = r#"
[perplexity]
default_search_model = "sonar"
default_research_model = "sonar-deep-research"
search_recency_filter = "week"
academic_mode = true
search_domain_filter = ["arxiv.org", "nature.com"]
return_images = true
return_related_questions = false
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(
            cfg.perplexity.default_search_model.as_deref(),
            Some("sonar")
        );
        assert_eq!(
            cfg.perplexity.default_research_model.as_deref(),
            Some("sonar-deep-research")
        );
        assert_eq!(cfg.perplexity.search_recency_filter, "week");
        assert!(cfg.perplexity.academic_mode);
        assert_eq!(
            cfg.perplexity.search_domain_filter,
            vec!["arxiv.org", "nature.com"]
        );
        assert!(cfg.perplexity.return_images);
        assert!(!cfg.perplexity.return_related_questions);
    }

    #[test]
    fn perplexity_example_config() {
        let example = include_str!("../../../../examples/roko-perplexity.toml");
        let cfg = RokoConfig::from_toml(example).expect("roko-perplexity example should parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);

        // Perplexity provider
        let pplx = cfg
            .providers
            .get("perplexity")
            .expect("perplexity provider");
        assert_eq!(pplx.kind, ProviderKind::PerplexityApi);
        assert_eq!(pplx.base_url.as_deref(), Some("https://api.perplexity.ai"));
        assert_eq!(pplx.api_key_env.as_deref(), Some("PERPLEXITY_API_KEY"));

        // Claude CLI provider
        let claude = cfg
            .providers
            .get("claude_cli")
            .expect("claude_cli provider");
        assert_eq!(claude.kind, ProviderKind::ClaudeCli);

        // Sonar models resolve to the perplexity provider
        for model_key in ["sonar", "sonar-pro", "sonar-deep-research"] {
            let model = cfg.models.get(model_key).expect(model_key);
            assert_eq!(
                model.provider, "perplexity",
                "{model_key} should use perplexity provider"
            );
            assert!(model.supports_search, "{model_key} should support search");
            assert!(
                model.supports_citations,
                "{model_key} should support citations"
            );
        }

        // Claude Opus resolves to claude_cli provider
        let claude_opus = cfg.models.get("claude-opus").expect("claude-opus model");
        assert_eq!(claude_opus.provider, "claude_cli");
        assert_eq!(claude_opus.slug, "claude-opus-4-6");
        assert!(claude_opus.supports_tools);

        // sonar-deep-research has async support
        let deep = cfg
            .models
            .get("sonar-deep-research")
            .expect("sonar-deep-research");
        assert!(deep.supports_async);

        // Perplexity search config
        assert_eq!(
            cfg.perplexity.default_search_model.as_deref(),
            Some("sonar")
        );
        assert_eq!(
            cfg.perplexity.default_research_model.as_deref(),
            Some("sonar-pro")
        );
        assert!(cfg.perplexity.academic_mode);
        assert_eq!(cfg.perplexity.search_recency_filter, "year");
        assert!(cfg.perplexity.return_related_questions);

        // Role overrides
        let researcher = cfg.agent.roles.get("researcher").expect("researcher role");
        assert_eq!(researcher.model.as_deref(), Some("sonar-pro"));
        let fact_checker = cfg
            .agent
            .roles
            .get("fact_checker")
            .expect("fact_checker role");
        assert_eq!(fact_checker.model.as_deref(), Some("sonar"));
    }

    #[test]
    fn gemini_config_defaults() {
        let cfg = RokoConfig::from_toml("").expect("parse empty");
        assert!(cfg.gemini.default_model.is_none());
        assert!(cfg.gemini.grounding_model.is_none());
        assert!(cfg.gemini.code_exec_model.is_none());
        assert!(cfg.gemini.embed_model.is_none());
        assert!(!cfg.gemini.use_free_tier);
        assert_eq!(cfg.gemini.thinking_level, "medium");
        assert!(!cfg.gemini.enable_context_caching);
        assert!(cfg.gemini.safety_settings.is_empty());
    }

    #[test]
    fn gemini_config_section_parses() {
        let toml = r#"
[gemini]
default_model = "gemini-2.5-flash"
grounding_model = "gemini-3-flash-preview"
code_exec_model = "gemini-2.5-pro"
embed_model = "gemini-embedding-2-preview"
use_free_tier = true
thinking_level = "high"
enable_context_caching = true

[[gemini.safety_settings]]
category = "HARM_CATEGORY_HATE_SPEECH"
threshold = "BLOCK_NONE"

[[gemini.safety_settings]]
category = "HARM_CATEGORY_HARASSMENT"
threshold = "BLOCK_LOW_AND_ABOVE"
"#;
        let cfg = RokoConfig::from_toml(toml).expect("parse");
        assert_eq!(
            cfg.gemini.default_model.as_deref(),
            Some("gemini-2.5-flash")
        );
        assert_eq!(
            cfg.gemini.grounding_model.as_deref(),
            Some("gemini-3-flash-preview")
        );
        assert_eq!(
            cfg.gemini.code_exec_model.as_deref(),
            Some("gemini-2.5-pro")
        );
        assert_eq!(
            cfg.gemini.embed_model.as_deref(),
            Some("gemini-embedding-2-preview")
        );
        assert!(cfg.gemini.use_free_tier);
        assert_eq!(cfg.gemini.thinking_level, "high");
        assert!(cfg.gemini.enable_context_caching);
        assert_eq!(cfg.gemini.safety_settings.len(), 2);
        assert_eq!(
            cfg.gemini.safety_settings[0],
            SafetySetting {
                category: "HARM_CATEGORY_HATE_SPEECH".to_string(),
                threshold: "BLOCK_NONE".to_string(),
            }
        );
        assert_eq!(
            cfg.gemini.safety_settings[1],
            SafetySetting {
                category: "HARM_CATEGORY_HARASSMENT".to_string(),
                threshold: "BLOCK_LOW_AND_ABOVE".to_string(),
            }
        );
    }

    #[test]
    fn gemini_example_config() {
        let example = include_str!("../../../../examples/roko-gemini.toml");
        let cfg = RokoConfig::from_toml(example).expect("roko-gemini example should parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);

        let provider = cfg.providers.get("gemini").expect("gemini provider");
        assert_eq!(provider.kind, ProviderKind::GeminiApi);
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://generativelanguage.googleapis.com")
        );
        assert_eq!(provider.api_key_env.as_deref(), Some("GEMINI_API_KEY"));

        assert_eq!(cfg.agent.default_model, "gemini-2-5-flash");
        assert_eq!(
            cfg.agent.tier_models.get("mechanical").map(String::as_str),
            Some("gemini-2-5-flash-lite")
        );
        assert_eq!(
            cfg.agent
                .tier_models
                .get("architectural")
                .map(String::as_str),
            Some("gemini-3-1-pro")
        );

        assert_eq!(
            cfg.gemini.default_model.as_deref(),
            Some("gemini-2-5-flash")
        );
        assert_eq!(
            cfg.gemini.grounding_model.as_deref(),
            Some("gemini-3-flash")
        );
        assert_eq!(
            cfg.gemini.code_exec_model.as_deref(),
            Some("gemini-2-5-pro")
        );
        assert_eq!(
            cfg.gemini.embed_model.as_deref(),
            Some("gemini-embedding-2")
        );
        assert!(cfg.gemini.use_free_tier);
        assert_eq!(cfg.gemini.thinking_level, "medium");
        assert!(cfg.gemini.enable_context_caching);
        assert_eq!(cfg.gemini.safety_settings.len(), 2);

        let expected = [
            ("gemini-2-5-flash-lite", "gemini-2.5-flash-lite"),
            ("gemini-2-5-flash", "gemini-2.5-flash"),
            ("gemini-2-5-pro", "gemini-2.5-pro"),
            ("gemini-3-1-pro", "gemini-3.1-pro-preview"),
            ("gemini-3-flash", "gemini-3-flash-preview"),
            ("gemini-3-1-flash-lite", "gemini-3.1-flash-lite-preview"),
            ("gemini-embedding-2", "gemini-embedding-2-preview"),
        ];

        for (model_key, slug) in expected {
            let model = cfg.models.get(model_key).expect(model_key);
            assert_eq!(model.provider, "gemini");
            assert_eq!(model.slug, slug);

            let resolved = crate::agent::resolve_model(&cfg, model_key);
            assert_eq!(resolved.model_key, model_key);
            assert_eq!(resolved.slug, slug);
            assert_eq!(resolved.provider_kind, ProviderKind::GeminiApi);
            assert!(resolved.provider_config.is_some());
            assert!(resolved.profile.is_some());
        }

        let flash_lite = cfg
            .models
            .get("gemini-2-5-flash-lite")
            .expect("flash-lite model");
        assert_eq!(flash_lite.tool_format, "openai_json");
        assert!(flash_lite.supports_caching);
        assert!(!flash_lite.supports_grounding);
        assert!(!flash_lite.supports_code_execution);

        let pro = cfg.models.get("gemini-2-5-pro").expect("pro model");
        assert_eq!(pro.tool_format, "gemini_native");
        assert!(pro.supports_grounding);
        assert!(pro.supports_code_execution);
        assert_eq!(pro.cost_input_per_m_high, Some(2.50));
        assert_eq!(pro.cost_output_per_m_high, Some(15.00));

        let pro_31 = cfg.models.get("gemini-3-1-pro").expect("3.1 pro model");
        assert_eq!(pro_31.thinking_level.as_deref(), Some("dynamic"));

        let embed = cfg
            .models
            .get("gemini-embedding-2")
            .expect("embedding model");
        assert!(embed.is_embedding_model);
        assert!(!embed.supports_tools);
    }

    #[test]
    fn multi_provider_config() {
        let example = include_str!("../../../../examples/roko-multi-provider.toml");
        let cfg = RokoConfig::from_toml(example).expect("roko-multi-provider example should parse");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);

        let claude = cfg
            .providers
            .get("claude_cli")
            .expect("claude_cli provider");
        assert_eq!(claude.kind, ProviderKind::ClaudeCli);
        assert_eq!(claude.command.as_deref(), Some("claude"));

        let gemini = cfg.providers.get("gemini").expect("gemini provider");
        assert_eq!(gemini.kind, ProviderKind::GeminiApi);
        assert_eq!(
            gemini.base_url.as_deref(),
            Some("https://generativelanguage.googleapis.com")
        );
        assert_eq!(gemini.api_key_env.as_deref(), Some("GEMINI_API_KEY"));

        let perplexity = cfg
            .providers
            .get("perplexity")
            .expect("perplexity provider");
        assert_eq!(perplexity.kind, ProviderKind::PerplexityApi);
        assert_eq!(
            perplexity.base_url.as_deref(),
            Some("https://api.perplexity.ai")
        );
        assert_eq!(
            perplexity.api_key_env.as_deref(),
            Some("PERPLEXITY_API_KEY")
        );

        assert_eq!(cfg.agent.default_model, "claude-opus");
        assert_eq!(
            cfg.agent.tier_models.get("mechanical").map(String::as_str),
            Some("gemini-2-5-flash-lite")
        );
        assert_eq!(
            cfg.agent.tier_models.get("focused").map(String::as_str),
            Some("gemini-2-5-flash")
        );
        assert_eq!(
            cfg.agent.tier_models.get("integrative").map(String::as_str),
            Some("claude-opus")
        );
        assert_eq!(
            cfg.agent
                .tier_models
                .get("architectural")
                .map(String::as_str),
            Some("claude-opus")
        );

        let researcher = cfg.agent.roles.get("researcher").expect("researcher role");
        assert_eq!(researcher.model.as_deref(), Some("sonar-pro"));
        let fact_checker = cfg
            .agent
            .roles
            .get("fact_checker")
            .expect("fact_checker role");
        assert_eq!(fact_checker.model.as_deref(), Some("sonar"));

        let expected = [
            ("claude-opus", ProviderKind::ClaudeCli, "claude-opus-4-6"),
            (
                "gemini-2-5-flash-lite",
                ProviderKind::GeminiApi,
                "gemini-2.5-flash-lite",
            ),
            (
                "gemini-2-5-flash",
                ProviderKind::GeminiApi,
                "gemini-2.5-flash",
            ),
            ("sonar-pro", ProviderKind::PerplexityApi, "sonar-pro"),
            ("sonar", ProviderKind::PerplexityApi, "sonar"),
        ];

        for (model_key, provider_kind, slug) in expected {
            let resolved = crate::agent::resolve_model(&cfg, model_key);
            assert_eq!(resolved.model_key, model_key);
            assert_eq!(resolved.slug, slug);
            assert_eq!(resolved.provider_kind, provider_kind);
            assert!(resolved.provider_config.is_some());
            assert!(resolved.profile.is_some());
        }

        assert_eq!(
            cfg.gemini.default_model.as_deref(),
            Some("gemini-2-5-flash")
        );
        assert_eq!(
            cfg.gemini.grounding_model.as_deref(),
            Some("gemini-2-5-flash")
        );
        assert_eq!(cfg.gemini.thinking_level, "medium");
        assert!(cfg.gemini.use_free_tier);

        assert_eq!(
            cfg.perplexity.default_search_model.as_deref(),
            Some("sonar")
        );
        assert_eq!(
            cfg.perplexity.default_research_model.as_deref(),
            Some("sonar-pro")
        );
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
    fn interpolate_vars_expands_env_references() {
        let env_fn = |key: &str| -> Option<String> {
            match key {
                "API_KEY" => Some("sk-secret-123".to_string()),
                "BASE_URL" => Some("https://api.example.com".to_string()),
                _ => None,
            }
        };
        assert_eq!(interpolate_vars("${API_KEY}", &env_fn), "sk-secret-123");
        assert_eq!(
            interpolate_vars("Bearer ${API_KEY}", &env_fn),
            "Bearer sk-secret-123"
        );
        assert_eq!(
            interpolate_vars("${BASE_URL}/v1", &env_fn),
            "https://api.example.com/v1"
        );
        // Missing var expands to empty string.
        assert_eq!(interpolate_vars("${MISSING_VAR}", &env_fn), "");
        // No ${} means no change.
        assert_eq!(interpolate_vars("plain text", &env_fn), "plain text");
    }

    #[test]
    fn interpolate_env_vars_with_resolves_provider_strings() {
        let env_fn = |key: &str| -> Option<String> {
            match key {
                "MY_KEY" => Some("resolved-key".to_string()),
                "MY_URL" => Some("https://resolved.example.com".to_string()),
                _ => None,
            }
        };
        let mut providers = HashMap::new();
        providers.insert(
            "test".to_string(),
            ProviderConfig {
                kind: ProviderKind::OpenAiCompat,
                base_url: Some("${MY_URL}/v1".to_string()),
                api_key_env: Some("${MY_KEY}".to_string()),
                command: None,
                args: None,
                timeout_ms: None,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            },
        );
        RokoConfig::interpolate_env_vars_with(&mut providers, &env_fn);
        let p = &providers["test"];
        assert_eq!(
            p.base_url.as_deref(),
            Some("https://resolved.example.com/v1")
        );
        assert_eq!(p.api_key_env.as_deref(), Some("resolved-key"));
    }

    #[test]
    fn resolve_file_secrets_reads_from_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let secret_path = dir.path().join("api_key");
        std::fs::write(&secret_path, "  file-secret-value  \n").expect("write secret");

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
        let p = &config.providers["test"];
        let resolved = p.extra_headers.as_ref().expect("headers");
        // The `_file` key is resolved to its base key.
        assert_eq!(
            resolved.get("authorization").map(String::as_str),
            Some("file-secret-value")
        );
        assert!(!resolved.contains_key("authorization_file"));
    }

    // ─── ConfigChangeReport tests ─────────────────────────────────────

    #[test]
    fn classify_changes_detects_hot_reloadable_budget_change() {
        let current = RokoConfig::default();
        let mut proposed = current.clone();
        proposed.budget.max_plan_usd += 5.0;
        let report = current.classify_changes(&proposed);
        assert!(report.has_changes());
        assert!(!report.needs_restart());
        assert!(report.hot_reloaded.contains(&"budget"));
        assert!(report.requires_restart.is_empty());
    }

    #[test]
    fn classify_changes_detects_restart_required_agent_change() {
        let current = RokoConfig::default();
        let mut proposed = current.clone();
        proposed.agent.default_model = "claude-opus-4-6".into();
        let report = current.classify_changes(&proposed);
        assert!(report.has_changes());
        assert!(report.needs_restart());
        assert!(report.requires_restart.contains(&"agent"));
    }

    #[test]
    fn classify_changes_no_changes_yields_empty_report() {
        let config = RokoConfig::default();
        let report = config.classify_changes(&config);
        assert!(!report.has_changes());
        assert_eq!(report.changed_count(), 0);
    }

    #[test]
    fn classify_changes_emits_budget_increase_warning() {
        let current = RokoConfig::default();
        let mut proposed = current.clone();
        proposed.budget.max_plan_usd = current.budget.max_plan_usd + 100.0;
        let report = current.classify_changes(&proposed);
        assert!(!report.warnings.is_empty());
        assert!(report.warnings[0].contains("max_plan_usd"));
    }

    // ─── SubscriptionTrigger tests ────────────────────────────────────

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
    fn subscription_trigger_file_watch_roundtrip() {
        let trigger = SubscriptionTrigger::FileWatch {
            paths: vec!["src/".into(), "tests/".into()],
            extensions: vec!["rs".into()],
            recursive: true,
        };
        let json = serde_json::to_string(&trigger).unwrap();
        let parsed: SubscriptionTrigger = serde_json::from_str(&json).unwrap();
        assert_eq!(trigger, parsed);
        assert_eq!(trigger.kind(), "file_watch");
    }

    #[test]
    fn subscription_trigger_webhook_roundtrip() {
        let trigger = SubscriptionTrigger::Webhook {
            event: "github.pull_request.*".into(),
        };
        let json = serde_json::to_string(&trigger).unwrap();
        let parsed: SubscriptionTrigger = serde_json::from_str(&json).unwrap();
        assert_eq!(trigger, parsed);
        assert_eq!(trigger.kind(), "webhook");
    }

    #[test]
    fn subscription_config_with_trigger_config_parses() {
        let toml_str = r#"
            template = "pr-review"
            trigger = "github.pull_request.*"
            debounce_ms = 500
            [trigger_config]
            type = "cron"
            schedule = "*/5 * * * *"
        "#;
        let config: SubscriptionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.template, "pr-review");
        assert_eq!(config.debounce_ms, 500);
        assert!(config.trigger_config.is_some());
        match config.trigger_config.unwrap() {
            SubscriptionTrigger::Cron { schedule } => {
                assert_eq!(schedule, "*/5 * * * *");
            }
            _ => panic!("expected Cron trigger"),
        }
    }

    #[test]
    fn subscription_config_without_trigger_config_parses() {
        let toml_str = r#"
            template = "reviewer"
            trigger = "github:push"
        "#;
        let config: SubscriptionConfig = toml::from_str(toml_str).unwrap();
        assert!(config.trigger_config.is_none());
        assert_eq!(config.debounce_ms, 0);
    }
}
