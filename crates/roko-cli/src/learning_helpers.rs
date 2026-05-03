//! Learning and efficiency helpers extracted from `orchestrate.rs`.
//!
//! Free functions for turn-level learning feedback, efficiency signals,
//! skill/playbook loading, experiment overrides, and episode distillation.

use std::{path::Path, sync::Arc};

use anyhow::Result;
use roko_agent::chat_types::FinishReason;
use roko_agent::model_call_service::ModelCallService;
use roko_core::agent::resolve_model;
use roko_core::config::schema::RokoConfig;
use roko_core::foundation::ModelCaller;
use roko_core::{AgentRole, Body, Engram, Kind};
use roko_learn::anomaly::AnomalyDetector;
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::events::{AgentEvent, EventBus as LearningEventBus};
use roko_learn::latency::LatencyRegistry;
use roko_learn::playbook::{Playbook, PlaybookStep, PlaybookStore, QueryContext};
use roko_learn::provider_health::{ErrorClass, ProviderHealthRegistry};
use roko_learn::runtime_feedback::LearningRuntime;
use roko_learn::skill_library::{Skill, SkillLibrary};
use tokio::io::AsyncWriteExt;

use crate::task_parser;

/// Efficiency signal tail — imported from the central defaults module.
const EFFICIENCY_SIGNAL_TAIL: usize = roko_core::defaults::DEFAULT_EFFICIENCY_SIGNAL_TAIL;

/// Resolve a configured model key or slug into the API slug learning stores use.
///
/// If `model` is empty, falls back to the config default model. Returns `None`
/// only when neither source yields a non-empty model slug.
pub(crate) fn resolve_capture_model_slug(
    config: &RokoConfig,
    model: Option<&str>,
) -> Option<String> {
    let requested = model.filter(|value| !value.trim().is_empty()).or_else(|| {
        let default_model = config.agent.default_model.trim();
        (!default_model.is_empty()).then_some(default_model)
    })?;
    let slug = resolve_model(config, requested).slug;
    (!slug.trim().is_empty()).then_some(slug)
}

/// Resolve the configured provider id for a model key or API slug.
pub(crate) fn provider_id_for_model(
    config: &RokoConfig,
    model_key_or_slug: &str,
) -> Option<String> {
    let models = config.effective_models();
    models
        .get(model_key_or_slug)
        .or_else(|| {
            models
                .values()
                .find(|profile| profile.slug == model_key_or_slug)
        })
        .map(|profile| profile.provider.clone())
        .filter(|provider| !provider.trim().is_empty())
}

/// Return the stable cascade model universe for a direct one-shot capture.
pub(crate) fn capture_runtime_model_slugs(config: &RokoConfig, episode_model: &str) -> Vec<String> {
    let mut model_slugs = config.model_slugs_for_cascade();
    if !episode_model.trim().is_empty() && !model_slugs.iter().any(|slug| slug == episode_model) {
        model_slugs.push(episode_model.to_string());
    }
    model_slugs.sort();
    model_slugs.dedup();
    model_slugs
}

/// Persist one provider-health outcome to `.roko/learn/provider-health.json`.
///
/// This writes the serialized registry used by config and TUI surfaces; it is
/// intentionally separate from the short-lived in-memory `ProviderHealthTracker`
/// used by `LearningRuntime` during a process.
pub(crate) fn record_persisted_provider_health(
    workdir: &Path,
    provider: &str,
    success: bool,
) -> Result<()> {
    let provider = provider.trim();
    if provider.is_empty() {
        return Ok(());
    }

    let path = workdir
        .join(".roko")
        .join("learn")
        .join("provider-health.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let registry = ProviderHealthRegistry::load_or_new(&path);
    if success {
        registry.record_success(provider);
    } else {
        registry.record_failure(provider, ErrorClass::Unknown);
    }
    registry.save(&path)?;
    Ok(())
}

// ─── TurnLearningFeedback ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct TurnLearningFeedback {
    pub task_id: String,
    pub model: String,
    pub provider: String,
    pub timestamp_ms: i64,
    pub prompt_hash: u64,
    pub ttft_ms: u64,
    pub total_ms: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub usage: roko_agent::Usage,
    pub success: bool,
}

pub(crate) fn publish_turn_learning_feedback(
    event_bus: &LearningEventBus,
    latency_registry: &LatencyRegistry,
    anomaly_detector: &mut AnomalyDetector,
    feedback: TurnLearningFeedback,
) {
    let mut rx = event_bus.subscribe();
    event_bus.publish(AgentEvent::TurnStarted {
        task_id: feedback.task_id.clone(),
        model: feedback.model.clone(),
        provider: feedback.provider.clone(),
        timestamp_ms: feedback.timestamp_ms,
    });
    event_bus.publish(AgentEvent::TurnCompleted {
        turn: 1,
        usage: feedback.usage,
        tool_call_count: 0,
        gate_passed: Some(feedback.success),
        finish_reason: if feedback.success {
            FinishReason::Stop
        } else {
            FinishReason::Error("agent failed".to_string())
        },
    });
    event_bus.publish(AgentEvent::CostRecorded {
        model: feedback.model.clone(),
        provider: feedback.provider.clone(),
        cost_usd: feedback.cost_usd,
        tokens: u64::from(feedback.usage.total_tokens()),
    });

    drain_turn_learning_events(&mut rx, latency_registry, anomaly_detector, &feedback);
}

fn drain_turn_learning_events(
    rx: &mut tokio::sync::broadcast::Receiver<AgentEvent>,
    latency_registry: &LatencyRegistry,
    anomaly_detector: &mut AnomalyDetector,
    feedback: &TurnLearningFeedback,
) {
    loop {
        match rx.try_recv() {
            Ok(AgentEvent::TurnStarted { .. }) => {
                if let Some(anomaly) = anomaly_detector.check_prompt(feedback.prompt_hash) {
                    tracing::warn!(
                        model = %feedback.model,
                        provider = %feedback.provider,
                        ?anomaly,
                        "learning anomaly detected from prompt"
                    );
                }
            }
            Ok(AgentEvent::TurnCompleted { .. }) => {
                latency_registry.record(
                    &feedback.model,
                    &feedback.provider,
                    feedback.ttft_ms as f64,
                    feedback.total_ms as f64,
                    feedback.output_tokens,
                );
                tracing::info!(
                    model = %feedback.model,
                    provider = %feedback.provider,
                    ttft_ms = feedback.ttft_ms,
                    total_ms = feedback.total_ms,
                    output_tokens = feedback.output_tokens,
                    "learning latency recorded"
                );
            }
            Ok(AgentEvent::CostRecorded { .. }) => {
                if let Some(anomaly) = anomaly_detector.check_cost(feedback.cost_usd) {
                    tracing::warn!(
                        model = %feedback.model,
                        provider = %feedback.provider,
                        ?anomaly,
                        "learning anomaly detected from cost"
                    );
                } else {
                    tracing::info!(
                        model = %feedback.model,
                        provider = %feedback.provider,
                        "learning anomaly scan complete"
                    );
                }
            }
            Ok(_) => {}
            Err(
                tokio::sync::broadcast::error::TryRecvError::Empty
                | tokio::sync::broadcast::error::TryRecvError::Closed,
            ) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "learning feedback lagged behind event stream");
            }
        }
    }
}

// ─── Efficiency signals ──────────────────────────────────────────────────

/// Convert the latest efficiency entries into the signals expected by the conductor.
pub(crate) fn build_efficiency_signals(text: &str, budget_usd: Option<f64>) -> Vec<Engram> {
    let mut signals = Vec::new();

    if let Some(budget_usd) = budget_usd.filter(|budget| *budget > 0.0) {
        signals.extend(build_cost_overrun_signals(text, budget_usd));
    }

    if let Some(signal) = build_context_window_pressure_signal(text) {
        signals.push(signal);
    }

    signals
}

/// Sum the cost from the latest valid efficiency events in the JSONL log.
pub(crate) fn latest_efficiency_cost(text: &str) -> Option<f64> {
    let mut total = 0.0;
    let mut seen = 0usize;

    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<AgentEfficiencyEvent>(trimmed) {
            total += event.cost_usd;
            seen += 1;
            if seen >= EFFICIENCY_SIGNAL_TAIL {
                break;
            }
        }
    }

    (seen > 0).then_some(total)
}

pub(crate) fn build_cost_overrun_signals(text: &str, budget_usd: f64) -> Vec<Engram> {
    let Some(cost_usd) = latest_efficiency_cost(text) else {
        return Vec::new();
    };

    vec![
        Engram::builder(Kind::Metric)
            .body(Body::text("plan cost"))
            .tag("name", "plan_cost")
            .tag("value", format!("{cost_usd:.6}"))
            .build(),
        Engram::builder(Kind::Metric)
            .body(Body::text("plan budget"))
            .tag("name", "plan_budget")
            .tag("value", format!("{budget_usd:.6}"))
            .build(),
    ]
}

pub(crate) fn build_context_window_pressure_signal(text: &str) -> Option<Engram> {
    let event = latest_efficiency_event(text)?;
    let body = Body::from_json(&event).unwrap_or_else(|_| {
        Body::text(format!(
            "{} tokens used on {}",
            event.total_prompt_tokens, event.model
        ))
    });

    Some(
        Engram::builder(Kind::TokenUsage)
            .body(body)
            .tag("plan_id", event.plan_id)
            .tag("task_id", event.task_id)
            .tag("role", event.role)
            .tag("model", event.model)
            .tag("tokens_used", event.total_prompt_tokens.to_string())
            .build(),
    )
}

pub(crate) fn latest_efficiency_event(text: &str) -> Option<AgentEfficiencyEvent> {
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<AgentEfficiencyEvent>(trimmed) {
            return Some(event);
        }
    }

    None
}

// ─── Signal loading ──────────────────────────────────────────────────────

pub(crate) async fn load_recent_signals(
    path: &Path,
    tail_len: usize,
) -> std::io::Result<Vec<Engram>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = tokio::fs::read_to_string(path).await?;
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(tail_len);
    let mut signals = Vec::with_capacity(lines.len().saturating_sub(start));
    for line in &lines[start..] {
        if let Ok(signal) = serde_json::from_str::<Engram>(line) {
            signals.push(signal);
        }
    }
    Ok(signals)
}

/// Load the latest efficiency entries and convert them into cost metric signals.
pub(crate) async fn load_efficiency_cost_signals(
    path: &Path,
    budget_usd: Option<f64>,
) -> std::io::Result<Vec<Engram>> {
    let Some(budget_usd) = budget_usd.filter(|budget| *budget > 0.0) else {
        return Ok(Vec::new());
    };

    let text = tokio::fs::read_to_string(path).await?;
    Ok(build_cost_overrun_signals(&text, budget_usd))
}

/// Synchronous variant used by the main conductor check path.
pub(crate) fn load_efficiency_signals_sync(
    path: &Path,
    budget_usd: Option<f64>,
) -> std::io::Result<Vec<Engram>> {
    let text = std::fs::read_to_string(path)?;
    Ok(build_efficiency_signals(&text, budget_usd))
}

// ─── Skill / playbook loading ────────────────────────────────────────────

pub(crate) async fn load_or_create_skill_library(path: &Path) -> Result<SkillLibrary> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
    {
        Ok(mut file) => {
            file.write_all(b"[]").await?;
            file.flush().await?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err.into()),
    }

    Ok(SkillLibrary::new(path).await?)
}

pub(crate) async fn load_or_create_playbook_store(path: &Path) -> Result<PlaybookStore> {
    tokio::fs::create_dir_all(path).await?;
    Ok(PlaybookStore::new(path))
}

// ─── Experiment overrides ────────────────────────────────────────────────

pub(crate) fn apply_concluded_experiment_overrides(learning: &LearningRuntime, workdir: &Path) {
    let overrides_path = crate::config_helpers::static_overrides_path(workdir);
    let winners = {
        let store = learning.experiment_store().lock();
        let winners = store.concluded_winners();
        if let Err(err) = store.apply_winners_to(&winners, &overrides_path) {
            tracing::warn!(error = %err, path = %overrides_path.display(), "failed to persist experiment winners");
        }
        winners
    };

    if winners.is_empty() {
        return;
    }

    match learning
        .cascade_router()
        .load_static_overrides(&overrides_path)
    {
        Ok(applied) => {
            tracing::info!(applied, path = %overrides_path.display(), "applied static routing overrides")
        }
        Err(err) => {
            tracing::warn!(error = %err, path = %overrides_path.display(), "failed to load static routing overrides")
        }
    }
}

// ─── Distillation hooks ──────────────────────────────────────────────────

/// Build a [`ModelCaller`] configured for episode distillation.
///
/// Uses the workspace's configured default model so distillation works in
/// environments that don't have Anthropic providers (e.g. Zhipu-only deploys).
///
/// This is `pub` (not `pub(crate)`) because the binary target and the library
/// target are compiled as separate crates — `pub(crate)` would make the
/// function invisible to the binary.
pub fn distillation_model_caller(workdir: &Path) -> Arc<dyn ModelCaller> {
    let config = roko_core::config::loader::load_config_unified(workdir).unwrap_or_default();
    let model = config.agent.default_model.clone();
    Arc::new(ModelCallService::new(model).with_config(config))
}

pub(crate) fn install_episode_distillation_hook(learning: &mut LearningRuntime, workdir: &Path) {
    let distillation_workdir = workdir.to_path_buf();
    let distillation_caller = distillation_model_caller(workdir);
    learning.set_episode_completion_hook(move |episode| {
        roko_neuro::spawn_episode_distillation(
            distillation_workdir.clone(),
            episode,
            Some(Arc::clone(&distillation_caller)),
        );
    });
}

/// Log a one-time note that episode distillation now uses the shared
/// `ModelCaller` path rather than a direct API-key fallback.
pub(crate) fn warn_if_distillation_disabled() {
    use std::sync::Once;
    static WARN: Once = Once::new();
    WARN.call_once(|| {
        tracing::debug!(
            "episode distillation uses the shared ModelCaller path; no direct API-key fallback remains"
        );
    });
}

// ─── Skill rendering ─────────────────────────────────────────────────────

/// Render up to 3 prior skills into a "## Prior Experience" context section.
///
/// Follows the same pattern as `render_knowledge_context` -- a markdown heading
/// followed by numbered entries with key metadata so the agent can leverage
/// successful approaches from prior tasks.
pub(crate) fn render_prior_experience(skills: &[Skill]) -> String {
    use std::fmt::Write as _;

    let mut content = String::from(
        "## Relevant Skills\n\nThe following skills were high-confidence matches for this task:\n",
    );
    for (idx, skill) in skills.iter().enumerate() {
        let _ = write!(
            content,
            "\n### {}. {} (confidence: {:.0}%)\n",
            idx + 1,
            skill.name,
            (skill.score.clamp(0.0, 1.0) * 100.0).round(),
        );
        let _ = writeln!(content, "Summary: {}", skill.summary);
        if skill.usage_count > 0 {
            let _ = writeln!(
                content,
                "Telemetry: {:.0}% success over {} uses",
                (skill.success_rate.clamp(0.0, 1.0) * 100.0).round(),
                skill.usage_count
            );
        }
        if skill.validated_count > 0 {
            let _ = writeln!(content, "Validated matches: {}", skill.validated_count);
        }
        if !skill.description.is_empty() {
            let _ = writeln!(content, "Description: {}", skill.description);
        }
        if !skill.files.is_empty() {
            let _ = writeln!(content, "Files: {}", skill.files.join(", "));
        }
        if !skill.pattern.is_empty() {
            let _ = writeln!(content, "Pattern:\n{}", skill.pattern);
        }
        if !skill.prompt_template.is_empty() {
            let _ = writeln!(content, "Template:\n{}", skill.prompt_template);
        }
    }
    content
}

// ─── Playbook helpers ────────────────────────────────────────────────────

pub(crate) fn playbook_query(
    task: &str,
    task_text: &str,
    task_def: Option<&task_parser::TaskDef>,
) -> String {
    let mut parts = vec![task.to_string(), task_text.to_string()];
    if let Some(task_def) = task_def {
        parts.push(task_def.title.clone());
        if let Some(description) = &task_def.description {
            parts.push(description.clone());
        }
        if !task_def.files.is_empty() {
            parts.push(task_def.files.join(" "));
        }
        if !task_def.acceptance.is_empty() {
            parts.push(task_def.acceptance.join(" "));
        }
    }
    parts.join("\n")
}

pub(crate) fn playbook_query_context(
    role: AgentRole,
    task: &str,
    task_text: &str,
    task_def: Option<&task_parser::TaskDef>,
) -> QueryContext {
    let task_title = task_def
        .map(|task_def| task_def.title.clone())
        .unwrap_or_else(|| task.to_string());
    QueryContext::new(
        task,
        task_title,
        playbook_query(task, task_text, task_def),
        role.label(),
        10,
        3,
    )
}

pub(crate) fn build_task_playbook(task_def: &task_parser::TaskDef) -> Playbook {
    let goal = task_def
        .description
        .clone()
        .unwrap_or_else(|| task_def.title.clone());
    let mut playbook = Playbook::new(task_def.id.clone(), goal);
    playbook.name = task_def.title.clone();

    let mut next_index = 0u32;
    playbook.steps.push(PlaybookStep::new(
        next_index,
        task_def
            .description
            .clone()
            .unwrap_or_else(|| task_def.title.clone()),
        task_def
            .role
            .clone()
            .unwrap_or_else(|| "execute_task".to_string()),
        if task_def.acceptance.is_empty() {
            vec!["task_success".to_string()]
        } else {
            task_def.acceptance.clone()
        },
    ));
    next_index += 1;

    if !task_def.files.is_empty() {
        playbook.steps.push(PlaybookStep::new(
            next_index,
            format!("Touch files: {}", task_def.files.join(", ")),
            "edit_file",
            task_def.files.clone(),
        ));
        next_index += 1;
    }

    if !task_def.verify.is_empty() {
        let signals = task_def
            .verify
            .iter()
            .map(|step| step.phase.clone())
            .collect::<Vec<_>>();
        playbook.steps.push(PlaybookStep::new(
            next_index,
            format!("Verify task with {} checks", task_def.verify.len()),
            "verify",
            signals,
        ));
    }

    playbook
}

// ─── Error signature extraction ──────────────────────────────────────────

pub(crate) fn learned_error_signature(last_gate_failure: Option<&str>) -> Option<String> {
    let failure = last_gate_failure?.trim();
    if failure.is_empty() {
        return None;
    }

    if let Some(code) = extract_rust_error_code(failure) {
        return Some(code);
    }

    failure
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(160).collect())
}

pub(crate) fn extract_rust_error_code(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.len() < 5 {
        return None;
    }

    for idx in 0..=bytes.len() - 5 {
        if bytes[idx] != b'E' {
            continue;
        }
        if bytes[idx + 1..idx + 5].iter().all(u8::is_ascii_digit) {
            return text.get(idx..idx + 5).map(str::to_string);
        }
    }

    None
}
