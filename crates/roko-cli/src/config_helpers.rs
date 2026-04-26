//! Configuration and path helpers extracted from `orchestrate.rs`.
//!
//! This module contains:
//! - `.roko/` layout path constructors
//! - `roko.toml` config loading
//! - Model routing helpers (provider maps, role overrides, backend matching)
//! - Misc config-level label/translation helpers

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use roko_agent::safety::provenance::CustodyLogger;
use roko_agent::task_runner::CostTable as RunnerCostTable;
use roko_agent::task_runner::ModelPricing as RunnerModelPricing;
use roko_core::agent::resolve_model;
use roko_core::config::schema::{LearningConfig as RuntimeLearningConfig, RokoConfig, RoleOverride};
use roko_core::{AgentRole, OperatingFrequency, TaskDomain};
use roko_fs::RokoLayout;
use roko_learn::prompt_experiment::DEFAULT_STATIC_OVERRIDES_PATH;

use crate::config::Config;

// ── Domain predicates ─────────────────────────────────────────────────

/// Whether this domain uses compiled (compile/test/clippy) gates.
pub(crate) fn domain_uses_compiled_gates(domain: &TaskDomain) -> bool {
    matches!(
        domain,
        TaskDomain::Code | TaskDomain::Chain | TaskDomain::Custom(_)
    )
}

/// Whether this domain requires git operations (worktrees, changed-files, commits).
pub(crate) fn domain_uses_git(domain: &TaskDomain) -> bool {
    matches!(domain, TaskDomain::Code | TaskDomain::Chain)
}

/// Resolve an `AgentRole` from a task's `role` string (kebab-case label).
pub(crate) fn resolve_task_role(role_str: Option<&str>) -> AgentRole {
    let label = match role_str {
        Some(s) if !s.is_empty() => s,
        _ => return AgentRole::Implementer,
    };
    // Serde deserialize from the JSON string form (kebab-case).
    let quoted = format!("\"{label}\"");
    serde_json::from_str::<AgentRole>(&quoted).unwrap_or(AgentRole::Implementer)
}

// ── Path helpers ──────────────────────────────────────────────────────

pub(crate) fn model_experiments_path(workdir: &Path) -> PathBuf {
    workdir
        .join(".roko")
        .join("learn")
        .join("model-experiments.json")
}

pub(crate) fn gate_artifact_store_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("artifacts")
}

pub(crate) fn gate_ratchet_path(workdir: &Path) -> PathBuf {
    workdir
        .join(".roko")
        .join("learn")
        .join("gate-ratchet.json")
}

pub(crate) fn daimon_state_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("daimon").join("affect.json")
}

pub(crate) fn latency_registry_path(workdir: &Path) -> PathBuf {
    workdir
        .join(".roko")
        .join("learn")
        .join("latency-stats.json")
}

pub(crate) fn static_overrides_path(workdir: &Path) -> PathBuf {
    workdir.join(DEFAULT_STATIC_OVERRIDES_PATH)
}

pub(crate) fn routing_log_path(workdir: &Path) -> PathBuf {
    RokoLayout::for_project(workdir)
        .learn_dir()
        .join("routing.jsonl")
}

pub(crate) fn custody_logger_for(workdir: &Path) -> CustodyLogger {
    CustodyLogger::new(RokoLayout::for_project(workdir).custody_log())
}

pub(crate) fn cfactor_history_path(workdir: &Path) -> PathBuf {
    RokoLayout::for_project(workdir)
        .learn_dir()
        .join("c-factor.jsonl")
}

pub(crate) fn conductor_policy_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("learn").join("conductor.json")
}

pub(crate) fn state_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("state")
}

pub(crate) fn executor_snapshot_path(workdir: &Path) -> PathBuf {
    state_dir(workdir).join("executor.json")
}

pub(crate) fn replan_ledger_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("learn").join("replans.json")
}

// ── Config loading ────────────────────────────────────────────────────

pub(crate) fn load_roko_config(workdir: &Path) -> Result<RokoConfig> {
    let path = workdir.join("roko.toml");
    if !path.exists() {
        return Ok(RokoConfig::default());
    }

    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    RokoConfig::from_toml(&text).with_context(|| format!("parse {}", path.display()))
}

pub(crate) fn runtime_learning_config(workdir: &Path) -> RuntimeLearningConfig {
    let path = workdir.join("roko.toml");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str::<RokoConfig>(&text).ok())
        .map(|cfg| cfg.learning)
        .unwrap_or_default()
}

// ── Label helpers ─────────────────────────────────────────────────────

pub(crate) fn frequency_label(frequency: OperatingFrequency) -> &'static str {
    match frequency {
        OperatingFrequency::Gamma => "gamma",
        OperatingFrequency::Theta => "theta",
        OperatingFrequency::Delta => "delta",
    }
}

pub(crate) fn mechanical_tier_model(config: &Config) -> Option<String> {
    config.agent.tier_models.get("mechanical").cloned()
}

// ── Cost table ────────────────────────────────────────────────────────

pub(crate) fn task_runner_cost_table(
    resolved: &roko_core::agent::ResolvedModel,
) -> RunnerCostTable {
    let mut cost_table = RunnerCostTable::default();

    if let Some(profile) = resolved.profile.as_ref() {
        cost_table.insert(
            resolved.slug.clone(),
            RunnerModelPricing {
                input_per_m: profile.cost_input_per_m.unwrap_or(0.0),
                output_per_m: profile.cost_output_per_m.unwrap_or(0.0),
                cache_read_per_m: profile.cost_cache_read_per_m.unwrap_or(0.0),
                cache_write_per_m: profile.cost_cache_write_per_m.unwrap_or(0.0),
            },
        );
    }

    cost_table
}

// ── Routing model helpers ─────────────────────────────────────────────

pub(crate) fn routing_model_provider_map(config: &RokoConfig) -> HashMap<String, String> {
    let mut providers = HashMap::new();
    for (model_key, profile) in config.effective_models() {
        providers.insert(model_key, profile.provider.clone());
        providers.entry(profile.slug).or_insert(profile.provider);
    }
    providers
}

pub(crate) fn provider_id_for_routing_model(
    config: &RokoConfig,
    model_providers: &HashMap<String, String>,
    model: &str,
) -> String {
    model_providers.get(model).cloned().unwrap_or_else(|| {
        let resolved = resolve_model(config, model);
        resolved
            .profile
            .map(|profile| profile.provider)
            .unwrap_or_else(|| resolved.provider_kind.label().to_owned())
    })
}

pub(crate) fn find_role_override<'a>(
    config: &'a RokoConfig,
    role_label: &str,
) -> Option<&'a RoleOverride> {
    config.agent.roles.get(role_label).or_else(|| {
        config
            .agent
            .roles
            .iter()
            .find_map(|(section_name, override_cfg)| {
                (override_cfg.resolved_role_name(section_name) == role_label)
                    .then_some(override_cfg)
            })
    })
}

pub(crate) fn resolved_role_label(config: &RokoConfig, role_label: &str) -> String {
    find_role_override(config, role_label)
        .map(|override_cfg| override_cfg.resolved_role_name(role_label).to_string())
        .unwrap_or_else(|| role_label.to_string())
}

pub(crate) fn model_matches_forced_backend(
    config: &RokoConfig,
    model_providers: &HashMap<String, String>,
    model: &str,
    forced_backend: &str,
) -> bool {
    let forced_backend = forced_backend.trim().to_ascii_lowercase();
    if forced_backend.is_empty() {
        return false;
    }

    let provider_id = provider_id_for_routing_model(config, model_providers, model);
    if provider_id.eq_ignore_ascii_case(&forced_backend) {
        return true;
    }

    match resolve_model(config, model).backend {
        roko_core::agent::AgentBackend::Claude => forced_backend == "claude",
        roko_core::agent::AgentBackend::Codex => {
            forced_backend == "codex"
                || forced_backend == "openai"
                || forced_backend == "openai_compat"
        }
        roko_core::agent::AgentBackend::Cursor => forced_backend == "cursor",
        roko_core::agent::AgentBackend::Ollama => forced_backend == "ollama",
        roko_core::agent::AgentBackend::OpenAi => {
            forced_backend == "openai" || forced_backend == "openai_compat"
        }
        roko_core::agent::AgentBackend::Perplexity => {
            forced_backend == "perplexity" || forced_backend == "sonar"
        }
        _ => false,
    }
}

pub(crate) fn apply_role_routing_override(
    config: &RokoConfig,
    role_label: &str,
    model_providers: &HashMap<String, String>,
    candidates: &[String],
) -> Option<(String, String)> {
    let role_override = find_role_override(config, role_label)?;

    if let Some(model) = role_override.model.as_deref().map(str::trim)
        && !model.is_empty()
    {
        return Some((model.to_string(), "role_model_override".to_string()));
    }

    if let Some(routing_overrides) = role_override.routing_overrides.as_ref() {
        if let Some(force_tier) = routing_overrides.force_tier.as_deref().map(str::trim)
            && let Some(model) = config.agent.tier_models.get(force_tier)
        {
            return Some((model.clone(), "role_force_tier".to_string()));
        }

        if let Some(force_backend) = routing_overrides.force_backend.as_deref()
            && let Some(model) = candidates
                .iter()
                .find(|model| {
                    model_matches_forced_backend(config, model_providers, model, force_backend)
                })
                .cloned()
        {
            // UX34: outcome is persisted to the cascade router's confidence
            // stats via record_outcome() in record_task_success/failure.
            return Some((model, "role_force_backend".to_string()));
        }
    }

    None
}
