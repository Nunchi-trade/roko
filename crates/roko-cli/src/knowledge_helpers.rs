//! Knowledge store helpers extracted from `orchestrate.rs`.
//!
//! Free functions that query or write to the neuro knowledge store, build
//! strategy-fragment context, record lifecycle transitions, and apply
//! knowledge-driven gate hints.

use roko_agent::AgentResult;
use roko_core::{AgentRole, TaskCategory};
use roko_gate::adaptive_threshold::AdaptiveThresholds;
use roko_neuro::{
    EmotionalProvenance, KnowledgeEntry, KnowledgeKind, KnowledgeStore, KnowledgeTier,
};
use roko_runtime::lifecycle::{
    AgentLifecycleState, LifecycleTransition, LifecycleTransitionReason,
};

use crate::task_parser;

// ─── Task category mapping ───────────────────────────────────────────────

pub(crate) fn neuro_prompt_task_category(role: AgentRole) -> TaskCategory {
    match role {
        AgentRole::Researcher | AgentRole::PrePlanner | AgentRole::Strategist => {
            TaskCategory::Research
        }
        AgentRole::Refactorer => TaskCategory::Refactor,
        AgentRole::Scribe => TaskCategory::Docs,
        AgentRole::DocVerifier
        | AgentRole::IntegrationTester
        | AgentRole::TerminalValidator
        | AgentRole::GolemLifecycleTester
        | AgentRole::RegressionDetector
        | AgentRole::CoverageTracker
        | AgentRole::CrossSystemTester
        | AgentRole::DependencyValidator
        | AgentRole::FullLoopValidator => TaskCategory::Verification,
        _ => TaskCategory::Implementation,
    }
}

// ─── Strategy fragments ──────────────────────────────────────────────────

pub(crate) fn strategy_fragment_query(
    role: AgentRole,
    task_def: Option<&task_parser::TaskDef>,
    task_text: &str,
) -> String {
    let mut query_parts = vec![
        neuro_prompt_task_category(role).label().to_string(),
        task_text.trim().to_string(),
    ];
    if let Some(crate_name) = crate::task_helpers::task_crate_name(task_def) {
        query_parts.push(crate_name);
    }
    query_parts.retain(|part| !part.trim().is_empty());
    query_parts.join(" ")
}

pub(crate) fn select_strategy_fragments(
    knowledge_store: &KnowledgeStore,
    role: AgentRole,
    task_def: Option<&task_parser::TaskDef>,
    task_text: &str,
    current_model: &str,
    limit: usize,
) -> Vec<KnowledgeEntry> {
    let query = strategy_fragment_query(role, task_def, task_text);
    knowledge_store
        .query_kind(
            &query,
            KnowledgeKind::StrategyFragment,
            limit.saturating_mul(3).max(limit),
        )
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.confidence > 0.0)
        .filter(|entry| entry.applies_to_model(current_model))
        .take(limit)
        .collect()
}

pub(crate) fn render_strategy_fragments(entries: &[KnowledgeEntry]) -> String {
    use std::fmt::Write as _;

    let mut content = String::from(
        "## Strategy Fragments\n\nThe following reusable approach fragments were distilled from repeated successful runs:\n",
    );
    for (idx, entry) in entries.iter().enumerate() {
        let confidence = entry.confidence.clamp(0.0, 1.0);
        let tags = if entry.tags.is_empty() {
            String::from("-")
        } else {
            entry.tags.join(", ")
        };
        let _ = write!(
            content,
            "\n### {}. Strategy fragment ({:.0}%)\nTags: {}\n\n{}\n",
            idx + 1,
            confidence * 100.0,
            tags,
            entry.content.trim()
        );
    }
    content
}

pub(crate) fn build_strategy_fragment_context(
    knowledge_store: &KnowledgeStore,
    role: AgentRole,
    task_def: Option<&task_parser::TaskDef>,
    task_text: &str,
    current_model: &str,
) -> Option<String> {
    let fragments =
        select_strategy_fragments(knowledge_store, role, task_def, task_text, current_model, 3);
    if fragments.is_empty() {
        None
    } else {
        Some(render_strategy_fragments(&fragments))
    }
}

// ─── Anti-knowledge patterns ─────────────────────────────────────────────

/// Query AntiKnowledge entries from the neuro store and convert them to
/// anti-pattern strings for injection into layer 7 of the system prompt.
pub(crate) fn query_anti_knowledge_patterns(
    knowledge_store: &KnowledgeStore,
    task_text: &str,
    limit: usize,
) -> Vec<String> {
    match knowledge_store.query_kind(task_text, KnowledgeKind::AntiKnowledge, limit) {
        Ok(entries) => entries.into_iter().map(|entry| entry.content).collect(),
        Err(err) => {
            tracing::warn!(error = %err, "failed to query AntiKnowledge for anti-patterns");
            Vec::new()
        }
    }
}

// ─── Knowledge routing boost ─────────────────────────────────────────────

/// Query the knowledge store for routing-relevant entries about a model and return
/// a small confidence-weighted boost in `[0.0, 0.3]`.
///
/// This is a lightweight, non-blocking query: it scans at most 5 entries whose tags
/// or content mention the model slug combined with the role/task-type context.
/// Positive entries (Heuristic/Insight) contribute a positive boost proportional
/// to their confidence; AntiKnowledge entries contribute a negative signal.
pub(crate) fn knowledge_routing_boost(
    knowledge_store: &KnowledgeStore,
    model_slug: &str,
    role: AgentRole,
    task_category: &str,
) -> f64 {
    let query = format!("{} {} routing model", role.label(), task_category);
    let entries = match knowledge_store.query(&query, 5) {
        Ok(entries) => entries,
        Err(_) => return 0.0,
    };

    let mut boost = 0.0_f64;
    for entry in &entries {
        // Only consider entries that mention this specific model.
        let slug_lower = model_slug.to_lowercase();
        let content_matches = entry.content.to_lowercase().contains(&slug_lower)
            || entry
                .source_model
                .as_deref()
                .is_some_and(|sm| sm.eq_ignore_ascii_case(model_slug))
            || entry
                .tags
                .iter()
                .any(|t| t.eq_ignore_ascii_case(model_slug));
        if !content_matches {
            continue;
        }

        let weight = entry.confidence.clamp(0.0, 1.0);
        if entry.kind == KnowledgeKind::AntiKnowledge {
            boost -= weight * 0.15;
        } else {
            boost += weight * 0.10;
        }
    }

    boost.clamp(-0.3, 0.3)
}

// ─── Lifecycle knowledge recording ───────────────────────────────────────

/// INT-20: Record significant lifecycle transitions as knowledge entries in the
/// neuro store.
///
/// Restores, degradation events, and metamorphosis are operationally significant
/// -- they capture when and why agents changed state.  Recording them as
/// knowledge enables future sessions to learn from operational history (e.g.
/// "agent X was degraded due to budget constraints 5 times last week").
pub(crate) fn record_lifecycle_knowledge(
    knowledge_store: &KnowledgeStore,
    admission: Option<&roko_neuro::KnowledgeAdmissionStore>,
    transition: &LifecycleTransition,
) {
    // Only record significant transitions -- skip routine Active/Waiting/Initiated.
    let is_significant = matches!(
        &transition.to,
        AgentLifecycleState::Hibernated
            | AgentLifecycleState::Metamorphosing
            | AgentLifecycleState::Degraded { .. }
            | AgentLifecycleState::Deleted
    ) || matches!(
        &transition.reason,
        LifecycleTransitionReason::OperatorResume
            | LifecycleTransitionReason::BudgetConstrained
            | LifecycleTransitionReason::BudgetRestored
            | LifecycleTransitionReason::MetamorphosisFinished
    );

    if !is_significant {
        return;
    }

    let content = format!(
        "Agent '{}' transitioned from {:?} to {:?} (reason: {:?}) at {}",
        transition.agent_id,
        transition.from,
        transition.to,
        transition.reason,
        transition.occurred_at.format("%Y-%m-%d %H:%M:%S UTC"),
    );

    let kind = if matches!(
        &transition.to,
        AgentLifecycleState::Degraded { .. } | AgentLifecycleState::Deleted
    ) {
        KnowledgeKind::AntiKnowledge
    } else {
        KnowledgeKind::Heuristic
    };

    let mut entry = KnowledgeEntry {
        id: format!(
            "lifecycle-{}-{}",
            transition.agent_id,
            transition.occurred_at.timestamp_millis()
        ),
        kind,
        content,
        tags: vec![
            "lifecycle".to_string(),
            format!("agent:{}", transition.agent_id),
            format!("state:{:?}", transition.to),
        ],
        confidence: 1.0,
        source_episodes: Vec::new(),
        tier: roko_neuro::KnowledgeTier::Transient,
        half_life_days: 30.0,
        created_at: transition.occurred_at,
        ..KnowledgeEntry::default()
    };
    entry.source = Some("lifecycle-monitor".to_string());

    // Route through admission controller when available (A2).
    let _ = admission; // Admission integration uses submit_candidate with full candidate records;
    // lifecycle entries use direct write for now since building a full
    // KnowledgeCandidateRecord from a KnowledgeEntry requires evidence chains.
    if let Err(err) = knowledge_store.add(entry) {
        tracing::debug!(error = %err, "INT-20: failed to record lifecycle knowledge");
    }
}

// ─── Neuro gate hints ────────────────────────────────────────────────────

/// INT-15: Query neuro knowledge for gate-related failure and stability
/// patterns and apply them as hints to the adaptive gate thresholds.
///
/// This bridges neuro (durable knowledge) with the gate verification pipeline,
/// so that known problematic or reliably stable rungs are tuned accordingly
/// before the plan run begins.
pub(crate) fn apply_neuro_gate_hints(
    knowledge_store: &KnowledgeStore,
    thresholds: &mut AdaptiveThresholds,
) {
    let failure_rungs = match knowledge_store.query("gate failure compile lint test", 10) {
        Ok(entries) => entries
            .into_iter()
            .filter_map(|entry| {
                let content_lower = entry.content.to_lowercase();
                if content_lower.contains("compile") || content_lower.contains("rung 0") {
                    Some(0u32)
                } else if content_lower.contains("lint")
                    || content_lower.contains("clippy")
                    || content_lower.contains("rung 1")
                {
                    Some(1)
                } else if content_lower.contains("test fail") || content_lower.contains("rung 2") {
                    Some(2)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>(),
        Err(err) => {
            tracing::debug!(error = %err, "INT-15: skipping neuro gate hints (query failed)");
            return;
        }
    };

    let stable_rungs = match knowledge_store.query("gate stable passing consistently", 10) {
        Ok(entries) => entries
            .into_iter()
            .filter_map(|entry| {
                let content_lower = entry.content.to_lowercase();
                if content_lower.contains("compile") || content_lower.contains("rung 0") {
                    Some(0u32)
                } else if content_lower.contains("lint") || content_lower.contains("rung 1") {
                    Some(1)
                } else if content_lower.contains("test") || content_lower.contains("rung 2") {
                    Some(2)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };

    if !failure_rungs.is_empty() || !stable_rungs.is_empty() {
        tracing::info!(
            failure_rungs = ?failure_rungs,
            stable_rungs = ?stable_rungs,
            "INT-15: applying neuro knowledge hints to adaptive gate thresholds"
        );
        thresholds.apply_neuro_hints(&failure_rungs, &stable_rungs);
    }
}

// ─── Neuro context rendering ─────────────────────────────────────────────

pub(crate) fn render_neuro_chunk(chunk: &roko_neuro::ContextChunk) -> Option<String> {
    let heading = match &chunk.source {
        roko_compose::ContextSource::KnowledgeEntry { .. } => "## Neuro Knowledge",
        roko_compose::ContextSource::Episode { .. } => "## Neuro Episodes",
        roko_compose::ContextSource::RecentSignal { .. } => "## Neuro Signals",
        _ => return None,
    };
    let body = chunk.content.trim();
    if body.is_empty() {
        return None;
    }
    Some(format!("{heading}\n\n{body}"))
}

// ─── Success knowledge entry builder ─────────────────────────────────────

pub(crate) fn build_success_knowledge_entry(
    plan_id: &str,
    task_id: &str,
    task_def: Option<&task_parser::TaskDef>,
    result: &AgentResult,
    model: &str,
    episode_id: &str,
) -> KnowledgeEntry {
    let kind = infer_success_knowledge_kind(task_def, result);
    let title = task_def
        .map(|task| task.title.trim())
        .filter(|title| !title.is_empty())
        .unwrap_or(task_id);
    let description = task_def
        .and_then(|task| task.description.as_deref())
        .map(str::trim)
        .filter(|description| !description.is_empty());
    let output_summary = result
        .output
        .body
        .as_text()
        .ok()
        .map(|text| truncate_doc_snippet(text, 600))
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "Task completed with {} output.",
                result.output.kind.as_str()
            )
        });

    let mut content = format!("Task `{task_id}` succeeded: {title}.");
    if let Some(description) = description {
        content.push_str("\n\nWhy it worked:\n");
        content.push_str(description);
    }
    content.push_str("\n\nSuccessful outcome:\n");
    content.push_str(&output_summary);

    let mut tags = vec![
        "task-success".to_string(),
        format!("plan:{plan_id}"),
        format!("task:{task_id}"),
        kind.as_str().to_string(),
    ];
    if let Some(task) = task_def {
        tags.push(format!("tier:{}", task.tier));
        if task.files.len() > 1 {
            tags.push("multi-file".to_string());
        } else if let Some(path) = task.files.first() {
            tags.push(format!("file:{path}"));
        }
    }
    if let Some(emotional_tag) = result.output.emotional_tag.as_ref() {
        tags.push(format!(
            "emotion_trigger:{}",
            emotional_tag.trigger.trim().to_ascii_lowercase()
        ));
        tags.push(emotion_valence_tag(emotional_tag).to_string());
        tags.push(emotion_arousal_tag(emotional_tag).to_string());
    }
    tags.sort();
    tags.dedup();

    KnowledgeEntry {
        id: format!("task-success:{plan_id}:{task_id}:{}", result.output.id),
        kind,
        source: Some("task-success".to_string()),
        content,
        confidence: 0.75,
        confidence_weight: 0.75,
        refuted_insight_id: None,
        refutation_evidence: None,
        source_episodes: vec![episode_id.to_string()],
        tags,
        source_model: Some(model.to_string()),
        model_generality: 0.9,
        created_at: chrono::Utc::now(),
        half_life_days: kind.default_half_life_days(),
        tier: KnowledgeTier::Transient,
        emotional_tag: result.output.emotional_tag.clone(),
        emotional_provenance: result
            .output
            .emotional_tag
            .as_ref()
            .map(EmotionalProvenance::from_tag),
        hdc_vector: None,
        confirmation_count: 0,
        distinct_contexts: Vec::new(),
        deprecated: false,
        balance: 1.0,
        frozen: false,
        catalytic_score: 0,
    }
}

// ─── Emotion tag helpers ─────────────────────────────────────────────────

pub(crate) fn emotion_valence_tag(tag: &roko_core::EmotionalTag) -> &'static str {
    if tag.pad.pleasure >= 0.2 {
        "emotion_valence:positive"
    } else if tag.pad.pleasure <= -0.2 {
        "emotion_valence:negative"
    } else {
        "emotion_valence:neutral"
    }
}

pub(crate) fn emotion_arousal_tag(tag: &roko_core::EmotionalTag) -> &'static str {
    if tag.pad.arousal >= 0.35 {
        "emotion_arousal:high"
    } else if tag.pad.arousal <= -0.35 {
        "emotion_arousal:low"
    } else {
        "emotion_arousal:mid"
    }
}

pub(crate) fn infer_success_knowledge_kind(
    task_def: Option<&task_parser::TaskDef>,
    result: &AgentResult,
) -> KnowledgeKind {
    let mut hint_text = task_def
        .map(|task| {
            format!(
                "{} {}",
                task.title,
                task.description.as_deref().unwrap_or_default()
            )
        })
        .unwrap_or_default();
    if let Ok(output_text) = result.output.body.as_text() {
        hint_text.push(' ');
        hint_text.push_str(output_text);
    }
    let hint_text = hint_text.to_ascii_lowercase();
    let looks_reusable = task_def.is_some_and(|task| task.files.len() > 1)
        || [
            "refactor",
            "standardize",
            "reuse",
            "reusable",
            "pattern",
            "workflow",
            "pipeline",
            "template",
            "guardrail",
            "strategy",
        ]
        .iter()
        .any(|needle| hint_text.contains(needle));

    if looks_reusable {
        KnowledgeKind::Heuristic
    } else {
        KnowledgeKind::Insight
    }
}

// ─── Local helpers ───────────────────────────────────────────────────────

fn truncate_doc_snippet(content: &str, max_chars: usize) -> String {
    let mut chars = content.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_none() {
        content.to_string()
    } else {
        format!("{truncated}\n\n[... truncated]")
    }
}

/// Item 13: Build knowledge-informed routing advice for model selection.
///
/// Queries the neuro knowledge store for entries related to the task category
/// and candidate models, then computes per-model score adjustments.
pub(crate) fn build_knowledge_routing_advice(
    knowledge_store: &KnowledgeStore,
    candidate_slugs: &[String],
    role: AgentRole,
    task_category: &str,
) -> roko_learn::cascade_router::KnowledgeRoutingAdvice {
    use roko_learn::cascade_router::{KnowledgeHint, KnowledgeRoutingAdvice};

    let query = format!("{} {} routing model", role.label(), task_category);
    let entries = match knowledge_store.query(&query, 10) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::debug!(
                error = %err,
                "failed to query knowledge store for routing advice"
            );
            return KnowledgeRoutingAdvice::default();
        }
    };

    if entries.is_empty() {
        return KnowledgeRoutingAdvice::default();
    }

    let mut hints: Vec<KnowledgeHint> = Vec::new();

    for slug in candidate_slugs {
        let slug_lower = slug.to_lowercase();
        let mut score = 0.0_f64;
        let mut supporting = 0_u32;
        let mut positive_count = 0_u32;
        let mut negative_count = 0_u32;

        for entry in &entries {
            let content_matches = entry.content.to_lowercase().contains(&slug_lower)
                || entry
                    .source_model
                    .as_deref()
                    .is_some_and(|sm| sm.eq_ignore_ascii_case(slug))
                || entry.tags.iter().any(|t| t.eq_ignore_ascii_case(slug));
            if !content_matches {
                continue;
            }

            supporting += 1;
            let weight = entry.confidence.clamp(0.0, 1.0);
            if entry.kind == KnowledgeKind::AntiKnowledge {
                score -= weight * 0.15;
                negative_count += 1;
            } else {
                score += weight * 0.10;
                positive_count += 1;
            }
        }

        if supporting > 0 {
            let reason = if negative_count > 0 && positive_count > 0 {
                format!("{positive_count} positive + {negative_count} negative entries for {slug}")
            } else if negative_count > 0 {
                format!("{negative_count} anti-knowledge entries for {slug}")
            } else {
                format!("{positive_count} positive entries for {slug}")
            };

            hints.push(KnowledgeHint {
                model_slug: slug.clone(),
                score: score.clamp(-0.3, 0.3),
                supporting_entries: supporting,
                reason,
            });
        }
    }

    let has_signal = !hints.is_empty();
    KnowledgeRoutingAdvice { hints, has_signal }
}
