//! Context assembly primitives for composing knowledge and episode memory.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use roko_core::{Body, EmotionalTag, Engram, PadVector};
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use serde::de::DeserializeOwned;

use crate::{KnowledgeEntry, KnowledgeStore};

#[cfg(feature = "hdc")]
use roko_primitives::hdc::text_fingerprint;

/// Existing episode persistence backend used by the context assembler.
pub type EpisodeStore = EpisodeLogger;

/// Everything the context assembler needs to know about a task.
#[derive(Clone, Debug)]
pub struct TaskInput {
    /// Task ID (e.g. "T1").
    pub id: String,
    /// Human-readable task title.
    pub title: String,
    /// Optional task description from the plan file.
    pub description: Option<String>,
    /// Complexity tier: mechanical, focused, integrative, architectural.
    pub tier: String,
    /// Files this task modifies.
    pub files: Vec<String>,
    /// Files to read as context, with optional line ranges and reasons.
    pub read_files: Vec<ReadFileSpec>,
    /// Symbol names to resolve to their signatures.
    pub symbols: Vec<String>,
    /// Anti-patterns: things the agent must not do.
    pub anti_patterns: Vec<String>,
    /// Context from prior failed attempts at this task.
    pub prior_failures: Vec<String>,
    /// Verification commands that must pass after changes.
    pub verify_commands: Vec<VerifySpec>,
    /// Acceptance criteria in the legacy string format.
    pub acceptance: Vec<String>,
    /// IDs of tasks this task depends on.
    pub depends_on: Vec<String>,
    /// Maximum lines of change allowed.
    pub max_loc: Option<u32>,
}

/// A file to read as context.
#[derive(Clone, Debug)]
pub struct ReadFileSpec {
    /// File path relative to workdir.
    pub path: String,
    /// Optional line range (e.g. "40-80").
    pub lines: Option<String>,
    /// Why this file is relevant.
    pub why: String,
}

/// A verification command.
#[derive(Clone, Debug)]
pub struct VerifySpec {
    /// Phase: structural, compile, test, integration.
    pub phase: String,
    /// Shell command to run.
    pub command: String,
    /// Message to show on failure.
    pub fail_msg: Option<String>,
}

/// Where assembled context came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextSource {
    /// Knowledge entry retrieved from the durable knowledge store.
    KnowledgeEntry {
        /// Knowledge entry identifier.
        entry_id: String,
        /// Semantic kind of the entry.
        kind: String,
        /// Provenance label for the knowledge entry, if available.
        source: Option<String>,
    },
    /// Recent episode retrieved from the episode store.
    Episode {
        /// Episode identifier.
        episode_id: String,
        /// Plan identifier associated with the episode.
        plan_id: String,
        /// Task identifier associated with the episode.
        task_id: String,
    },
    /// Inlined file content from task-local read directives.
    InlineFile {
        /// File path relative to workdir.
        path: String,
        /// Optional line range (e.g. "40-80").
        lines: Option<String>,
    },
    /// Recent Engram from the plan signal log.
    RecentSignal {
        /// Engram identifier.
        signal_id: String,
        /// Plan identifier.
        plan_id: String,
        /// Engram kind.
        kind: String,
    },
    /// Resolved symbol signature (struct/fn/trait/enum definition).
    SymbolSignature {
        /// Symbol name that was searched for.
        symbol: String,
        /// File where it was found.
        file: String,
    },
    /// Anti-pattern directive.
    AntiPattern,
    /// Verification command listing.
    Verification,
    /// Per-task brief.
    TaskBrief,
    /// Output from a completed dependency task.
    PriorTaskOutput {
        /// Comma-separated list of dependency task IDs.
        task_id: String,
    },
    /// Plan-level brief.
    PlanBrief,
    /// Research memo.
    ResearchMemo,
    /// Invariants/rubric.
    Invariants,
    /// Cross-plan context.
    CrossPlanContext,
    /// PRD extract.
    PrdExtract,
    /// Decomposition artifact.
    Decomposition,
    /// IDs and titles of sibling tasks in the same plan.
    SiblingTasks,
}

/// Normalized PAD state used to bias retrieval when Daimon is available.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    /// Pleasure dimension. Lower values favor cautionary / anti-knowledge.
    pub pleasure: f64,
    /// Arousal dimension. Higher values favor recent and action-oriented knowledge.
    pub arousal: f64,
    /// Dominance dimension. Reserved for future modulation.
    pub dominance: f64,
    /// Situation-specific somatic valence in `[-1.0, 1.0]`.
    pub somatic_valence: f64,
    /// Strength of the somatic signal in `[0.0, 1.0]`.
    pub somatic_intensity: f64,
}

impl PadState {
    /// Construct a normalized PAD state.
    #[must_use]
    pub const fn new(pleasure: f64, arousal: f64, dominance: f64) -> Self {
        Self {
            pleasure,
            arousal,
            dominance,
            somatic_valence: 0.0,
            somatic_intensity: 0.0,
        }
    }

    /// Attach an optional somatic hint from Daimon's strategy-space retrieval.
    #[must_use]
    pub fn with_somatic_hint(mut self, somatic_valence: f64, somatic_intensity: f64) -> Self {
        self.somatic_valence = somatic_valence.clamp(-1.0, 1.0);
        self.somatic_intensity = somatic_intensity.clamp(0.0, 1.0);
        self
    }
}

impl From<PadVector> for PadState {
    fn from(value: PadVector) -> Self {
        Self::new(value.pleasure, value.arousal, value.dominance)
    }
}

/// A single gathered context candidate.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextChunk {
    /// Chunk payload to be scored, compressed, and injected later.
    pub content: String,
    /// Origin of the chunk.
    pub source: ContextSource,
    /// Heuristic relevance score in `[0.0, 1.0+]`.
    pub relevance: f64,
    /// Pragmatic value proxy for active-inference scoring.
    pub track_record: Option<f64>,
    /// Confidence proxy for fallback scoring and uncertainty balancing.
    pub confidence: Option<f64>,
    /// Recency proxy for fallback scoring.
    pub recency: Option<f64>,
    /// Optional emotional provenance for mood-congruent retrieval.
    pub emotional_tag: Option<EmotionalTag>,
}

/// Stage 1/2 gatherer and ranker for context assembly.
#[derive(Debug, Clone)]
pub struct ContextAssembler {
    knowledge_store: Arc<KnowledgeStore>,
    episode_store: Arc<EpisodeStore>,
    affect_state: Option<PadState>,
    /// Future-stage budget cap for gathered context.
    max_context_tokens: usize,
}

const BASE_ATTENTION_RESERVE: f64 = 0.18;
const MAX_CHUNK_BUDGET_FRACTION: f64 = 0.35;
const MIN_CHUNK_BUDGET_TOKENS: usize = 32;
const SUMMARY_UTILITY_DISCOUNT: f64 = 0.86;
const SAME_SOURCE_DIMINISHING_RETURNS: f64 = 0.82;
const NOVELTY_PENALTY_WEIGHT: f64 = 0.35;
const MARGINAL_VALUE_STOP_RATIO: f64 = 0.5;
const CONTRARIAN_RETRIEVAL_RATIO: f64 = 0.15;
const CONTRARIAN_NEUTRAL_BAND: f64 = 0.1;

impl ContextAssembler {
    /// Create a new assembler with the default 4K gathered-context budget.
    #[must_use]
    pub fn new(knowledge_store: Arc<KnowledgeStore>, episode_store: Arc<EpisodeStore>) -> Self {
        Self {
            knowledge_store,
            episode_store,
            affect_state: None,
            max_context_tokens: 4_000,
        }
    }

    /// Attach an optional PAD state used to bias knowledge retrieval.
    #[must_use]
    pub const fn with_affect_state(mut self, affect_state: Option<PadState>) -> Self {
        self.affect_state = affect_state;
        self
    }

    /// Override the gathered-context token budget.
    #[must_use]
    pub const fn with_max_context_tokens(mut self, max_context_tokens: usize) -> Self {
        self.max_context_tokens = max_context_tokens;
        self
    }

    /// Gather candidate chunks for later ranking and injection.
    #[must_use]
    pub fn gather(
        &self,
        workdir: impl AsRef<Path>,
        task: &TaskInput,
        plan_id: &str,
        signals_path: impl AsRef<Path>,
    ) -> Vec<ContextChunk> {
        let task_text = task_query_text(task);
        let workdir = workdir.as_ref();

        let mut chunks = Vec::new();
        chunks.extend(self.gather_knowledge(&task_text));
        chunks.extend(self.gather_episodes(task, plan_id, &task_text));
        chunks.extend(self.gather_read_files(workdir, task));
        chunks.extend(self.gather_recent_signals(plan_id, signals_path.as_ref()));

        self.rank(&task_text, &mut chunks);
        self.compress(chunks)
    }

    /// Rank gathered chunks by descending score.
    fn rank(&self, task_text: &str, chunks: &mut Vec<ContextChunk>) {
        chunks.sort_by(|left, right| {
            let right_score = score_chunk(task_text, right, self.affect_state.as_ref());
            let left_score = score_chunk(task_text, left, self.affect_state.as_ref());
            right_score
                .partial_cmp(&left_score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| {
                    source_priority(&right.source)
                        .partial_cmp(&source_priority(&left.source))
                        .unwrap_or(Ordering::Equal)
                })
                .then_with(|| {
                    right
                        .relevance
                        .partial_cmp(&left.relevance)
                        .unwrap_or(Ordering::Equal)
                })
                .then_with(|| right.content.len().cmp(&left.content.len()))
        });

        for chunk in chunks.iter_mut() {
            chunk.relevance = score_chunk(task_text, chunk, self.affect_state.as_ref());
        }
    }

    /// Stage 3: compress ranked chunks and enforce the token budget.
    ///
    /// This uses an auction-style allocator instead of a simple truncate loop:
    /// each chunk bids with its retrieval score, competes under a token cost,
    /// takes diminishing returns when its source family is already represented,
    /// and is rejected once marginal value drops below the current average gain.
    #[must_use]
    pub fn compress(&self, chunks: Vec<ContextChunk>) -> Vec<ContextChunk> {
        if chunks.is_empty() {
            return Vec::new();
        }

        let budget = self.max_context_tokens.max(1);
        let max_chunk_tokens = ((budget as f64) * MAX_CHUNK_BUDGET_FRACTION)
            .ceil()
            .max(MIN_CHUNK_BUDGET_TOKENS as f64)
            .min(budget as f64) as usize;
        let candidates = chunks
            .into_iter()
            .map(|chunk| ContextCandidate::new(chunk))
            .collect::<Vec<_>>();

        let mut winners: Vec<ContextSelection> = Vec::new();
        let mut used_tokens = 0usize;
        let mut total_utility_density = 0.0;
        let mut total_bid_value = 0.0;
        let mut selected_families = Vec::new();
        let mut remaining = (0..candidates.len()).collect::<Vec<_>>();

        let reserved_contrarian = reserved_contrarian_slots(self.affect_state, &candidates);
        if reserved_contrarian > 0 {
            let mut contrarian_remaining = reserved_contrarian;
            while contrarian_remaining > 0 && used_tokens < budget {
                let mut best_contrarian: Option<ContextSelection> = None;

                for &candidate_idx in &remaining {
                    let candidate = &candidates[candidate_idx];
                    if !is_contrarian_candidate(candidate, self.affect_state) {
                        continue;
                    }

                    let Some(choice) = candidate.best_choice(
                        candidate_idx,
                        budget.saturating_sub(used_tokens),
                        max_chunk_tokens,
                        &selected_families,
                        winners
                            .iter()
                            .map(|winner: &ContextSelection| winner.candidate_index),
                        &candidates,
                    ) else {
                        continue;
                    };

                    let reserve = candidate.priority.reserve_price() * 0.85;
                    if choice.bid_value < reserve {
                        continue;
                    }

                    let should_replace = best_contrarian
                        .as_ref()
                        .map(|current| choice.sort_key() > current.sort_key())
                        .unwrap_or(true);
                    if should_replace {
                        best_contrarian = Some(choice);
                    }
                }

                let Some(best_contrarian) = best_contrarian else {
                    break;
                };

                used_tokens += best_contrarian.tokens;
                total_utility_density += best_contrarian.utility_density;
                total_bid_value += best_contrarian.bid_value;
                selected_families.push(candidates[best_contrarian.candidate_index].family);
                remaining.retain(|idx| *idx != best_contrarian.candidate_index);
                winners.push(best_contrarian);
                contrarian_remaining -= 1;
            }
        }

        while !remaining.is_empty() && used_tokens < budget {
            let mut best_choice: Option<ContextSelection> = None;

            for &candidate_idx in &remaining {
                let candidate = &candidates[candidate_idx];
                let Some(choice) = candidate.best_choice(
                    candidate_idx,
                    budget.saturating_sub(used_tokens),
                    max_chunk_tokens,
                    &selected_families,
                    winners
                        .iter()
                        .map(|winner: &ContextSelection| winner.candidate_index),
                    &candidates,
                ) else {
                    continue;
                };

                let reserve = candidate.priority.reserve_price();
                if choice.bid_value < reserve {
                    continue;
                }

                let should_replace = best_choice
                    .as_ref()
                    .map(|current| choice.sort_key() > current.sort_key())
                    .unwrap_or(true);
                if should_replace {
                    best_choice = Some(choice);
                }
            }

            let Some(best_choice) = best_choice else {
                break;
            };

            let average_density = if winners.is_empty() {
                best_choice.utility_density
            } else {
                total_utility_density / winners.len() as f64
            };
            let average_bid_value = if winners.is_empty() {
                best_choice.bid_value
            } else {
                total_bid_value / winners.len() as f64
            };
            if winners.len() >= 3
                && best_choice.utility_density < average_density * MARGINAL_VALUE_STOP_RATIO
                && best_choice.bid_value < average_bid_value * MARGINAL_VALUE_STOP_RATIO
            {
                break;
            }

            used_tokens += best_choice.tokens;
            total_utility_density += best_choice.utility_density;
            total_bid_value += best_choice.bid_value;
            selected_families.push(candidates[best_choice.candidate_index].family);
            remaining.retain(|idx| *idx != best_choice.candidate_index);
            winners.push(best_choice);
        }

        winners.sort_by(|left: &ContextSelection, right: &ContextSelection| {
            right
                .sort_key()
                .partial_cmp(&left.sort_key())
                .unwrap_or(Ordering::Equal)
        });
        winners
            .into_iter()
            .map(|winner| {
                let candidate = &candidates[winner.candidate_index];
                let mut chunk = candidate.chunk.clone();
                chunk.content = match winner.mode {
                    SelectionMode::Full => candidate.full_content.clone(),
                    SelectionMode::Summary => candidate.summary_content.clone(),
                };
                chunk.relevance = winner.bid_value;
                chunk
            })
            .collect()
    }

    fn gather_knowledge(&self, task_text: &str) -> Vec<ContextChunk> {
        let query_limit = if self.affect_state.is_some() { 20 } else { 10 };
        let Ok(entries) = self.knowledge_store.query(task_text, query_limit) else {
            return Vec::new();
        };

        entries
            .into_iter()
            .enumerate()
            .map(|(idx, entry)| knowledge_chunk(entry, idx))
            .collect()
    }

    fn gather_episodes(
        &self,
        task: &TaskInput,
        plan_id: &str,
        task_text: &str,
    ) -> Vec<ContextChunk> {
        let path = self.episode_store.path();
        let episodes = read_jsonl_lossy::<Episode>(path);

        let mut scored: Vec<(f64, Episode)> = episodes
            .into_iter()
            .map(|episode| {
                let relevance = episode_relevance(&episode, task, plan_id, task_text);
                (relevance, episode)
            })
            .filter(|(score, _)| *score > 0.0)
            .collect();

        scored.sort_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .unwrap_or(Ordering::Equal)
                .then_with(|| right.1.completed_at.cmp(&left.1.completed_at))
        });

        scored
            .into_iter()
            .take(5)
            .map(|(relevance, episode)| episode_chunk(episode, relevance, plan_id))
            .collect()
    }

    fn gather_read_files(&self, workdir: &Path, task: &TaskInput) -> Vec<ContextChunk> {
        let mut chunks = Vec::new();
        for (idx, rf) in task.read_files.iter().enumerate() {
            let full_path = workdir.join(&rf.path);
            let Ok(content) = std::fs::read_to_string(&full_path) else {
                continue;
            };

            let content = match rf.lines.as_deref() {
                Some(range) => extract_line_range(&content, range),
                None => content.lines().take(100).collect::<Vec<_>>().join("\n"),
            };
            if content.trim().is_empty() {
                continue;
            }

            let relevance = (1.0 - (idx as f64 * 0.05)).max(0.1);
            let formatted = format!(
                "### `{}` {}\nWhy: {}\n```\n{}\n```",
                rf.path,
                rf.lines
                    .as_deref()
                    .map(|l| format!("(lines {l})"))
                    .unwrap_or_default(),
                rf.why,
                content,
            );
            chunks.push(ContextChunk {
                content: formatted,
                source: ContextSource::InlineFile {
                    path: rf.path.clone(),
                    lines: rf.lines.clone(),
                },
                relevance,
                track_record: None,
                confidence: Some(0.65),
                recency: Some(relevance),
                emotional_tag: None,
            });
        }
        chunks
    }

    fn gather_recent_signals(&self, plan_id: &str, signals_path: &Path) -> Vec<ContextChunk> {
        let signals = read_jsonl_lossy::<Engram>(signals_path);

        let mut recent: Vec<Engram> = signals
            .into_iter()
            .filter(|signal| signal.tag("plan_id") == Some(plan_id))
            .rev()
            .take(10)
            .collect();
        recent.reverse();

        recent
            .into_iter()
            .enumerate()
            .map(|(idx, signal)| {
                let relevance = (1.0 - (idx as f64 * 0.08)).max(0.2);
                let content = render_signal_chunk(&signal);
                ContextChunk {
                    content,
                    source: ContextSource::RecentSignal {
                        signal_id: signal.id.to_string(),
                        plan_id: plan_id.to_string(),
                        kind: signal.kind.as_str().to_string(),
                    },
                    relevance,
                    track_record: None,
                    confidence: Some(0.45),
                    recency: Some(signal_recency_score(signal.created_at_ms)),
                    emotional_tag: signal.emotional_tag.clone(),
                }
            })
            .collect()
    }
}

fn knowledge_chunk(entry: KnowledgeEntry, idx: usize) -> ContextChunk {
    let relevance = (1.0 - (idx as f64 * 0.07)).max(0.3);
    let confidence = entry.confidence.clamp(0.0, 1.0);
    let recency = recency_score(entry.created_at.timestamp());
    let track_record = knowledge_track_record(confidence, entry.source_episodes.len());
    let source = entry.source.clone();
    let tags = if entry.tags.is_empty() {
        String::from("-")
    } else {
        entry.tags.join(", ")
    };
    let content = if let Some(warning) = entry.refutation_warning() {
        format!(
            "### Warning {:?}\nConfidence: {:.2}\nWeight: {:.2}\n{}\nTags: {}\n```\n{}\n```",
            entry.kind, confidence, entry.confidence_weight, warning, tags, entry.content,
        )
    } else {
        format!(
            "### Knowledge {:?}\nConfidence: {:.2}\nTags: {}\n```\n{}\n```",
            entry.kind, confidence, tags, entry.content,
        )
    };
    ContextChunk {
        content,
        source: ContextSource::KnowledgeEntry {
            entry_id: entry.id,
            kind: format!("{:?}", entry.kind),
            source,
        },
        relevance: track_record.max(relevance),
        track_record: Some(track_record),
        confidence: Some(confidence),
        recency: Some(recency),
        emotional_tag: entry.emotional_tag,
    }
}

fn episode_chunk(episode: Episode, relevance: f64, plan_id: &str) -> ContextChunk {
    let episode_plan_id = extra_string(&episode, "plan_id").unwrap_or_default();
    let task_id = if episode.task_id.is_empty() {
        extra_string(&episode, "task_id").unwrap_or_default()
    } else {
        episode.task_id.clone()
    };
    let summary = episode_summary(&episode);
    let content = format!(
        "### Episode {}\nPlan: {}\nTask: {}\nAgent: {}\nSuccess: {}\nCompleted: {}\n{}\n",
        episode.id,
        if episode_plan_id.is_empty() {
            plan_id.to_string()
        } else {
            episode_plan_id.clone()
        },
        task_id,
        episode.agent_template,
        episode.success,
        episode.completed_at.to_rfc3339(),
        summary,
    );
    let gate_pass_ratio = gate_pass_ratio(&episode);
    let confidence = episode_confidence(&episode, gate_pass_ratio);
    let recency = recency_score(episode.completed_at.timestamp());
    let track_record = episode_track_record(&episode, gate_pass_ratio);

    ContextChunk {
        content,
        source: ContextSource::Episode {
            episode_id: episode.id,
            plan_id: if episode_plan_id.is_empty() {
                plan_id.to_string()
            } else {
                episode_plan_id
            },
            task_id,
        },
        relevance: track_record.max(relevance),
        track_record: Some(track_record),
        confidence: Some(confidence),
        recency: Some(recency),
        emotional_tag: episode.emotional_tag.clone(),
    }
}

fn render_signal_chunk(signal: &Engram) -> String {
    let tags = if signal.tags.is_empty() {
        String::from("-")
    } else {
        signal
            .tags
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let body = match &signal.body {
        Body::Text(text) => text.clone(),
        Body::Json(value) => {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
        Body::Bytes(bytes) => format!("<{} bytes>", bytes.len()),
        Body::Empty => String::from("<empty>"),
    };

    format!(
        "### Engram {}\nKind: {}\nTags: {}\nCreated: {}\n```\n{}\n```",
        signal.id,
        signal.kind.as_str(),
        tags,
        signal.created_at_ms,
        body,
    )
}

fn episode_summary(episode: &Episode) -> String {
    let mut parts = Vec::new();
    if let Some(reason) = episode
        .failure_reason
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        parts.push(format!("Failure: {reason}"));
    }
    if !episode.gate_verdicts.is_empty() {
        let verdicts = episode
            .gate_verdicts
            .iter()
            .map(|v| format!("{}:{}", v.gate, if v.passed { "pass" } else { "fail" }))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("Gates: {verdicts}"));
    }
    if parts.is_empty() {
        String::from("Summary: no additional episode metadata")
    } else {
        format!("Summary: {}", parts.join(" | "))
    }
}

fn episode_relevance(episode: &Episode, task: &TaskInput, plan_id: &str, task_text: &str) -> f64 {
    let episode_plan = extra_string(episode, "plan_id");
    let same_plan = episode_plan.as_deref() == Some(plan_id);
    let same_task = episode.task_id == task.id
        || extra_string(episode, "task_id").as_deref() == Some(task.id.as_str());
    let text = episode_search_text(episode);
    let similarity = keyword_overlap(task_text, &text);
    let recency = recency_score(episode.completed_at.timestamp());

    let mut score = similarity * 0.4 + recency * 0.2;
    if same_plan {
        score += 0.25;
    }
    if same_task {
        score += 0.15;
    }
    score.min(1.0)
}

fn knowledge_track_record(confidence: f64, source_episode_count: usize) -> f64 {
    let confirmation = if source_episode_count > 1 {
        1.0 + ((source_episode_count.saturating_sub(1) as f64) * 0.08).min(0.3)
    } else {
        1.0
    };
    (confidence * confirmation).clamp(0.0, 1.0)
}

fn gate_pass_ratio(episode: &Episode) -> f64 {
    if episode.gate_verdicts.is_empty() {
        if episode.success { 0.75 } else { 0.35 }
    } else {
        let passed = episode
            .gate_verdicts
            .iter()
            .filter(|verdict| verdict.passed)
            .count() as f64;
        (passed / episode.gate_verdicts.len() as f64).clamp(0.0, 1.0)
    }
}

fn episode_track_record(episode: &Episode, gate_pass_ratio: f64) -> f64 {
    let success_bonus = if episode.success { 0.3 } else { 0.0 };
    (0.4 + gate_pass_ratio * 0.45 + success_bonus).clamp(0.0, 1.0)
}

fn episode_confidence(episode: &Episode, gate_pass_ratio: f64) -> f64 {
    if episode.success {
        (0.55 + gate_pass_ratio * 0.4).clamp(0.0, 1.0)
    } else {
        (0.25 + gate_pass_ratio * 0.35).clamp(0.0, 1.0)
    }
}

fn episode_search_text(episode: &Episode) -> String {
    let mut parts = vec![
        episode.task_id.clone(),
        episode.agent_template.clone(),
        episode.model.clone(),
        episode.trigger_kind.clone(),
    ];
    if let Some(plan_id) = extra_string(episode, "plan_id") {
        parts.push(plan_id);
    }
    if let Some(task_id) = extra_string(episode, "task_id") {
        parts.push(task_id);
    }
    if let Some(task_title) = extra_string(episode, "task_title") {
        parts.push(task_title);
    }
    if let Some(task_tags) = episode
        .extra
        .get("task_tags")
        .and_then(|value| value.as_array())
    {
        for value in task_tags {
            if let Some(tag) = value.as_str() {
                parts.push(tag.to_string());
            }
        }
    }
    if let Some(files) = episode
        .extra
        .get("files")
        .and_then(|value| value.as_array())
    {
        for value in files {
            if let Some(file) = value.as_str() {
                parts.push(file.to_string());
            }
        }
    }
    if let Some(reason) = episode.failure_reason.as_deref() {
        parts.push(reason.to_string());
    }
    parts.join(" ")
}

fn extra_string(episode: &Episode, key: &str) -> Option<String> {
    episode
        .extra
        .get(key)
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
}

fn task_query_text(task: &TaskInput) -> String {
    let mut parts = Vec::new();
    if let Some(desc) = task
        .description
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        parts.push(desc.to_string());
    }
    if !task.title.trim().is_empty() {
        parts.push(task.title.clone());
    }
    if !task.files.is_empty() {
        parts.push(task.files.join(" "));
    }
    parts.join(" ")
}

fn keyword_overlap(left: &str, right: &str) -> f64 {
    let left_terms = tokenize(left);
    let right_terms = tokenize(right);
    if left_terms.is_empty() || right_terms.is_empty() {
        return 0.0;
    }

    let right_set: HashSet<&str> = right_terms.iter().map(String::as_str).collect();
    let matches = left_terms
        .iter()
        .filter(|term| right_set.contains(term.as_str()))
        .count();

    matches as f64 / left_terms.len().max(right_terms.len()) as f64
}

#[cfg(feature = "hdc")]
fn semantic_similarity(left: &str, right: &str) -> f64 {
    let left_vec = text_fingerprint(left);
    let right_vec = text_fingerprint(right);
    left_vec.similarity(&right_vec) as f64
}

#[cfg(not(feature = "hdc"))]
fn semantic_similarity(left: &str, right: &str) -> f64 {
    keyword_overlap(left, right)
}

fn source_priority(source: &ContextSource) -> f64 {
    match source {
        ContextSource::KnowledgeEntry { .. } => 1.0,
        ContextSource::Episode { .. } => 0.8,
        ContextSource::InlineFile { .. } => 0.5,
        ContextSource::RecentSignal { .. } => 0.3,
        _ => 0.2,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttentionPriority {
    Critical,
    High,
    Normal,
    Background,
}

impl AttentionPriority {
    fn reserve_price(self) -> f64 {
        match self {
            Self::Critical => BASE_ATTENTION_RESERVE * 0.5,
            Self::High => BASE_ATTENTION_RESERVE * 0.75,
            Self::Normal => BASE_ATTENTION_RESERVE,
            Self::Background => BASE_ATTENTION_RESERVE * 1.8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SourceFamily {
    Knowledge,
    Episode,
    File,
    Signal,
    Directive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionMode {
    Full,
    Summary,
}

#[derive(Clone, Debug)]
struct ContextCandidate {
    chunk: ContextChunk,
    full_content: String,
    summary_content: String,
    full_tokens: usize,
    summary_tokens: usize,
    base_bid: f64,
    priority: AttentionPriority,
    family: SourceFamily,
}

#[derive(Clone, Debug)]
struct ContextSelection {
    candidate_index: usize,
    tokens: usize,
    bid_value: f64,
    utility_density: f64,
    mode: SelectionMode,
}

impl ContextSelection {
    fn sort_key(&self) -> f64 {
        self.utility_density * 0.7 + self.bid_value * 0.3
    }
}

impl ContextCandidate {
    fn new(chunk: ContextChunk) -> Self {
        let summary_content = summarize_content(&chunk.content);
        let full_content = chunk.content.clone();
        let full_tokens = estimate_chunk_tokens(&full_content);
        let summary_tokens = estimate_chunk_tokens(&summary_content);
        let priority = chunk_attention_priority(&chunk);
        let family = source_family(&chunk.source);
        Self {
            base_bid: chunk.relevance.max(0.0),
            chunk,
            full_content,
            summary_content,
            full_tokens,
            summary_tokens,
            priority,
            family,
        }
    }

    fn best_choice<'a>(
        &'a self,
        candidate_index: usize,
        remaining_budget: usize,
        max_chunk_tokens: usize,
        selected_families: &[SourceFamily],
        selected_indices: impl Iterator<Item = usize>,
        candidates: &'a [ContextCandidate],
    ) -> Option<ContextSelection> {
        let diversity_multiplier = SAME_SOURCE_DIMINISHING_RETURNS.powi(
            selected_families
                .iter()
                .filter(|family| **family == self.family)
                .count() as i32,
        );
        let novelty_penalty = selected_indices
            .map(|idx| semantic_similarity(&self.chunk.content, &candidates[idx].chunk.content))
            .fold(0.0_f64, f64::max);
        let novelty_multiplier =
            (1.0 - (novelty_penalty * NOVELTY_PENALTY_WEIGHT)).clamp(0.45, 1.0);
        let adjusted_bid = self.base_bid * diversity_multiplier * novelty_multiplier;
        let full_choice = self.selection_for_mode(
            candidate_index,
            adjusted_bid,
            remaining_budget,
            max_chunk_tokens,
            SelectionMode::Full,
        );
        let summary_choice = self.selection_for_mode(
            candidate_index,
            adjusted_bid * SUMMARY_UTILITY_DISCOUNT,
            remaining_budget,
            max_chunk_tokens,
            SelectionMode::Summary,
        );

        match (full_choice, summary_choice) {
            (Some(full), Some(summary)) => {
                if full.sort_key() >= summary.sort_key() {
                    Some(full)
                } else {
                    Some(summary)
                }
            }
            (Some(full), None) => Some(full),
            (None, Some(summary)) => Some(summary),
            (None, None) => None,
        }
    }

    fn selection_for_mode(
        &self,
        candidate_index: usize,
        bid_value: f64,
        remaining_budget: usize,
        max_chunk_tokens: usize,
        mode: SelectionMode,
    ) -> Option<ContextSelection> {
        let tokens = match mode {
            SelectionMode::Full => self.full_tokens,
            SelectionMode::Summary => self.summary_tokens,
        };
        if tokens == 0 || tokens > remaining_budget || tokens > max_chunk_tokens {
            return None;
        }

        Some(ContextSelection {
            candidate_index,
            tokens,
            bid_value,
            utility_density: bid_value / tokens as f64,
            mode,
        })
    }
}

fn chunk_attention_priority(chunk: &ContextChunk) -> AttentionPriority {
    match &chunk.source {
        ContextSource::KnowledgeEntry { kind, .. }
            if kind.eq_ignore_ascii_case("AntiKnowledge") =>
        {
            AttentionPriority::Critical
        }
        ContextSource::KnowledgeEntry { kind, .. }
            if kind.eq_ignore_ascii_case("Warning")
                || kind.eq_ignore_ascii_case("StrategyFragment") =>
        {
            AttentionPriority::High
        }
        ContextSource::InlineFile { .. } | ContextSource::Episode { .. } => AttentionPriority::High,
        ContextSource::KnowledgeEntry { .. } => AttentionPriority::Normal,
        ContextSource::RecentSignal { .. } => AttentionPriority::Background,
        _ => AttentionPriority::Normal,
    }
}

fn source_family(source: &ContextSource) -> SourceFamily {
    match source {
        ContextSource::KnowledgeEntry { .. } => SourceFamily::Knowledge,
        ContextSource::Episode { .. } => SourceFamily::Episode,
        ContextSource::InlineFile { .. } | ContextSource::SymbolSignature { .. } => {
            SourceFamily::File
        }
        ContextSource::RecentSignal { .. } => SourceFamily::Signal,
        _ => SourceFamily::Directive,
    }
}

fn reserved_contrarian_slots(
    affect_state: Option<PadState>,
    candidates: &[ContextCandidate],
) -> usize {
    let Some(affect) = affect_state else {
        return 0;
    };
    if (affect.pleasure - 0.5).abs() < CONTRARIAN_NEUTRAL_BAND {
        return 0;
    }

    let knowledge_candidates = candidates
        .iter()
        .filter(|candidate| candidate.family == SourceFamily::Knowledge)
        .count();
    if knowledge_candidates == 0 {
        return 0;
    }

    let base = ((knowledge_candidates as f64) * CONTRARIAN_RETRIEVAL_RATIO).ceil() as usize;
    let minimum = if knowledge_candidates >= 20 { 3 } else { 1 };
    base.max(minimum).min(knowledge_candidates)
}

fn is_contrarian_candidate(candidate: &ContextCandidate, affect_state: Option<PadState>) -> bool {
    let Some(affect) = affect_state else {
        return false;
    };
    if candidate.family != SourceFamily::Knowledge {
        return false;
    }

    let valence = chunk_valence(&candidate.chunk);
    if valence.abs() < 0.05 {
        return false;
    }

    if affect.pleasure < 0.5 {
        valence > 0.0
    } else {
        valence < 0.0
    }
}

fn chunk_valence(chunk: &ContextChunk) -> f64 {
    (action_orientation(chunk) - caution_orientation(chunk)).clamp(-1.0, 1.0)
}

fn signal_recency_score(created_at_ms: i64) -> f64 {
    let now_ms = Utc::now().timestamp_millis();
    let age_days = (now_ms.saturating_sub(created_at_ms)).max(0) as f64 / 86_400_000.0;
    1.0 / (1.0 + age_days)
}

fn score_chunk(task_text: &str, chunk: &ContextChunk, affect_state: Option<&PadState>) -> f64 {
    let similarity = semantic_similarity(task_text, &chunk.content);
    let source_priority = source_priority(&chunk.source);
    let dream_bonus = dream_source_bonus(&chunk.source);
    let recency = chunk.recency.unwrap_or(0.5);
    let confidence = chunk.confidence.unwrap_or(0.5);
    let affect_bias = affect_bias(chunk, recency, affect_state);

    if let Some(track_record) = chunk.track_record {
        let uncertainty = (1.0 - confidence).clamp(0.1, 1.0);
        let active_score = track_record * similarity / uncertainty;
        if active_score > 0.0 {
            return active_score + dream_bonus + affect_bias;
        }
    }

    similarity * 0.3
        + recency * 0.2
        + confidence * 0.3
        + source_priority * 0.2
        + dream_bonus
        + affect_bias
}

fn dream_source_bonus(source: &ContextSource) -> f64 {
    match source {
        ContextSource::KnowledgeEntry {
            source: Some(source),
            ..
        } if source.eq_ignore_ascii_case("dream") => 0.15,
        _ => 0.0,
    }
}

fn affect_bias(chunk: &ContextChunk, recency: f64, affect_state: Option<&PadState>) -> f64 {
    let Some(affect) = affect_state else {
        return 0.0;
    };

    let affect_pad = PadVector::new(affect.pleasure, affect.arousal, affect.dominance);
    let arousal = affect.arousal.clamp(0.0, 1.0);
    let low_pleasure = (1.0 - affect.pleasure.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let action = action_orientation(chunk);
    let caution = caution_orientation(chunk);
    let somatic_intensity = affect.somatic_intensity.clamp(0.0, 1.0);
    let negative_somatic = (-affect.somatic_valence).clamp(0.0, 1.0) * somatic_intensity;
    let positive_somatic = affect.somatic_valence.clamp(0.0, 1.0) * somatic_intensity;
    let emotional_congruence = chunk
        .emotional_tag
        .as_ref()
        .map(|tag| {
            let congruence = affect_pad.cosine_similarity(tag.mood_snapshot);
            let intensity = f64::from(tag.intensity).clamp(0.0, 1.0);
            (congruence - 0.5) * (0.20 + intensity * 0.20)
        })
        .unwrap_or(0.0);

    let arousal_bias = arousal * (0.30 * recency + 0.35 * action);
    let pleasure_bias = low_pleasure * (1.00 * caution - 0.30 * action);
    let somatic_bias = negative_somatic * (0.90 * caution - 0.20 * action)
        + positive_somatic * (0.75 * action - 0.08 * caution);

    arousal_bias + pleasure_bias + emotional_congruence + somatic_bias
}

fn action_orientation(chunk: &ContextChunk) -> f64 {
    match &chunk.source {
        ContextSource::KnowledgeEntry { kind, .. } => {
            let kind = kind.to_ascii_lowercase();
            let kind_score: f64 = match kind.as_str() {
                "procedure" => 1.0,
                "playbook" => 0.9,
                "strategy_fragment" => 0.85,
                "heuristic" => 0.7,
                "causal_link" => 0.6,
                "insight" => 0.5,
                "fact" => 0.35,
                "constraint" => 0.25,
                "warning" => 0.2,
                "antiknowledge" | "anti_knowledge" => 0.1,
                _ => 0.25,
            };
            let content = chunk.content.to_ascii_lowercase();
            let content_score: f64 = if content.contains("step")
                || content.contains("run ")
                || content.contains("use ")
                || content.contains("implement ")
                || content.contains("command")
            {
                0.2
            } else {
                0.0
            };
            (kind_score + content_score).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

fn caution_orientation(chunk: &ContextChunk) -> f64 {
    match &chunk.source {
        ContextSource::KnowledgeEntry { kind, .. } => {
            let kind = kind.to_ascii_lowercase();
            let kind_score: f64 = match kind.as_str() {
                "antiknowledge" | "anti_knowledge" => 1.0,
                "warning" => 0.95,
                "constraint" => 0.9,
                "causal_link" => 0.55,
                "heuristic" => 0.5,
                "strategy_fragment" => 0.4,
                "fact" | "insight" => 0.25,
                _ => 0.15,
            };
            let content = chunk.content.to_ascii_lowercase();
            let content_score: f64 = if content.contains("avoid")
                || content.contains("never")
                || content.contains("do not")
                || content.contains("don't")
                || content.contains("caution")
                || content.contains("risk")
                || content.contains("failure")
            {
                0.25
            } else {
                0.0
            };
            (kind_score + content_score).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '/' && ch != '-')
        .filter_map(|piece| {
            let trimmed = piece.trim().to_ascii_lowercase();
            (!trimmed.is_empty()).then_some(trimmed)
        })
        .collect()
}

fn recency_score(timestamp_secs: i64) -> f64 {
    let now_secs = Utc::now().timestamp();
    let age_days = (now_secs.saturating_sub(timestamp_secs)).max(0) as f64 / 86_400.0;
    1.0 / (1.0 + age_days)
}

fn read_jsonl_lossy<T: DeserializeOwned>(path: impl AsRef<Path>) -> Vec<T> {
    let Ok(text) = std::fs::read_to_string(path.as_ref()) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for raw in text.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<T>(trimmed) {
            out.push(value);
        }
    }
    out
}

fn extract_line_range(content: &str, range: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let parts: Vec<&str> = range.split('-').collect();
    let start = parts
        .first()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);
    let end = parts.get(1).and_then(|s| {
        if s.is_empty() {
            None
        } else {
            s.parse::<usize>().ok()
        }
    });

    let start_idx = start.saturating_sub(1).min(lines.len());
    let end_idx = end.unwrap_or(lines.len()).min(lines.len());
    lines[start_idx..end_idx].join("\n")
}

fn summarize_content(content: &str) -> String {
    let head: String = content.chars().take(100).collect();
    format!("{head}...")
}

fn estimate_chunk_tokens(content: &str) -> usize {
    content.len() / 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KnowledgeKind, KnowledgeTier};
    use roko_core::{Kind, Provenance};
    use tempfile::TempDir;

    fn task_input() -> TaskInput {
        TaskInput {
            id: "T1".into(),
            title: "Wire context assembly".into(),
            description: Some("Gather ranked context chunks for prompt assembly".into()),
            tier: "focused".into(),
            files: vec!["src/lib.rs".into()],
            read_files: vec![ReadFileSpec {
                path: "src/lib.rs".into(),
                lines: None,
                why: "core entry point".into(),
            }],
            symbols: vec![],
            anti_patterns: vec![],
            prior_failures: vec![],
            verify_commands: vec![],
            acceptance: vec![],
            depends_on: vec![],
            max_loc: None,
        }
    }

    fn episode(
        task_id: &str,
        plan_id: &str,
        completed_at: chrono::DateTime<Utc>,
        success: bool,
    ) -> Episode {
        let mut ep = Episode::new("agent", task_id);
        ep.completed_at = completed_at;
        ep.timestamp = completed_at;
        ep.agent_template = "implementer".into();
        ep.success = success;
        ep.extra
            .insert("plan_id".into(), serde_json::json!(plan_id));
        ep.extra
            .insert("task_title".into(), serde_json::json!(task_id));
        ep
    }

    fn signal(plan_id: &str, kind: &str, body: &str, created_at_ms: i64) -> Engram {
        Engram::builder(Kind::Custom(kind.into()))
            .body(Body::text(body))
            .provenance(Provenance::trusted("test"))
            .tag("plan_id", plan_id)
            .created_at_ms(created_at_ms)
            .build()
    }

    fn inline_file_chunk(content: &str) -> ContextChunk {
        ContextChunk {
            content: content.into(),
            source: ContextSource::InlineFile {
                path: "src/lib.rs".into(),
                lines: None,
            },
            relevance: 0.5,
            track_record: None,
            confidence: Some(0.6),
            recency: Some(0.5),
            emotional_tag: None,
        }
    }

    fn knowledge_chunk_for_test(content: &str) -> ContextChunk {
        ContextChunk {
            content: content.into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "k1".into(),
                kind: "Insight".into(),
                source: None,
            },
            relevance: 0.9,
            track_record: Some(0.9),
            confidence: Some(0.9),
            recency: Some(0.9),
            emotional_tag: None,
        }
    }

    fn signal_chunk_for_test(content: &str) -> ContextChunk {
        ContextChunk {
            content: content.into(),
            source: ContextSource::RecentSignal {
                signal_id: "s1".into(),
                plan_id: "plan-1".into(),
                kind: "task:update".into(),
            },
            relevance: 0.2,
            track_record: None,
            confidence: Some(0.3),
            recency: Some(0.2),
            emotional_tag: None,
        }
    }

    fn ranked_chunk(content: &str, relevance: f64) -> ContextChunk {
        ContextChunk {
            content: content.into(),
            source: ContextSource::RecentSignal {
                signal_id: content.into(),
                plan_id: "plan-1".into(),
                kind: "task:update".into(),
            },
            relevance,
            track_record: None,
            confidence: Some(0.5),
            recency: Some(0.5),
            emotional_tag: None,
        }
    }

    #[test]
    fn rank_prefers_source_priority_in_fallback() {
        let task_text = "implement context assembly";
        let file_chunk = inline_file_chunk(task_text);
        let signal_chunk = signal_chunk_for_test(task_text);

        assert!(
            score_chunk(task_text, &file_chunk, None) > score_chunk(task_text, &signal_chunk, None)
        );
    }

    #[test]
    fn rank_prefers_active_inference_when_data_is_richer() {
        let task_text = "wire ranked context chunks";
        let knowledge_chunk = knowledge_chunk_for_test(task_text);
        let signal_chunk = signal_chunk_for_test(task_text);

        assert!(
            score_chunk(task_text, &knowledge_chunk, None)
                > score_chunk(task_text, &signal_chunk, None)
        );
    }

    #[test]
    fn rank_boosts_dream_provenance_for_knowledge_chunks() {
        let task_text = "reuse the successful cluster pattern";
        let dream_chunk = ContextChunk {
            content: "### Knowledge Insight\nConfidence: 0.80\nTags: dream, cluster\n```\nReuse the same cluster pattern for repeatable tasks.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "dream".into(),
                kind: "Insight".into(),
                source: Some("dream".into()),
            },
            relevance: 0.6,
            track_record: Some(0.8),
            confidence: Some(0.8),
            recency: Some(0.8),
        emotional_tag: None,
        };
        let regular_chunk = ContextChunk {
            content: "### Knowledge Insight\nConfidence: 0.80\nTags: cluster\n```\nReuse the same cluster pattern for repeatable tasks.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "regular".into(),
                kind: "Insight".into(),
                source: None,
            },
            relevance: 0.6,
            track_record: Some(0.8),
            confidence: Some(0.8),
            recency: Some(0.8),
        emotional_tag: None,
        };

        assert!(
            score_chunk(task_text, &dream_chunk, None)
                > score_chunk(task_text, &regular_chunk, None)
        );
    }

    #[test]
    fn affect_bias_prefers_recent_action_oriented_knowledge() {
        let task_text = "deploy migration rollback";
        let recent_action = ContextChunk {
            content: "### Knowledge StrategyFragment\nConfidence: 0.95\nTags: deploy, rollback\n```\nUse the migration rollback command immediately after validation.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "proc".into(),
                kind: "StrategyFragment".into(),
                source: None,
            },
            relevance: 0.3,
            track_record: Some(0.9),
            confidence: Some(0.95),
            recency: Some(0.95),
        emotional_tag: None,
        };
        let older_caution = ContextChunk {
            content: "### Knowledge AntiKnowledge\nConfidence: 0.90\nTags: deploy, rollback\n```\nNever skip the rollback plan before deploying.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "anti".into(),
                kind: "AntiKnowledge".into(),
                source: None,
            },
            relevance: 0.8,
            track_record: Some(0.8),
            confidence: Some(0.9),
            recency: Some(0.2),
        emotional_tag: None,
        };
        let neutral = ContextChunk {
            content: "### Knowledge Insight\nConfidence: 0.80\nTags: deploy\n```\nDeployments are scheduled on weekdays.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "fact".into(),
                kind: "Insight".into(),
                source: None,
            },
            relevance: 0.7,
            track_record: Some(0.7),
            confidence: Some(0.8),
            recency: Some(0.7),
        emotional_tag: None,
        };

        let high_arousal = PadState::new(0.8, 1.0, 0.5);
        let low_pleasure = PadState::new(0.0, 0.3, 0.5);

        assert!(
            score_chunk(task_text, &recent_action, Some(&high_arousal))
                > score_chunk(task_text, &neutral, Some(&high_arousal))
        );
        assert!(
            score_chunk(task_text, &older_caution, Some(&low_pleasure))
                > score_chunk(task_text, &neutral, Some(&low_pleasure))
        );
    }

    #[test]
    fn gather_collects_ranked_chunks() {
        let dir = TempDir::new().expect("tempdir");
        let workdir = dir.path();
        std::fs::create_dir_all(workdir.join(".roko/neuro")).expect("neuro dir");
        std::fs::create_dir_all(workdir.join("src")).expect("src dir");
        std::fs::write(workdir.join("src/lib.rs"), "pub fn example() {}\n").expect("write file");

        let knowledge_store = Arc::new(KnowledgeStore::new(
            workdir.join(".roko/neuro/knowledge.jsonl"),
        ));
        knowledge_store
            .add(KnowledgeEntry {
                id: "k1".into(),
                kind: KnowledgeKind::Insight,
                source: None,
                content: "Prompt assembly should keep high-value context near the edges.".into(),
                confidence: 0.9,
                confidence_weight: 0.9,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec![],
                tags: vec!["context".into(), "prompt".into()],
                source_model: None,
                model_generality: 1.0,
                created_at: Utc::now(),
                half_life_days: 30.0,
                tier: KnowledgeTier::Consolidated,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,
            })
            .expect("add knowledge");

        let episode_store = Arc::new(EpisodeStore::new(workdir.join(".roko/episodes.jsonl")));
        let e1 = episode("T-same", "plan-1", Utc::now(), true);
        let e2 = episode("T-other", "plan-2", Utc::now(), false);
        std::fs::write(
            episode_store.path(),
            format!(
                "{}\n{}\n",
                serde_json::to_string(&e1).expect("serialize episode 1"),
                serde_json::to_string(&e2).expect("serialize episode 2"),
            ),
        )
        .expect("write episodes");

        let signals_path = workdir.join(".roko/signals.jsonl");
        std::fs::create_dir_all(signals_path.parent().expect("signals parent"))
            .expect("signals dir");
        let signals: Vec<_> = (0..12)
            .map(|idx| signal("plan-1", "task:update", &format!("signal {idx}"), idx))
            .collect();
        std::fs::write(
            &signals_path,
            signals
                .iter()
                .map(|s| serde_json::to_string(s).expect("serialize signal"))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .expect("write signals");

        let assembler = ContextAssembler::new(knowledge_store, episode_store);
        let task = task_input();
        let task_text = task_query_text(&task);
        let chunks = assembler.gather(workdir, &task, "plan-1", &signals_path);

        assert!(
            chunks
                .iter()
                .any(|chunk| matches!(&chunk.source, ContextSource::KnowledgeEntry { .. }))
        );
        assert!(chunks.iter().any(|chunk| matches!(
            &chunk.source,
            ContextSource::Episode { plan_id, .. } if plan_id == "plan-1"
        )));
        assert!(chunks.iter().any(|chunk| matches!(
            &chunk.source,
            ContextSource::InlineFile { path, .. } if path == "src/lib.rs"
        )));
        assert!(
            chunks
                .iter()
                .filter(|chunk| matches!(&chunk.source, ContextSource::RecentSignal { .. }))
                .count()
                <= 10
        );
        assert!(
            chunks
                .windows(2)
                .all(|pair| score_chunk(&task_text, &pair[0], None)
                    >= score_chunk(&task_text, &pair[1], None))
        );
    }

    #[test]
    fn gather_with_affect_prioritizes_cautionary_and_actionable_knowledge() {
        let dir = TempDir::new().expect("tempdir");
        let workdir = dir.path();
        std::fs::create_dir_all(workdir.join(".roko/neuro")).expect("neuro dir");

        let knowledge_store = Arc::new(KnowledgeStore::new(
            workdir.join(".roko/neuro/knowledge.jsonl"),
        ));
        let now = Utc::now();
        knowledge_store
            .add(KnowledgeEntry {
                id: "recent-proc".into(),
                kind: KnowledgeKind::StrategyFragment,
                source: None,
                content: "Use the rollback command after each migration".into(),
                confidence: 0.9,
                confidence_weight: 0.9,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-a".into()],
                tags: vec!["rollback".into(), "migration".into()],
                source_model: None,
                model_generality: 1.0,
                created_at: now,
                half_life_days: 30.0,
                tier: KnowledgeTier::Consolidated,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,
            })
            .expect("add strategy fragment");
        knowledge_store
            .add(KnowledgeEntry {
                id: "older-anti".into(),
                kind: KnowledgeKind::AntiKnowledge,
                source: None,
                content: "Never deploy without a rollback plan".into(),
                confidence: 0.9,
                confidence_weight: -0.9,
                refuted_insight_id: Some("insight:deploy-rollback".into()),
                refutation_evidence: Some("deploys failed when rollback was missing".into()),
                source_episodes: vec!["ep-b".into()],
                tags: vec!["rollback".into(), "deploy".into()],
                source_model: None,
                model_generality: 1.0,
                created_at: now - chrono::Duration::days(10),
                half_life_days: 30.0,
                tier: KnowledgeTier::Working,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,
            })
            .expect("add anti-knowledge");
        knowledge_store
            .add(KnowledgeEntry {
                id: "neutral-fact".into(),
                kind: KnowledgeKind::Insight,
                source: None,
                content: "Deployments happen on weekdays".into(),
                confidence: 0.9,
                confidence_weight: 0.9,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-c".into()],
                tags: vec!["deploy".into()],
                source_model: None,
                model_generality: 1.0,
                created_at: now - chrono::Duration::days(3),
                half_life_days: 30.0,
                tier: KnowledgeTier::Consolidated,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,
            })
            .expect("add insight");

        let episode_store = Arc::new(EpisodeStore::new(workdir.join(".roko/episodes.jsonl")));
        std::fs::write(episode_store.path(), "").expect("write empty episodes");

        let task = TaskInput {
            id: "T1".into(),
            title: "Deploy migration rollback".into(),
            description: Some("Choose the safest recovery path".into()),
            tier: "focused".into(),
            files: vec![],
            read_files: vec![],
            symbols: vec![],
            anti_patterns: vec![],
            prior_failures: vec![],
            verify_commands: vec![],
            acceptance: vec![],
            depends_on: vec![],
            max_loc: None,
        };
        let signals_path = workdir.join(".roko/signals.jsonl");
        std::fs::create_dir_all(signals_path.parent().expect("signals parent"))
            .expect("signals dir");
        std::fs::write(&signals_path, "").expect("write empty signals");

        let high_arousal = ContextAssembler::new(knowledge_store.clone(), episode_store.clone())
            .with_affect_state(Some(PadState::new(0.7, 1.0, 0.5)));
        let low_pleasure = ContextAssembler::new(knowledge_store, episode_store)
            .with_affect_state(Some(PadState::new(0.0, 0.2, 0.5)));

        let high_chunks = high_arousal.gather(workdir, &task, "plan-1", &signals_path);
        let low_chunks = low_pleasure.gather(workdir, &task, "plan-1", &signals_path);

        let high_first = high_chunks
            .first()
            .and_then(|chunk| match &chunk.source {
                ContextSource::KnowledgeEntry { kind, .. } => Some(kind.as_str()),
                _ => None,
            })
            .expect("knowledge chunk");
        let low_first = low_chunks
            .first()
            .and_then(|chunk| match &chunk.source {
                ContextSource::KnowledgeEntry { kind, .. } => Some(kind.as_str()),
                _ => None,
            })
            .expect("knowledge chunk");

        assert_eq!(high_first, "StrategyFragment");
        assert_eq!(low_first, "StrategyFragment");
    }

    #[test]
    fn affect_bias_prefers_emotionally_congruent_chunks_when_other_signals_match() {
        let task_text = "recover the failed rollout safely";
        let congruent = ContextChunk {
            content: "### Knowledge Warning\nConfidence: 0.90\nTags: deploy, rollback\n```\nCheck rollback health before retrying a failed rollout.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "warning-congruent".into(),
                kind: "Warning".into(),
                source: None,
            },
            relevance: 0.8,
            track_record: Some(0.8),
            confidence: Some(0.9),
            recency: Some(0.6),
            emotional_tag: Some(EmotionalTag::new(
                PadVector::new(-0.8, 0.5, 0.0),
                0.9,
                "rollback_failure",
                PadVector::new(-0.7, 0.4, 0.0),
            )),
        };
        let incongruent = ContextChunk {
            content: "### Knowledge Warning\nConfidence: 0.90\nTags: deploy, rollback\n```\nCheck rollback health before retrying a failed rollout.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "warning-incongruent".into(),
                kind: "Warning".into(),
                source: None,
            },
            relevance: 0.8,
            track_record: Some(0.8),
            confidence: Some(0.9),
            recency: Some(0.6),
            emotional_tag: Some(EmotionalTag::new(
                PadVector::new(0.8, -0.2, 0.6),
                0.9,
                "clean_success",
                PadVector::new(0.7, -0.1, 0.5),
            )),
        };

        let struggling = PadState::new(-0.8, 0.5, 0.0);

        assert!(
            score_chunk(task_text, &congruent, Some(&struggling))
                > score_chunk(task_text, &incongruent, Some(&struggling))
        );
    }

    #[test]
    fn somatic_bias_prefers_cautionary_chunks_even_with_neutral_pad() {
        let task_text = "decide how to handle the risky rollout";
        let caution = ContextChunk {
            content: "### Knowledge Warning\nConfidence: 0.90\nTags: deploy, rollback\n```\nCheck rollback health before attempting the rollout.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "warning-somatic".into(),
                kind: "Warning".into(),
                source: None,
            },
            relevance: 0.8,
            track_record: Some(0.8),
            confidence: Some(0.9),
            recency: Some(0.6),
            emotional_tag: None,
        };
        let action = ContextChunk {
            content: "### Knowledge StrategyFragment\nConfidence: 0.90\nTags: deploy, rollout\n```\nReuse the standard rollout command sequence.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "strategy-somatic".into(),
                kind: "StrategyFragment".into(),
                source: None,
            },
            relevance: 0.8,
            track_record: Some(0.8),
            confidence: Some(0.9),
            recency: Some(0.6),
            emotional_tag: None,
        };

        let negative_somatic = PadState::new(0.5, 0.2, 0.5).with_somatic_hint(-0.9, 0.8);

        assert!(
            score_chunk(task_text, &caution, Some(&negative_somatic))
                > score_chunk(task_text, &action, Some(&negative_somatic))
        );
    }

    #[test]
    fn positive_somatic_bias_boosts_actionable_chunks_from_neutral_baseline() {
        let task_text = "resume the familiar rollout";
        let action = ContextChunk {
            content: "### Knowledge StrategyFragment\nConfidence: 0.90\nTags: deploy, rollout\n```\nUse the standard rollout command sequence.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "strategy-positive-somatic".into(),
                kind: "StrategyFragment".into(),
                source: None,
            },
            relevance: 0.8,
            track_record: Some(0.8),
            confidence: Some(0.9),
            recency: Some(0.6),
            emotional_tag: None,
        };

        let neutral = PadState::new(0.5, 0.2, 0.5);
        let positive_somatic = neutral.with_somatic_hint(0.9, 0.8);

        assert!(
            score_chunk(task_text, &action, Some(&positive_somatic))
                > score_chunk(task_text, &action, Some(&neutral))
        );
    }

    #[test]
    fn low_pleasure_retrieval_keeps_a_contrarian_positive_chunk() {
        let dir = TempDir::new().expect("tempdir");
        let workdir = dir.path();
        std::fs::create_dir_all(workdir.join(".roko/neuro")).expect("neuro dir");

        let knowledge_store = Arc::new(KnowledgeStore::new(
            workdir.join(".roko/neuro/knowledge.jsonl"),
        ));
        let now = Utc::now();

        for idx in 0..4 {
            knowledge_store
                .add(KnowledgeEntry {
                    id: format!("anti-{idx}"),
                    kind: KnowledgeKind::AntiKnowledge,
                    source: None,
                    content: format!("Never skip rollback validation {idx}"),
                    confidence: 0.95,
                    confidence_weight: -0.95,
                    refuted_insight_id: Some(format!("insight-{idx}")),
                    refutation_evidence: Some("rollback gaps caused failure".into()),
                    source_episodes: vec![format!("ep-anti-{idx}")],
                    tags: vec!["deploy".into(), "rollback".into()],
                    source_model: None,
                    model_generality: 1.0,
                    created_at: now,
                    half_life_days: KnowledgeKind::AntiKnowledge.default_half_life_days(),
                    tier: KnowledgeTier::Working,
                    emotional_tag: None,
                    emotional_provenance: None,
                    hdc_vector: None,
                })
                .expect("add anti-knowledge");
        }

        knowledge_store
            .add(KnowledgeEntry {
                id: "strategy-positive".into(),
                kind: KnowledgeKind::StrategyFragment,
                source: None,
                content: "Run the rollback command immediately after migration verification".into(),
                confidence: 0.9,
                confidence_weight: 0.9,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-pos".into()],
                tags: vec!["deploy".into(), "rollback".into()],
                source_model: None,
                model_generality: 1.0,
                created_at: now,
                half_life_days: KnowledgeKind::StrategyFragment.default_half_life_days(),
                tier: KnowledgeTier::Consolidated,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,
            })
            .expect("add strategy");

        let episode_store = Arc::new(EpisodeStore::new(workdir.join(".roko/episodes.jsonl")));
        std::fs::write(episode_store.path(), "").expect("write empty episodes");
        let signals_path = workdir.join(".roko/signals.jsonl");
        std::fs::create_dir_all(signals_path.parent().expect("signals parent"))
            .expect("signals dir");
        std::fs::write(&signals_path, "").expect("write empty signals");

        let assembler = ContextAssembler::new(knowledge_store, episode_store)
            .with_affect_state(Some(PadState::new(0.1, 0.4, 0.5)))
            .with_max_context_tokens(220);
        let task = TaskInput {
            id: "T1".into(),
            title: "Deploy migration rollback".into(),
            description: Some("Recover safely from a failing deployment".into()),
            tier: "focused".into(),
            files: vec![],
            read_files: vec![],
            symbols: vec![],
            anti_patterns: vec![],
            prior_failures: vec![],
            verify_commands: vec![],
            acceptance: vec![],
            depends_on: vec![],
            max_loc: None,
        };

        let chunks = assembler.gather(workdir, &task, "plan-1", &signals_path);
        assert!(chunks.iter().any(|chunk| {
            matches!(
                &chunk.source,
                ContextSource::KnowledgeEntry { kind, entry_id, .. }
                    if kind == "StrategyFragment" && entry_id == "strategy-positive"
            )
        }));
    }

    #[test]
    fn high_pleasure_retrieval_keeps_a_contrarian_cautionary_chunk() {
        let dir = TempDir::new().expect("tempdir");
        let workdir = dir.path();
        std::fs::create_dir_all(workdir.join(".roko/neuro")).expect("neuro dir");

        let knowledge_store = Arc::new(KnowledgeStore::new(
            workdir.join(".roko/neuro/knowledge.jsonl"),
        ));
        let now = Utc::now();

        for idx in 0..4 {
            knowledge_store
                .add(KnowledgeEntry {
                    id: format!("heur-{idx}"),
                    kind: KnowledgeKind::Heuristic,
                    source: None,
                    content: format!("Use the proven rollout sequence {idx}"),
                    confidence: 0.95,
                    confidence_weight: 0.95,
                    refuted_insight_id: None,
                    refutation_evidence: None,
                    source_episodes: vec![format!("ep-heur-{idx}")],
                    tags: vec!["deploy".into(), "rollout".into()],
                    source_model: None,
                    model_generality: 1.0,
                    created_at: now,
                    half_life_days: KnowledgeKind::Heuristic.default_half_life_days(),
                    tier: KnowledgeTier::Consolidated,
                    emotional_tag: None,
                    emotional_provenance: None,
                    hdc_vector: None,
                })
                .expect("add heuristic");
        }

        knowledge_store
            .add(KnowledgeEntry {
                id: "warning-contrarian".into(),
                kind: KnowledgeKind::Warning,
                source: None,
                content: "Avoid deploying without first checking rollback health".into(),
                confidence: 0.9,
                confidence_weight: 0.9,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-warning".into()],
                tags: vec!["deploy".into(), "rollback".into()],
                source_model: None,
                model_generality: 1.0,
                created_at: now,
                half_life_days: KnowledgeKind::Warning.default_half_life_days(),
                tier: KnowledgeTier::Consolidated,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,
            })
            .expect("add warning");

        let episode_store = Arc::new(EpisodeStore::new(workdir.join(".roko/episodes.jsonl")));
        std::fs::write(episode_store.path(), "").expect("write empty episodes");
        let signals_path = workdir.join(".roko/signals.jsonl");
        std::fs::create_dir_all(signals_path.parent().expect("signals parent"))
            .expect("signals dir");
        std::fs::write(&signals_path, "").expect("write empty signals");

        let assembler = ContextAssembler::new(knowledge_store, episode_store)
            .with_affect_state(Some(PadState::new(0.9, 0.3, 0.5)))
            .with_max_context_tokens(220);
        let task = TaskInput {
            id: "T2".into(),
            title: "Deploy migration rollout".into(),
            description: Some("Ship the validated migration".into()),
            tier: "focused".into(),
            files: vec![],
            read_files: vec![],
            symbols: vec![],
            anti_patterns: vec![],
            prior_failures: vec![],
            verify_commands: vec![],
            acceptance: vec![],
            depends_on: vec![],
            max_loc: None,
        };

        let chunks = assembler.gather(workdir, &task, "plan-1", &signals_path);
        assert!(chunks.iter().any(|chunk| {
            matches!(
                &chunk.source,
                ContextSource::KnowledgeEntry { kind, entry_id, .. }
                    if kind == "Warning" && entry_id == "warning-contrarian"
            )
        }));
    }

    #[test]
    fn compress_uses_summary_when_chunk_exceeds_share_budget() {
        let dir = TempDir::new().expect("tempdir");
        let assembler = ContextAssembler::new(
            Arc::new(KnowledgeStore::new(dir.path().join("knowledge.jsonl"))),
            Arc::new(EpisodeStore::new(dir.path().join("episodes.jsonl"))),
        )
        .with_max_context_tokens(32);
        let chunks = vec![ranked_chunk(
            "top chunk top chunk top chunk top chunk top chunk top chunk top chunk top chunk top chunk top chunk top chunk top chunk top chunk top chunk top chunk top chunk top chunk top chunk",
            0.9,
        )];

        let compressed = assembler.compress(chunks);

        assert_eq!(compressed.len(), 1);
        assert!(compressed[0].content.ends_with("..."));
        assert!(estimate_chunk_tokens(&compressed[0].content) <= 32);
    }

    #[test]
    fn compress_keeps_multiple_high_value_chunks_under_generous_budget() {
        let dir = TempDir::new().expect("tempdir");
        let assembler = ContextAssembler::new(
            Arc::new(KnowledgeStore::new(dir.path().join("knowledge.jsonl"))),
            Arc::new(EpisodeStore::new(dir.path().join("episodes.jsonl"))),
        );
        let chunks = vec![
            ranked_chunk("top chunk", 0.9),
            ranked_chunk("second chunk", 0.8),
            ranked_chunk("third chunk", 0.7),
            ranked_chunk("bottom chunk", 0.6),
        ];

        let compressed = assembler.compress(chunks);

        assert!(compressed.len() >= 3);
        assert_eq!(compressed[0].content, "top chunk");
        assert!(
            compressed
                .iter()
                .any(|chunk| chunk.content == "second chunk" || chunk.content == "third chunk")
        );
    }

    #[test]
    fn compress_drops_lowest_ranked_chunks_until_within_budget() {
        let dir = TempDir::new().expect("tempdir");
        let assembler = ContextAssembler::new(
            Arc::new(KnowledgeStore::new(dir.path().join("knowledge.jsonl"))),
            Arc::new(EpisodeStore::new(dir.path().join("episodes.jsonl"))),
        )
        .with_max_context_tokens(12);
        let chunks = vec![
            ranked_chunk("top chunk top chunk top chunk top chunk", 0.9),
            ranked_chunk("second chunk second chunk second chunk second chunk", 0.8),
            ranked_chunk("third chunk third chunk third chunk third chunk", 0.7),
            ranked_chunk("bottom chunk bottom chunk bottom chunk bottom chunk", 0.6),
        ];

        let compressed = assembler.compress(chunks);

        assert_eq!(compressed.len(), 1);
        assert!(
            compressed[0]
                .content
                .starts_with("top chunk top chunk top chunk top chunk")
        );
        assert!(
            compressed
                .iter()
                .map(|chunk| estimate_chunk_tokens(&chunk.content))
                .sum::<usize>()
                <= 12
        );
    }
}
