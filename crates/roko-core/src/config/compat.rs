//! Legacy Mori config compatibility reader.
//!
//! Reads the flat `ConfigState` format used by Mori's `.mori/config.toml`
//! and converts it to a [`RokoConfig`]. This is a one-way migration: the
//! result can be written back as a `roko.toml`.

use std::collections::HashMap;

use serde::Deserialize;

use super::schema::{
    AgentConfig, AgentRoleToggles, AttentionConfig, BudgetConfig, CURRENT_SCHEMA_VERSION,
    ChainConfig, ConductorConfig, DemurrageConfig, DeployConfig, EnergyConfig, GatesConfig,
    GeminiConfig, GithubWebhookConfig, GoalsConfig, ImmuneConfig, LearningConfig,
    OneirographyConfig, PerplexityConfig, PipelineConfig, PrdConfig, ProjectConfig, RelayConfig,
    RokoConfig, RoleOverride, RoutingConfig, SchedulerConfig, ServeConfig, ServerConfig,
    TemporalConfig, ToolsConfig, TuiConfig, WatcherConfig, WebhooksConfig,
};

/// Subset of Mori's `ConfigState` that we recognize.
///
/// Uses `#[serde(default)]` everywhere so we can tolerate any subset of
/// fields. Unknown fields are silently ignored via the flattened deny list.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct MoriConfig {
    // -- models --
    #[serde(alias = "default_model")]
    codex_default_model: Option<String>,
    cursor_default_model: Option<String>,
    claude_default_model: Option<String>,
    conductor_model: Option<String>,
    role_models: HashMap<String, String>,
    auto_fix_model: Option<String>,
    fallback_model: Option<String>,

    // -- efforts --
    default_effort: Option<String>,
    role_effort: HashMap<String, String>,

    // -- context --
    context_limit_k: Option<u32>,
    role_context_k: HashMap<String, u32>,

    // -- gates --
    clippy_enabled: Option<bool>,
    skip_tests: Option<bool>,
    max_iterations: Option<u32>,

    // -- routing --
    routing_mode: Option<String>,
    fast_task_model: Option<String>,
    standard_task_model: Option<String>,
    complex_task_model: Option<String>,
    context_strategy: Option<String>,

    // -- conductor --
    max_agents: Option<usize>,
    max_parallel_plans: Option<usize>,
    parallel_enabled: Option<bool>,
    express_mode: Option<bool>,
    auto_advance_batch: Option<bool>,
    auto_merge_on_complete: Option<bool>,
    pre_plan: Option<bool>,
    max_auto_fix_attempts: Option<u32>,
    warm_implementers_per_plan: Option<usize>,

    // -- role toggles --
    architect_enabled: Option<bool>,
    auditor_enabled: Option<bool>,
    scribe_enabled: Option<bool>,
    critic_enabled: Option<bool>,

    // -- project --
    fresh_base_branch: Option<String>,

    // -- learning --
    auto_playbook_refresh: Option<bool>,
    knowledge_file_intel: Option<bool>,
    knowledge_warnings: Option<bool>,
    knowledge_wave_context: Option<bool>,
    knowledge_error_patterns: Option<bool>,
    learning_min_occurrences: Option<usize>,
    file_intel_max_entries: Option<usize>,
    warning_max_entries: Option<usize>,

    // -- agent mode --
    agent_bare_mode: Option<bool>,
}

/// Convert a Mori-format TOML string into a [`RokoConfig`].
///
/// Unrecognized fields are silently ignored. Missing fields receive
/// Roko defaults.
pub fn from_mori_toml(text: &str) -> Result<RokoConfig, toml::de::Error> {
    let m: MoriConfig = toml::from_str(text)?;
    Ok(convert(&m))
}

fn convert(m: &MoriConfig) -> RokoConfig {
    RokoConfig {
        config_version: 1,
        schema_version: CURRENT_SCHEMA_VERSION,
        project: convert_project(m),
        prd: PrdConfig::default(),
        agent: convert_agent(m),
        gates: convert_gates(m),
        routing: convert_routing(m),
        pipeline: PipelineConfig::default(),
        budget: BudgetConfig::default(),
        conductor: convert_conductor(m),
        watcher: WatcherConfig::default(),
        learning: convert_learning(m),
        demurrage: DemurrageConfig::default(),
        tui: TuiConfig::default(),
        serve: ServeConfig::default(),
        scheduler: SchedulerConfig::default(),
        webhooks: WebhooksConfig {
            github: GithubWebhookConfig::default(),
        },
        providers: HashMap::new(),
        models: HashMap::new(),
        subscriptions: Vec::new(),
        server: ServerConfig::default(),
        deploy: DeployConfig::default(),
        perplexity: PerplexityConfig::default(),
        gemini: GeminiConfig::default(),
        attention: AttentionConfig::default(),
        chain: ChainConfig::default(),
        relay: RelayConfig::default(),
        immune: ImmuneConfig::default(),
        temporal: TemporalConfig::default(),
        goals: GoalsConfig::default(),
        energy: EnergyConfig::default(),
        tools: ToolsConfig::default(),
        oneirography: OneirographyConfig::default(),
        agents: Vec::new(),
    }
}

fn convert_project(m: &MoriConfig) -> ProjectConfig {
    let d = ProjectConfig::default();
    ProjectConfig {
        fresh_base_branch: m.fresh_base_branch.clone().unwrap_or(d.fresh_base_branch),
        ..d
    }
}

fn convert_agent(m: &MoriConfig) -> AgentConfig {
    let d = AgentConfig::default();

    let default_model = m
        .codex_default_model
        .clone()
        .or_else(|| m.claude_default_model.clone())
        .unwrap_or(d.default_model);

    let default_effort = m
        .default_effort
        .as_deref()
        .map(normalize_effort)
        .unwrap_or(d.default_effort);

    let mut roles: HashMap<String, RoleOverride> = HashMap::new();
    for (k, v) in &m.role_models {
        roles.entry(k.clone()).or_default().model = Some(v.clone());
    }
    for (k, v) in &m.role_effort {
        roles.entry(k.clone()).or_default().effort = Some(normalize_effort(v));
    }
    for (k, v) in &m.role_context_k {
        roles.entry(k.clone()).or_default().context_limit_k = Some(*v);
    }
    if let Some(cm) = &m.cursor_default_model {
        roles.entry("_cursor_default".into()).or_default().model = Some(cm.clone());
    }
    if let Some(cm) = &m.claude_default_model {
        roles.entry("_claude_default".into()).or_default().model = Some(cm.clone());
    }

    AgentConfig {
        default_model,
        default_backend: d.default_backend,
        default_effort,
        temperament: d.temperament,
        context_limit_k: m.context_limit_k.unwrap_or(d.context_limit_k),
        bare_mode: m.agent_bare_mode.unwrap_or(d.bare_mode),
        command: None,
        args: None,
        timeout_ms: None,
        env: None,
        tier_models: d.tier_models,
        fallback_model: m.fallback_model.clone().or(d.fallback_model),
        roles,
        policy_manifests: Vec::new(),
        data_llm: None,
        mode: Default::default(),
        extensions: Vec::new(),
        domain: None,
    }
}

fn convert_gates(m: &MoriConfig) -> GatesConfig {
    let d = GatesConfig::default();
    GatesConfig {
        clippy_enabled: m.clippy_enabled.unwrap_or(d.clippy_enabled),
        skip_tests: m.skip_tests.unwrap_or(d.skip_tests),
        max_iterations: m.max_iterations.unwrap_or(d.max_iterations),
        domain_gates: HashMap::new(),
    }
}

fn convert_routing(m: &MoriConfig) -> RoutingConfig {
    let d = RoutingConfig::default();
    RoutingConfig {
        mode: m.routing_mode.clone().unwrap_or(d.mode),
        algorithm: d.algorithm,
        discount_factor: d.discount_factor,
        fast_task_model: m.fast_task_model.clone().unwrap_or(d.fast_task_model),
        standard_task_model: m
            .standard_task_model
            .clone()
            .unwrap_or(d.standard_task_model),
        complex_task_model: m.complex_task_model.clone().unwrap_or(d.complex_task_model),
        weights: d.weights,
        context_strategy: m.context_strategy.clone().unwrap_or(d.context_strategy),
    }
}

fn convert_conductor(m: &MoriConfig) -> ConductorConfig {
    let d = ConductorConfig::default();
    ConductorConfig {
        max_agents: m.max_agents.unwrap_or(d.max_agents),
        max_parallel_plans: m.max_parallel_plans.unwrap_or(d.max_parallel_plans),
        parallel_enabled: m.parallel_enabled.unwrap_or(d.parallel_enabled),
        express_mode: m.express_mode.unwrap_or(d.express_mode),
        auto_advance_batch: m.auto_advance_batch.unwrap_or(d.auto_advance_batch),
        auto_merge_on_complete: m.auto_merge_on_complete.unwrap_or(d.auto_merge_on_complete),
        pre_plan: m.pre_plan.unwrap_or(d.pre_plan),
        max_auto_fix_attempts: m.max_auto_fix_attempts.unwrap_or(d.max_auto_fix_attempts),
        auto_fix_model: m.auto_fix_model.clone().unwrap_or(d.auto_fix_model),
        conductor_model: m.conductor_model.clone(),
        warm_implementers_per_plan: m
            .warm_implementers_per_plan
            .unwrap_or(d.warm_implementers_per_plan),
        enabled_roles: AgentRoleToggles {
            architect: m.architect_enabled.unwrap_or(true),
            auditor: m.auditor_enabled.unwrap_or(true),
            scribe: m.scribe_enabled.unwrap_or(true),
            critic: m.critic_enabled.unwrap_or(true),
        },
    }
}

fn convert_learning(m: &MoriConfig) -> LearningConfig {
    let d = LearningConfig::default();
    LearningConfig {
        auto_playbook_refresh: m.auto_playbook_refresh.unwrap_or(d.auto_playbook_refresh),
        knowledge_file_intel: m.knowledge_file_intel.unwrap_or(d.knowledge_file_intel),
        knowledge_warnings: m.knowledge_warnings.unwrap_or(d.knowledge_warnings),
        knowledge_wave_context: m.knowledge_wave_context.unwrap_or(d.knowledge_wave_context),
        knowledge_error_patterns: m
            .knowledge_error_patterns
            .unwrap_or(d.knowledge_error_patterns),
        learning_min_occurrences: m
            .learning_min_occurrences
            .unwrap_or(d.learning_min_occurrences),
        file_intel_max_entries: m.file_intel_max_entries.unwrap_or(d.file_intel_max_entries),
        warning_max_entries: m.warning_max_entries.unwrap_or(d.warning_max_entries),
        replan_on_gate_failure: d.replan_on_gate_failure,
        replan_max_per_plan: d.replan_max_per_plan,
        replan_gate_attempts: d.replan_gate_attempts,
        use_lookahead_router: d.use_lookahead_router,
        lookahead_threshold: d.lookahead_threshold,
    }
}

/// Normalize Mori effort names to Roko's kebab-case names.
fn normalize_effort(s: &str) -> String {
    match s.trim().to_ascii_lowercase().as_str() {
        "low" => "low".into(),
        "medium" | "med" => "medium".into(),
        "high" | "hi" => "high".into(),
        "max" | "maximum" => "max".into(),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_mori_config_produces_defaults() {
        let cfg = from_mori_toml("").expect("parse empty");
        assert_eq!(cfg.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(cfg.agent.context_limit_k, 200);
    }

    #[test]
    fn mori_models_migrate() {
        let toml = r#"
codex_default_model = "gpt-5.4-mini"
claude_default_model = "claude-haiku-4-5"
cursor_default_model = "composer-2-fast"
conductor_model = "claude-sonnet-4-6"
"#;
        let cfg = from_mori_toml(toml).expect("parse");
        assert_eq!(cfg.agent.default_model, "gpt-5.4-mini");
        assert_eq!(
            cfg.conductor.conductor_model.as_deref(),
            Some("claude-sonnet-4-6")
        );
        // Backend defaults stored in per-role map.
        assert_eq!(
            cfg.agent
                .roles
                .get("_claude_default")
                .and_then(|r| r.model.as_deref()),
            Some("claude-haiku-4-5")
        );
    }

    #[test]
    fn mori_role_models_migrate() {
        let toml = r#"
[role_models]
scribe = "claude-sonnet-4-6"
implementer = "gpt-5.4"

[role_effort]
architect = "High"
"#;
        let cfg = from_mori_toml(toml).expect("parse");
        let scribe = cfg.agent.roles.get("scribe").expect("scribe");
        assert_eq!(scribe.model.as_deref(), Some("claude-sonnet-4-6"));
        let imp = cfg.agent.roles.get("implementer").expect("implementer");
        assert_eq!(imp.model.as_deref(), Some("gpt-5.4"));
        let arch = cfg.agent.roles.get("architect").expect("architect");
        assert_eq!(arch.effort.as_deref(), Some("high"));
    }

    #[test]
    fn mori_gates_migrate() {
        let toml = r#"
clippy_enabled = false
skip_tests = true
max_iterations = 5
"#;
        let cfg = from_mori_toml(toml).expect("parse");
        assert!(!cfg.gates.clippy_enabled);
        assert!(cfg.gates.skip_tests);
        assert_eq!(cfg.gates.max_iterations, 5);
    }

    #[test]
    fn mori_conductor_fields_migrate() {
        let toml = r#"
max_agents = 16
parallel_enabled = true
express_mode = true
architect_enabled = false
critic_enabled = false
auto_advance_batch = false
"#;
        let cfg = from_mori_toml(toml).expect("parse");
        assert_eq!(cfg.conductor.max_agents, 16);
        assert!(cfg.conductor.parallel_enabled);
        assert!(cfg.conductor.express_mode);
        assert!(!cfg.conductor.enabled_roles.architect);
        assert!(!cfg.conductor.enabled_roles.critic);
        assert!(!cfg.conductor.auto_advance_batch);
        // defaults
        assert!(cfg.conductor.enabled_roles.auditor);
        assert!(cfg.conductor.enabled_roles.scribe);
    }

    #[test]
    fn mori_learning_fields_migrate() {
        let toml = r#"
auto_playbook_refresh = false
knowledge_warnings = false
learning_min_occurrences = 10
"#;
        let cfg = from_mori_toml(toml).expect("parse");
        assert!(!cfg.learning.auto_playbook_refresh);
        assert!(!cfg.learning.knowledge_warnings);
        assert_eq!(cfg.learning.learning_min_occurrences, 10);
    }

    #[test]
    fn mori_routing_fields_migrate() {
        let toml = r#"
routing_mode = "auto_override"
fast_task_model = "gpt-5.4-mini"
standard_task_model = "gpt-5.4-mini"
complex_task_model = "gpt-5.4"
context_strategy = "hybrid"
"#;
        let cfg = from_mori_toml(toml).expect("parse");
        assert_eq!(cfg.routing.mode, "auto_override");
        assert_eq!(cfg.routing.fast_task_model, "gpt-5.4-mini");
        assert_eq!(cfg.routing.context_strategy, "hybrid");
    }

    #[test]
    fn mori_context_k_per_role() {
        let toml = r#"
context_limit_k = 128

[role_context_k]
implementer = 300
conductor = 50
"#;
        let cfg = from_mori_toml(toml).expect("parse");
        assert_eq!(cfg.agent.context_limit_k, 128);
        assert_eq!(
            cfg.agent
                .roles
                .get("implementer")
                .and_then(|r| r.context_limit_k),
            Some(300)
        );
        assert_eq!(
            cfg.agent
                .roles
                .get("conductor")
                .and_then(|r| r.context_limit_k),
            Some(50)
        );
    }

    #[test]
    fn converted_config_roundtrips() {
        let mori_toml = r#"
codex_default_model = "gpt-5.4-mini"
clippy_enabled = true
max_agents = 4
parallel_enabled = false
architect_enabled = true
"#;
        let cfg = from_mori_toml(mori_toml).expect("parse");
        let text = cfg.to_toml().expect("serialize");
        let back = RokoConfig::from_toml(&text).expect("re-parse");
        assert_eq!(cfg, back);
    }

    #[test]
    fn normalize_effort_variants() {
        assert_eq!(normalize_effort("Low"), "low");
        assert_eq!(normalize_effort("High"), "high");
        assert_eq!(normalize_effort("hi"), "high");
        assert_eq!(normalize_effort("med"), "medium");
        assert_eq!(normalize_effort("Maximum"), "max");
        assert_eq!(normalize_effort("custom"), "custom");
    }

    #[test]
    fn mori_default_model_alias() {
        // Mori used "default_model" as an alias for "codex_default_model"
        let toml = r#"
default_model = "o3-mini"
"#;
        let cfg = from_mori_toml(toml).expect("parse");
        assert_eq!(cfg.agent.default_model, "o3-mini");
    }

    #[test]
    fn unknown_mori_fields_are_ignored() {
        let toml = r#"
codex_default_model = "gpt-5.4"
unknown_future_field = true
another_thing = "hello"
"#;
        let cfg = from_mori_toml(toml).expect("parse");
        assert_eq!(cfg.agent.default_model, "gpt-5.4");
    }
}
