//! Append-only JSONL episode logger.
//!
//! Implements the episode-logger component described in
//! `tmp/roko-progress/COMPONENTS/learn/episode-logger.md` and parity
//! checklist §16.1.1–§16.1.3. Each agent turn produces one [`Episode`]
//! record that is persisted as a single line of JSON on disk. The log is
//! append-only: records are never modified in place, and concurrent
//! writers are serialized through a process-wide [`parking_lot::Mutex`].
//!
//! The reader is tolerant: lines that fail to parse (a common outcome of
//! a crash mid-write or of forward-compatible schema changes) are
//! surfaced through a dedicated error variant rather than corrupting the
//! whole stream — callers choose whether to stop or continue.
//!
//! # Example
//!
//! ```no_run
//! use roko_learn::episode_logger::{Episode, EpisodeLogger};
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let logger = EpisodeLogger::new("/tmp/episodes.jsonl");
//! let ep = Episode::new("agent-1", "task-42");
//! logger.append(&ep).await?;
//! let all = EpisodeLogger::read_all("/tmp/episodes.jsonl").await?;
//! assert_eq!(all.len(), 1);
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex as SyncMutex;
use roko_core::{EmotionalTag, Engram};
use roko_primitives::hdc::{HdcVector, text_fingerprint};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;

/// Maximum serialized size (in bytes) of a single episode's `extra`
/// field. Enforced in [`EpisodeLogger::append`] so that a runaway
/// optimizer cannot blow up the log.
const MAX_EXTRA_BYTES: usize = 16 * 1024;
const TEXT_FINGERPRINT_KEY: &str = "text_fingerprint";
const METADATA_FINGERPRINT_KEY: &str = "metadata_fingerprint";
const TEMPLATE_SUGGESTION_MIN_SIMILARITY: f64 = 0.7;
const TEMPLATE_SUGGESTION_MAX_AGE_DAYS: i64 = 30;
const TEMPLATE_SUGGESTION_MAX_CANDIDATES: usize = 256;

/// Errors that can occur while appending to or reading from an episode
/// log.
#[derive(Debug, Error)]
pub enum LoggerError {
    /// An underlying filesystem call failed.
    #[error("episode logger i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization of an [`Episode`] to JSON failed. In practice this
    /// can only happen if a caller stuffs a non-serializable value into
    /// the `extra` map.
    #[error("episode logger serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    /// A JSONL line could not be deserialized as an [`Episode`]. The
    /// offending 1-based line number and the parser diagnostic are
    /// attached.
    #[error("episode logger parse error on line {line}: {source}")]
    Parse {
        /// 1-based line index within the JSONL file.
        line: usize,
        /// Underlying `serde_json` error.
        #[source]
        source: serde_json::Error,
    },
    /// The caller's `extra` map exceeds [`MAX_EXTRA_BYTES`] once
    /// serialized.
    #[error("episode `extra` field too large: {size} bytes (max {max})")]
    ExtraTooLarge {
        /// Serialized size in bytes.
        size: usize,
        /// Configured maximum.
        max: usize,
    },
}

/// Verdict produced by a single gate run on behalf of an agent turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateVerdict {
    /// Gate identifier ("compile", "test", "lint", …).
    #[serde(default)]
    pub gate: String,
    /// Whether the gate passed.
    #[serde(default)]
    pub passed: bool,
    /// Optional short diagnostic (hashed, never raw output).
    #[serde(default)]
    pub signature: Option<String>,
}

impl GateVerdict {
    /// Construct a new verdict.
    #[must_use]
    pub fn new(gate: impl Into<String>, passed: bool) -> Self {
        Self {
            gate: gate.into(),
            passed,
            signature: None,
        }
    }

    /// Attach an error signature to the verdict.
    #[must_use]
    pub fn with_signature(mut self, signature: impl Into<String>) -> Self {
        self.signature = Some(signature.into());
        self
    }
}

/// Token / cost / wall-clock accounting for one agent turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    /// Prompt/input tokens consumed.
    #[serde(default)]
    pub input_tokens: u64,
    /// Completion/output tokens produced.
    #[serde(default)]
    pub output_tokens: u64,
    /// Tokens read from the provider-side cache.
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Tokens written to the provider-side cache.
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Dollar cost after cache discounts.
    #[serde(default)]
    pub cost_usd: f64,
    /// Dollar cost if the cache were cold (for regret accounting).
    #[serde(default)]
    pub cost_usd_without_cache: f64,
    /// Wall-clock latency, in milliseconds.
    #[serde(default)]
    pub wall_ms: u64,
}

impl Usage {
    /// Convenience constructor for the two most common fields.
    #[must_use]
    pub const fn tokens(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 0.0,
            cost_usd_without_cache: 0.0,
            wall_ms: 0,
        }
    }
}

/// One episode per completed agent turn.
///
/// The schema is intentionally forward-compatible: every field carries
/// `#[serde(default)]` so that older log lines continue to deserialize
/// after new fields are added.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Episode {
    /// Logical episode kind (for example `"agent_turn"`, `"gate"`, `"replan"`).
    #[serde(default)]
    pub kind: String,
    /// Stable episode identifier (hash-derived).
    #[serde(default)]
    pub id: String,
    /// Wall-clock timestamp captured when the episode was constructed.
    #[serde(default = "Utc::now")]
    pub timestamp: DateTime<Utc>,
    /// Agent that produced the turn (e.g. `"claude-implementer"`).
    #[serde(default)]
    pub agent_id: String,
    /// Task identifier the agent was working on.
    #[serde(default)]
    pub task_id: String,
    /// Hash of the input signal that seeded the turn.
    #[serde(default)]
    pub input_signal_hash: String,
    /// Hash of the output signal the agent produced.
    #[serde(default)]
    pub output_signal_hash: String,
    /// Stable identifier for the episode record.
    #[serde(default)]
    pub episode_id: String,
    /// Template or role name used to dispatch the agent.
    #[serde(default)]
    pub agent_template: String,
    /// Model slug used for the dispatch.
    #[serde(default)]
    pub model: String,
    /// Backend/provider slug used for the dispatch.
    #[serde(default)]
    pub backend: String,
    /// Trigger kind that caused the dispatch.
    #[serde(default)]
    pub trigger_kind: String,
    /// Hash of the trigger signal.
    #[serde(default)]
    pub trigger_signal_hash: String,
    /// Time when the dispatch started.
    #[serde(default = "Utc::now")]
    pub started_at: DateTime<Utc>,
    /// Time when the dispatch completed.
    #[serde(default = "Utc::now")]
    pub completed_at: DateTime<Utc>,
    /// Dispatch duration in seconds.
    #[serde(default)]
    pub duration_secs: f64,
    /// Individual gate verdicts observed for the turn.
    #[serde(default)]
    pub gate_verdicts: Vec<GateVerdict>,
    /// Token / cost / latency accounting.
    #[serde(default)]
    pub usage: Usage,
    /// Whether the turn is considered successful overall.
    #[serde(default)]
    pub success: bool,
    /// Number of agent turns observed for this episode.
    #[serde(default)]
    pub turns: u64,
    /// Total tokens consumed by the agent run.
    #[serde(default)]
    pub tokens_used: u64,
    /// External actions emitted while handling the trigger.
    #[serde(default)]
    pub external_actions: Vec<serde_json::Value>,
    /// Optional short failure reason (hashed, never raw output).
    #[serde(default)]
    pub failure_reason: Option<String>,
    /// Optional post-gate reflection text for retry and playbook learning.
    #[serde(default)]
    pub reflection: Option<String>,
    /// Optional short reasoning summary for auditing and debugging.
    #[serde(default)]
    pub reasoning_summary: Option<String>,
    /// Optional opaque HDC fingerprint derived from the episode prompt/outcome pair.
    #[serde(default)]
    pub hdc_fingerprint: Option<String>,
    /// Optional affect signature captured when the episode completed.
    #[serde(default)]
    pub emotional_tag: Option<EmotionalTag>,
    /// Mark this episode as a headline — headline episodes are never
    /// pruned by [`EpisodeLogger::compact`], regardless of age or count.
    #[serde(default)]
    pub headline: bool,
    /// Forward-compat extension bag. Must serialize to ≤
    /// [`MAX_EXTRA_BYTES`].
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Episode {
    /// Construct a minimal episode for `agent_id` / `task_id` with a
    /// hash-derived id and `timestamp = Utc::now()`. All other fields
    /// take their defaults.
    #[must_use]
    pub fn new(agent_id: impl Into<String>, task_id: impl Into<String>) -> Self {
        let agent_id = agent_id.into();
        let task_id = task_id.into();
        let timestamp = Utc::now();
        let started_at = timestamp.clone();
        let completed_at = timestamp;
        let id = derive_id(&agent_id, &task_id, completed_at.clone());
        Self {
            kind: String::new(),
            id,
            timestamp,
            agent_id,
            task_id,
            input_signal_hash: String::new(),
            output_signal_hash: String::new(),
            episode_id: String::new(),
            agent_template: String::new(),
            model: String::new(),
            backend: String::new(),
            trigger_kind: String::new(),
            trigger_signal_hash: String::new(),
            started_at,
            completed_at,
            duration_secs: 0.0,
            gate_verdicts: Vec::new(),
            usage: Usage::default(),
            success: false,
            turns: 0,
            tokens_used: 0,
            external_actions: Vec::new(),
            failure_reason: None,
            reflection: None,
            reasoning_summary: None,
            hdc_fingerprint: None,
            emotional_tag: None,
            headline: false,
            extra: HashMap::new(),
        }
    }

    /// Record the turn as successful.
    #[must_use]
    pub const fn succeeded(mut self) -> Self {
        self.success = true;
        self
    }

    /// Attach a failure reason and mark the turn as failed.
    #[must_use]
    pub fn failed(mut self, reason: impl Into<String>) -> Self {
        self.success = false;
        self.failure_reason = Some(reason.into());
        self
    }

    /// Attach an emotional tag to the episode.
    #[must_use]
    pub fn with_emotional_tag(mut self, emotional_tag: EmotionalTag) -> Self {
        self.emotional_tag = Some(emotional_tag);
        self
    }

    /// Attach a deterministic fingerprint of the completed episode text.
    ///
    /// # Panics
    ///
    /// Panics if the computed fingerprint cannot be serialized into JSON for
    /// storage in the episode metadata map.
    pub fn attach_text_fingerprint(&mut self) {
        let text = self.completion_fingerprint_text();
        let fingerprint = text_fingerprint(&text);
        self.extra.insert(
            TEXT_FINGERPRINT_KEY.to_string(),
            serde_json::to_value(fingerprint)
                .expect("HDC text fingerprint serialization should not fail"),
        );
    }

    /// Attach a deterministic HDC fingerprint of the episode's structured
    /// metadata: agent_id, task_id, model, gate verdicts, and success.
    ///
    /// This fingerprint captures the *shape* of the execution (who ran it,
    /// what model, which gates fired, did it succeed) rather than the
    /// textual content. It enables similarity search across episodes with
    /// structurally similar execution profiles.
    ///
    /// # Panics
    ///
    /// Panics if the computed metadata fingerprint cannot be serialized into
    /// JSON for storage in the episode metadata map.
    pub fn attach_metadata_fingerprint(&mut self) {
        let text = self.metadata_fingerprint_text();
        let fingerprint = text_fingerprint(&text);
        self.extra.insert(
            METADATA_FINGERPRINT_KEY.to_string(),
            serde_json::to_value(fingerprint)
                .expect("HDC metadata fingerprint serialization should not fail"),
        );
    }

    /// Attach both text and metadata fingerprints in one call.
    pub fn attach_all_fingerprints(&mut self) {
        self.attach_text_fingerprint();
        self.attach_metadata_fingerprint();
    }

    fn completion_fingerprint_text(&self) -> String {
        let actions =
            serde_json::to_string(&self.external_actions).unwrap_or_else(|_| "[]".to_string());
        let outcome = if self.success {
            "success".to_string()
        } else {
            self.failure_reason
                .as_deref()
                .filter(|reason| !reason.trim().is_empty())
                .unwrap_or("failure")
                .to_string()
        };

        format!(
            "trigger_kind={}\nagent_template={}\nactions={}\noutcome={}",
            self.trigger_kind, self.agent_template, actions, outcome
        )
    }

    fn metadata_fingerprint_text(&self) -> String {
        let gate_summary: String = self
            .gate_verdicts
            .iter()
            .map(|gv| format!("{}:{}", gv.gate, if gv.passed { "pass" } else { "fail" }))
            .collect::<Vec<_>>()
            .join(",");

        format!(
            "agent_id={}\ntask_id={}\nmodel={}\ngates=[{}]\nsuccess={}",
            self.agent_id, self.task_id, self.model, gate_summary, self.success
        )
    }
}

/// Decomposed components of an episode importance score.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EpisodeImportanceComponents {
    /// How surprising the observed outcome was for the surrounding context.
    pub surprisal: f64,
    /// How unlike recent episodes the current episode is.
    pub novelty: f64,
    /// How difficult the episode appears from usage and gate complexity.
    pub difficulty: f64,
    /// How much the episode changed routing evidence relative to peers.
    pub information_gain: f64,
    /// How much the episode increases corpus diversity.
    pub diversity: f64,
}

impl EpisodeImportanceComponents {
    /// Weighted aggregate score in `[0, 1]`.
    #[must_use]
    pub fn score(self) -> f64 {
        (self.surprisal * 0.3
            + self.novelty * 0.25
            + self.difficulty * 0.2
            + self.information_gain * 0.15
            + self.diversity * 0.1)
            .clamp(0.0, 1.0)
    }
}

/// Replay priority tiers derived from episode importance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodePriorityTier {
    /// Highly informative or surprising episodes.
    Critical,
    /// Strong training signal worth replaying soon.
    High,
    /// Ordinary episodes with moderate signal.
    Normal,
    /// Low-signal episodes that can be replayed last.
    Background,
}

impl EpisodePriorityTier {
    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Normal => "normal",
            Self::Background => "background",
        }
    }
}

/// Compatibility alias for the decomposed importance score components.
pub type ImportanceComponents = EpisodeImportanceComponents;

/// Aggregate importance record for one episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeImportance {
    /// Stable episode identifier.
    pub episode_id: String,
    /// Weighted importance score in `[0, 1]`.
    pub score: f64,
    /// Interpretable contributing components.
    pub components: EpisodeImportanceComponents,
    /// Priority tier derived from the score.
    pub tier: EpisodePriorityTier,
}

impl EpisodeImportance {
    /// Compute an importance record for `episode` relative to `history`.
    #[must_use]
    pub fn from_episode(episode: &Episode, history: &[Episode]) -> Self {
        let components = importance_components(episode, history);
        Self {
            episode_id: episode.id.clone(),
            score: components.score(),
            components,
            tier: importance_tier(episode, history),
        }
    }
}

/// Configuration for future hot/warm/cold episode compaction tiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeStorageConfig {
    /// Days to keep episodes in the hot JSONL tier.
    pub hot_retention_days: u32,
    /// Days to keep episodes in the warm compressed tier.
    pub warm_retention_days: u32,
    /// Compression level for the warm tier.
    pub zstd_level: i32,
    /// Maximum cold-tier summaries per slice.
    pub cold_max_summaries: usize,
}

/// Cold-tier summary of many compressed episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompressedEpisodeSummary {
    /// HDC superposition across the summarized episodes.
    pub hdc_superposition: HdcVector,
    /// Number of merged episodes.
    pub episode_count: u32,
    /// Aggregate pass rate.
    pub pass_rate: f64,
    /// Average cost in USD across summarized episodes.
    pub avg_cost_usd: f64,
    /// Average duration in milliseconds across summarized episodes.
    pub avg_duration_ms: f64,
}

/// Configuration for future cluster-level consolidation of episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeClusterConfig {
    /// Minimum similarity required to join a cluster.
    pub min_similarity: f64,
    /// Minimum number of members before surfacing a cluster.
    pub min_cluster_size: usize,
}

/// Cluster of structurally similar episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeCluster {
    /// Stable cluster identifier.
    pub cluster_id: String,
    /// Episode ids assigned to the cluster.
    pub episode_ids: Vec<String>,
    /// Medoid fingerprint representing the cluster.
    pub medoid: Option<HdcVector>,
    /// Aggregate pass rate for cluster members.
    pub pass_rate: f64,
}

/// Summary of how an episode cluster changed over time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterEvolution {
    /// Cluster identifier.
    pub cluster_id: String,
    /// Number of members in the prior snapshot.
    pub previous_size: usize,
    /// Number of members in the new snapshot.
    pub current_size: usize,
    /// Whether the cluster remained active.
    pub still_active: bool,
}

/// Derive a stable id by hashing `(agent_id, task_id, timestamp)` with
/// Rust's default hasher. Not cryptographic — collisions are acceptable
/// because ids are scoped to a single log file.
fn derive_id(agent_id: &str, task_id: &str, timestamp: DateTime<Utc>) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    agent_id.hash(&mut hasher);
    task_id.hash(&mut hasher);
    timestamp
        .timestamp_nanos_opt()
        .unwrap_or(0)
        .hash(&mut hasher);
    format!("ep_{:016x}", hasher.finish())
}

fn suggest_template_from_episodes(episodes: &[Episode], signal: &Engram) -> Option<String> {
    let cutoff = Utc::now() - chrono::Duration::days(TEMPLATE_SUGGESTION_MAX_AGE_DAYS);
    let signal_fingerprint = text_fingerprint(&signal_fingerprint_text(signal));

    let mut best: Option<(f64, String)> = None;
    for episode in episodes
        .iter()
        .rev()
        .filter(|episode| episode.completed_at >= cutoff || episode.timestamp >= cutoff)
        .take(TEMPLATE_SUGGESTION_MAX_CANDIDATES)
    {
        let Some(template) = normalized_template(&episode.agent_template) else {
            continue;
        };
        let Some(episode_fingerprint) = episode_fingerprint(episode) else {
            continue;
        };
        let similarity = signal_fingerprint.similarity(&episode_fingerprint) as f64;
        let importance = importance_score(episode, episodes);
        let combined_score = similarity * (0.6 + importance * 0.4);
        if combined_score <= TEMPLATE_SUGGESTION_MIN_SIMILARITY {
            continue;
        }

        let should_replace = best
            .as_ref()
            .is_none_or(|(best_similarity, _)| combined_score > *best_similarity);
        if should_replace {
            best = Some((combined_score, template));
        }
    }

    best.map(|(_, template)| template)
}

/// Compute a composite importance score for `episode` relative to `history`.
#[must_use]
pub fn importance_score(episode: &Episode, history: &[Episode]) -> f64 {
    importance_components(episode, history).score()
}

/// Break the episode importance score into interpretable components.
#[must_use]
pub fn importance_components(
    episode: &Episode,
    history: &[Episode],
) -> EpisodeImportanceComponents {
    let peers: Vec<&Episode> = history
        .iter()
        .filter(|peer| !same_episode(peer, episode))
        .collect();

    EpisodeImportanceComponents {
        surprisal: surprisal_score(episode, &peers),
        novelty: novelty_score(episode, &peers),
        difficulty: difficulty_score(episode),
        information_gain: information_gain_score(episode, &peers),
        diversity: diversity_score(episode, &peers),
    }
}

/// Collapse the importance score into a replay tier.
#[must_use]
pub fn importance_tier(episode: &Episode, history: &[Episode]) -> EpisodePriorityTier {
    match importance_score(episode, history) {
        score if score >= 0.8 => EpisodePriorityTier::Critical,
        score if score >= 0.6 => EpisodePriorityTier::High,
        score if score >= 0.35 => EpisodePriorityTier::Normal,
        _ => EpisodePriorityTier::Background,
    }
}

/// Rank episodes by importance, highest score first.
#[must_use]
pub fn prioritize_by_importance<'a>(
    episodes: &'a [Episode],
    history: &[Episode],
) -> Vec<&'a Episode> {
    let mut ranked: Vec<(&Episode, f64)> = episodes
        .iter()
        .map(|episode| (episode, importance_score(episode, history)))
        .collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.0.timestamp.cmp(&left.0.timestamp))
    });
    ranked.into_iter().map(|(episode, _)| episode).collect()
}

fn episode_fingerprint(episode: &Episode) -> Option<HdcVector> {
    // Prefer the text fingerprint; fall back to metadata fingerprint.
    episode
        .extra
        .get(TEXT_FINGERPRINT_KEY)
        .or_else(|| episode.extra.get(METADATA_FINGERPRINT_KEY))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn normalized_template(template: &str) -> Option<String> {
    let trimmed = template.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn signal_fingerprint_text(signal: &Engram) -> String {
    let body_bytes = signal.body.canonical_bytes();
    let body = String::from_utf8_lossy(&body_bytes);
    let agent_template = signal
        .tag("agent_template")
        .or_else(|| signal.tag("template"))
        .unwrap_or(signal.kind.as_str());
    let outcome = signal.tag("outcome").unwrap_or("unknown");

    format!(
        "trigger_kind={}\nagent_template={}\nactions={}\noutcome={}",
        signal.kind.as_str(),
        agent_template,
        body,
        outcome
    )
}

fn same_episode(lhs: &Episode, rhs: &Episode) -> bool {
    (!lhs.id.is_empty() && lhs.id == rhs.id)
        || (!lhs.episode_id.is_empty() && lhs.episode_id == rhs.episode_id)
}

fn episode_model_label(episode: &Episode) -> String {
    normalized_template(&episode.model).unwrap_or_else(|| "unknown-model".to_string())
}

fn episode_template_label(episode: &Episode) -> String {
    normalized_template(&episode.agent_template)
        .or_else(|| normalized_template(&episode.agent_id))
        .unwrap_or_else(|| "unknown-template".to_string())
}

fn episode_task_label(episode: &Episode) -> String {
    normalized_template(&episode.task_id).unwrap_or_else(|| "unknown-task".to_string())
}

fn surprisal_score(episode: &Episode, history: &[&Episode]) -> f64 {
    let mut matching = Vec::new();
    for peer in history {
        if episode_task_label(peer) == episode_task_label(episode)
            || episode_template_label(peer) == episode_template_label(episode)
            || episode_model_label(peer) == episode_model_label(episode)
        {
            matching.push(*peer);
        }
    }
    if matching.is_empty() {
        matching = history.to_vec();
    }

    if matching.is_empty() {
        return 0.5;
    }

    let successes = matching.iter().filter(|peer| peer.success).count() as f64;
    let total = matching.len() as f64;
    let expected_success = (successes / total).clamp(0.0, 1.0);
    let p_outcome = if episode.success {
        expected_success
    } else {
        1.0 - expected_success
    };
    (1.0 - p_outcome).clamp(0.0, 1.0)
}

fn novelty_score(episode: &Episode, history: &[&Episode]) -> f64 {
    let Some(fingerprint) = episode_fingerprint(episode) else {
        return 0.5;
    };

    let mut best_similarity: f64 = 0.0;
    for peer in history.iter().take(64) {
        if let Some(peer_fingerprint) = episode_fingerprint(peer) {
            best_similarity = best_similarity.max(fingerprint.similarity(&peer_fingerprint) as f64);
        }
    }

    (1.0 - best_similarity).clamp(0.0, 1.0)
}

fn difficulty_score(episode: &Episode) -> f64 {
    let token_pressure = ((episode.usage.input_tokens
        + episode.usage.output_tokens
        + episode.usage.cache_write_tokens) as f64)
        .ln_1p()
        / 16.0_f64.ln_1p();
    let duration_pressure = episode.duration_secs.max(0.0).ln_1p() / 10.0_f64.ln_1p();
    let gate_pressure = (episode.gate_verdicts.len() as f64 / 8.0).clamp(0.0, 1.0);
    let model_pressure = match episode.model.to_ascii_lowercase().as_str() {
        model if model.contains("opus") || model.contains("pro") => 1.0,
        model if model.contains("sonnet") || model.contains("gpt-4") => 0.75,
        model if model.contains("haiku") || model.contains("mini") || model.contains("fast") => {
            0.35
        }
        _ => 0.55,
    };

    (token_pressure * 0.35
        + duration_pressure * 0.20
        + gate_pressure * 0.15
        + model_pressure * 0.30)
        .clamp(0.0, 1.0)
}

fn information_gain_score(episode: &Episode, history: &[&Episode]) -> f64 {
    if history.is_empty() {
        return if episode.success { 0.25 } else { 0.55 };
    }

    let task_matches: Vec<&Episode> = history
        .iter()
        .copied()
        .filter(|peer| episode_task_label(peer) == episode_task_label(episode))
        .collect();
    let model_matches: Vec<&Episode> = history
        .iter()
        .copied()
        .filter(|peer| episode_model_label(peer) == episode_model_label(episode))
        .collect();

    let model_shift = 1.0
        - if history.is_empty() {
            0.0
        } else {
            dominant_share(
                history,
                |peer| episode_model_label(peer),
                episode_model_label(episode),
            )
        };

    let task_shift = if task_matches.is_empty() {
        0.75
    } else {
        1.0 - dominant_share(
            &task_matches,
            |peer| episode_template_label(peer),
            episode_template_label(episode),
        )
    };

    let outcome_shift = if task_matches.is_empty() {
        if episode.success { 0.15 } else { 0.65 }
    } else {
        let success_rate = task_matches.iter().filter(|peer| peer.success).count() as f64
            / task_matches.len() as f64;
        (episode.success as u8 as f64 - success_rate).abs()
    };

    let gate_shift = if model_matches.is_empty() {
        0.3
    } else {
        let pass_rate = model_matches
            .iter()
            .filter(|peer| peer.gate_verdicts.iter().all(|verdict| verdict.passed))
            .count() as f64
            / model_matches.len() as f64;
        (1.0 - pass_rate).clamp(0.0, 1.0)
    };

    (model_shift * 0.35 + task_shift * 0.25 + outcome_shift * 0.20 + gate_shift * 0.20)
        .clamp(0.0, 1.0)
}

fn diversity_score(episode: &Episode, history: &[&Episode]) -> f64 {
    if history.is_empty() {
        return 0.5;
    }

    let distinct_models = distinct_count(history, |peer| episode_model_label(peer));
    let distinct_templates = distinct_count(history, |peer| episode_template_label(peer));
    let distinct_tasks = distinct_count(history, |peer| episode_task_label(peer));
    let distinct_outcomes = distinct_count(history, |peer| {
        if peer.success {
            "success".to_string()
        } else {
            "failure".to_string()
        }
    });

    let history_scale = (history.len().max(1) as f64).min(16.0);
    let novelty_anchor = if episode.success { 0.0 } else { 0.05 };
    let distinctness = ((distinct_models + distinct_templates + distinct_tasks + distinct_outcomes)
        as f64)
        / (4.0 * history_scale);

    (distinctness + novelty_anchor).clamp(0.0, 1.0)
}

fn distinct_count<F>(episodes: &[&Episode], mut key: F) -> usize
where
    F: FnMut(&Episode) -> String,
{
    let mut set = std::collections::BTreeSet::new();
    for episode in episodes {
        set.insert(key(episode));
    }
    set.len()
}

fn dominant_share<F>(episodes: &[&Episode], mut key: F, current_key: String) -> f64
where
    F: FnMut(&Episode) -> String,
{
    if episodes.is_empty() {
        return 0.0;
    }

    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for episode in episodes {
        *counts.entry(key(episode)).or_default() += 1;
    }
    let dominant = counts.values().copied().max().unwrap_or(0) as f64;
    let total = episodes.len() as f64;
    let current = counts.get(&current_key).copied().unwrap_or(0) as f64;
    (dominant.max(current) / total).clamp(0.0, 1.0)
}

/// Append-only JSONL episode logger.
///
/// Cheap to clone: the inner mutex lives behind an [`Arc`], so multiple
/// tasks can share a single logger and serialize their writes through
/// the same lock. A logger does *not* keep a file handle open between
/// calls — each `append` opens, writes, fsyncs, and closes. That keeps
/// the surface area tiny and avoids the "forgot to flush on drop"
/// footgun.
#[derive(Debug, Clone)]
pub struct EpisodeLogger {
    inner: Arc<LoggerInner>,
}

#[derive(Debug)]
struct LoggerInner {
    path: PathBuf,
    /// Counter of successful appends — protected by `parking_lot` for
    /// synchronous introspection even off the tokio runtime.
    writes: SyncMutex<u64>,
    /// Async mutex that serializes `append` across `.await` points so
    /// concurrent tasks never interleave bytes mid-line.
    write_gate: AsyncMutex<()>,
}

impl EpisodeLogger {
    /// Create a logger that writes to `path`. The file is created lazily
    /// on first `append`.
    #[must_use]
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            inner: Arc::new(LoggerInner {
                path: path.as_ref().to_path_buf(),
                writes: SyncMutex::new(0),
                write_gate: AsyncMutex::new(()),
            }),
        }
    }

    /// Return the path the logger writes to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Number of episodes successfully appended through this logger
    /// instance. Safe to call from any thread.
    #[must_use]
    pub fn write_count(&self) -> u64 {
        *self.inner.writes.lock()
    }

    /// Append a single episode to the log. The write is held under a
    /// process-local mutex, so concurrent callers never interleave
    /// bytes. The caller's task is suspended until the data has been
    /// flushed to the OS.
    ///
    /// # Errors
    ///
    /// Returns [`LoggerError::Io`] on any filesystem failure,
    /// [`LoggerError::Serde`] if the episode cannot be encoded, and
    /// [`LoggerError::ExtraTooLarge`] if `episode.extra` exceeds
    /// [`MAX_EXTRA_BYTES`] bytes when serialized.
    pub async fn append(&self, episode: &Episode) -> Result<(), LoggerError> {
        let extra_size = serde_json::to_vec(&episode.extra)?.len();
        if extra_size > MAX_EXTRA_BYTES {
            return Err(LoggerError::ExtraTooLarge {
                size: extra_size,
                max: MAX_EXTRA_BYTES,
            });
        }
        let mut line = serde_json::to_string(episode)?;
        line.push('\n');
        // Serialize writers within this process so concurrent appends
        // cannot interleave bytes across a single JSONL record.
        let gate = self.inner.write_gate.lock().await;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.inner.path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        // Durability: a crash mid-write can leave at most a partial
        // trailing line, which the reader tolerates.
        file.sync_all().await?;
        drop(gate);
        *self.inner.writes.lock() += 1;
        Ok(())
    }

    /// Read every well-formed episode from `path`, preserving write
    /// order.
    ///
    /// If the file does not exist, an empty vector is returned.
    /// Malformed lines (truncated tail, schema drift, …) produce a
    /// [`LoggerError::Parse`] with the offending line number. Callers
    /// that want to tolerate partial tails can match on that variant
    /// and recover.
    ///
    /// # Errors
    ///
    /// Returns [`LoggerError::Io`] if the file cannot be opened/read
    /// (other than "missing"), or [`LoggerError::Parse`] on the first
    /// unparseable line.
    pub async fn read_all(path: impl AsRef<Path>) -> Result<Vec<Episode>, LoggerError> {
        let path = path.as_ref();
        let bytes = match tokio::fs::read(path).await {
            Ok(b) => b,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(LoggerError::Io(err)),
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut out = Vec::new();
        for (idx, raw) in text.lines().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            let episode: Episode =
                serde_json::from_str(raw).map_err(|source| LoggerError::Parse {
                    line: idx + 1,
                    source,
                })?;
            out.push(episode);
        }
        Ok(out)
    }

    /// Like [`Self::read_all`] but silently drops any line that fails
    /// to parse. Useful for tolerating a truncated tail after a crash.
    ///
    /// # Errors
    ///
    /// Returns [`LoggerError::Io`] on filesystem failure. Parse errors
    /// are swallowed.
    pub async fn read_all_lossy(path: impl AsRef<Path>) -> Result<Vec<Episode>, LoggerError> {
        let path = path.as_ref();
        let bytes = match tokio::fs::read(path).await {
            Ok(b) => b,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(LoggerError::Io(err)),
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut out = Vec::new();
        for raw in text.lines() {
            if raw.trim().is_empty() {
                continue;
            }
            if let Ok(ep) = serde_json::from_str::<Episode>(raw) {
                out.push(ep);
            }
        }
        Ok(out)
    }

    /// Suggest a template by comparing a signal against recent episode fingerprints.
    ///
    /// Exact subscription resolution happens upstream. This helper is a
    /// fallback: it reads recent episodes from `path`, fingerprints the
    /// incoming `signal`, and returns the most similar episode template when
    /// the HDC similarity exceeds `0.7`.
    ///
    /// # Errors
    ///
    /// Returns a logger error if the episode log cannot be read.
    pub async fn suggest_template_from_recent_episodes(
        path: impl AsRef<Path>,
        signal: &Engram,
    ) -> Result<Option<String>, LoggerError> {
        let episodes = Self::read_all_lossy(path).await?;
        Ok(suggest_template_from_episodes(&episodes, signal))
    }

    /// Run age-based and size-based retention, pruning oldest episodes
    /// first while preserving those marked as [`Episode::headline`].
    ///
    /// The compaction is atomic: survivors are written to a temporary
    /// `.compacting` sibling, fsynced, then renamed over the original
    /// file.
    ///
    /// # Errors
    ///
    /// Returns [`LoggerError::Io`] on filesystem failure or
    /// [`LoggerError::Serde`] if a surviving episode cannot be
    /// re-serialized.
    pub async fn compact(
        &self,
        now: DateTime<Utc>,
        policy: &RetentionPolicy,
    ) -> Result<CompactStats, LoggerError> {
        let _gate = self.inner.write_gate.lock().await;

        let episodes = Self::read_all_lossy(&self.inner.path).await?;
        let before = episodes.len();

        let age_cutoff = now - chrono::Duration::days(i64::from(policy.max_age_days));

        // Phase 1: age-based pruning — drop episodes older than cutoff
        // unless they are headlines.
        let mut survivors: Vec<Episode> = episodes
            .into_iter()
            .filter(|ep| ep.headline || ep.timestamp >= age_cutoff)
            .collect();

        // Phase 2: size-based pruning — if still over max_episodes, drop
        // the oldest non-headline episodes first.
        if survivors.len() > policy.max_episodes {
            // Partition into headlines (always kept) and normals.
            let (headlines, mut normals): (Vec<Episode>, Vec<Episode>) =
                survivors.into_iter().partition(|ep| ep.headline);

            // Sort normals by timestamp descending so we can truncate the
            // tail (oldest).
            normals.sort_by_key(|b| std::cmp::Reverse(b.timestamp));

            let keep_normals = policy.max_episodes.saturating_sub(headlines.len());
            normals.truncate(keep_normals);

            // Recombine and sort by timestamp ascending (original write
            // order) for the rewritten file.
            survivors = headlines.into_iter().chain(normals).collect();
            survivors.sort_by_key(|a| a.timestamp);
        }

        let after = survivors.len();
        let removed = before.saturating_sub(after);

        // Write survivors to a temporary sibling.
        let compacting_path = self.inner.path.with_extension("compacting");
        {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&compacting_path)
                .await?;
            for ep in &survivors {
                let mut line = serde_json::to_string(ep)?;
                line.push('\n');
                file.write_all(line.as_bytes()).await?;
            }
            file.flush().await?;
            file.sync_all().await?;
        }

        // Compute bytes reclaimed.
        let original_size = tokio::fs::metadata(&self.inner.path)
            .await
            .map_or(0, |m| m.len());
        let new_size = tokio::fs::metadata(&compacting_path)
            .await
            .map_or(0, |m| m.len());
        let bytes_reclaimed = original_size.saturating_sub(new_size);

        // Atomic rename over the original.
        tokio::fs::rename(&compacting_path, &self.inner.path).await?;

        Ok(CompactStats {
            before,
            after,
            removed,
            bytes_reclaimed,
        })
    }
}

/// Age-based + size-based retention configuration.
///
/// Used by [`EpisodeLogger::compact`] to decide which episodes to keep.
#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    /// Maximum number of episodes retained after compaction.
    pub max_episodes: usize,
    /// Maximum age in days — episodes older than this are pruned (unless
    /// marked as [`Episode::headline`]).
    pub max_age_days: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_episodes: 200,
            max_age_days: 90,
        }
    }
}

/// Statistics returned by [`EpisodeLogger::compact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactStats {
    /// Number of episodes before compaction.
    pub before: usize,
    /// Number of episodes after compaction.
    pub after: usize,
    /// Number of episodes removed.
    pub removed: usize,
    /// Approximate bytes reclaimed on disk.
    pub bytes_reclaimed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdc_fingerprint::{
        decode as decode_hdc_fingerprint, encode as encode_hdc_fingerprint, fingerprint_episode,
    };
    use roko_core::{Body, Engram, Kind};
    use tempfile::TempDir;

    fn tmp_log() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("create tempdir");
        let path = dir.path().join("episodes.jsonl");
        (dir, path)
    }

    fn sample(agent: &str, task: &str, success: bool) -> Episode {
        let mut ep = Episode::new(agent, task);
        ep.success = success;
        ep.usage = Usage::tokens(100, 50);
        ep.gate_verdicts.push(GateVerdict::new("compile", success));
        ep
    }

    fn suggestion_signal(kind: Kind, body: &str) -> Engram {
        Engram::builder(kind).body(Body::text(body)).build()
    }

    fn episode_for_signal(template: &str, signal: &Engram, completed_at: DateTime<Utc>) -> Episode {
        let mut episode = Episode::new("agent-a", "task-a");
        episode.agent_template = template.to_string();
        episode.timestamp = completed_at;
        episode.started_at = completed_at;
        episode.completed_at = completed_at;
        episode.extra.insert(
            TEXT_FINGERPRINT_KEY.to_string(),
            serde_json::to_value(text_fingerprint(&signal_fingerprint_text(signal)))
                .expect("serialize fingerprint"),
        );
        episode
    }

    #[tokio::test]
    async fn empty_log_returns_empty_vec() {
        let (_dir, path) = tmp_log();
        let episodes = EpisodeLogger::read_all(&path).await.expect("read empty");
        assert!(episodes.is_empty());
    }

    #[tokio::test]
    async fn single_append_and_read() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let ep = sample("agent-a", "task-1", true);
        logger.append(&ep).await.expect("append");
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].agent_id, "agent-a");
        assert_eq!(all[0].task_id, "task-1");
        assert!(all[0].success);
        assert_eq!(all[0].gate_verdicts.len(), 1);
        assert_eq!(all[0].gate_verdicts[0].gate, "compile");
    }

    #[tokio::test]
    async fn backend_round_trips_through_jsonl_append_and_read() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let mut ep = sample("agent-a", "task-1", true);
        ep.backend = "anthropic".to_string();
        logger.append(&ep).await.expect("append");

        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].backend, "anthropic");
    }

    #[tokio::test]
    async fn hdc_fingerprint_round_trips_through_jsonl_append_and_read() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let mut ep = sample("agent-a", "task-1", true);
        let vector = fingerprint_episode("prompt body", "successful outcome");
        let encoded = encode_hdc_fingerprint(&vector);
        let decoded = decode_hdc_fingerprint(&encoded).expect("decode");
        assert_eq!(vector, decoded);

        ep.hdc_fingerprint = Some(encoded.clone());
        logger.append(&ep).await.expect("append");

        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].hdc_fingerprint.as_deref(), Some(encoded.as_str()));
    }

    #[tokio::test]
    async fn legacy_json_without_backend_defaults_to_empty_string() {
        let (_dir, path) = tmp_log();
        let mut episode =
            serde_json::to_value(sample("agent-a", "task-1", true)).expect("serialize episode");
        episode
            .as_object_mut()
            .expect("episode object")
            .remove("backend");
        tokio::fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string(&episode).expect("serialize legacy episode")
            ),
        )
        .await
        .expect("write legacy episode");

        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].backend, "");
    }

    #[tokio::test]
    async fn emotional_tag_round_trips_through_jsonl() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let ep = sample("agent-a", "task-1", true).with_emotional_tag(EmotionalTag::new(
            roko_core::PadVector::new(-0.2, 0.5, -0.1),
            0.8,
            "gate_failure",
            roko_core::PadVector::new(-0.2, 0.5, -0.1),
        ));
        logger.append(&ep).await.expect("append");

        let all = EpisodeLogger::read_all(&path).await.expect("read");
        let tag = all[0].emotional_tag.as_ref().expect("emotional tag");
        assert_eq!(tag.trigger, "gate_failure");
        assert!((tag.intensity - 0.8).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn suggest_template_from_recent_episodes_returns_best_match() {
        let (_dir, path) = tmp_log();
        let signal = suggestion_signal(Kind::Task, "implement similarity fallback");
        let matched = episode_for_signal("code-implementer", &signal, Utc::now());
        tokio::fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string(&matched).expect("serialize episode")
            ),
        )
        .await
        .expect("write episodes");

        let suggestion = EpisodeLogger::suggest_template_from_recent_episodes(&path, &signal)
            .await
            .expect("suggest template");
        assert_eq!(suggestion.as_deref(), Some("code-implementer"));
    }

    #[tokio::test]
    async fn suggest_template_from_recent_episodes_accepts_similar_unmatched_events() {
        let (_dir, path) = tmp_log();
        let signal = suggestion_signal(Kind::Task, "implement similarity fallback");
        let signal_fingerprint = text_fingerprint(&signal_fingerprint_text(&signal));
        let similar_fingerprint = HdcVector::bundle(&[
            &signal_fingerprint,
            &signal_fingerprint,
            &HdcVector::from_seed(b"template-similarity-noise"),
        ]);
        let similarity = signal_fingerprint.similarity(&similar_fingerprint);
        assert!(
            similarity > 0.7,
            "expected similar fingerprints to cluster above the 0.7 cutoff, got {similarity}"
        );

        let mut episode = Episode::new("agent-a", "task-a");
        episode.agent_template = "code-implementer".to_string();
        episode.timestamp = Utc::now();
        episode.started_at = episode.timestamp;
        episode.completed_at = episode.timestamp;
        episode.extra.insert(
            TEXT_FINGERPRINT_KEY.to_string(),
            serde_json::to_value(similar_fingerprint).expect("serialize fingerprint"),
        );

        tokio::fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string(&episode).expect("serialize episode")
            ),
        )
        .await
        .expect("write episodes");

        let suggestion = EpisodeLogger::suggest_template_from_recent_episodes(&path, &signal)
            .await
            .expect("suggest template");
        assert_eq!(suggestion.as_deref(), Some("code-implementer"));
    }

    #[tokio::test]
    async fn suggest_template_from_recent_episodes_ignores_low_similarity() {
        let (_dir, path) = tmp_log();
        let signal = suggestion_signal(Kind::Task, "implement similarity fallback");
        let other_signal = suggestion_signal(Kind::Task, "completely different request");
        let episode = episode_for_signal("code-implementer", &other_signal, Utc::now());
        tokio::fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string(&episode).expect("serialize episode")
            ),
        )
        .await
        .expect("write episodes");

        let suggestion = EpisodeLogger::suggest_template_from_recent_episodes(&path, &signal)
            .await
            .expect("suggest template");
        assert!(suggestion.is_none());
    }

    #[tokio::test]
    async fn suggest_template_from_recent_episodes_ignores_old_matches() {
        let (_dir, path) = tmp_log();
        let signal = suggestion_signal(Kind::Task, "implement similarity fallback");
        let old_match = episode_for_signal(
            "code-implementer",
            &signal,
            Utc::now() - chrono::Duration::days(60),
        );
        tokio::fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string(&old_match).expect("serialize episode")
            ),
        )
        .await
        .expect("write episodes");

        let suggestion = EpisodeLogger::suggest_template_from_recent_episodes(&path, &signal)
            .await
            .expect("suggest template");
        assert!(suggestion.is_none());
    }

    #[tokio::test]
    async fn multi_append_preserves_order() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        for i in 0..5 {
            let ep = sample("agent-a", &format!("task-{i}"), i % 2 == 0);
            logger.append(&ep).await.expect("append");
        }
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 5);
        for (i, ep) in all.iter().enumerate() {
            assert_eq!(ep.task_id, format!("task-{i}"));
            assert_eq!(ep.success, i % 2 == 0);
        }
    }

    #[tokio::test]
    async fn persists_across_reopens() {
        let (_dir, path) = tmp_log();
        {
            let logger = EpisodeLogger::new(&path);
            logger
                .append(&sample("a", "first", true))
                .await
                .expect("append 1");
        }
        {
            let logger = EpisodeLogger::new(&path);
            logger
                .append(&sample("a", "second", false))
                .await
                .expect("append 2");
        }
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].task_id, "first");
        assert_eq!(all[1].task_id, "second");
    }

    #[tokio::test]
    async fn invalid_line_returns_parse_error() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        logger
            .append(&sample("a", "ok", true))
            .await
            .expect("append");
        // Hand-append a malformed line to simulate a crash tail.
        tokio::fs::write(
            &path,
            format!(
                "{}\n{{not json\n",
                serde_json::to_string(&sample("a", "ok", true)).expect("serialize")
            ),
        )
        .await
        .expect("write");
        let err = EpisodeLogger::read_all(&path).await.unwrap_err();
        match err {
            LoggerError::Parse { line, .. } => assert_eq!(line, 2),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_all_lossy_tolerates_bad_lines() {
        let (_dir, path) = tmp_log();
        let good = serde_json::to_string(&sample("a", "ok", true)).expect("serialize");
        tokio::fs::write(&path, format!("{good}\n{{broken\n{good}\n"))
            .await
            .expect("write");
        let all = EpisodeLogger::read_all_lossy(&path)
            .await
            .expect("lossy read");
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn concurrent_appends_do_not_interleave() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let mut handles = Vec::new();
        for worker in 0..8_u32 {
            let logger = logger.clone();
            handles.push(tokio::spawn(async move {
                for i in 0..10_u32 {
                    let ep = sample(&format!("worker-{worker}"), &format!("t-{i}"), true);
                    logger.append(&ep).await.expect("append");
                }
            }));
        }
        for h in handles {
            h.await.expect("join");
        }
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 80);
        // Every line parsed successfully → no interleaving.
        for ep in &all {
            assert!(ep.agent_id.starts_with("worker-"));
        }
    }

    #[tokio::test]
    async fn extra_too_large_is_rejected() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let mut ep = sample("a", "big", true);
        let big_string: String = "x".repeat(MAX_EXTRA_BYTES + 1);
        ep.extra
            .insert("payload".to_string(), serde_json::Value::String(big_string));
        let err = logger.append(&ep).await.unwrap_err();
        match err {
            LoggerError::ExtraTooLarge { size, max } => {
                assert!(size > max);
                assert_eq!(max, MAX_EXTRA_BYTES);
            }
            other => panic!("unexpected error: {other:?}"),
        }
        // Nothing should have been written.
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn failure_reason_round_trips() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let ep = Episode::new("a", "t").failed("E0277:Send+Sync");
        logger.append(&ep).await.expect("append");
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 1);
        assert!(!all[0].success);
        assert_eq!(all[0].failure_reason.as_deref(), Some("E0277:Send+Sync"));
    }

    #[tokio::test]
    async fn episode_reasoning_round_trips() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let mut ep = sample("agent-a", "task-1", true);
        let reasoning = "reason ".repeat(80);
        ep.reasoning_summary = Some(reasoning.chars().take(500).collect());

        logger.append(&ep).await.expect("append");

        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].reasoning_summary, ep.reasoning_summary);
        assert_eq!(
            all[0]
                .reasoning_summary
                .as_ref()
                .expect("reasoning summary")
                .chars()
                .count(),
            500
        );
    }

    #[test]
    fn episode_text_fingerprint_is_written_to_metadata() {
        let mut ep = Episode::new("agent-a", "task-a");
        ep.trigger_kind = "webhook_dispatch".to_string();
        ep.agent_template = "template-a".to_string();
        ep.external_actions = vec![serde_json::json!({
            "kind": "comment",
            "target": "issue-1"
        })];
        ep.success = false;
        ep.failure_reason = Some("gated".to_string());

        ep.attach_text_fingerprint();

        let stored = ep
            .extra
            .get(TEXT_FINGERPRINT_KEY)
            .cloned()
            .expect("fingerprint metadata");
        let decoded: roko_primitives::hdc::HdcVector =
            serde_json::from_value(stored).expect("decode fingerprint");
        let expected = text_fingerprint(
            "trigger_kind=webhook_dispatch\nagent_template=template-a\nactions=[{\"kind\":\"comment\",\"target\":\"issue-1\"}]\noutcome=gated",
        );
        assert_eq!(decoded, expected);
    }

    #[tokio::test]
    async fn empty_and_whitespace_lines_ignored() {
        let (_dir, path) = tmp_log();
        let good = serde_json::to_string(&sample("a", "ok", true)).expect("serialize");
        tokio::fs::write(&path, format!("\n{good}\n\n\n"))
            .await
            .expect("write");
        let all = EpisodeLogger::read_all(&path).await.expect("read");
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn ids_are_populated_and_distinct() {
        let a = Episode::new("agent", "t1");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let b = Episode::new("agent", "t2");
        assert!(a.id.starts_with("ep_"));
        assert!(b.id.starts_with("ep_"));
        assert_ne!(a.id, b.id);
    }

    // ---- Retention / GC tests (§16.1.3) ----

    /// Helper: build an episode with a specific timestamp.
    fn episode_at(agent: &str, task: &str, ts: DateTime<Utc>) -> Episode {
        let mut ep = sample(agent, task, true);
        ep.timestamp = ts;
        // Re-derive id so it's unique per timestamp.
        ep.id = format!("ep_{agent}_{task}_{}", ts.timestamp());
        ep
    }

    #[tokio::test]
    async fn compact_size_exactly_n() {
        // Exactly max_episodes → nothing pruned.
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 5,
            max_age_days: 365,
        };
        for i in 0..5u32 {
            let ep = episode_at(
                "a",
                &format!("t{i}"),
                now - chrono::Duration::hours(i64::from(i)),
            );
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 5);
        assert_eq!(stats.after, 5);
        assert_eq!(stats.removed, 0);
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert_eq!(remaining.len(), 5);
    }

    #[tokio::test]
    async fn compact_size_n_plus_one() {
        // max_episodes + 1 → oldest one pruned.
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 5,
            max_age_days: 365,
        };
        for i in 0..6u32 {
            let ep = episode_at(
                "a",
                &format!("t{i}"),
                now - chrono::Duration::hours(i64::from(5 - i)),
            );
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 6);
        assert_eq!(stats.after, 5);
        assert_eq!(stats.removed, 1);
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert_eq!(remaining.len(), 5);
        // The oldest episode (t0, earliest timestamp) should be gone.
        // Episodes were appended in ascending timestamp order (5-i hours ago),
        // so t0 is the oldest.
        assert!(remaining.iter().all(|ep| ep.task_id != "t0"));
    }

    #[tokio::test]
    async fn compact_size_n_minus_one() {
        // max_episodes - 1 → nothing pruned.
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 5,
            max_age_days: 365,
        };
        for i in 0..4u32 {
            let ep = episode_at(
                "a",
                &format!("t{i}"),
                now - chrono::Duration::hours(i64::from(i)),
            );
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 4);
        assert_eq!(stats.after, 4);
        assert_eq!(stats.removed, 0);
    }

    #[tokio::test]
    async fn compact_age_prunes_old_episodes() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 1000,
            max_age_days: 30,
        };
        // 3 recent, 2 old (> 30 days).
        for i in 0..3u32 {
            let ep = episode_at(
                "a",
                &format!("recent-{i}"),
                now - chrono::Duration::days(i64::from(i)),
            );
            logger.append(&ep).await.unwrap();
        }
        for i in 0..2u32 {
            let ep = episode_at(
                "a",
                &format!("old-{i}"),
                now - chrono::Duration::days(31 + i64::from(i)),
            );
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 5);
        assert_eq!(stats.after, 3);
        assert_eq!(stats.removed, 2);
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert!(remaining.iter().all(|ep| ep.task_id.starts_with("recent-")));
    }

    #[tokio::test]
    async fn compact_preserves_headlines() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 2,
            max_age_days: 10,
        };
        // 1 headline that is old (would be pruned by age) and 3 normals.
        let mut headline_ep = episode_at("a", "headline-old", now - chrono::Duration::days(100));
        headline_ep.headline = true;
        logger.append(&headline_ep).await.unwrap();
        for i in 0..3u32 {
            let ep = episode_at(
                "a",
                &format!("normal-{i}"),
                now - chrono::Duration::hours(i64::from(i)),
            );
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        // Headline survives age and size pruning. max_episodes=2 means
        // 1 headline + at most 1 normal (the most recent one).
        assert_eq!(stats.before, 4);
        assert_eq!(stats.after, 2);
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().any(|ep| ep.task_id == "headline-old"));
        // The kept normal should be the most recent one (normal-0,
        // which is 0 hours ago).
        assert!(remaining.iter().any(|ep| ep.task_id == "normal-0"));
    }

    #[tokio::test]
    async fn compact_combined_age_and_size() {
        // Age removes some, then size cap removes more.
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 3,
            max_age_days: 30,
        };
        // 2 old episodes (pruned by age).
        for i in 0..2u32 {
            let ep = episode_at(
                "a",
                &format!("old-{i}"),
                now - chrono::Duration::days(60 + i64::from(i)),
            );
            logger.append(&ep).await.unwrap();
        }
        // 5 recent episodes → after age pruning only 5 remain, then
        // size cap prunes to 3.
        for i in 0..5u32 {
            let ep = episode_at(
                "a",
                &format!("recent-{i}"),
                now - chrono::Duration::hours(i64::from(i)),
            );
            logger.append(&ep).await.unwrap();
        }
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 7);
        assert_eq!(stats.after, 3);
        assert_eq!(stats.removed, 4);
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert_eq!(remaining.len(), 3);
        // Should have the 3 most-recent episodes.
        let ids: Vec<&str> = remaining.iter().map(|ep| ep.task_id.as_str()).collect();
        assert!(ids.contains(&"recent-0"));
        assert!(ids.contains(&"recent-1"));
        assert!(ids.contains(&"recent-2"));
    }

    #[tokio::test]
    async fn compact_empty_log() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy::default();
        // Compact on a non-existent file should succeed gracefully.
        let stats = logger.compact(now, &policy).await.unwrap();
        assert_eq!(stats.before, 0);
        assert_eq!(stats.after, 0);
        assert_eq!(stats.removed, 0);
    }

    #[tokio::test]
    async fn compact_preserves_write_order() {
        let (_dir, path) = tmp_log();
        let logger = EpisodeLogger::new(&path);
        let now = Utc::now();
        let policy = RetentionPolicy {
            max_episodes: 3,
            max_age_days: 365,
        };
        // Write 5 episodes with ascending timestamps.
        for i in 0..5u32 {
            let ep = episode_at(
                "a",
                &format!("t{i}"),
                now - chrono::Duration::hours(i64::from(4 - i)),
            );
            logger.append(&ep).await.unwrap();
        }
        logger.compact(now, &policy).await.unwrap();
        let remaining = EpisodeLogger::read_all(&path).await.unwrap();
        assert_eq!(remaining.len(), 3);
        // Should be in ascending timestamp order (most recent 3).
        for pair in remaining.windows(2) {
            assert!(pair[0].timestamp <= pair[1].timestamp);
        }
    }

    #[tokio::test]
    async fn retention_policy_defaults() {
        let policy = RetentionPolicy::default();
        assert_eq!(policy.max_episodes, 200);
        assert_eq!(policy.max_age_days, 90);
    }

    #[test]
    fn episode_metadata_fingerprint_is_deterministic() {
        let mut ep1 = Episode::new("agent-a", "task-a");
        ep1.model = "claude-3.5-sonnet".to_string();
        ep1.success = true;
        ep1.gate_verdicts = vec![
            GateVerdict::new("compile", true),
            GateVerdict::new("test", true),
        ];
        ep1.attach_metadata_fingerprint();

        let mut ep2 = Episode::new("agent-a", "task-a");
        ep2.model = "claude-3.5-sonnet".to_string();
        ep2.success = true;
        ep2.gate_verdicts = vec![
            GateVerdict::new("compile", true),
            GateVerdict::new("test", true),
        ];
        ep2.attach_metadata_fingerprint();

        let fp1: HdcVector = serde_json::from_value(
            ep1.extra
                .get(METADATA_FINGERPRINT_KEY)
                .cloned()
                .expect("metadata fingerprint"),
        )
        .expect("deserialize fp1");
        let fp2: HdcVector = serde_json::from_value(
            ep2.extra
                .get(METADATA_FINGERPRINT_KEY)
                .cloned()
                .expect("metadata fingerprint"),
        )
        .expect("deserialize fp2");
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn episode_metadata_fingerprint_differs_by_success() {
        let mut success_ep = Episode::new("agent-a", "task-a");
        success_ep.model = "claude-3.5-sonnet".to_string();
        success_ep.success = true;
        success_ep.attach_metadata_fingerprint();

        let mut failure_ep = Episode::new("agent-a", "task-a");
        failure_ep.model = "claude-3.5-sonnet".to_string();
        failure_ep.success = false;
        failure_ep.attach_metadata_fingerprint();

        let fp_success: HdcVector = serde_json::from_value(
            success_ep
                .extra
                .get(METADATA_FINGERPRINT_KEY)
                .cloned()
                .unwrap(),
        )
        .unwrap();
        let fp_failure: HdcVector = serde_json::from_value(
            failure_ep
                .extra
                .get(METADATA_FINGERPRINT_KEY)
                .cloned()
                .unwrap(),
        )
        .unwrap();
        assert_ne!(fp_success, fp_failure);
    }

    #[test]
    fn attach_all_fingerprints_populates_both_keys() {
        let mut ep = Episode::new("agent-a", "task-a");
        ep.trigger_kind = "webhook_dispatch".to_string();
        ep.agent_template = "template-a".to_string();
        ep.model = "claude-3.5-sonnet".to_string();
        ep.success = true;
        ep.gate_verdicts = vec![GateVerdict::new("compile", true)];

        ep.attach_all_fingerprints();

        assert!(
            ep.extra.contains_key(TEXT_FINGERPRINT_KEY),
            "text fingerprint should be present"
        );
        assert!(
            ep.extra.contains_key(METADATA_FINGERPRINT_KEY),
            "metadata fingerprint should be present"
        );
    }

    #[tokio::test]
    async fn suggest_template_falls_back_to_metadata_fingerprint() {
        let (_dir, path) = tmp_log();
        let signal = suggestion_signal(Kind::Task, "implement similarity fallback");

        // Create an episode with only a metadata fingerprint (no text fingerprint).
        let mut episode = Episode::new("agent-a", "task-a");
        episode.agent_template = "code-implementer".to_string();
        episode.timestamp = Utc::now();
        episode.started_at = episode.timestamp;
        episode.completed_at = episode.timestamp;
        // Encode the signal text as a metadata fingerprint to simulate
        // the scenario where only metadata_fingerprint is available.
        episode.extra.insert(
            METADATA_FINGERPRINT_KEY.to_string(),
            serde_json::to_value(text_fingerprint(&signal_fingerprint_text(&signal)))
                .expect("serialize fingerprint"),
        );

        tokio::fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string(&episode).expect("serialize episode")
            ),
        )
        .await
        .expect("write episodes");

        let suggestion = EpisodeLogger::suggest_template_from_recent_episodes(&path, &signal)
            .await
            .expect("suggest template");
        assert_eq!(suggestion.as_deref(), Some("code-implementer"));
    }

    #[test]
    fn importance_score_rewards_novel_difficult_episodes() {
        let mut routine = Episode::new("agent-a", "task-a");
        routine.model = "claude-haiku-3-5".to_string();
        routine.agent_template = "implementer".to_string();
        routine.success = true;
        routine.usage.input_tokens = 120;
        routine.usage.output_tokens = 40;
        routine.duration_secs = 12.0;

        let mut novel = Episode::new("agent-b", "task-b");
        novel.model = "claude-opus-4".to_string();
        novel.agent_template = "architect".to_string();
        novel.success = false;
        novel.usage.input_tokens = 1_600;
        novel.usage.output_tokens = 900;
        novel.duration_secs = 240.0;
        novel.gate_verdicts = vec![
            GateVerdict::new("compile", false),
            GateVerdict::new("test", false),
        ];
        novel.attach_metadata_fingerprint();

        let history = vec![routine.clone(), routine.clone(), routine.clone()];
        let routine_score = importance_score(&routine, &history);
        let novel_score = importance_score(&novel, &history);

        assert!(novel_score > routine_score);
        assert!(matches!(
            importance_tier(&novel, &history),
            EpisodePriorityTier::Critical | EpisodePriorityTier::High
        ));
    }

    #[test]
    fn prioritize_by_importance_orders_high_signal_first() {
        let mut low = Episode::new("agent-a", "task-a");
        low.model = "claude-haiku-3-5".to_string();
        low.agent_template = "implementer".to_string();

        let mut high = Episode::new("agent-b", "task-b");
        high.model = "claude-opus-4".to_string();
        high.agent_template = "architect".to_string();
        high.success = false;
        high.usage.input_tokens = 2_000;
        high.usage.output_tokens = 1_200;
        high.duration_secs = 420.0;
        high.gate_verdicts = vec![GateVerdict::new("compile", false)];
        high.attach_metadata_fingerprint();

        let episodes = [low.clone(), high.clone()];
        let ranked = prioritize_by_importance(&episodes, &[low, high.clone()]);
        assert_eq!(
            ranked.first().map(|episode| episode.id.as_str()),
            Some(high.id.as_str())
        );
    }
}
