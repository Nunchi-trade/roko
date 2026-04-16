//! NREM replay planning and episode selection.
//!
//! This module keeps the replay surface small and deterministic while still
//! exposing the four replay modes described in the batch docs.

use std::collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use chrono::{DateTime, Utc};
use roko_learn::episode_logger::{Episode, GateVerdict};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Replay mode used by the NREM replay planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamReplayMode {
    /// Sample episodes uniformly using a deterministic pseudo-random ordering.
    Random,
    /// Prioritize episodes with the largest outcome signal.
    Consequence,
    /// Follow the earliest failure chains back toward likely root causes.
    Causal,
    /// Replay counterfactual variants of the strongest episodes.
    Hypothetical,
}

impl Default for DreamReplayMode {
    fn default() -> Self {
        Self::Random
    }
}

/// Replay planning policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamReplayPolicy {
    /// Which replay mode to use.
    #[serde(default)]
    pub mode: DreamReplayMode,
    /// Maximum number of episodes to replay in a single pass.
    #[serde(default = "default_max_episodes")]
    pub max_episodes: usize,
    /// How many prior episodes of the same signature are needed before the
    /// novelty score drops sharply.
    #[serde(default = "default_novelty_window")]
    pub novelty_window: usize,
    /// Half-life used by the recency decay term, in hours.
    #[serde(default = "default_recency_half_life_hours")]
    pub recency_half_life_hours: f64,
}

impl Default for DreamReplayPolicy {
    fn default() -> Self {
        Self {
            mode: DreamReplayMode::Random,
            max_episodes: 24,
            novelty_window: 12,
            recency_half_life_hours: 24.0,
        }
    }
}

/// Summary of the replay batch selected for the current dream pass.
#[derive(Debug, Clone, PartialEq)]
pub struct DreamReplayBatch {
    /// Replay mode that produced the batch.
    pub mode: DreamReplayMode,
    /// Selected episodes in replay order.
    pub episodes: Vec<Episode>,
    /// Total Mattar-Daw utility accumulated by the batch.
    pub utility_score: f64,
    /// Number of hypothetical variants emitted for the batch.
    pub hypothetical_count: usize,
}

/// Select replay episodes according to the supplied policy.
#[must_use]
pub fn select_replay_episodes(
    episodes: &[Episode],
    policy: &DreamReplayPolicy,
    now: DateTime<Utc>,
) -> DreamReplayBatch {
    if episodes.is_empty() {
        return DreamReplayBatch {
            mode: policy.mode,
            episodes: Vec::new(),
            utility_score: 0.0,
            hypothetical_count: 0,
        };
    }

    let candidates = score_candidates(episodes, policy, now);
    let selected = match policy.mode {
        DreamReplayMode::Random => select_random(candidates, policy.max_episodes),
        DreamReplayMode::Consequence => select_consequence(candidates, policy.max_episodes),
        DreamReplayMode::Causal => select_causal(candidates, policy.max_episodes),
        DreamReplayMode::Hypothetical => select_hypothetical(candidates, policy.max_episodes),
    };
    let utility_score = selected
        .iter()
        .map(|candidate| candidate.utility)
        .sum::<f64>();
    let hypothetical_count = selected
        .iter()
        .filter(|candidate| candidate.episode.extra.contains_key("dream:hypothetical"))
        .count();

    DreamReplayBatch {
        mode: policy.mode,
        episodes: selected
            .into_iter()
            .map(|candidate| candidate.episode)
            .collect(),
        utility_score,
        hypothetical_count,
    }
}

#[derive(Debug, Clone)]
struct ReplayCandidate {
    episode: Episode,
    utility: f64,
    random_rank: u64,
}

fn score_candidates(
    episodes: &[Episode],
    policy: &DreamReplayPolicy,
    now: DateTime<Utc>,
) -> Vec<ReplayCandidate> {
    let mut ordered: Vec<&Episode> = episodes.iter().collect();
    ordered.sort_by(|left, right| {
        left.timestamp
            .cmp(&right.timestamp)
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut seen_signatures = BTreeMap::<u64, usize>::new();
    let mut out = Vec::with_capacity(ordered.len());
    for episode in ordered {
        let signature = episode_signature(episode);
        let novelty_hits = seen_signatures.entry(signature).or_insert(0);
        let novelty = 1.0 / (1.0 + (*novelty_hits as f64 / policy.novelty_window.max(1) as f64));
        *novelty_hits += 1;

        let reward = episode_reward_magnitude(episode);
        let recency = recency_decay(episode.timestamp, now, policy.recency_half_life_hours);
        let utility = reward * novelty * recency;
        let random_rank = pseudo_random_rank(episode, signature);
        out.push(ReplayCandidate {
            episode: episode.clone(),
            utility,
            random_rank,
        });
    }
    out
}

fn select_random(
    mut candidates: Vec<ReplayCandidate>,
    max_episodes: usize,
) -> Vec<ReplayCandidate> {
    candidates.sort_by(|left, right| {
        left.random_rank
            .cmp(&right.random_rank)
            .then_with(|| left.episode.timestamp.cmp(&right.episode.timestamp))
    });
    candidates.truncate(max_episodes);
    candidates
}

fn select_consequence(
    mut candidates: Vec<ReplayCandidate>,
    max_episodes: usize,
) -> Vec<ReplayCandidate> {
    candidates.sort_by(|left, right| {
        right
            .utility
            .partial_cmp(&left.utility)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.episode.timestamp.cmp(&right.episode.timestamp))
    });
    candidates.truncate(max_episodes);
    candidates
}

fn select_causal(
    mut candidates: Vec<ReplayCandidate>,
    max_episodes: usize,
) -> Vec<ReplayCandidate> {
    let original_candidates = candidates.clone();
    let mut groups = BTreeMap::<String, Vec<ReplayCandidate>>::new();
    for candidate in candidates.drain(..) {
        groups
            .entry(replay_chain_key(&candidate.episode))
            .or_default()
            .push(candidate);
    }

    let mut roots = Vec::new();
    for mut chain in groups.into_values() {
        chain.sort_by(|left, right| {
            left.episode
                .timestamp
                .cmp(&right.episode.timestamp)
                .then_with(|| left.episode.id.cmp(&right.episode.id))
        });

        let failure_index = chain
            .iter()
            .position(|candidate| !candidate.episode.success);
        if let Some(index) = failure_index {
            roots.push(chain[index].clone());
            if index > 0 {
                roots.push(chain[index - 1].clone());
            }
        }
    }

    if roots.is_empty() {
        return select_consequence(original_candidates, max_episodes);
    }

    dedupe_by_episode_id(&mut roots);
    roots.sort_by(|left, right| {
        left.episode
            .timestamp
            .cmp(&right.episode.timestamp)
            .then_with(|| left.episode.id.cmp(&right.episode.id))
    });
    roots.truncate(max_episodes);
    roots
}

fn select_hypothetical(
    candidates: Vec<ReplayCandidate>,
    max_episodes: usize,
) -> Vec<ReplayCandidate> {
    let mut selected = select_consequence(candidates, max_episodes);
    for (index, candidate) in selected.iter_mut().enumerate() {
        candidate.episode = hypothetical_variant(&candidate.episode, index);
        candidate.utility *= 0.95;
    }
    selected
}

fn dedupe_by_episode_id(candidates: &mut Vec<ReplayCandidate>) {
    let mut seen = BTreeSet::new();
    candidates.retain(|candidate| seen.insert(candidate.episode.id.clone()));
}

fn episode_reward_magnitude(episode: &Episode) -> f64 {
    let gate_total = episode.gate_verdicts.len().max(1) as f64;
    let pass_count = episode
        .gate_verdicts
        .iter()
        .filter(|verdict| verdict.passed)
        .count() as f64;
    let fail_count = gate_total - pass_count;
    let outcome_strength = if episode.success {
        1.0 + pass_count / gate_total
    } else {
        1.0 + fail_count / gate_total
    };
    let token_factor = (episode.tokens_used.max(1) as f64).log10().clamp(0.0, 3.0) * 0.05;
    outcome_strength + token_factor
}

fn recency_decay(timestamp: DateTime<Utc>, now: DateTime<Utc>, half_life_hours: f64) -> f64 {
    let age_hours = (now - timestamp).num_seconds().max(0) as f64 / 3600.0;
    let half_life = half_life_hours.max(0.1);
    (-age_hours / half_life).exp()
}

fn episode_signature(episode: &Episode) -> u64 {
    let mut hasher = DefaultHasher::new();
    episode.task_id.hash(&mut hasher);
    episode.model.hash(&mut hasher);
    episode.trigger_kind.hash(&mut hasher);
    episode.success.hash(&mut hasher);
    episode.failure_reason.hash(&mut hasher);
    for GateVerdict {
        gate,
        passed,
        signature,
    } in &episode.gate_verdicts
    {
        gate.hash(&mut hasher);
        passed.hash(&mut hasher);
        signature.hash(&mut hasher);
    }
    hasher.finish()
}

fn pseudo_random_rank(episode: &Episode, signature: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    signature.hash(&mut hasher);
    episode.id.hash(&mut hasher);
    episode.timestamp.timestamp_millis().hash(&mut hasher);
    hasher.finish()
}

fn replay_chain_key(episode: &Episode) -> String {
    if episode.task_id.trim().is_empty() {
        episode.id.clone()
    } else {
        episode.task_id.clone()
    }
}

fn hypothetical_variant(episode: &Episode, index: usize) -> Episode {
    let mut variant = episode.clone();
    variant.id = format!("{}-hyp-{index}", episode.id);
    if !variant.episode_id.trim().is_empty() {
        variant.episode_id = format!("{}-hyp-{index}", episode.episode_id);
    }
    variant.model = counterfactual_model(&episode.model);
    variant.trigger_kind = "dream:hypothetical".to_string();
    variant.extra.insert(
        "dream:hypothetical".to_string(),
        json!({
            "mode": "hypothetical",
            "source_episode_id": episode.id,
            "source_model": episode.model,
            "counterfactual_model": variant.model,
            "source_success": episode.success,
        }),
    );
    if !variant.success {
        variant.success = true;
        variant.failure_reason = None;
    }
    variant
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

fn default_max_episodes() -> usize {
    24
}

fn default_novelty_window() -> usize {
    12
}

fn default_recency_half_life_hours() -> f64 {
    24.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episode(
        id: &str,
        task_id: &str,
        model: &str,
        success: bool,
        failure_reason: Option<&str>,
        minutes_ago: i64,
        tokens_used: u64,
    ) -> Episode {
        let timestamp = Utc::now() - chrono::Duration::minutes(minutes_ago);
        let mut episode = Episode::new("agent", task_id);
        episode.id = id.to_string();
        episode.task_id = task_id.to_string();
        episode.model = model.to_string();
        episode.success = success;
        episode.failure_reason = failure_reason.map(str::to_owned);
        episode.timestamp = timestamp;
        episode.started_at = timestamp;
        episode.completed_at = timestamp;
        episode.tokens_used = tokens_used;
        episode
    }

    #[test]
    fn consequence_mode_prefers_high_signal_episodes() {
        let episodes = vec![
            episode("a", "task-1", "haiku", true, None, 30, 10),
            episode("b", "task-2", "haiku", false, Some("timeout"), 5, 500),
            episode("c", "task-3", "haiku", true, None, 2, 20),
        ];
        let policy = DreamReplayPolicy {
            mode: DreamReplayMode::Consequence,
            max_episodes: 2,
            ..DreamReplayPolicy::default()
        };
        let batch = select_replay_episodes(&episodes, &policy, Utc::now());
        assert_eq!(batch.episodes.len(), 2);
        assert_eq!(batch.episodes[0].id, "b");
    }

    #[test]
    fn causal_mode_returns_failure_chain_roots() {
        let episodes = vec![
            episode("a", "task-1", "haiku", true, None, 30, 10),
            episode("b", "task-1", "haiku", false, Some("timeout"), 20, 10),
            episode("c", "task-1", "haiku", false, Some("timeout"), 10, 10),
            episode("d", "task-2", "haiku", true, None, 5, 10),
        ];
        let policy = DreamReplayPolicy {
            mode: DreamReplayMode::Causal,
            max_episodes: 4,
            ..DreamReplayPolicy::default()
        };
        let batch = select_replay_episodes(&episodes, &policy, Utc::now());
        assert!(!batch.episodes.is_empty());
        assert!(batch.episodes.iter().any(|episode| episode.id == "b"));
    }

    #[test]
    fn hypothetical_mode_mutates_selected_episodes() {
        let episodes = vec![episode(
            "a",
            "task-1",
            "claude-haiku-4-5",
            false,
            Some("timeout"),
            5,
            20,
        )];
        let policy = DreamReplayPolicy {
            mode: DreamReplayMode::Hypothetical,
            max_episodes: 1,
            ..DreamReplayPolicy::default()
        };
        let batch = select_replay_episodes(&episodes, &policy, Utc::now());
        assert_eq!(batch.episodes.len(), 1);
        assert!(batch.episodes[0].id.contains("-hyp-0"));
        assert!(batch.episodes[0].success);
        assert_eq!(batch.episodes[0].trigger_kind, "dream:hypothetical");
    }
}
