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
use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::Serialize;

use crate::{KnowledgeEntry, KnowledgeKind};

/// Default garbage-collection threshold for knowledge entries.
pub const DEFAULT_GC_MIN_CONFIDENCE: f64 = 0.05;
/// Multiplier applied when a knowledge entry has multiple independent sources.
const CONFIRMATION_BOOST: f64 = 1.5;

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

/// Aggregate statistics for a durable knowledge store snapshot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnowledgeStats {
    /// Total number of retained knowledge entries.
    pub total_entries: usize,
    /// Number of entries per semantic kind.
    pub kind_counts: BTreeMap<String, usize>,
    /// Mean confidence across all entries.
    pub average_confidence: Option<f64>,
    /// Oldest entry in the store, if any.
    pub oldest_entry: Option<KnowledgeEntry>,
    /// Newest entry in the store, if any.
    pub newest_entry: Option<KnowledgeEntry>,
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
    /// by confidence, recency, and a 1.5× confirmation boost for entries
    /// backed by multiple independent episodes. When the `hdc` feature is
    /// enabled, HDC similarity is added as an extra signal.
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
                let confidence = effective_confidence(&entry);
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

    /// Compute aggregate statistics over the current knowledge corpus.
    ///
    /// The snapshot is derived from the current on-disk entries and
    /// ignores malformed JSONL lines, matching the store's tolerant read
    /// behavior.
    ///
    /// # Errors
    ///
    /// Returns an error if the backing file cannot be read.
    pub fn stats(&self) -> Result<KnowledgeStats> {
        let entries = self.read_all()?;
        let total_entries = entries.len();
        let mut kind_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut confidence_sum = 0.0;
        let mut oldest_entry: Option<&KnowledgeEntry> = None;
        let mut newest_entry: Option<&KnowledgeEntry> = None;

        for entry in &entries {
            *kind_counts
                .entry(knowledge_kind_label(entry.kind).to_owned())
                .or_insert(0) += 1;
            confidence_sum += entry.confidence;

            if oldest_entry
                .map(|current| entry.created_at < current.created_at)
                .unwrap_or(true)
            {
                oldest_entry = Some(entry);
            }
            if newest_entry
                .map(|current| entry.created_at > current.created_at)
                .unwrap_or(true)
            {
                newest_entry = Some(entry);
            }
        }

        let average_confidence = if total_entries > 0 {
            Some(confidence_sum / total_entries as f64)
        } else {
            None
        };

        Ok(KnowledgeStats {
            total_entries,
            kind_counts,
            average_confidence,
            oldest_entry: oldest_entry.cloned(),
            newest_entry: newest_entry.cloned(),
        })
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
            .filter(|entry| effective_confidence(entry) >= threshold)
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

    #[cfg(feature = "hdc")]
    /// Build an in-memory HDC index over the current knowledge store.
    ///
    /// The index fingerprints each entry's content once and keeps the
    /// resulting vectors resident for fast similarity search.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read.
    pub fn memory_index(&self) -> Result<MemoryIndex> {
        Ok(MemoryIndex::from_entries(self.read_all()?))
    }
}

#[cfg(feature = "hdc")]
/// A precomputed HDC index over durable knowledge entries.
///
/// The index fingerprints each entry's content with
/// [`bardo_primitives::hdc::HdcVector::from_seed`] and stores the
/// resulting vectors alongside the source entries. Searches fingerprint
/// the query string once and rank entries by HDC similarity, which keeps
/// semantic lookup fast when the corpus is already indexed.
#[derive(Debug, Clone)]
pub struct MemoryIndex {
    entries: Vec<IndexedKnowledgeEntry>,
}

#[cfg(feature = "hdc")]
#[derive(Debug, Clone)]
struct IndexedKnowledgeEntry {
    entry: KnowledgeEntry,
    fingerprint: HdcVector,
}

#[cfg(feature = "hdc")]
/// One HDC search result from a [`MemoryIndex`].
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryHit {
    /// The matched knowledge entry.
    pub entry: KnowledgeEntry,
    /// Similarity against the query fingerprint in the range `0.0..=1.0`.
    pub similarity: f64,
}

#[cfg(feature = "hdc")]
impl MemoryIndex {
    /// Build an index from a collection of knowledge entries.
    ///
    /// Each entry is fingerprinted from its content. Empty content still
    /// receives a deterministic vector, so the index remains total.
    #[must_use]
    pub fn from_entries(entries: Vec<KnowledgeEntry>) -> Self {
        let entries = entries
            .into_iter()
            .map(|entry| {
                let fingerprint = fingerprint_entry(&entry);
                IndexedKnowledgeEntry { entry, fingerprint }
            })
            .collect();
        Self { entries }
    }

    /// Number of indexed entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index contains no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Search the index for the `limit` most similar entries to `query`.
    ///
    /// The query is fingerprinted once, then compared against each
    /// precomputed entry vector. Results are sorted from highest to
    /// lowest similarity.
    #[must_use]
    pub fn search(&self, query: &str, limit: usize) -> Vec<MemoryHit> {
        if limit == 0 || self.entries.is_empty() {
            return Vec::new();
        }

        let query_fingerprint = HdcVector::from_seed(query.as_bytes());
        let mut scored: Vec<MemoryHit> = self
            .entries
            .iter()
            .map(|indexed| MemoryHit {
                entry: indexed.entry.clone(),
                similarity: query_fingerprint.similarity(&indexed.fingerprint) as f64,
            })
            .collect();

        scored.sort_by(compare_hits);
        scored.truncate(limit);
        scored
    }

    /// Return the indexed entries with their precomputed fingerprints.
    ///
    /// This is mainly useful for testing and for consumers that want to
    /// inspect or reuse the durable entries directly.
    #[must_use]
    pub fn entries(&self) -> Vec<KnowledgeEntry> {
        self.entries
            .iter()
            .map(|indexed| indexed.entry.clone())
            .collect()
    }
}

#[cfg(feature = "hdc")]
fn fingerprint_entry(entry: &KnowledgeEntry) -> HdcVector {
    HdcVector::from_seed(entry.content.as_bytes())
}

#[cfg(feature = "hdc")]
fn compare_hits(left: &MemoryHit, right: &MemoryHit) -> std::cmp::Ordering {
    right
        .similarity
        .partial_cmp(&left.similarity)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            effective_confidence(&right.entry)
                .partial_cmp(&effective_confidence(&left.entry))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| right.entry.created_at.cmp(&left.entry.created_at))
        .then_with(|| left.entry.id.cmp(&right.entry.id))
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
    let half_life = effective_half_life_days(entry);
    0.5_f64.powf(age / half_life)
}

fn effective_half_life_days(entry: &KnowledgeEntry) -> f64 {
    if entry.half_life_days.is_finite() && entry.half_life_days > 0.0 {
        entry.half_life_days
    } else {
        entry.kind.default_half_life_days()
    }
}

fn effective_confidence(entry: &KnowledgeEntry) -> f64 {
    entry.confidence.clamp(0.0, 1.0) * confirmation_boost(entry)
}

fn confirmation_boost(entry: &KnowledgeEntry) -> f64 {
    if entry.source_episodes.len() >= 2 {
        CONFIRMATION_BOOST
    } else {
        1.0
    }
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
            effective_confidence(&right.1)
                .partial_cmp(&effective_confidence(&left.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| right.1.created_at.cmp(&left.1.created_at))
}

fn knowledge_kind_label(kind: KnowledgeKind) -> &'static str {
    match kind {
        KnowledgeKind::Fact => "fact",
        KnowledgeKind::Insight => "insight",
        KnowledgeKind::Procedure => "procedure",
        KnowledgeKind::Heuristic => "heuristic",
        KnowledgeKind::Playbook => "playbook",
        KnowledgeKind::Constraint => "constraint",
        KnowledgeKind::AntiKnowledge => "anti_knowledge",
    }
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
    use crate::KnowledgeKind;
    use tempfile::TempDir;

    fn entry(
        kind: KnowledgeKind,
        id: &str,
        content: &str,
        tags: &[&str],
        confidence: f64,
        source_episodes: &[&str],
        created_at: DateTime<Utc>,
    ) -> KnowledgeEntry {
        KnowledgeEntry {
            id: id.to_owned(),
            kind,
            content: content.to_owned(),
            confidence,
            source_episodes: source_episodes
                .iter()
                .map(|source| (*source).to_owned())
                .collect(),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            created_at,
            half_life_days: kind.default_half_life_days(),
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
                KnowledgeKind::Fact,
                "k1",
                "Rust async actors and memory stores",
                &["rust", "async"],
                1.0,
                &["ep-a"],
                now,
            ))
            .expect("add first");
        store
            .add(entry(
                KnowledgeKind::Fact,
                "k2",
                "Rust data pipelines",
                &["rust"],
                0.8,
                &["ep-b"],
                now - Duration::days(10),
            ))
            .expect("add second");
        store
            .add(entry(
                KnowledgeKind::Fact,
                "k3",
                "Completely unrelated note",
                &["misc"],
                0.01,
                &[],
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
                KnowledgeKind::Insight,
                "k1",
                "A durable heuristic",
                &["heuristic"],
                1.0,
                &["ep-a", "ep-b"],
                created_at,
            ))
            .expect("add");

        store.decay().expect("decay");
        let all = store.read_all().expect("read");
        assert_eq!(all.len(), 1);
        assert!((all[0].confidence - 0.5).abs() < 0.05);
    }

    #[test]
    fn decay_uses_kind_specific_half_lives() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let created_at = Utc::now() - Duration::days(30);

        store
            .add(entry(
                KnowledgeKind::Fact,
                "fact",
                "Long-lived factual memory",
                &["fact"],
                1.0,
                &[],
                created_at,
            ))
            .expect("add fact");
        store
            .add(entry(
                KnowledgeKind::Insight,
                "insight",
                "Short-lived insight",
                &["insight"],
                1.0,
                &["ep-a", "ep-b"],
                created_at,
            ))
            .expect("add insight");
        store
            .add(entry(
                KnowledgeKind::Heuristic,
                "heuristic",
                "Mid-lived heuristic",
                &["heuristic"],
                1.0,
                &[],
                created_at,
            ))
            .expect("add heuristic");

        store.decay().expect("decay");
        let all = store.read_all().expect("read");
        let fact = all.iter().find(|entry| entry.id == "fact").expect("fact");
        let insight = all
            .iter()
            .find(|entry| entry.id == "insight")
            .expect("insight");
        let heuristic = all
            .iter()
            .find(|entry| entry.id == "heuristic")
            .expect("heuristic");

        assert!(fact.confidence > heuristic.confidence);
        assert!(heuristic.confidence > insight.confidence);
        assert!((insight.confidence - 0.5).abs() < 0.05);
        assert!(fact.confidence > 0.9);
        assert!((heuristic.confidence - 0.79).abs() < 0.05);
    }

    #[test]
    fn confirmation_boost_retains_validated_entries_through_gc() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let created_at = Utc::now() - Duration::days(30);

        store
            .add(entry(
                KnowledgeKind::Insight,
                "single",
                "Single-source insight",
                &["insight"],
                0.4,
                &["ep-a"],
                created_at,
            ))
            .expect("add single");
        store
            .add(entry(
                KnowledgeKind::Insight,
                "validated",
                "Validated insight",
                &["insight"],
                0.4,
                &["ep-a", "ep-b"],
                created_at,
            ))
            .expect("add validated");

        store.gc(0.5).expect("gc");
        let all = store.read_all().expect("read");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "validated");
    }

    #[test]
    fn stats_aggregate_by_kind_and_age() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(KnowledgeEntry {
                id: "oldest".to_owned(),
                kind: KnowledgeKind::Fact,
                content: "first".to_owned(),
                confidence: 0.8,
                source_episodes: Vec::new(),
                tags: Vec::new(),
                created_at: now - Duration::days(3),
                half_life_days: KnowledgeKind::Fact.default_half_life_days(),
                hdc_vector: None,
            })
            .expect("add oldest");
        store
            .add(KnowledgeEntry {
                id: "middle".to_owned(),
                kind: KnowledgeKind::Procedure,
                content: "second".to_owned(),
                confidence: 0.6,
                source_episodes: Vec::new(),
                tags: Vec::new(),
                created_at: now - Duration::days(1),
                half_life_days: KnowledgeKind::Procedure.default_half_life_days(),
                hdc_vector: None,
            })
            .expect("add middle");
        store
            .add(KnowledgeEntry {
                id: "newest".to_owned(),
                kind: KnowledgeKind::Fact,
                content: "third".to_owned(),
                confidence: 1.0,
                source_episodes: Vec::new(),
                tags: Vec::new(),
                created_at: now,
                half_life_days: KnowledgeKind::Fact.default_half_life_days(),
                hdc_vector: None,
            })
            .expect("add newest");

        let stats = store.stats().expect("stats");
        assert_eq!(stats.total_entries, 3);
        assert_eq!(stats.kind_counts.get("fact"), Some(&2));
        assert_eq!(stats.kind_counts.get("procedure"), Some(&1));
        assert!((stats.average_confidence.expect("average") - 0.8).abs() < f64::EPSILON);
        assert_eq!(stats.oldest_entry.as_ref().map(|entry| entry.id.as_str()), Some("oldest"));
        assert_eq!(stats.newest_entry.as_ref().map(|entry| entry.id.as_str()), Some("newest"));
    }

    #[cfg(feature = "hdc")]
    #[test]
    fn memory_index_search_prefers_matching_content() {
        let now = Utc::now();
        let index = MemoryIndex::from_entries(vec![
            entry(
                KnowledgeKind::Fact,
                "k1",
                "rust async memory retrieval",
                &["rust", "memory"],
                1.0,
                &["ep-a"],
                now,
            ),
            entry(
                KnowledgeKind::Fact,
                "k2",
                "postgres maintenance routine",
                &["db"],
                0.9,
                &[],
                now,
            ),
        ]);

        assert_eq!(index.len(), 2);
        assert!(!index.is_empty());

        let hits = index.search("rust async memory retrieval", 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry.id, "k1");
        assert!(hits[0].similarity >= 0.99);
    }

    #[cfg(feature = "hdc")]
    #[test]
    fn knowledge_store_builds_memory_index() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(entry(
                KnowledgeKind::Fact,
                "k1",
                "semantic retrieval over durable knowledge",
                &["memory"],
                1.0,
                &["ep-a"],
                now,
            ))
            .expect("add first");
        store
            .add(entry(
                KnowledgeKind::Fact,
                "k2",
                "completely unrelated topic",
                &["misc"],
                0.8,
                &[],
                now,
            ))
            .expect("add second");

        let index = store.memory_index().expect("index");
        assert_eq!(index.entries().len(), 2);
        let hits = index.search("semantic retrieval over durable knowledge", 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry.id, "k1");
    }
}
