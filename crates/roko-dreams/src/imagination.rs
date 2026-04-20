//! REM imagination and counterfactual synthesis.
//!
//! The implementation is intentionally lightweight: it builds a small causal
//! summary from episodes, evaluates plausibility inside a trust region, and
//! emits hypothetical knowledge entries for wake-cycle validation.

use std::collections::{BTreeMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use chrono::{DateTime, Utc};
use roko_learn::episode_logger::Episode;
use roko_neuro::{KnowledgeEntry, KnowledgeKind, KnowledgeTier};
use roko_primitives::hdc::text_fingerprint;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Counterfactual query evaluated against a causal model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterfactualQuery {
    /// Episode being interrogated.
    pub episode_id: String,
    /// Intervention variable and replacement value.
    pub intervention: (String, String),
}

/// Three creativity modes used by REM imagination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImaginationMode {
    /// Merge patterns from two episodes.
    Combinational,
    /// Extend a known pattern into a nearby domain.
    Exploratory,
    /// Invert an assumption from a successful pattern.
    Transformational,
}

impl Default for ImaginationMode {
    fn default() -> Self {
        Self::Combinational
    }
}

/// Lightweight causal summary built from observed episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalModel {
    /// Episodes indexed by id so counterfactual queries can resolve a base case.
    pub episodes_by_id: BTreeMap<String, Episode>,
    /// Observed values per variable.
    pub variables: BTreeMap<String, BTreeMap<String, usize>>,
}

impl CausalModel {
    /// Build a causal summary from the supplied episodes.
    #[must_use]
    pub fn from_episodes(episodes: &[Episode]) -> Self {
        let mut episodes_by_id = BTreeMap::new();
        let mut variables = BTreeMap::new();
        for episode in episodes {
            episodes_by_id.insert(episode.id.clone(), episode.clone());
            bump_variable(&mut variables, "model", &episode.model);
            bump_variable(&mut variables, "task_id", &episode.task_id);
            bump_variable(&mut variables, "trigger_kind", &episode.trigger_kind);
            bump_variable(
                &mut variables,
                "outcome",
                if episode.success {
                    "success"
                } else {
                    "failure"
                },
            );
            if let Some(reason) = episode
                .failure_reason
                .as_deref()
                .map(str::trim)
                .filter(|reason| !reason.is_empty())
            {
                bump_variable(&mut variables, "failure_reason", reason);
            }
        }
        Self {
            episodes_by_id,
            variables,
        }
    }

    fn base_episode(&self, query: &CounterfactualQuery) -> Option<&Episode> {
        self.episodes_by_id.get(&query.episode_id)
    }

    fn variable_support(&self, variable: &str, value: &str) -> usize {
        self.variables
            .get(variable)
            .and_then(|values| values.get(value))
            .copied()
            .unwrap_or_default()
    }
}

/// Outcome of a single counterfactual imagination step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImaginationOutcome {
    /// Query that was evaluated.
    pub query: CounterfactualQuery,
    /// Creativity mode used to interpret the query.
    pub mode: ImaginationMode,
    /// Whether the query stayed within the trust region.
    pub plausible: bool,
    /// Confidence in the resulting hypothesis.
    pub confidence: f64,
    /// Projected change in success probability.
    pub projected_success_delta: f64,
    /// Human-readable summary of the counterfactual.
    pub narrative: String,
}

/// Evaluate a counterfactual query inside the supplied causal model.
#[must_use]
pub fn imagine(
    query: &CounterfactualQuery,
    model: &CausalModel,
    mode: ImaginationMode,
) -> ImaginationOutcome {
    let Some(base) = model.base_episode(query) else {
        return ImaginationOutcome {
            query: query.clone(),
            mode,
            plausible: false,
            confidence: 0.0,
            projected_success_delta: 0.0,
            narrative: format!("unknown episode {}", query.episode_id),
        };
    };

    let (variable, new_value) = (&query.intervention.0, &query.intervention.1);
    let support = model.variable_support(variable, new_value);
    let current_value = current_value_for(base, variable);
    let similarity = if current_value.is_empty() {
        0.0
    } else {
        text_fingerprint(&current_value).similarity(&text_fingerprint(new_value)) as f64
    };
    let trust_region = trust_region_floor(variable, support);
    let plausible = similarity >= trust_region || support > 0;
    let projected_success_delta =
        projected_delta(variable, &current_value, new_value, mode, base.success);
    let confidence = (0.35 + similarity * 0.4 + support.min(3) as f64 * 0.1).clamp(0.0, 0.98);
    let narrative = format!(
        "If {} changed from {} to {}, the {} pattern would probably {}",
        variable,
        if current_value.is_empty() {
            "unknown"
        } else {
            current_value.as_str()
        },
        new_value,
        mode.label(),
        if projected_success_delta >= 0.0 {
            "strengthen"
        } else {
            "weaken"
        },
    );

    ImaginationOutcome {
        query: query.clone(),
        mode,
        plausible,
        confidence,
        projected_success_delta,
        narrative,
    }
}

/// Synthesize hypothetical knowledge entries from a batch of episodes.
#[must_use]
pub fn synthesize_hypotheses(
    episodes: &[Episode],
    created_at: DateTime<Utc>,
) -> Vec<KnowledgeEntry> {
    let model = CausalModel::from_episodes(episodes);
    let mut entries = Vec::new();
    if episodes.is_empty() {
        return entries;
    }

    if let Some((left, right)) = choose_combinational_pair(episodes) {
        let query = CounterfactualQuery {
            episode_id: left.id.clone(),
            intervention: ("model".to_string(), right.model.clone()),
        };
        let outcome = imagine(&query, &model, ImaginationMode::Combinational);
        if outcome.plausible {
            entries.push(hypothetical_entry(
                KnowledgeKind::Heuristic,
                &format!(
                    "Combine {} with {}: reuse the successful routing discipline from one episode in the other.",
                    left.task_id, right.task_id
                ),
                &[left.id.clone(), right.id.clone()],
                &[
                    "dream".to_string(),
                    "rem".to_string(),
                    "counterfactual".to_string(),
                    "combinational".to_string(),
                ],
                Some(right.model.clone()),
                created_at,
            ));
        }
    }

    if let Some(base) = choose_exploratory_source(episodes) {
        let query = CounterfactualQuery {
            episode_id: base.id.clone(),
            intervention: ("task_id".to_string(), format!("{}::adjacent", base.task_id)),
        };
        let outcome = imagine(&query, &model, ImaginationMode::Exploratory);
        if outcome.plausible {
            entries.push(hypothetical_entry(
                KnowledgeKind::Heuristic,
                &format!(
                    "Extend {} into a neighboring task shape while keeping the same control pattern.",
                    base.task_id
                ),
                &[base.id.clone()],
                &[
                    "dream".to_string(),
                    "rem".to_string(),
                    "counterfactual".to_string(),
                    "exploratory".to_string(),
                ],
                Some(base.model.clone()),
                created_at,
            ));
        }
    }

    if let Some(base) = choose_transformational_source(episodes) {
        let counterfactual_model = counterfactual_model(&base.model);
        let query = CounterfactualQuery {
            episode_id: base.id.clone(),
            intervention: ("model".to_string(), counterfactual_model.clone()),
        };
        let outcome = imagine(&query, &model, ImaginationMode::Transformational);
        entries.push(hypothetical_entry(
            if base.success {
                KnowledgeKind::Insight
            } else {
                KnowledgeKind::Warning
            },
            &format!(
                "What if {} had used {} instead of {}?",
                base.task_id, counterfactual_model, base.model
            ),
            &[base.id.clone()],
            &[
                "dream".to_string(),
                "rem".to_string(),
                "counterfactual".to_string(),
                "transformational".to_string(),
            ],
            Some(base.model.clone()),
            created_at,
        ));
        if outcome.plausible && outcome.projected_success_delta > 0.0 {
            entries.push(hypothetical_entry(
                KnowledgeKind::Heuristic,
                &format!(
                    "A transformed version of {} might recover the failure mode {}.",
                    base.task_id,
                    base.failure_reason.as_deref().unwrap_or("unknown")
                ),
                &[base.id.clone()],
                &[
                    "dream".to_string(),
                    "rem".to_string(),
                    "counterfactual".to_string(),
                    "validated".to_string(),
                ],
                Some(counterfactual_model),
                created_at,
            ));
        }
    }

    entries
}

/// Build a counterfactual episode by mutating a base episode.
#[must_use]
pub fn counterfactual_episode(base: &Episode, query: &CounterfactualQuery) -> Episode {
    let mut episode = base.clone();
    if query.intervention.0 == "model" {
        episode.model = query.intervention.1.clone();
    } else if query.intervention.0 == "task_id" {
        episode.task_id = query.intervention.1.clone();
    } else if query.intervention.0 == "trigger_kind" {
        episode.trigger_kind = query.intervention.1.clone();
    }
    episode.id = format!("{}-cf", base.id);
    episode.episode_id = if base.episode_id.trim().is_empty() {
        format!("{}-cf", base.id)
    } else {
        format!("{}-cf", base.episode_id)
    };
    episode.extra.insert(
        "dream:counterfactual".to_string(),
        json!({
            "episode_id": base.id,
            "variable": query.intervention.0,
            "value": query.intervention.1,
        }),
    );
    episode
}

fn choose_combinational_pair(episodes: &[Episode]) -> Option<(&Episode, &Episode)> {
    let mut successes: Vec<&Episode> = episodes.iter().filter(|episode| episode.success).collect();
    successes.sort_by(|left, right| {
        right
            .tokens_used
            .cmp(&left.tokens_used)
            .then_with(|| left.task_id.cmp(&right.task_id))
    });
    let first = *successes.first()?;
    let second = successes
        .iter()
        .copied()
        .find(|episode| episode.task_id != first.task_id || episode.model != first.model)?;
    Some((first, second))
}

fn choose_exploratory_source(episodes: &[Episode]) -> Option<&Episode> {
    episodes
        .iter()
        .filter(|episode| episode.success)
        .max_by(|left, right| {
            left.tokens_used
                .cmp(&right.tokens_used)
                .then_with(|| left.timestamp.cmp(&right.timestamp))
        })
}

fn choose_transformational_source(episodes: &[Episode]) -> Option<&Episode> {
    episodes
        .iter()
        .max_by(|left, right| {
            left.failure_reason
                .is_some()
                .cmp(&right.failure_reason.is_some())
                .then_with(|| left.timestamp.cmp(&right.timestamp))
        })
        .or_else(|| episodes.first())
}

fn current_value_for(episode: &Episode, variable: &str) -> String {
    match variable {
        "model" => episode.model.clone(),
        "task_id" => episode.task_id.clone(),
        "trigger_kind" => episode.trigger_kind.clone(),
        "outcome" => {
            if episode.success {
                "success".to_string()
            } else {
                "failure".to_string()
            }
        }
        "failure_reason" => episode.failure_reason.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

fn trust_region_floor(variable: &str, support: usize) -> f64 {
    let base = match variable {
        "model" => 0.32,
        "task_id" => 0.28,
        "trigger_kind" => 0.24,
        "failure_reason" => 0.20,
        _ => 0.30,
    };
    let support_bonus = (support.min(4) as f64) * 0.05;
    (base - support_bonus).clamp(0.10, 0.45)
}

fn projected_delta(
    variable: &str,
    current_value: &str,
    new_value: &str,
    mode: ImaginationMode,
    current_success: bool,
) -> f64 {
    let capability_gain = if is_more_capable(variable, current_value, new_value) {
        0.35
    } else if is_less_capable(variable, current_value, new_value) {
        -0.30
    } else {
        0.05
    };
    let mode_bias = match mode {
        ImaginationMode::Combinational => 0.10,
        ImaginationMode::Exploratory => 0.05,
        ImaginationMode::Transformational => -0.05,
    };
    let success_bias = if current_success { 0.10 } else { 0.20 };
    let delta: f64 = capability_gain + mode_bias + success_bias;
    delta.clamp(-0.75, 0.75)
}

fn is_more_capable(variable: &str, current_value: &str, new_value: &str) -> bool {
    if variable != "model" {
        return false;
    }
    capability_rank(new_value) > capability_rank(current_value)
}

fn is_less_capable(variable: &str, current_value: &str, new_value: &str) -> bool {
    if variable != "model" {
        return false;
    }
    capability_rank(new_value) < capability_rank(current_value)
}

fn capability_rank(value: &str) -> usize {
    let normalized = value.to_ascii_lowercase();
    if normalized.contains("opus") {
        3
    } else if normalized.contains("sonnet") {
        2
    } else if normalized.contains("haiku") {
        1
    } else {
        0
    }
}

fn counterfactual_model(model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return "dream-counterfactual-model".to_string();
    }
    if trimmed.contains("haiku") {
        return trimmed.replace("haiku", "sonnet");
    }
    if trimmed.contains("sonnet") {
        return trimmed.replace("sonnet", "opus");
    }
    if trimmed.contains("fast") {
        return trimmed.replace("fast", "standard");
    }
    format!("{trimmed}-counterfactual")
}

fn hypothetical_entry(
    kind: KnowledgeKind,
    content: &str,
    source_episodes: &[String],
    tags: &[String],
    source_model: Option<String>,
    created_at: DateTime<Utc>,
) -> KnowledgeEntry {
    let mut source_episodes = source_episodes.to_vec();
    source_episodes.sort();
    source_episodes.dedup();
    let mut tags = tags.to_vec();
    tags.sort();
    tags.dedup();
    let source_model = source_model
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty());
    KnowledgeEntry {
        id: dream_imagination_id(kind, content, &source_episodes, &tags),
        kind,
        source: Some("dream".to_string()),
        content: content.to_string(),
        confidence: 0.78,
        confidence_weight: 0.78,
        refuted_insight_id: None,
        refutation_evidence: None,
        source_episodes,
        tags,
        source_model,
        model_generality: 0.85,
        created_at,
        half_life_days: kind.default_half_life_days(),
        tier: KnowledgeTier::Working,
        emotional_tag: None,
        emotional_provenance: None,
        hdc_vector: None,

        confirmation_count: 0,

        distinct_contexts: Vec::new(),

        deprecated: false,
        balance: 1.0,
        frozen: false,
    }
}

fn dream_imagination_id(
    kind: KnowledgeKind,
    content: &str,
    source_episodes: &[String],
    tags: &[String],
) -> String {
    let mut hasher = DefaultHasher::new();
    format!("{kind:?}").hash(&mut hasher);
    content.hash(&mut hasher);
    source_episodes.hash(&mut hasher);
    tags.hash(&mut hasher);
    format!("dream-imagination-{:016x}", hasher.finish())
}

fn bump_variable(
    variables: &mut BTreeMap<String, BTreeMap<String, usize>>,
    variable: &str,
    value: &str,
) {
    variables
        .entry(variable.to_string())
        .or_default()
        .entry(value.to_string())
        .and_modify(|count| *count += 1)
        .or_insert(1);
}

impl ImaginationMode {
    fn label(self) -> &'static str {
        match self {
            Self::Combinational => "combinational",
            Self::Exploratory => "exploratory",
            Self::Transformational => "transformational",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episode(id: &str, task_id: &str, model: &str, success: bool, minutes_ago: i64) -> Episode {
        let timestamp = Utc::now() - chrono::Duration::minutes(minutes_ago);
        let mut episode = Episode::new("agent", task_id);
        episode.id = id.to_string();
        episode.task_id = task_id.to_string();
        episode.model = model.to_string();
        episode.success = success;
        if !success {
            episode.failure_reason = Some("timeout".to_string());
        }
        episode.timestamp = timestamp;
        episode.started_at = timestamp;
        episode.completed_at = timestamp;
        episode
    }

    #[test]
    fn imagine_preserves_mode_and_output_shape() {
        let episodes = vec![episode("a", "task-1", "claude-haiku-4-5", true, 5)];
        let model = CausalModel::from_episodes(&episodes);
        let query = CounterfactualQuery {
            episode_id: "a".to_string(),
            intervention: ("model".to_string(), "claude-sonnet-4-5".to_string()),
        };
        let combinational = imagine(&query, &model, ImaginationMode::Combinational);
        let exploratory = imagine(&query, &model, ImaginationMode::Exploratory);
        let transformational = imagine(&query, &model, ImaginationMode::Transformational);

        for outcome in [&combinational, &exploratory, &transformational] {
            assert_eq!(outcome.query, query);
            assert_eq!(outcome.query.episode_id, "a");
            assert_eq!(outcome.query.intervention.0, "model");
            assert!(outcome.plausible);
            assert!(outcome.confidence > 0.0);
            assert!(outcome.narrative.contains("If model changed from"));
        }
        assert!(combinational.narrative.contains("combinational"));
        assert!(exploratory.narrative.contains("exploratory"));
        assert!(transformational.narrative.contains("transformational"));
        assert!(combinational.projected_success_delta > exploratory.projected_success_delta);
        assert!(exploratory.projected_success_delta > transformational.projected_success_delta);
    }

    #[test]
    fn synthesize_hypotheses_emits_tagged_working_entries() {
        let episodes = vec![
            episode("a", "task-1", "claude-haiku-4-5", true, 3),
            episode("b", "task-2", "claude-haiku-4-5", true, 2),
            episode("c", "task-3", "", false, 1),
        ];
        let entries = synthesize_hypotheses(&episodes, Utc::now());
        assert_eq!(entries.len(), 3);
        assert!(
            entries
                .iter()
                .all(|entry| entry.source.as_deref() == Some("dream"))
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.tier == KnowledgeTier::Working)
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.tags.iter().any(|tag| tag == "dream"))
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.tags.iter().any(|tag| tag == "counterfactual"))
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.tags.iter().any(|tag| tag == "rem"))
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.tags.iter().any(|tag| tag == "combinational"))
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.tags.iter().any(|tag| tag == "exploratory"))
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.tags.iter().any(|tag| tag == "transformational"))
        );
    }
}
