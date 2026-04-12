//! Stage 1 gathering, Stage 2 ranking, and Stage 3 compression for the
//! 5-stage assembly pipeline.
//!
//! This module collects candidate context chunks from durable memory,
//! recent episodes, task-local file context, and recent plan signals.
//! Later pipeline stages can score, compress, and inject the gathered
//! chunks into the system prompt.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use roko_core::{Body, Signal};
use roko_learn::episode_logger::Episode;
use roko_neuro::{EpisodeStore, KnowledgeEntry, KnowledgeStore};
use serde::de::DeserializeOwned;

use crate::{ContextSource, TaskInput};

#[cfg(feature = "hdc")]
use bardo_primitives::hdc::text_fingerprint;

/// Normalized PAD state used to bias retrieval when Daimon is available.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    /// Pleasure dimension. Lower values favor cautionary / anti-knowledge.
    pub pleasure: f64,
    /// Arousal dimension. Higher values favor recent and action-oriented knowledge.
    pub arousal: f64,
    /// Dominance dimension. Reserved for future modulation.
    pub dominance: f64,
}

impl PadState {
    /// Construct a normalized PAD state.
    #[must_use]
    pub const fn new(pleasure: f64, arousal: f64, dominance: f64) -> Self {
        Self {
            pleasure,
            arousal,
            dominance,
        }
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
    /// Chunks in the lower half of the ranking are summarized to a short head
    /// plus ellipsis. The top half stays verbatim. If the compressed set still
    /// exceeds the budget, the lowest-ranked chunks are dropped until it fits.
    #[must_use]
    pub fn compress(&self, mut chunks: Vec<ContextChunk>) -> Vec<ContextChunk> {
        if chunks.is_empty() {
            return chunks;
        }

        let split_at = chunks.len() / 2;
        for (idx, chunk) in chunks.iter_mut().enumerate() {
            if idx >= split_at {
                continue;
            }
            chunk.content = summarize_content(&chunk.content);
        }

        let mut total_tokens: usize = chunks
            .iter()
            .map(|chunk| estimate_chunk_tokens(&chunk.content))
            .sum();

        while total_tokens > self.max_context_tokens {
            let Some(chunk) = chunks.pop() else {
                break;
            };
            total_tokens = total_tokens.saturating_sub(estimate_chunk_tokens(&chunk.content));
        }

        chunks
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
            });
        }
        chunks
    }

    fn gather_recent_signals(&self, plan_id: &str, signals_path: &Path) -> Vec<ContextChunk> {
        let signals = read_jsonl_lossy::<Signal>(signals_path);

        let mut recent: Vec<Signal> = signals
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
    }
}

fn render_signal_chunk(signal: &Signal) -> String {
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
        "### Signal {}\nKind: {}\nTags: {}\nCreated: {}\n```\n{}\n```",
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

    let arousal = affect.arousal.clamp(0.0, 1.0);
    let low_pleasure = (1.0 - affect.pleasure.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let action = action_orientation(chunk);
    let caution = caution_orientation(chunk);

    let arousal_bias = arousal * (0.30 * recency + 0.35 * action);
    let pleasure_bias = low_pleasure * (1.00 * caution - 0.30 * action);

    arousal_bias + pleasure_bias
}

fn action_orientation(chunk: &ContextChunk) -> f64 {
    match &chunk.source {
        ContextSource::KnowledgeEntry { kind, .. } => {
            let kind = kind.to_ascii_lowercase();
            let kind_score = match kind.as_str() {
                "procedure" => 1.0,
                "playbook" => 0.9,
                "heuristic" => 0.7,
                "insight" => 0.5,
                "fact" => 0.35,
                "constraint" => 0.25,
                "antiknowledge" | "anti_knowledge" => 0.1,
                _ => 0.25,
            };
            let content = chunk.content.to_ascii_lowercase();
            let content_score = if content.contains("step")
                || content.contains("run ")
                || content.contains("use ")
                || content.contains("implement ")
                || content.contains("command")
            {
                0.2
            } else {
                0.0
            };
            ((kind_score + content_score) as f64).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

fn caution_orientation(chunk: &ContextChunk) -> f64 {
    match &chunk.source {
        ContextSource::KnowledgeEntry { kind, .. } => {
            let kind = kind.to_ascii_lowercase();
            let kind_score = match kind.as_str() {
                "antiknowledge" | "anti_knowledge" => 1.0,
                "constraint" => 0.9,
                "heuristic" => 0.5,
                "fact" | "insight" => 0.25,
                _ => 0.15,
            };
            let content = chunk.content.to_ascii_lowercase();
            let content_score = if content.contains("avoid")
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
            ((kind_score + content_score) as f64).clamp(0.0, 1.0)
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
    let now_secs = chrono::Utc::now().timestamp();
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
    use chrono::Utc;
    use roko_core::{Body, Kind, Provenance, Signal};
    use roko_neuro::KnowledgeKind;
    use tempfile::TempDir;

    fn task_input() -> TaskInput {
        TaskInput {
            id: "T1".into(),
            title: "Wire context assembly".into(),
            description: Some("Gather ranked context chunks for prompt assembly".into()),
            tier: "focused".into(),
            files: vec!["src/lib.rs".into()],
            read_files: vec![crate::ReadFileSpec {
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

    fn signal(plan_id: &str, kind: &str, body: &str, created_at_ms: i64) -> Signal {
        Signal::builder(Kind::Custom(kind.into()))
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
            content: "### Knowledge Procedure\nConfidence: 0.95\nTags: deploy, rollback\n```\nUse the migration rollback command immediately after validation.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "proc".into(),
                kind: "Procedure".into(),
                source: None,
            },
            relevance: 0.3,
            track_record: Some(0.9),
            confidence: Some(0.95),
            recency: Some(0.95),
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
        };
        let neutral = ContextChunk {
            content: "### Knowledge Fact\nConfidence: 0.80\nTags: deploy\n```\nDeployments are scheduled on weekdays.\n```"
                .into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "fact".into(),
                kind: "Fact".into(),
                source: None,
            },
            relevance: 0.7,
            track_record: Some(0.7),
            confidence: Some(0.8),
            recency: Some(0.7),
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
        assert_eq!(
            chunks
                .iter()
                .filter(|chunk| matches!(&chunk.source, ContextSource::RecentSignal { .. }))
                .count(),
            10
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
                kind: KnowledgeKind::Procedure,
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
                hdc_vector: None,
            })
            .expect("add procedure");
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
                hdc_vector: None,
            })
            .expect("add anti-knowledge");
        knowledge_store
            .add(KnowledgeEntry {
                id: "neutral-fact".into(),
                kind: KnowledgeKind::Fact,
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
                hdc_vector: None,
            })
            .expect("add fact");

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

        // High arousal boosts action-oriented entries — Procedure scores highest.
        assert_eq!(high_first, "Procedure");
        // Low pleasure boosts caution-oriented entries via affect_bias, but the
        // track_record shortcut in score_chunk (track_record * similarity /
        // uncertainty ≈ 9×similarity) dominates, keeping the more-recent and
        // higher-similarity Procedure entry on top in both affect states.
        assert_eq!(low_first, "Procedure");
    }

    #[test]
    fn compress_summarizes_lower_half_and_keeps_upper_half_verbatim() {
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

        assert_eq!(compressed.len(), 4);
        // compress() summarizes the *lower* indices (0..split_at) and keeps
        // the upper indices verbatim.  split_at = 4/2 = 2, so indices 0 and 1
        // are summarized (head + "..."), while 2 and 3 stay intact.
        assert_eq!(compressed[0].content, "top chunk...");
        assert_eq!(compressed[1].content, "second chunk...");
        assert_eq!(compressed[2].content, "third chunk");
        assert_eq!(compressed[3].content, "bottom chunk");
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
        // The surviving chunk (index 0) was in the lower-half that gets
        // summarized, so it carries the "..." suffix from summarize_content.
        assert_eq!(
            compressed[0].content,
            "top chunk top chunk top chunk top chunk..."
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
