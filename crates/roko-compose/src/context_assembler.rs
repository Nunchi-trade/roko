//! Stage 1 gathering and Stage 2 ranking for the 5-stage assembly pipeline.
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
            max_context_tokens: 4_000,
        }
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
        chunks
    }

    /// Rank gathered chunks by descending score.
    fn rank(&self, task_text: &str, chunks: &mut Vec<ContextChunk>) {
        chunks.sort_by(|left, right| {
            let right_score = score_chunk(task_text, right);
            let left_score = score_chunk(task_text, left);
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
            chunk.relevance = score_chunk(task_text, chunk);
        }
    }

    fn gather_knowledge(&self, task_text: &str) -> Vec<ContextChunk> {
        let Ok(entries) = self.knowledge_store.query(task_text, 10) else {
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
    let content = format!(
        "### Knowledge {:?}\nConfidence: {:.2}\nTags: {}\n```\n{}\n```",
        entry.kind,
        confidence,
        if entry.tags.is_empty() {
            String::from("-")
        } else {
            entry.tags.join(", ")
        },
        entry.content,
    );
    ContextChunk {
        content,
        source: ContextSource::KnowledgeEntry {
            entry_id: entry.id,
            kind: format!("{:?}", entry.kind),
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
        Body::Json(value) => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
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
    if let Some(reason) = episode.failure_reason.as_deref().filter(|s| !s.trim().is_empty()) {
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
        let passed = episode.gate_verdicts.iter().filter(|verdict| verdict.passed).count() as f64;
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
    if let Some(task_tags) = episode.extra.get("task_tags").and_then(|value| value.as_array()) {
        for value in task_tags {
            if let Some(tag) = value.as_str() {
                parts.push(tag.to_string());
            }
        }
    }
    if let Some(files) = episode.extra.get("files").and_then(|value| value.as_array()) {
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
    if let Some(desc) = task.description.as_deref().filter(|text| !text.trim().is_empty()) {
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

fn score_chunk(task_text: &str, chunk: &ContextChunk) -> f64 {
    let similarity = semantic_similarity(task_text, &chunk.content);
    let source_priority = source_priority(&chunk.source);
    let recency = chunk.recency.unwrap_or(0.5);
    let confidence = chunk.confidence.unwrap_or(0.5);

    if let Some(track_record) = chunk.track_record {
        let uncertainty = (1.0 - confidence).clamp(0.1, 1.0);
        let active_score = track_record * similarity / uncertainty;
        if active_score > 0.0 {
            return active_score;
        }
    }

    similarity * 0.3 + recency * 0.2 + confidence * 0.3 + source_priority * 0.2
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
        ep.extra.insert("plan_id".into(), serde_json::json!(plan_id));
        ep.extra.insert("task_title".into(), serde_json::json!(task_id));
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

    #[test]
    fn rank_prefers_source_priority_in_fallback() {
        let task_text = "implement context assembly";
        let file_chunk = inline_file_chunk(task_text);
        let signal_chunk = signal_chunk_for_test(task_text);

        assert!(score_chunk(task_text, &file_chunk) > score_chunk(task_text, &signal_chunk));
    }

    #[test]
    fn rank_prefers_active_inference_when_data_is_richer() {
        let task_text = "wire ranked context chunks";
        let knowledge_chunk = knowledge_chunk_for_test(task_text);
        let signal_chunk = signal_chunk_for_test(task_text);

        assert!(score_chunk(task_text, &knowledge_chunk) > score_chunk(task_text, &signal_chunk));
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
                content: "Prompt assembly should keep high-value context near the edges.".into(),
                confidence: 0.9,
                source_episodes: vec![],
                tags: vec!["context".into(), "prompt".into()],
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
        std::fs::create_dir_all(signals_path.parent().expect("signals parent")).expect("signals dir");
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

        assert!(chunks.iter().any(|chunk| matches!(
            &chunk.source,
            ContextSource::KnowledgeEntry { .. }
        )));
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
        assert!(chunks
            .windows(2)
            .all(|pair| score_chunk(&task_text, &pair[0]) >= score_chunk(&task_text, &pair[1])));
    }
}
