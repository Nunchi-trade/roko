//! Append-only JSONL knowledge store.
//!
//! Knowledge entries live at `.roko/neuro/knowledge.jsonl` by default.
//! Writes append one JSON record per line, while maintenance operations
//! (`decay` and `gc`) rewrite the file atomically through a temporary
//! sibling.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use crate::KnowledgeEntry;

/// Default garbage-collection threshold for knowledge entries.
pub const DEFAULT_GC_MIN_CONFIDENCE: f64 = 0.05;

#[cfg(feature = "hdc")]
const HDC_VECTOR_BYTES: usize = 1280;

#[cfg(feature = "hdc")]
use bardo_primitives::hdc::HdcVector;

/// Persistent knowledge store backed by an append-only JSONL file.
///
/// The store is cheap to clone: it holds the path and a process-local
/// write gate so that concurrent maintenance operations never interleave
/// file rewrites.
#[derive(Debug, Clone)]
pub struct KnowledgeStore {
    path: PathBuf,
    write_gate: Arc<Mutex<()>>,
}

impl KnowledgeStore {
    /// Construct a store pointed at an explicit JSONL path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_gate: Arc::new(Mutex::new(())),
        }
    }

    /// Construct a store from a `.roko/` root.
    ///
    /// The resulting file is `.roko/neuro/knowledge.jsonl`.
    #[must_use]
    pub fn for_roko_dir(roko_dir: impl AsRef<Path>) -> Self {
        Self::new(
            roko_dir
                .as_ref()
                .join("neuro")
                .join("knowledge.jsonl"),
        )
    }

    /// Construct a store from a workspace root.
    ///
    /// The resulting file is `<workdir>/.roko/neuro/knowledge.jsonl`.
    #[must_use]
    pub fn for_workdir(workdir: impl AsRef<Path>) -> Self {
        Self::new(
            workdir
                .as_ref()
                .join(".roko")
                .join("neuro")
                .join("knowledge.jsonl"),
        )
    }

    /// Construct a store from an existing Roko layout.
    #[must_use]
    pub fn for_layout(layout: &roko_fs::RokoLayout) -> Self {
        Self::for_roko_dir(layout.root())
    }

    /// Path of the backing JSONL file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append a knowledge entry to the JSONL log.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, the entry
    /// cannot be serialized, or the write fails.
    pub fn add(&self, entry: KnowledgeEntry) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).context("create knowledge directory")?;
        }

        let _guard = self.write_gate.lock();
        let mut line = serde_json::to_string(&entry).context("serialize knowledge entry")?;
        line.push('\n');

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("open knowledge store at {}", self.path.display()))?;
        file.write_all(line.as_bytes())
            .context("append knowledge entry")?;
        file.flush().context("flush knowledge entry")?;
        file.sync_all().context("sync knowledge entry")?;
        Ok(())
    }

    /// Query the store for entries relevant to `topic`.
    ///
    /// Relevance is scored by keyword overlap in tags/content, multiplied
    /// by confidence and recency. When the `hdc` feature is enabled, HDC
    /// similarity is added as an extra signal.
    ///
    /// # Errors
    ///
    /// Returns an error if the backing file cannot be read.
    pub fn query(&self, topic: &str, limit: usize) -> Result<Vec<KnowledgeEntry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let now = Utc::now();
        let entries = self.read_all()?;
        let topic_terms = tokenize(topic);
        let topic_norm = normalize(topic);

        let mut scored: Vec<(f64, KnowledgeEntry)> = entries
            .into_iter()
            .filter_map(|entry| {
                let keyword_score = keyword_score(&entry, &topic_terms, &topic_norm);
                let recency = recency_factor(&entry, now);
                let confidence = entry.confidence.clamp(0.0, 1.0);
                let score = keyword_score * confidence * recency;

                #[cfg(feature = "hdc")]
                let score = score + hdc_similarity(&entry, topic);

                (score > 0.0).then_some((score, entry))
            })
            .collect();

        scored.sort_by(|left, right| compare_scores(left, right));
        Ok(scored
            .into_iter()
            .take(limit)
            .map(|(_, entry)| entry)
            .collect())
    }

    /// Decay confidence for old entries using their configured half-life.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read or rewritten.
    pub fn decay(&self) -> Result<()> {
        let _guard = self.write_gate.lock();
        let now = Utc::now();
        let mut entries = self.read_all()?;

        for entry in &mut entries {
            let factor = recency_factor(entry, now);
            entry.confidence = (entry.confidence.max(0.0) * factor).clamp(0.0, 1.0);
        }

        self.rewrite_all(&entries)
    }

    /// Garbage-collect entries whose confidence falls below `min_confidence`.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read or rewritten.
    pub fn gc(&self, min_confidence: f64) -> Result<()> {
        let _guard = self.write_gate.lock();
        let threshold = min_confidence.max(0.0);
        let entries = self
            .read_all()?
            .into_iter()
            .filter(|entry| entry.confidence >= threshold)
            .collect::<Vec<_>>();
        self.rewrite_all(&entries)
    }

    fn read_all(&self) -> Result<Vec<KnowledgeEntry>> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err).with_context(|| {
                format!("open knowledge store at {}", self.path.display())
            }),
        };

        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for (line_idx, line) in reader.lines().enumerate() {
            let line = line.with_context(|| {
                format!("read knowledge line {} from {}", line_idx + 1, self.path.display())
            })?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<KnowledgeEntry>(&line) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    fn rewrite_all(&self, entries: &[KnowledgeEntry]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).context("create knowledge directory")?;
        }

        let tmp_path = self.path.with_extension("jsonl.tmp");
        {
            let mut tmp = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&tmp_path)
                .with_context(|| format!("open temp knowledge file {}", tmp_path.display()))?;
            for entry in entries {
                let line = serde_json::to_string(entry).context("serialize knowledge entry")?;
                tmp.write_all(line.as_bytes())
                    .context("write knowledge entry")?;
                tmp.write_all(b"\n").context("write newline")?;
            }
            tmp.flush().context("flush knowledge rewrite")?;
            tmp.sync_all().context("sync knowledge rewrite")?;
        }

        fs::rename(&tmp_path, &self.path).with_context(|| {
            format!(
                "replace knowledge store {} with {}",
                self.path.display(),
                tmp_path.display()
            )
        })?;
        Ok(())
    }
}

fn normalize(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_alphanumeric() { ch.to_ascii_lowercase() } else { ' ' })
        .collect::<String>()
}

fn tokenize(text: &str) -> Vec<String> {
    normalize(text)
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn keyword_score(entry: &KnowledgeEntry, terms: &[String], topic_norm: &str) -> f64 {
    let content = normalize(&entry.content);
    let tags: Vec<String> = entry.tags.iter().map(|tag| normalize(tag)).collect();

    let mut score = 0.0;
    if !topic_norm.is_empty() {
        if content.contains(topic_norm) {
            score += 1.0;
        }
        if tags.iter().any(|tag| tag.contains(topic_norm) || topic_norm.contains(tag)) {
            score += 1.0;
        }
    }

    for term in terms {
        if content.contains(term) || tags.iter().any(|tag| tag.contains(term) || term.contains(tag)) {
            score += 1.0;
        }
    }

    score
}

fn recency_factor(entry: &KnowledgeEntry, now: DateTime<Utc>) -> f64 {
    let age = now
        .signed_duration_since(entry.created_at)
        .num_seconds()
        .max(0) as f64
        / 86_400.0;
    let half_life = entry.half_life_days.max(1e-6);
    0.5_f64.powf(age / half_life)
}

fn compare_scores(
    left: &(f64, KnowledgeEntry),
    right: &(f64, KnowledgeEntry),
) -> std::cmp::Ordering {
    right
        .0
        .partial_cmp(&left.0)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            right
                .1
                .confidence
                .partial_cmp(&left.1.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| right.1.created_at.cmp(&left.1.created_at))
}

#[cfg(feature = "hdc")]
fn hdc_similarity(entry: &KnowledgeEntry, topic: &str) -> f64 {
    let Some(vector) = entry.hdc_vector.as_deref() else {
        return 0.0;
    };
    let Ok(bytes) = <[u8; HDC_VECTOR_BYTES]>::try_from(vector) else {
        return 0.0;
    };
    let entry_vec = HdcVector::from_bytes(&bytes);
    let topic_vec = HdcVector::from_seed(topic.as_bytes());
    topic_vec.similarity(&entry_vec) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use tempfile::TempDir;
    use crate::KnowledgeKind;

    fn entry(id: &str, content: &str, tags: &[&str], confidence: f64, created_at: DateTime<Utc>) -> KnowledgeEntry {
        KnowledgeEntry {
            id: id.to_owned(),
            kind: KnowledgeKind::Fact,
            content: content.to_owned(),
            confidence,
            source_episodes: Vec::new(),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            created_at,
            half_life_days: 30.0,
            hdc_vector: None,
        }
    }

    #[test]
    fn add_query_and_gc_roundtrip() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(entry(
                "k1",
                "Rust async actors and memory stores",
                &["rust", "async"],
                1.0,
                now,
            ))
            .expect("add first");
        store
            .add(entry(
                "k2",
                "Rust data pipelines",
                &["rust"],
                0.8,
                now - Duration::days(10),
            ))
            .expect("add second");
        store
            .add(entry(
                "k3",
                "Completely unrelated note",
                &["misc"],
                0.01,
                now,
            ))
            .expect("add third");

        let results = store.query("rust async", 2).expect("query");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "k1");
        assert_eq!(results[1].id, "k2");

        store.gc(DEFAULT_GC_MIN_CONFIDENCE).expect("gc");
        let all = store.read_all().expect("read after gc");
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|entry| entry.id != "k3"));
    }

    #[test]
    fn decay_reduces_old_entries() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let created_at = Utc::now() - Duration::days(30);

        store
            .add(entry(
                "k1",
                "A durable heuristic",
                &["heuristic"],
                1.0,
                created_at,
            ))
            .expect("add");

        store.decay().expect("decay");
        let all = store.read_all().expect("read");
        assert_eq!(all.len(), 1);
        assert!((all[0].confidence - 0.5).abs() < 0.05);
    }
}
