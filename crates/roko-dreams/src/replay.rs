//! NREM replay planning and episode selection.
//!
//! This module keeps the replay surface small and deterministic while still
//! exposing the four replay modes described in the batch docs.

use std::collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use chrono::{DateTime, Utc};
use roko_core::PadVector;
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

/// Mattar-Daw utility score for replay candidate prioritization.
///
/// `U(episode) = gain * need * (1/spacing)` -- replay episodes with high
/// expected learning value AND high policy relevance.
///
/// Reference: Mattar & Daw (2018) *Nature Neuroscience*.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ReplayUtility {
    /// Expected improvement from replaying this episode.  Derived from
    /// prediction error: gate failures and surprising outcomes produce high gain.
    pub gain: f64,
    /// How policy-relevant this episode is.  Combines recency-weighted visit
    /// frequency with knowledge-confidence signals.
    pub need: f64,
    /// Inverse of time since last replay (spaced-repetition term).  Episodes
    /// not recently replayed get higher spacing scores.
    pub spacing_inv: f64,
    /// Final utility: `gain * need * spacing_inv`.
    pub utility: f64,
}

impl ReplayUtility {
    /// Compute the Mattar-Daw utility score for a single episode.
    #[must_use]
    pub fn compute(
        episode: &Episode,
        novelty: f64,
        recency: f64,
        config: &MattarDawConfig,
    ) -> Self {
        let gain = Self::compute_gain(episode) * config.gain_weight;
        let need = Self::compute_need(novelty, recency) * config.need_weight;
        let spacing_inv = Self::compute_spacing_inv(episode, recency);
        let utility = gain * need * spacing_inv;
        Self {
            gain,
            need,
            spacing_inv,
            utility,
        }
    }

    /// Gain: derived from prediction error.  Gate failures have high gain
    /// (more to learn), clean passes have low gain.
    fn compute_gain(episode: &Episode) -> f64 {
        let gate_total = episode.gate_verdicts.len().max(1) as f64;
        let fail_count = episode.gate_verdicts.iter().filter(|v| !v.passed).count() as f64;
        // Prediction error: how surprising was the outcome?
        let error_rate = fail_count / gate_total;
        let surprise = if episode.success {
            // Successful but with some failures -- moderately surprising
            1.0 + error_rate * 0.5
        } else {
            // Failed -- high prediction error
            1.5 + error_rate
        };
        // Token usage as a secondary signal (complex tasks have more to learn)
        let complexity = (episode.tokens_used.max(1) as f64).log10().clamp(0.0, 3.0) * 0.1;
        surprise + complexity
    }

    /// Need: how much the current policy needs updating.  Combines novelty
    /// (low for frequently-seen patterns) with recency (recent episodes are
    /// more policy-relevant).
    fn compute_need(novelty: f64, recency: f64) -> f64 {
        // Novel episodes have high need; familiar ones decay
        let novelty_term = novelty.clamp(0.0, 1.0);
        // Recent episodes are more relevant to the current policy
        let recency_term = recency.clamp(0.0, 1.0);
        // Weighted combination: novelty matters more than raw recency
        0.6 * novelty_term + 0.4 * recency_term
    }

    /// Spacing inverse: inverse of effective "time since last useful replay".
    /// Uses recency as a proxy -- episodes replayed long ago get higher scores
    /// (spaced repetition effect).
    fn compute_spacing_inv(episode: &Episode, recency: f64) -> f64 {
        // If already recently replayed (high recency), spacing is low
        // If not recently seen (low recency), spacing is high
        let base_spacing = 1.0 - recency.clamp(0.0, 0.99);
        // Boost for episodes that were never replayed (no dream marker)
        let never_replayed = !episode.extra.contains_key("dream:replayed");
        let boost = if never_replayed { 1.5 } else { 1.0 };
        (base_spacing * boost).max(0.01)
    }
}

/// Configurable weights for Mattar-Daw gain/need terms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MattarDawConfig {
    /// Weight applied to the gain term (default 1.0).
    #[serde(default = "default_mattar_daw_weight")]
    pub gain_weight: f64,
    /// Weight applied to the need term (default 1.0).
    #[serde(default = "default_mattar_daw_weight")]
    pub need_weight: f64,
}

impl Default for MattarDawConfig {
    fn default() -> Self {
        Self {
            gain_weight: 1.0,
            need_weight: 1.0,
        }
    }
}

fn default_mattar_daw_weight() -> f64 {
    1.0
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
    /// Mattar-Daw gain/need weights.
    #[serde(default)]
    pub mattar_daw: MattarDawConfig,
}

impl Default for DreamReplayPolicy {
    fn default() -> Self {
        Self {
            mode: DreamReplayMode::Random,
            max_episodes: 24,
            novelty_window: 12,
            recency_half_life_hours: 24.0,
            mattar_daw: MattarDawConfig::default(),
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

/// Select replay episodes with optional emotional biasing from the daimon.
///
/// When `emotional_context` is `Some`, the PAD vector modulates replay:
/// - Negative pleasure (valence) biases toward replaying failures
/// - High arousal increases effective `max_episodes` (more intense replay)
#[must_use]
pub fn select_replay_episodes_with_affect(
    episodes: &[Episode],
    policy: &DreamReplayPolicy,
    now: DateTime<Utc>,
    emotional_context: Option<&PadVector>,
) -> DreamReplayBatch {
    let Some(pad) = emotional_context else {
        return select_replay_episodes(episodes, policy, now);
    };

    // Arousal-based intensity: high arousal (>0) increases max_episodes by up to 50%.
    let arousal_factor = 1.0 + 0.5 * pad.arousal.max(0.0);
    let effective_max = ((policy.max_episodes as f64) * arousal_factor).round() as usize;

    let adjusted_policy = DreamReplayPolicy {
        max_episodes: effective_max,
        ..policy.clone()
    };

    // Score candidates with emotional bias.
    let mut candidates = score_candidates(episodes, &adjusted_policy, now);

    // Negative pleasure biases toward failure episodes (multiplier on failed episodes).
    // At pleasure = -1.0, failures get 1.5x utility; at pleasure = 0 or positive, no change.
    let failure_bias = 1.0 + 0.5 * (-pad.pleasure).max(0.0);
    for candidate in &mut candidates {
        if !candidate.episode.success {
            candidate.utility *= failure_bias;
        }
    }

    let selected = match adjusted_policy.mode {
        DreamReplayMode::Random => select_random(candidates, effective_max),
        DreamReplayMode::Consequence => select_consequence(candidates, effective_max),
        DreamReplayMode::Causal => select_causal(candidates, effective_max),
        DreamReplayMode::Hypothetical => select_hypothetical(candidates, effective_max),
    };

    let utility_score = selected.iter().map(|c| c.utility).sum::<f64>();
    let hypothetical_count = selected
        .iter()
        .filter(|c| c.episode.extra.contains_key("dream:hypothetical"))
        .count();

    DreamReplayBatch {
        mode: adjusted_policy.mode,
        episodes: selected.into_iter().map(|c| c.episode).collect(),
        utility_score,
        hypothetical_count,
    }
}

#[derive(Debug, Clone)]
struct ReplayCandidate {
    episode: Episode,
    utility: f64,
    #[allow(dead_code)]
    mattar_daw: ReplayUtility,
    random_rank: u64,
}

/// Compute Mattar-Daw utility score for a single episode.
///
/// Public entry point for external callers that want the decomposed score
/// without running the full replay selection pipeline.
#[must_use]
pub fn compute_replay_utility(
    episode: &Episode,
    policy: &DreamReplayPolicy,
    now: DateTime<Utc>,
) -> ReplayUtility {
    let recency = recency_decay(episode.timestamp, now, policy.recency_half_life_hours);
    // Standalone call uses novelty=1.0 (unknown context)
    ReplayUtility::compute(episode, 1.0, recency, &policy.mattar_daw)
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

        let recency = recency_decay(episode.timestamp, now, policy.recency_half_life_hours);
        let mattar_daw = ReplayUtility::compute(episode, novelty, recency, &policy.mattar_daw);
        let utility = mattar_daw.utility;
        let random_rank = pseudo_random_rank(episode, signature);
        out.push(ReplayCandidate {
            episode: episode.clone(),
            utility,
            mattar_daw,
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
        gate_verdicts: Vec<GateVerdict>,
    ) -> Episode {
        let timestamp = Utc::now() - chrono::Duration::minutes(minutes_ago);
        let mut episode = Episode::new("agent", task_id);
        episode.id = id.to_string();
        episode.task_id = task_id.to_string();
        episode.model = model.to_string();
        episode.success = success;
        episode.failure_reason = failure_reason.map(str::to_owned);
        episode.gate_verdicts = gate_verdicts;
        episode.timestamp = timestamp;
        episode.started_at = timestamp;
        episode.completed_at = timestamp;
        episode.tokens_used = tokens_used;
        episode
    }

    #[test]
    fn consequence_mode_prefers_higher_utility_and_reports_score() {
        let episodes = vec![
            episode(
                "a",
                "task-1",
                "haiku",
                true,
                None,
                10,
                1_000,
                vec![
                    GateVerdict::new("compile", true),
                    GateVerdict::new("test", true),
                ],
            ),
            episode(
                "b",
                "task-2",
                "haiku",
                true,
                None,
                10,
                100,
                vec![GateVerdict::new("compile", true)],
            ),
            episode(
                "c",
                "task-2",
                "haiku",
                true,
                None,
                10,
                100,
                vec![GateVerdict::new("compile", true)],
            ),
        ];
        let policy = DreamReplayPolicy {
            mode: DreamReplayMode::Consequence,
            max_episodes: 3,
            ..DreamReplayPolicy::default()
        };
        let now = Utc::now();
        let batch = select_replay_episodes(&episodes, &policy, now);
        let scored = score_candidates(&episodes, &policy, now);
        let scored_by_id = scored
            .into_iter()
            .map(|candidate| (candidate.episode.id, candidate.utility))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            batch
                .episodes
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        assert_eq!(batch.mode, DreamReplayMode::Consequence);
        assert_eq!(batch.hypothetical_count, 0);
        let expected_utility = batch
            .episodes
            .iter()
            .map(|episode| scored_by_id[&episode.id])
            .sum::<f64>();
        assert!((batch.utility_score - expected_utility).abs() < 1e-9);
    }

    #[test]
    fn causal_mode_returns_failure_chain_roots() {
        let episodes = vec![
            episode(
                "a",
                "task-1",
                "haiku",
                true,
                None,
                30,
                10,
                vec![GateVerdict::new("compile", true)],
            ),
            episode(
                "b",
                "task-1",
                "haiku",
                false,
                Some("timeout"),
                20,
                10,
                vec![GateVerdict::new("compile", false)],
            ),
            episode(
                "c",
                "task-1",
                "haiku",
                false,
                Some("timeout"),
                10,
                10,
                vec![GateVerdict::new("compile", false)],
            ),
            episode(
                "d",
                "task-2",
                "haiku",
                true,
                None,
                5,
                10,
                vec![GateVerdict::new("compile", true)],
            ),
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
            vec![GateVerdict::new("compile", false)],
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

    #[test]
    fn random_mode_is_deterministic_and_respects_max_episodes() {
        let episodes = vec![
            episode(
                "a",
                "task-1",
                "haiku",
                true,
                None,
                10,
                1_000,
                vec![GateVerdict::new("compile", true)],
            ),
            episode(
                "b",
                "task-2",
                "haiku",
                true,
                None,
                10,
                100,
                vec![GateVerdict::new("compile", true)],
            ),
            episode(
                "c",
                "task-3",
                "haiku",
                true,
                None,
                10,
                100,
                vec![GateVerdict::new("compile", true)],
            ),
        ];
        let policy = DreamReplayPolicy {
            mode: DreamReplayMode::Random,
            max_episodes: 2,
            ..DreamReplayPolicy::default()
        };
        let first = select_replay_episodes(&episodes, &policy, Utc::now());
        let second = select_replay_episodes(&episodes, &policy, Utc::now());

        assert_eq!(first.mode, DreamReplayMode::Random);
        assert_eq!(first.episodes.len(), 2);
        assert_eq!(
            first
                .episodes
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>(),
            second
                .episodes
                .iter()
                .map(|episode| episode.id.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn emotional_context_none_delegates_to_base() {
        let episodes = vec![episode(
            "a",
            "task-1",
            "haiku",
            true,
            None,
            10,
            100,
            vec![GateVerdict::new("compile", true)],
        )];
        let policy = DreamReplayPolicy::default();
        let now = Utc::now();
        let base = select_replay_episodes(&episodes, &policy, now);
        let with_affect = select_replay_episodes_with_affect(&episodes, &policy, now, None);
        assert_eq!(base.episodes.len(), with_affect.episodes.len());
    }

    #[test]
    fn high_arousal_increases_effective_max() {
        let episodes: Vec<_> = (0..10)
            .map(|i| {
                episode(
                    &format!("ep-{i}"),
                    &format!("task-{i}"),
                    "haiku",
                    true,
                    None,
                    i * 5,
                    100,
                    vec![GateVerdict::new("compile", true)],
                )
            })
            .collect();
        let policy = DreamReplayPolicy {
            mode: DreamReplayMode::Consequence,
            max_episodes: 4,
            ..DreamReplayPolicy::default()
        };
        let now = Utc::now();
        let neutral = select_replay_episodes(&episodes, &policy, now);
        let high_arousal = PadVector::new(0.0, 1.0, 0.0);
        let biased =
            select_replay_episodes_with_affect(&episodes, &policy, now, Some(&high_arousal));
        assert!(biased.episodes.len() >= neutral.episodes.len());
    }

    #[test]
    fn negative_pleasure_biases_toward_failures() {
        let episodes = vec![
            episode(
                "success",
                "task-1",
                "haiku",
                true,
                None,
                5,
                100,
                vec![GateVerdict::new("compile", true)],
            ),
            episode(
                "failure",
                "task-2",
                "haiku",
                false,
                Some("error"),
                5,
                100,
                vec![GateVerdict::new("compile", false)],
            ),
        ];
        let policy = DreamReplayPolicy {
            mode: DreamReplayMode::Consequence,
            max_episodes: 2,
            ..DreamReplayPolicy::default()
        };
        let now = Utc::now();
        let negative_pad = PadVector::new(-1.0, 0.0, 0.0);
        let biased =
            select_replay_episodes_with_affect(&episodes, &policy, now, Some(&negative_pad));
        assert!(!biased.episodes.is_empty());
        assert!(!biased.episodes[0].success);
    }

    #[test]
    fn mattar_daw_utility_prefers_failed_episodes() {
        let success = episode(
            "s",
            "task-1",
            "haiku",
            true,
            None,
            10,
            100,
            vec![GateVerdict::new("compile", true)],
        );
        let failure = episode(
            "f",
            "task-2",
            "haiku",
            false,
            Some("timeout"),
            10,
            100,
            vec![GateVerdict::new("compile", false)],
        );
        let config = MattarDawConfig::default();
        let recency = 0.5;
        let novelty = 1.0;
        let u_success = ReplayUtility::compute(&success, novelty, recency, &config);
        let u_failure = ReplayUtility::compute(&failure, novelty, recency, &config);
        // Failed episodes should have higher gain (more prediction error)
        assert!(u_failure.gain > u_success.gain);
        assert!(u_failure.utility > u_success.utility);
    }

    #[test]
    fn mattar_daw_configurable_weights() {
        let ep = episode(
            "a",
            "task-1",
            "haiku",
            false,
            Some("err"),
            10,
            100,
            vec![GateVerdict::new("compile", false)],
        );
        let default_config = MattarDawConfig::default();
        let boosted = MattarDawConfig {
            gain_weight: 2.0,
            need_weight: 1.0,
        };
        let u_default = ReplayUtility::compute(&ep, 1.0, 0.5, &default_config);
        let u_boosted = ReplayUtility::compute(&ep, 1.0, 0.5, &boosted);
        assert!(u_boosted.utility > u_default.utility);
    }

    #[test]
    fn compute_replay_utility_standalone() {
        let ep = episode(
            "a",
            "task-1",
            "haiku",
            true,
            None,
            30,
            100,
            vec![GateVerdict::new("compile", true)],
        );
        let policy = DreamReplayPolicy::default();
        let u = compute_replay_utility(&ep, &policy, Utc::now());
        assert!(u.utility > 0.0);
        assert!(u.gain > 0.0);
        assert!(u.need > 0.0);
        assert!(u.spacing_inv > 0.0);
    }
}
