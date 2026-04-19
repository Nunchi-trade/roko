//! Append-only JSONL knowledge store.
//!
//! Knowledge entries live at `.roko/neuro/knowledge.jsonl` by default.
//! Writes append one JSON record per line, while maintenance operations
//! (`decay` and `gc`) rewrite the file atomically through a temporary
//! sibling.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[cfg(feature = "hdc")]
use crate::hdc::KnowledgeHdcEncoder;
use crate::{KnowledgeEntry, KnowledgeKind, KnowledgeTier, NeuroStore};

/// Default garbage-collection threshold for knowledge entries.
pub const DEFAULT_GC_MIN_CONFIDENCE: f64 = 0.05;
/// Minimum total query score an entry must exceed to be returned.
pub const QUERY_SCORE_FLOOR: f64 = 0.0;
/// Minimum retained confidence for AntiKnowledge entries.
const ANTI_KNOWLEDGE_CONFIDENCE_FLOOR: f64 = 0.3;
/// Multiplier applied when a knowledge entry has multiple independent sources.
const CONFIRMATION_BOOST: f64 = 1.5;
/// Minimum number of shared tags for two entries to be considered similar.
const MIN_TAG_OVERLAP: usize = 1;
/// Minimum number of shared content keywords for two entries to be
/// considered similar (applied when tag overlap meets the threshold).
const MIN_KEYWORD_OVERLAP: usize = 2;
#[cfg(feature = "hdc")]
const HDC_SIMILARITY_BASELINE: f64 = 0.5;

/// HDC similarity threshold at which an AntiKnowledge match logs a warning.
#[cfg(feature = "hdc")]
const ANTI_KNOWLEDGE_WARN_THRESHOLD: f64 = 0.5;
/// HDC similarity threshold at which a new entry's confidence is discounted.
#[cfg(feature = "hdc")]
const ANTI_KNOWLEDGE_DISCOUNT_THRESHOLD: f64 = 0.7;
/// HDC similarity threshold at which a new entry is rejected entirely.
#[cfg(feature = "hdc")]
const ANTI_KNOWLEDGE_REJECT_THRESHOLD: f64 = 0.9;
/// Confidence multiplier applied when a new entry conflicts with AntiKnowledge
/// at the discount threshold.
#[cfg(feature = "hdc")]
const ANTI_KNOWLEDGE_DISCOUNT_FACTOR: f64 = 0.5;

/// A record emitted when a newly ingested knowledge entry overlaps with
/// an existing entry, indicating that an insight has been independently
/// confirmed by a separate episode.
///
/// These records are consumed by the C-Factor metrics
/// (`knowledge_integration_rate` and `convergence_velocity`) in
/// `roko-learn`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeConfirmationRecord {
    /// Timestamp of the confirmation event.
    pub created_at: DateTime<Utc>,
    /// Combined source episodes from the existing entry and the new entry.
    pub source_episodes: Vec<String>,
    /// ID of the existing entry that was confirmed.
    pub confirmed_entry_id: String,
    /// ID of the new entry that confirmed the existing one.
    pub confirming_entry_id: String,
}

/// A record of a conflict detected between a newly ingested entry and an
/// existing AntiKnowledge entry. Emitted during `ingest()` for observability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AntiKnowledgeConflict {
    /// ID of the new entry that conflicts with AntiKnowledge.
    pub entry_id: String,
    /// ID of the existing AntiKnowledge entry.
    pub anti_knowledge_id: String,
    /// HDC similarity score between the two entries.
    pub similarity: f64,
    /// Action taken: "warned", "discounted", or "rejected".
    pub action: String,
}

const HDC_VECTOR_BYTES: usize = 1280;

#[cfg(feature = "hdc")]
use roko_primitives::hdc::HdcVector;

/// Persistent knowledge store backed by an append-only JSONL file.
///
/// The store is cheap to clone: it holds the path and a process-local
/// write gate so that concurrent maintenance operations never interleave
/// file rewrites.
///
/// When new entries overlap with existing entries (by tag and keyword
/// similarity), the store emits [`KnowledgeConfirmationRecord`]s to a
/// sibling JSONL file. These records feed the C-Factor metrics
/// `knowledge_integration_rate` and `convergence_velocity`.
#[derive(Debug, Clone)]
pub struct KnowledgeStore {
    path: PathBuf,
    confirmations_path: PathBuf,
    write_gate: Arc<Mutex<()>>,
}

/// Aggregate statistics for a durable knowledge store snapshot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnowledgeStats {
    /// Total number of retained knowledge entries.
    pub total_entries: usize,
    /// Number of entries per semantic kind.
    pub kind_counts: BTreeMap<String, usize>,
    /// Number of entries per validation tier.
    pub tier_counts: BTreeMap<String, usize>,
    /// Number of entries per source label.
    pub source_counts: BTreeMap<String, usize>,
    /// Number of AntiKnowledge entries.
    pub anti_knowledge_count: usize,
    /// Mean confidence across all entries.
    pub average_confidence: Option<f64>,
    /// Oldest entry in the store, if any.
    pub oldest_entry: Option<KnowledgeEntry>,
    /// Newest entry in the store, if any.
    pub newest_entry: Option<KnowledgeEntry>,
}

/// Score breakdown for one knowledge query result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnowledgeQueryBreakdown {
    /// Keyword overlap between the query and the entry tags/content.
    pub keyword_score: f64,
    /// Confidence after anti-knowledge floors, confirmation boosts, and
    /// emotional consolidation adjustments.
    pub effective_confidence: f64,
    /// Exponential freshness multiplier derived from effective half-life.
    pub recency_factor: f64,
    /// Retrieval multiplier derived from emotional congruence and intensity.
    pub emotional_boost: f64,
    /// Optional HDC similarity contribution when the `hdc` feature is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hdc_similarity: Option<f64>,
}

/// One scored hit returned from the durable knowledge query path.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnowledgeQueryHit {
    /// The matched entry.
    pub entry: KnowledgeEntry,
    /// Total score used for ranking.
    pub total_score: f64,
    /// Individual scoring components that contributed to `total_score`.
    pub breakdown: KnowledgeQueryBreakdown,
}

/// Versioned header written as the first line of a backup JSONL file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupHeader {
    /// Backup format version. Currently `1`.
    pub version: u32,
    /// When the backup was created.
    pub created_at: DateTime<Utc>,
    /// Number of entries in the backup.
    pub entry_count: usize,
    /// Path of the source knowledge store that was exported.
    pub source_path: String,
}

/// Filter criteria for [`KnowledgeStore::export`].
#[derive(Debug, Clone, Default)]
pub struct ExportFilter {
    /// Only export entries of these kinds. `None` means all kinds.
    pub kinds: Option<Vec<KnowledgeKind>>,
    /// Minimum confidence threshold.
    pub min_confidence: Option<f64>,
    /// Only export entries with at least one of these tags.
    pub tags: Option<Vec<String>>,
    /// Only export entries created after this timestamp.
    pub since: Option<DateTime<Utc>>,
}

impl ExportFilter {
    fn matches(&self, entry: &KnowledgeEntry) -> bool {
        if let Some(kinds) = &self.kinds {
            if !kinds.contains(&entry.kind) {
                return false;
            }
        }
        if let Some(min) = self.min_confidence {
            if entry.confidence < min {
                return false;
            }
        }
        if let Some(required_tags) = &self.tags {
            if !required_tags.iter().any(|t| entry.tags.contains(t)) {
                return false;
            }
        }
        if let Some(since) = self.since {
            if entry.created_at < since {
                return false;
            }
        }
        true
    }
}

/// Options for [`KnowledgeStore::import`].
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Confidence multiplier applied to each imported entry (default 0.85).
    pub confidence_discount: f64,
    /// Whether to reset all imported entries to `KnowledgeTier::Transient`.
    pub reset_tier: bool,
    /// Label recorded in the `source` field of each imported entry.
    pub source_label: String,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            confidence_discount: 0.85,
            reset_tier: true,
            source_label: "restore".to_owned(),
        }
    }
}

/// One similarity-ranked hit returned from the durable fingerprint query path.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnowledgeSimilarityHit {
    /// The matched entry.
    pub entry: KnowledgeEntry,
    /// Raw Hamming similarity against the supplied fingerprint.
    pub similarity: f32,
}

impl KnowledgeStore {
    /// Construct a store pointed at an explicit JSONL path.
    ///
    /// Confirmation records are written to a sibling file named
    /// `knowledge-confirmations.jsonl` in the same directory.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let confirmations_path = path
            .parent()
            .map(|parent| parent.join("knowledge-confirmations.jsonl"))
            .unwrap_or_else(|| PathBuf::from("knowledge-confirmations.jsonl"));
        Self {
            path,
            confirmations_path,
            write_gate: Arc::new(Mutex::new(())),
        }
    }

    /// Construct a store from a `.roko/` root.
    ///
    /// The resulting file is `.roko/neuro/knowledge.jsonl`.
    #[must_use]
    pub fn for_roko_dir(roko_dir: impl AsRef<Path>) -> Self {
        Self::new(roko_dir.as_ref().join("neuro").join("knowledge.jsonl"))
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

    /// Path of the confirmation records JSONL file.
    #[must_use]
    pub fn confirmations_path(&self) -> &Path {
        &self.confirmations_path
    }

    /// Append a knowledge entry to the JSONL log.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, the entry
    /// cannot be serialized, or the write fails.
    pub fn add(&self, entry: KnowledgeEntry) -> Result<()> {
        self.ingest(vec![entry])
    }

    /// NEURO-07: Append entries with source-channel confidence discounting.
    ///
    /// Each entry's confidence is multiplied by the channel's trust discount
    /// before being ingested into the store.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, an entry
    /// cannot be serialized, or the write fails.
    pub fn ingest_with_source(
        &self,
        mut entries: Vec<KnowledgeEntry>,
        channel: crate::SourceChannel,
    ) -> Result<()> {
        crate::apply_source_discount(&mut entries, channel);
        self.ingest(entries)
    }

    /// Append a batch of knowledge entries to the JSONL log.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, an entry
    /// cannot be serialized, or the write fails.
    pub fn ingest(&self, entries: Vec<KnowledgeEntry>) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).context("create knowledge directory")?;
        }

        let _guard = self.write_gate.lock();
        let existing = self.read_all().unwrap_or_default();
        let entries = dedupe_entries_for_ingest(prepare_entries_for_ingest(entries), &existing);
        if entries.is_empty() {
            return Ok(());
        }

        // NEURO-04: Check new non-AntiKnowledge entries against existing
        // AntiKnowledge entries using HDC similarity. Entries that are
        // near-duplicates of refuted knowledge are rejected; moderate
        // conflicts have their confidence discounted.
        #[cfg(feature = "hdc")]
        let entries = check_against_anti_knowledge(entries, &existing);
        if entries.is_empty() {
            return Ok(());
        }

        let mut has_antiknowledge = false;
        for entry in &entries {
            if entry.kind == KnowledgeKind::AntiKnowledge
                && entry
                    .refuted_insight_id
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|refuted_id| !refuted_id.is_empty())
            {
                has_antiknowledge = true;
                break;
            }
        }

        if has_antiknowledge {
            let mut current = existing;
            current.extend(entries.iter().cloned());

            for anti in &entries {
                if anti.kind != KnowledgeKind::AntiKnowledge {
                    continue;
                }

                let Some(refuted_id) = anti.refuted_insight_id.as_deref().map(str::trim) else {
                    continue;
                };
                if refuted_id.is_empty() {
                    continue;
                }

                if let Some(original) = current.iter_mut().find(|entry| entry.id == refuted_id) {
                    original.confidence *= 0.5;
                }
            }

            self.rewrite_all(&current)?;
            return Ok(());
        }

        // Detect confirmations by comparing new entries against existing ones.
        let confirmations = detect_confirmations(&existing, &entries);

        // Apply tier promotions for confirmed entries.
        if !confirmations.is_empty() {
            let mut updated_existing = existing;
            for confirmation in &confirmations {
                if let Some(entry) = updated_existing
                    .iter_mut()
                    .find(|e| e.id == confirmation.confirmed_entry_id)
                {
                    entry.confirmation_count = entry.confirmation_count.saturating_add(1);

                    // Add distinct context from the confirming entry's source episodes.
                    if let Some(confirming) = entries
                        .iter()
                        .find(|e| e.id == confirmation.confirming_entry_id)
                    {
                        for ep in &confirming.source_episodes {
                            if !entry.distinct_contexts.contains(ep) {
                                entry.distinct_contexts.push(ep.clone());
                            }
                        }
                    }

                    // Auto-promote based on thresholds.
                    match entry.tier {
                        KnowledgeTier::Transient if entry.confirmation_count >= 2 => {
                            entry.tier = KnowledgeTier::Working;
                        }
                        KnowledgeTier::Working if entry.distinct_contexts.len() >= 3 => {
                            entry.tier = KnowledgeTier::Consolidated;
                        }
                        _ => {}
                    }
                }
            }
            self.rewrite_all(&updated_existing)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("open knowledge store at {}", self.path.display()))?;
        for entry in entries {
            let mut line = serde_json::to_string(&entry).context("serialize knowledge entry")?;
            line.push('\n');
            file.write_all(line.as_bytes())
                .context("append knowledge entry")?;
        }
        file.flush().context("flush knowledge entry")?;
        file.sync_all().context("sync knowledge entry")?;

        // Append confirmation records to the sibling JSONL file.
        if !confirmations.is_empty() {
            self.append_confirmations(&confirmations)?;
        }

        Ok(())
    }

    /// Query the store for entries relevant to `topic`.
    ///
    /// Relevance is scored by keyword overlap in tags/content, multiplied
    /// by confidence, recency, and a 1.5× confirmation boost for entries
    /// backed by multiple independent episodes. When the `hdc` feature is
    /// enabled, HDC similarity is added as an extra signal. Only entries with
    /// `total_score > QUERY_SCORE_FLOOR` are returned.
    ///
    /// # Errors
    ///
    /// Returns an error if the backing file cannot be read.
    pub fn query(&self, topic: &str, limit: usize) -> Result<Vec<KnowledgeEntry>> {
        Ok(self
            .query_hits(topic, limit)?
            .into_iter()
            .map(|hit| hit.entry)
            .collect())
    }

    /// Query the store by a serialized 10,240-bit fingerprint.
    ///
    /// Entries without a valid stored fingerprint are skipped. Results are
    /// ranked by raw Hamming similarity and then by effective confidence.
    ///
    /// # Errors
    ///
    /// Returns an error if `fingerprint` is not 1280 bytes long or the
    /// backing file cannot be read.
    pub fn query_similar(
        &self,
        fingerprint: &[u8],
        limit: usize,
    ) -> Result<Vec<KnowledgeSimilarityHit>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        ensure!(
            fingerprint.len() == HDC_VECTOR_BYTES,
            "knowledge fingerprints must be {HDC_VECTOR_BYTES} bytes, got {}",
            fingerprint.len()
        );

        let entries = self.read_all()?;
        let mut scored = entries
            .into_iter()
            .filter_map(|entry| {
                let similarity = similarity_against_entry(fingerprint, &entry)?;
                Some(KnowledgeSimilarityHit { entry, similarity })
            })
            .collect::<Vec<_>>();

        scored.sort_by(compare_similarity_hits);
        scored.truncate(limit);
        Ok(scored)
    }

    /// Query the store for scored hits relevant to `topic`.
    ///
    /// The current contract is:
    ///
    /// `total_score = keyword_score * effective_confidence * recency_factor * emotional_boost + hdc_similarity`
    ///
    /// where `hdc_similarity` is zero when the `hdc` feature is disabled or
    /// the entry has no valid stored HDC vector.
    ///
    /// # Errors
    ///
    /// Returns an error if the backing file cannot be read.
    pub fn query_hits(&self, topic: &str, limit: usize) -> Result<Vec<KnowledgeQueryHit>> {
        self.query_hits_filtered(topic, limit, |_| true)
    }

    /// Query the store for entries of a specific knowledge kind relevant to
    /// `topic`.
    ///
    /// This is a thin extension over [`KnowledgeStore::query`] used by prompt
    /// assembly to recall only the highest-tier distilled guidance (for
    /// example, StrategyFragment entries) without pulling lower-tier noise into the
    /// prompt.
    ///
    /// # Errors
    ///
    /// Returns an error if the backing file cannot be read.
    pub fn query_kind(
        &self,
        topic: &str,
        kind: KnowledgeKind,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>> {
        Ok(self
            .query_kind_hits(topic, kind, limit)?
            .into_iter()
            .map(|hit| hit.entry)
            .collect())
    }

    /// Query the store for scored hits of a specific kind relevant to `topic`.
    ///
    /// # Errors
    ///
    /// Returns an error if the backing file cannot be read.
    pub fn query_kind_hits(
        &self,
        topic: &str,
        kind: KnowledgeKind,
        limit: usize,
    ) -> Result<Vec<KnowledgeQueryHit>> {
        self.query_hits_filtered(topic, limit, |entry| entry.kind == kind)
    }

    /// Filter all entries by their validation tier.
    ///
    /// # Errors
    ///
    /// Returns an error if the backing file cannot be read.
    pub fn by_tier(&self, tier: KnowledgeTier) -> Result<Vec<KnowledgeEntry>> {
        Ok(self
            .read_all()?
            .into_iter()
            .filter(|entry| entry.tier == tier)
            .collect())
    }

    fn query_hits_filtered<F>(
        &self,
        topic: &str,
        limit: usize,
        mut include: F,
    ) -> Result<Vec<KnowledgeQueryHit>>
    where
        F: FnMut(&KnowledgeEntry) -> bool,
    {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let now = Utc::now();
        let entries = self.read_all()?;
        let topic_terms = tokenize(topic);
        let topic_norm = normalize(topic);

        let mut scored: Vec<KnowledgeQueryHit> = entries
            .into_iter()
            .filter_map(|entry| {
                if !include(&entry) {
                    return None;
                }
                score_entry_for_query(entry, &topic_terms, &topic_norm, topic, now)
            })
            .collect();

        scored.sort_by(compare_hit_scores);
        scored.truncate(limit);
        Ok(scored)
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
        let mut tier_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut source_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut anti_knowledge_count = 0usize;
        let mut confidence_sum = 0.0;
        let mut oldest_entry: Option<&KnowledgeEntry> = None;
        let mut newest_entry: Option<&KnowledgeEntry> = None;

        for entry in &entries {
            *kind_counts
                .entry(knowledge_kind_label(entry.kind).to_owned())
                .or_insert(0) += 1;

            let tier_label = match entry.tier {
                KnowledgeTier::Transient => "transient",
                KnowledgeTier::Working => "working",
                KnowledgeTier::Consolidated => "consolidated",
                KnowledgeTier::Persistent => "persistent",
            };
            *tier_counts.entry(tier_label.to_owned()).or_insert(0) += 1;

            if let Some(source) = entry.source.as_deref() {
                let trimmed = source.trim();
                if !trimmed.is_empty() {
                    *source_counts.entry(trimmed.to_owned()).or_insert(0) += 1;
                }
            }

            if entry.kind == KnowledgeKind::AntiKnowledge {
                anti_knowledge_count += 1;
            }

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
            tier_counts,
            source_counts,
            anti_knowledge_count,
            average_confidence,
            oldest_entry: oldest_entry.cloned(),
            newest_entry: newest_entry.cloned(),
        })
    }

    /// Export the knowledge store to a JSONL file with versioned backup header.
    ///
    /// Entries can be filtered by kind, minimum confidence, tags, and date.
    /// Returns the number of entries exported.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read or the output cannot be
    /// written.
    pub fn export(
        &self,
        output: &Path,
        filter: &ExportFilter,
    ) -> Result<usize> {
        let entries = self.read_all()?;
        let filtered: Vec<_> = entries
            .into_iter()
            .filter(|e| filter.matches(e))
            .collect();
        let count = filtered.len();

        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).context("create export directory")?;
        }

        let header = BackupHeader {
            version: 1,
            created_at: Utc::now(),
            entry_count: count,
            source_path: self.path.display().to_string(),
        };

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(output)
            .with_context(|| format!("create export file at {}", output.display()))?;

        let header_line = serde_json::to_string(&header).context("serialize backup header")?;
        writeln!(file, "{header_line}").context("write backup header")?;

        for entry in &filtered {
            let line = serde_json::to_string(entry).context("serialize knowledge entry")?;
            writeln!(file, "{line}").context("write knowledge entry")?;
        }

        file.flush().context("flush export")?;
        file.sync_all().context("sync export")?;

        Ok(count)
    }

    /// Import knowledge entries from a versioned JSONL backup file.
    ///
    /// Restored entries are reset to [`KnowledgeTier::Transient`] and their
    /// confidence is multiplied by the given discount factor (default 0.85)
    /// per the backup/restore spec. The source label is recorded on each
    /// imported entry.
    ///
    /// Returns the number of entries imported.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, has an unsupported
    /// version, or entries cannot be ingested.
    pub fn import(
        &self,
        input: &Path,
        options: &ImportOptions,
    ) -> Result<usize> {
        let file = File::open(input)
            .with_context(|| format!("open import file at {}", input.display()))?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        // Read and validate the header line.
        let header_line = lines
            .next()
            .ok_or_else(|| anyhow::anyhow!("import file is empty"))?
            .context("read import header")?;

        let header: BackupHeader = serde_json::from_str(&header_line)
            .context("parse backup header (is this a versioned backup file?)")?;

        if header.version > 1 {
            anyhow::bail!(
                "unsupported backup version {} (this build supports version 1)",
                header.version
            );
        }

        let mut entries = Vec::new();
        for line in lines {
            let line = line.context("read import line")?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(mut entry) = serde_json::from_str::<KnowledgeEntry>(&line) {
                if options.reset_tier {
                    entry.tier = KnowledgeTier::Transient;
                }
                entry.confidence *= options.confidence_discount;
                entry.source = Some(options.source_label.clone());
                entries.push(entry);
            }
        }

        let count = entries.len();
        if !entries.is_empty() {
            self.ingest(entries)?;
        }

        Ok(count)
    }

    /// Decay confidence for old entries using their configured half-life.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read or rewritten.
    pub fn decay(&self) -> Result<usize> {
        let _guard = self.write_gate.lock();
        let now = Utc::now();
        let mut entries = self.read_all()?;
        let decayed = entries.len();

        for entry in &mut entries {
            let factor = recency_factor(entry, now);
            let decayed_confidence = (entry.confidence.max(0.0) * factor).clamp(0.0, 1.0);
            entry.confidence = if entry.kind == KnowledgeKind::AntiKnowledge {
                decayed_confidence.max(ANTI_KNOWLEDGE_CONFIDENCE_FLOOR)
            } else {
                decayed_confidence
            };
        }

        self.rewrite_all(&entries)?;
        Ok(decayed)
    }

    /// Garbage-collect entries whose confidence falls below `min_confidence`.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read or rewritten.
    pub fn gc(&self, min_confidence: f64) -> Result<usize> {
        let _guard = self.write_gate.lock();
        let threshold = min_confidence.max(0.0);
        let before = self.read_all()?;
        let before_len = before.len();
        let entries = before
            .into_iter()
            .filter(|entry| {
                entry.kind == KnowledgeKind::AntiKnowledge
                    || effective_confidence(entry) >= threshold
            })
            .collect::<Vec<_>>();
        let removed = before_len.saturating_sub(entries.len());
        self.rewrite_all(&entries)?;
        Ok(removed)
    }

    /// NEURO-08: Garbage-collect entries while preserving the last
    /// representative of each worldview cluster.
    ///
    /// Uses tag-overlap clustering to group related entries. If all entries
    /// in a cluster would be removed, the highest-confidence entry is kept.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read or rewritten.
    pub fn gc_preserving_worldviews(
        &self,
        min_confidence: f64,
        min_tag_overlap: usize,
    ) -> Result<usize> {
        let _guard = self.write_gate.lock();
        let before = self.read_all()?;
        let before_len = before.len();
        let entries =
            crate::gc_with_worldview_preservation(before, min_confidence, min_tag_overlap);
        let removed = before_len.saturating_sub(entries.len());
        self.rewrite_all(&entries)?;
        Ok(removed)
    }

    /// Mutate matching entries in place and rewrite the store atomically.
    ///
    /// Returns the number of entries that changed.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot be read or rewritten.
    pub fn update_entries<F>(&self, mut update: F) -> Result<usize>
    where
        F: FnMut(&mut KnowledgeEntry) -> bool,
    {
        let _guard = self.write_gate.lock();
        let mut entries = self.read_all()?;
        let mut changed = 0usize;
        for entry in &mut entries {
            if update(entry) {
                changed += 1;
            }
        }
        if changed > 0 {
            self.rewrite_all(&entries)?;
        }
        Ok(changed)
    }

    /// Read all knowledge entries from the store.
    ///
    /// # Errors
    ///
    /// Returns an error if the store file cannot be read or any stored entry
    /// cannot be decoded from JSON.
    pub fn read_all(&self) -> Result<Vec<KnowledgeEntry>> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("open knowledge store at {}", self.path.display()));
            }
        };

        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for (line_idx, line) in reader.lines().enumerate() {
            let line = line.with_context(|| {
                format!(
                    "read knowledge line {} from {}",
                    line_idx + 1,
                    self.path.display()
                )
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

    /// Read all confirmation records from the confirmations JSONL file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read.
    pub fn read_confirmations(&self) -> Result<Vec<KnowledgeConfirmationRecord>> {
        let file = match File::open(&self.confirmations_path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "open confirmations file at {}",
                        self.confirmations_path.display()
                    )
                });
            }
        };

        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for line in reader.lines() {
            let line = line.context("read confirmation line")?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(record) = serde_json::from_str::<KnowledgeConfirmationRecord>(&line) {
                records.push(record);
            }
        }
        Ok(records)
    }

    fn append_confirmations(&self, records: &[KnowledgeConfirmationRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        if let Some(parent) = self.confirmations_path.parent() {
            fs::create_dir_all(parent).context("create confirmations directory")?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.confirmations_path)
            .with_context(|| {
                format!(
                    "open confirmations file at {}",
                    self.confirmations_path.display()
                )
            })?;
        for record in records {
            let mut line =
                serde_json::to_string(record).context("serialize confirmation record")?;
            line.push('\n');
            file.write_all(line.as_bytes())
                .context("append confirmation record")?;
        }
        file.flush().context("flush confirmation records")?;
        file.sync_all().context("sync confirmation records")?;
        Ok(())
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

impl NeuroStore for KnowledgeStore {
    fn init(path: &Path) -> Result<Self> {
        Ok(Self::new(path))
    }

    fn query(&self, topic: &str, limit: usize) -> Result<Vec<KnowledgeEntry>> {
        KnowledgeStore::query(self, topic, limit)
    }

    fn query_similar(
        &self,
        fingerprint: &[u8],
        limit: usize,
    ) -> Result<Vec<KnowledgeSimilarityHit>> {
        KnowledgeStore::query_similar(self, fingerprint, limit)
    }

    fn ingest(&mut self, entries: Vec<KnowledgeEntry>) -> Result<()> {
        KnowledgeStore::ingest(self, entries)
    }

    fn decay(&mut self) -> Result<usize> {
        KnowledgeStore::decay(self)
    }

    fn gc(&mut self, min_confidence: f64) -> Result<usize> {
        KnowledgeStore::gc(self, min_confidence)
    }
}

#[cfg(feature = "hdc")]
/// A precomputed HDC index over durable knowledge entries.
///
/// The index fingerprints each entry's content with
/// [`roko_primitives::hdc::HdcVector::from_seed`] and stores the
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

        let query_fingerprint = KnowledgeHdcEncoder.encode_query(query);
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
    if let Some(vector) = entry.hdc_vector.as_deref()
        && let Ok(bytes) = <[u8; HDC_VECTOR_BYTES]>::try_from(vector)
    {
        return HdcVector::from_bytes(&bytes);
    }
    KnowledgeHdcEncoder.encode_entry(entry)
}

#[cfg(feature = "hdc")]
fn prepare_entries_for_ingest(entries: Vec<KnowledgeEntry>) -> Vec<KnowledgeEntry> {
    entries
        .into_iter()
        .map(normalize_entry_for_ingest)
        .collect()
}

#[cfg(not(feature = "hdc"))]
fn prepare_entries_for_ingest(entries: Vec<KnowledgeEntry>) -> Vec<KnowledgeEntry> {
    entries
        .into_iter()
        .map(normalize_entry_for_ingest)
        .collect()
}

#[cfg(feature = "hdc")]
fn ensure_hdc_vector(mut entry: KnowledgeEntry) -> KnowledgeEntry {
    let has_valid_vector = entry
        .hdc_vector
        .as_ref()
        .is_some_and(|vector| vector.len() == HDC_VECTOR_BYTES);
    if !has_valid_vector {
        entry.hdc_vector = Some(fingerprint_entry(&entry).to_bytes().to_vec());
    }
    entry
}

fn normalize_entry_for_ingest(entry: KnowledgeEntry) -> KnowledgeEntry {
    let entry = normalize_entry_tier(entry);
    #[cfg(feature = "hdc")]
    {
        return ensure_hdc_vector(entry);
    }
    #[cfg(not(feature = "hdc"))]
    {
        entry
    }
}

fn normalize_entry_tier(mut entry: KnowledgeEntry) -> KnowledgeEntry {
    let inferred = inferred_retention_tier(&entry);
    if inferred.multiplier() > entry.tier.multiplier() {
        entry.tier = inferred;
    }
    entry
}

fn inferred_retention_tier(entry: &KnowledgeEntry) -> KnowledgeTier {
    let source_count = entry.source_episodes.len();
    let confidence = entry.confidence.clamp(0.0, 1.0);

    match entry.kind {
        KnowledgeKind::StrategyFragment if source_count >= 3 => KnowledgeTier::Persistent,
        KnowledgeKind::StrategyFragment => KnowledgeTier::Working,
        KnowledgeKind::Warning if source_count >= 2 || confidence >= 0.85 => {
            KnowledgeTier::Consolidated
        }
        KnowledgeKind::Warning => KnowledgeTier::Working,
        KnowledgeKind::AntiKnowledge => KnowledgeTier::Working,
        _ if source_count >= 4 || confidence >= 0.9 => KnowledgeTier::Consolidated,
        _ if source_count >= 2 || confidence >= 0.7 => KnowledgeTier::Working,
        _ => KnowledgeTier::Transient,
    }
}

fn compare_similarity_hits(
    left: &KnowledgeSimilarityHit,
    right: &KnowledgeSimilarityHit,
) -> std::cmp::Ordering {
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
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
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
        if tags
            .iter()
            .any(|tag| tag.contains(topic_norm) || topic_norm.contains(tag))
        {
            score += 1.0;
        }
    }

    for term in terms {
        if content.contains(term)
            || tags
                .iter()
                .any(|tag| tag.contains(term) || term.contains(tag))
        {
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
    entry.effective_half_life_days()
}

fn effective_confidence(entry: &KnowledgeEntry) -> f64 {
    bounded_confidence(entry) * confirmation_boost(entry) * entry.emotional_consolidation_boost()
}

fn bounded_confidence(entry: &KnowledgeEntry) -> f64 {
    let confidence = entry.confidence.clamp(0.0, 1.0);
    if entry.kind == KnowledgeKind::AntiKnowledge {
        confidence.max(ANTI_KNOWLEDGE_CONFIDENCE_FLOOR)
    } else {
        confidence
    }
}

fn confirmation_boost(entry: &KnowledgeEntry) -> f64 {
    if entry.source_episodes.len() >= 2 {
        CONFIRMATION_BOOST
    } else {
        1.0
    }
}

fn compare_hit_scores(left: &KnowledgeQueryHit, right: &KnowledgeQueryHit) -> std::cmp::Ordering {
    right
        .total_score
        .partial_cmp(&left.total_score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            right
                .breakdown
                .effective_confidence
                .partial_cmp(&left.breakdown.effective_confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| right.entry.created_at.cmp(&left.entry.created_at))
}

fn emotional_retrieval_boost(entry: &KnowledgeEntry) -> f64 {
    entry.emotional_retrieval_boost()
}

fn knowledge_kind_label(kind: KnowledgeKind) -> &'static str {
    kind.as_str()
}

fn similarity_against_entry(fingerprint: &[u8], entry: &KnowledgeEntry) -> Option<f32> {
    let stored = entry.hdc_vector.as_deref()?;
    if stored.len() != HDC_VECTOR_BYTES {
        return None;
    }

    let differing_bits = fingerprint
        .iter()
        .zip(stored.iter())
        .map(|(left, right)| (left ^ right).count_ones())
        .sum::<u32>();
    Some(1.0 - (differing_bits as f32 / (HDC_VECTOR_BYTES * 8) as f32))
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
    let topic_vec = KnowledgeHdcEncoder.encode_query(topic);
    (topic_vec.similarity(&entry_vec) as f64 - HDC_SIMILARITY_BASELINE).max(0.0)
}

fn score_entry_for_query(
    entry: KnowledgeEntry,
    topic_terms: &[String],
    topic_norm: &str,
    _topic: &str,
    now: DateTime<Utc>,
) -> Option<KnowledgeQueryHit> {
    let keyword = keyword_score(&entry, topic_terms, topic_norm);
    let recency = recency_factor(&entry, now);
    let confidence = effective_confidence(&entry);
    let emotional = emotional_retrieval_boost(&entry);

    #[cfg(feature = "hdc")]
    let hdc = {
        let similarity = hdc_similarity(&entry, _topic);
        (similarity > 0.0).then_some(similarity)
    };
    #[cfg(feature = "hdc")]
    let hdc_contribution = hdc.unwrap_or(0.0);

    #[cfg(not(feature = "hdc"))]
    let hdc: Option<f64> = None;
    #[cfg(not(feature = "hdc"))]
    let hdc_contribution = 0.0;

    let total = keyword * confidence * recency * emotional + hdc_contribution;
    (total > QUERY_SCORE_FLOOR).then_some(KnowledgeQueryHit {
        entry,
        total_score: total,
        breakdown: KnowledgeQueryBreakdown {
            keyword_score: keyword,
            effective_confidence: confidence,
            recency_factor: recency,
            emotional_boost: emotional,
            hdc_similarity: hdc,
        },
    })
}

/// Compare two knowledge entries for topic-level similarity using tag
/// overlap and content keyword matching. This is deliberately lightweight
/// (no ML, no embedding) to keep `ingest()` fast.
fn entries_are_similar(existing: &KnowledgeEntry, new_entry: &KnowledgeEntry) -> bool {
    // Skip AntiKnowledge entries -- they are refutations, not confirmations.
    if existing.kind == KnowledgeKind::AntiKnowledge
        || new_entry.kind == KnowledgeKind::AntiKnowledge
    {
        return false;
    }

    // Tag overlap: normalize and intersect.
    let existing_tags: HashSet<String> = existing.tags.iter().map(|tag| normalize(tag)).collect();
    let new_tags: HashSet<String> = new_entry.tags.iter().map(|tag| normalize(tag)).collect();
    let tag_overlap = existing_tags.intersection(&new_tags).count();

    if tag_overlap < MIN_TAG_OVERLAP {
        return false;
    }

    // Content keyword overlap: tokenize and intersect.
    let existing_keywords: HashSet<String> = tokenize(&existing.content).into_iter().collect();
    let new_keywords: HashSet<String> = tokenize(&new_entry.content).into_iter().collect();
    let keyword_overlap = existing_keywords.intersection(&new_keywords).count();

    keyword_overlap >= MIN_KEYWORD_OVERLAP
}

fn dedupe_entries_for_ingest(
    entries: Vec<KnowledgeEntry>,
    existing: &[KnowledgeEntry],
) -> Vec<KnowledgeEntry> {
    let mut seen_ids = existing
        .iter()
        .filter(|entry| !entry.id.trim().is_empty())
        .map(|entry| entry.id.clone())
        .collect::<HashSet<_>>();

    entries
        .into_iter()
        .filter(|entry| {
            let id = entry.id.trim();
            id.is_empty() || seen_ids.insert(id.to_string())
        })
        .collect()
}

/// Scan new entries against existing entries to find confirmations.
///
/// Returns a list of confirmation records for each (existing, new) pair
/// where the entries are similar enough to indicate independent
/// confirmation of the same insight.
fn detect_confirmations(
    existing: &[KnowledgeEntry],
    new_entries: &[KnowledgeEntry],
) -> Vec<KnowledgeConfirmationRecord> {
    let now = Utc::now();
    let mut confirmations = Vec::new();

    for new_entry in new_entries {
        for existing_entry in existing {
            if existing_entry.id == new_entry.id {
                continue;
            }
            if !entries_are_similar(existing_entry, new_entry) {
                continue;
            }

            // Merge source episodes from both entries.
            let mut source_episodes: Vec<String> = existing_entry
                .source_episodes
                .iter()
                .chain(new_entry.source_episodes.iter())
                .cloned()
                .collect();
            source_episodes.sort();
            source_episodes.dedup();

            confirmations.push(KnowledgeConfirmationRecord {
                created_at: now,
                source_episodes,
                confirmed_entry_id: existing_entry.id.clone(),
                confirming_entry_id: new_entry.id.clone(),
            });
        }
    }

    confirmations
}

/// Check new non-AntiKnowledge entries against existing AntiKnowledge entries
/// using HDC similarity. Returns the filtered/modified list of entries:
/// - similarity > 0.9: entry rejected entirely
/// - similarity > 0.7: entry confidence discounted by 0.5x
/// - similarity > 0.5: warning logged
#[cfg(feature = "hdc")]
fn check_against_anti_knowledge(
    entries: Vec<KnowledgeEntry>,
    existing: &[KnowledgeEntry],
) -> Vec<KnowledgeEntry> {
    let anti_entries: Vec<_> = existing
        .iter()
        .filter(|e| e.kind == KnowledgeKind::AntiKnowledge)
        .collect();

    if anti_entries.is_empty() {
        return entries;
    }

    // Pre-encode all AntiKnowledge entries.
    let encoder = KnowledgeHdcEncoder;
    let anti_vectors: Vec<_> = anti_entries
        .iter()
        .map(|e| (e, fingerprint_entry(e)))
        .collect();

    let mut result = Vec::with_capacity(entries.len());

    for mut entry in entries {
        if entry.kind == KnowledgeKind::AntiKnowledge {
            result.push(entry);
            continue;
        }

        let entry_vec = encoder.encode_entry(&entry);
        let mut worst_similarity = 0.0_f64;
        let mut worst_anti_id = String::new();

        for (anti_entry, anti_vec) in &anti_vectors {
            let sim = entry_vec.similarity(anti_vec) as f64;
            if sim > worst_similarity {
                worst_similarity = sim;
                worst_anti_id = anti_entry.id.clone();
            }
        }

        if worst_similarity > ANTI_KNOWLEDGE_REJECT_THRESHOLD {
            tracing::warn!(
                entry_id = %entry.id,
                anti_knowledge_id = %worst_anti_id,
                similarity = worst_similarity,
                "rejecting entry: near-duplicate of refuted AntiKnowledge"
            );
            continue; // reject
        }

        if worst_similarity > ANTI_KNOWLEDGE_DISCOUNT_THRESHOLD {
            tracing::warn!(
                entry_id = %entry.id,
                anti_knowledge_id = %worst_anti_id,
                similarity = worst_similarity,
                "discounting entry confidence: conflicts with AntiKnowledge"
            );
            entry.confidence *= ANTI_KNOWLEDGE_DISCOUNT_FACTOR;
        } else if worst_similarity > ANTI_KNOWLEDGE_WARN_THRESHOLD {
            tracing::warn!(
                entry_id = %entry.id,
                anti_knowledge_id = %worst_anti_id,
                similarity = worst_similarity,
                "potential conflict with AntiKnowledge"
            );
        }

        result.push(entry);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KnowledgeKind, KnowledgeTier};
    use chrono::Duration;
    use roko_core::PadVector;
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
            source: None,
            content: content.to_owned(),
            confidence,
            confidence_weight: confidence,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: source_episodes
                .iter()
                .map(|source| (*source).to_owned())
                .collect(),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            source_model: None,
            model_generality: 1.0,
            created_at,
            half_life_days: kind.default_half_life_days(),
            tier: KnowledgeTier::Consolidated,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,

            confirmation_count: 0,

            distinct_contexts: Vec::new(),

            deprecated: false,
        }
    }

    #[test]
    fn add_query_and_gc_roundtrip() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(entry(
                KnowledgeKind::Insight,
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
                KnowledgeKind::Insight,
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
                KnowledgeKind::Insight,
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
    fn query_prefers_entries_validated_across_diverse_emotional_states() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        let mut high_diversity = entry(
            KnowledgeKind::Warning,
            "k-diverse",
            "Check rollback health before retrying a failed rollout",
            &["deploy", "rollback"],
            0.8,
            &["ep-a", "ep-b"],
            now,
        );
        high_diversity.emotional_provenance = Some(crate::EmotionalProvenance {
            average_pad: PadVector::new(-0.2, 0.3, 0.0),
            discovery_emotion: "negative_high_arousal".to_string(),
            validation_arc: Some(crate::ValidationArc::Redemptive),
            emotional_diversity: 1.0,
        });

        let mut low_diversity = entry(
            KnowledgeKind::Warning,
            "k-narrow",
            "Check rollback health before retrying a failed rollout",
            &["deploy", "rollback"],
            0.8,
            &["ep-c", "ep-d"],
            now,
        );
        low_diversity.emotional_provenance = Some(crate::EmotionalProvenance {
            average_pad: PadVector::new(-0.2, 0.3, 0.0),
            discovery_emotion: "negative_high_arousal".to_string(),
            validation_arc: Some(crate::ValidationArc::Stable),
            emotional_diversity: 0.0,
        });

        store.add(low_diversity).expect("add narrow");
        store.add(high_diversity).expect("add diverse");

        let results = store
            .query("retry failed rollout rollback health", 2)
            .expect("query");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "k-diverse");
        assert_eq!(results[1].id, "k-narrow");
    }

    #[test]
    fn query_hits_expose_scoring_breakdown() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(entry(
                KnowledgeKind::Insight,
                "k1",
                "Rust async actors and memory stores",
                &["rust", "async"],
                0.9,
                &["ep-a", "ep-b"],
                now,
            ))
            .expect("add first");
        store
            .add(entry(
                KnowledgeKind::Warning,
                "k2",
                "Retry loops can amplify flaky async tests",
                &["testing"],
                0.8,
                &["ep-c"],
                now - Duration::days(10),
            ))
            .expect("add second");

        let hits = store.query_hits("rust async", 5).expect("query hits");
        assert!(!hits.is_empty());
        assert_eq!(hits[0].entry.id, "k1");
        assert!(hits[0].total_score > QUERY_SCORE_FLOOR);
        assert!(hits[0].breakdown.keyword_score >= 2.0);
        assert!(hits[0].breakdown.effective_confidence > hits[0].entry.confidence);
        assert!(hits[0].breakdown.recency_factor > 0.9);
    }

    #[test]
    fn query_kind_hits_filter_by_kind() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(entry(
                KnowledgeKind::Insight,
                "k1",
                "Prefer small async state machines",
                &["async"],
                0.9,
                &["ep-a"],
                now,
            ))
            .expect("add insight");
        store
            .add(entry(
                KnowledgeKind::StrategyFragment,
                "k2",
                "Break async migrations into small compileable steps",
                &["async", "migration"],
                0.95,
                &["ep-b", "ep-c", "ep-d"],
                now,
            ))
            .expect("add strategy fragment");

        let hits = store
            .query_kind_hits("async migration", KnowledgeKind::StrategyFragment, 5)
            .expect("query kind hits");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry.id, "k2");
        assert_eq!(hits[0].entry.kind, KnowledgeKind::StrategyFragment);
    }

    #[test]
    fn query_similar_ranks_by_hamming_similarity() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        let mut exact = entry(
            KnowledgeKind::Insight,
            "k-exact",
            "Exact fingerprint match",
            &["fingerprint"],
            0.9,
            &["ep-a"],
            now,
        );
        exact.hdc_vector = Some(vec![0; HDC_VECTOR_BYTES]);

        let mut close = entry(
            KnowledgeKind::Insight,
            "k-close",
            "Close fingerprint match",
            &["fingerprint"],
            0.8,
            &["ep-b"],
            now,
        );
        let mut close_fp = vec![0; HDC_VECTOR_BYTES];
        close_fp[0] = 0b0000_0011;
        close.hdc_vector = Some(close_fp);

        let mut far = entry(
            KnowledgeKind::Insight,
            "k-far",
            "Far fingerprint match",
            &["fingerprint"],
            0.7,
            &["ep-c"],
            now,
        );
        far.hdc_vector = Some(vec![0xFF; HDC_VECTOR_BYTES]);

        store.ingest(vec![far, close, exact]).expect("ingest");

        let query = vec![0; HDC_VECTOR_BYTES];
        let hits = store.query_similar(&query, 3).expect("query similar");

        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].entry.id, "k-exact");
        assert_eq!(hits[1].entry.id, "k-close");
        assert_eq!(hits[2].entry.id, "k-far");
        assert!(hits[0].similarity > hits[1].similarity);
        assert!(hits[1].similarity > hits[2].similarity);
    }

    #[test]
    fn query_similar_rejects_invalid_fingerprint_length() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));

        let error = store
            .query_similar(&[0_u8; 16], 1)
            .expect_err("invalid fingerprint length should fail");

        assert!(
            error
                .to_string()
                .contains("knowledge fingerprints must be 1280 bytes")
        );
    }

    #[test]
    fn ingest_skips_duplicate_ids() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();
        let duplicate = entry(
            KnowledgeKind::Insight,
            "dup",
            "Keep one durable copy",
            &["dup"],
            0.8,
            &["ep-a"],
            now,
        );

        store
            .ingest(vec![duplicate.clone(), duplicate.clone()])
            .expect("ingest duplicates");
        store.add(duplicate).expect("add duplicate again");

        let all = store.read_all().expect("read all");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "dup");
    }

    #[cfg(feature = "hdc")]
    #[test]
    fn hdc_only_unrelated_entries_do_not_clear_query_floor() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(entry(
                KnowledgeKind::Insight,
                "k1",
                "Completely unrelated note about shell prompts",
                &["misc"],
                0.0,
                &["ep-a"],
                now,
            ))
            .expect("add unrelated");

        let hits = store
            .query_hits("database migrations", 5)
            .expect("query hits");
        assert!(hits.is_empty());
    }

    #[test]
    fn query_prefers_emotionally_reinforced_entries() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        let mut neutral = entry(
            KnowledgeKind::Warning,
            "k-neutral",
            "Prefer the rollback path when rollout validation fails",
            &["deploy", "rollback"],
            0.8,
            &["ep-a"],
            now,
        );
        neutral.emotional_provenance = Some(crate::EmotionalProvenance {
            average_pad: PadVector::new(-0.1, 0.2, 0.0),
            discovery_emotion: "neutral_mid_arousal".to_string(),
            validation_arc: Some(crate::ValidationArc::Stable),
            emotional_diversity: 0.0,
        });

        let mut reinforced = entry(
            KnowledgeKind::Warning,
            "k-reinforced",
            "Prefer the rollback path when rollout validation fails",
            &["deploy", "rollback"],
            0.8,
            &["ep-b"],
            now,
        );
        reinforced.emotional_tag = Some(roko_core::EmotionalTag::new(
            PadVector::new(-0.8, 0.4, 0.0),
            0.95,
            "rollback_failure",
            PadVector::new(-0.7, 0.3, 0.0),
        ));
        reinforced.emotional_provenance = Some(crate::EmotionalProvenance {
            average_pad: PadVector::new(-0.8, 0.4, 0.0),
            discovery_emotion: "negative_high_arousal".to_string(),
            validation_arc: Some(crate::ValidationArc::Redemptive),
            emotional_diversity: 1.0,
        });

        store.add(neutral).expect("add neutral");
        store.add(reinforced).expect("add reinforced");

        let results = store
            .query("rollback rollout validation failure", 2)
            .expect("query");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "k-reinforced");
    }

    #[test]
    fn decay_preserves_antiknowledge_confidence_floor() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let created_at = Utc::now() - Duration::days(365);

        store
            .add(KnowledgeEntry {
                id: "anti-floor".to_owned(),
                kind: KnowledgeKind::AntiKnowledge,
                source: None,
                content: "This previously successful pattern regressed badly.".to_owned(),
                confidence: 0.8,
                confidence_weight: -0.8,
                refuted_insight_id: Some("insight-1".to_owned()),
                refutation_evidence: Some("repeated gate failures".to_owned()),
                source_episodes: vec!["ep-a".to_owned()],
                tags: vec!["anti_knowledge".to_owned(), "regression".to_owned()],
                source_model: None,
                model_generality: 1.0,
                created_at,
                half_life_days: KnowledgeKind::AntiKnowledge.default_half_life_days(),
                tier: KnowledgeTier::Working,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
            })
            .expect("add anti knowledge");

        store.decay().expect("decay");
        let all = store.read_all().expect("read");
        assert_eq!(all.len(), 1);
        assert!((all[0].confidence - ANTI_KNOWLEDGE_CONFIDENCE_FLOOR).abs() < f64::EPSILON);
    }

    #[test]
    fn decay_uses_kind_specific_half_lives() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let created_at = Utc::now() - Duration::days(30);

        store
            .add(entry(
                KnowledgeKind::StrategyFragment,
                "strategy",
                "Reusable long-lived strategy fragment",
                &["strategy_fragment"],
                1.0,
                &[],
                created_at,
            ))
            .expect("add strategy fragment");
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
        let strategy = all
            .iter()
            .find(|entry| entry.id == "strategy")
            .expect("strategy");
        let insight = all
            .iter()
            .find(|entry| entry.id == "insight")
            .expect("insight");
        let heuristic = all
            .iter()
            .find(|entry| entry.id == "heuristic")
            .expect("heuristic");

        assert!(heuristic.confidence > insight.confidence);
        assert!(insight.confidence > strategy.confidence);
        assert!((insight.confidence - 0.5).abs() < 0.05);
        assert!((strategy.confidence - 0.22).abs() < 0.05);
        assert!((heuristic.confidence - 0.79).abs() < 0.05);
    }

    #[test]
    fn decay_drops_below_half_after_two_half_lives() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let created_at = Utc::now() - Duration::days(60);

        store
            .add(entry(
                KnowledgeKind::Insight,
                "old-insight",
                "A stale but valid insight",
                &["insight"],
                1.0,
                &["ep-a", "ep-b"],
                created_at,
            ))
            .expect("add");

        store.decay().expect("decay");
        let all = store.read_all().expect("read");
        let confidence = all
            .iter()
            .find(|entry| entry.id == "old-insight")
            .expect("old insight")
            .confidence;
        assert!(confidence < 0.5);
        assert!((confidence - 0.25).abs() < 0.05);
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
    fn gc_preserves_antiknowledge_even_below_threshold() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(KnowledgeEntry {
                id: "anti-gc".to_owned(),
                kind: KnowledgeKind::AntiKnowledge,
                source: None,
                content: "This optimization path is deceptively harmful.".to_owned(),
                confidence: 0.01,
                confidence_weight: -0.4,
                refuted_insight_id: Some("insight-2".to_owned()),
                refutation_evidence: Some("caused repeated failures".to_owned()),
                source_episodes: vec!["ep-a".to_owned()],
                tags: vec!["anti_knowledge".to_owned(), "optimization".to_owned()],
                source_model: None,
                model_generality: 1.0,
                created_at: now,
                half_life_days: KnowledgeKind::AntiKnowledge.default_half_life_days(),
                tier: KnowledgeTier::Working,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
            })
            .expect("add anti knowledge");

        store.gc(0.95).expect("gc");
        let all = store.read_all().expect("read");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "anti-gc");
        assert!(
            (effective_confidence(&all[0]) - ANTI_KNOWLEDGE_CONFIDENCE_FLOOR).abs() < f64::EPSILON
        );
    }

    #[test]
    fn antiknowledge_halves_refuted_entry_confidence() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(entry(
                KnowledgeKind::Insight,
                "insight-1",
                "A reusable insight",
                &["insight"],
                1.0,
                &["ep-a"],
                now,
            ))
            .expect("add original");
        store
            .add(KnowledgeEntry {
                id: "anti-1".to_owned(),
                kind: KnowledgeKind::AntiKnowledge,
                source: None,
                content: "Previous insight insight-1 was wrong because it failed in practice."
                    .to_owned(),
                confidence: 0.9,
                confidence_weight: -0.9,
                refuted_insight_id: Some("insight-1".to_owned()),
                refutation_evidence: Some("it failed in practice".to_owned()),
                source_episodes: vec!["ep-b".to_owned()],
                tags: vec!["anti_knowledge".to_owned(), "insight".to_owned()],
                source_model: None,
                model_generality: 1.0,
                created_at: now,
                half_life_days: KnowledgeKind::AntiKnowledge.default_half_life_days(),
                tier: KnowledgeTier::Working,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
            })
            .expect("add anti knowledge");

        let all = store.read_all().expect("read");
        let original = all
            .iter()
            .find(|entry| entry.id == "insight-1")
            .expect("original");
        let anti = all.iter().find(|entry| entry.id == "anti-1").expect("anti");

        assert!((original.confidence - 0.5).abs() < f64::EPSILON);
        assert_eq!(anti.kind, KnowledgeKind::AntiKnowledge);
    }

    #[test]
    fn stats_aggregate_by_kind_and_age() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(KnowledgeEntry {
                id: "oldest".to_owned(),
                kind: KnowledgeKind::Insight,
                source: None,
                content: "first".to_owned(),
                confidence: 0.8,
                confidence_weight: 0.8,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: Vec::new(),
                tags: Vec::new(),
                source_model: None,
                model_generality: 1.0,
                created_at: now - Duration::days(3),
                half_life_days: KnowledgeKind::Insight.default_half_life_days(),
                tier: KnowledgeTier::Consolidated,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
            })
            .expect("add oldest");
        store
            .add(KnowledgeEntry {
                id: "middle".to_owned(),
                kind: KnowledgeKind::StrategyFragment,
                source: None,
                content: "second".to_owned(),
                confidence: 0.6,
                confidence_weight: 0.6,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: Vec::new(),
                tags: Vec::new(),
                source_model: None,
                model_generality: 1.0,
                created_at: now - Duration::days(1),
                half_life_days: KnowledgeKind::StrategyFragment.default_half_life_days(),
                tier: KnowledgeTier::Consolidated,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
            })
            .expect("add middle");
        store
            .add(KnowledgeEntry {
                id: "newest".to_owned(),
                kind: KnowledgeKind::Insight,
                source: None,
                content: "third".to_owned(),
                confidence: 1.0,
                confidence_weight: 1.0,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: Vec::new(),
                tags: Vec::new(),
                source_model: None,
                model_generality: 1.0,
                created_at: now,
                half_life_days: KnowledgeKind::Insight.default_half_life_days(),
                tier: KnowledgeTier::Consolidated,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
            })
            .expect("add newest");

        let stats = store.stats().expect("stats");
        assert_eq!(stats.total_entries, 3);
        assert_eq!(stats.kind_counts.get("insight"), Some(&2));
        assert_eq!(stats.kind_counts.get("strategy_fragment"), Some(&1));
        assert!((stats.average_confidence.expect("average") - 0.8).abs() < f64::EPSILON);
        assert_eq!(
            stats.oldest_entry.as_ref().map(|entry| entry.id.as_str()),
            Some("oldest")
        );
        assert_eq!(
            stats.newest_entry.as_ref().map(|entry| entry.id.as_str()),
            Some("newest")
        );
    }

    #[cfg(feature = "hdc")]
    #[test]
    fn memory_index_search_prefers_matching_content() {
        let now = Utc::now();
        let index = MemoryIndex::from_entries(vec![
            entry(
                KnowledgeKind::Insight,
                "k1",
                "rust async memory retrieval",
                &["rust", "memory"],
                1.0,
                &["ep-a"],
                now,
            ),
            entry(
                KnowledgeKind::Insight,
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
                KnowledgeKind::Insight,
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
                KnowledgeKind::Insight,
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

    #[cfg(feature = "hdc")]
    #[test]
    fn causal_links_match_queries_by_cause_and_effect() {
        let now = Utc::now();
        let index = MemoryIndex::from_entries(vec![
            entry(
                KnowledgeKind::CausalLink,
                "k1",
                "high complexity -> more review",
                &[
                    "cause:high complexity",
                    "effect:more review",
                    "domain:coding",
                ],
                0.9,
                &["ep-a"],
                now,
            ),
            entry(
                KnowledgeKind::Insight,
                "k2",
                "postgres vacuum keeps tables healthy",
                &["postgres"],
                0.9,
                &["ep-b"],
                now,
            ),
        ]);

        let cause_hits = index.search("high complexity", 1);
        assert_eq!(cause_hits.len(), 1);
        assert_eq!(cause_hits[0].entry.id, "k1");

        let effect_hits = index.search("more review", 1);
        assert_eq!(effect_hits.len(), 1);
        assert_eq!(effect_hits[0].entry.id, "k1");
    }

    #[cfg(feature = "hdc")]
    #[test]
    fn ingest_populates_hdc_vector_when_feature_is_enabled() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(entry(
                KnowledgeKind::Insight,
                "k1",
                "semantic retrieval over durable knowledge",
                &["memory"],
                1.0,
                &["ep-a"],
                now,
            ))
            .expect("add entry");

        let all = store.read_all().expect("read");
        let vector = all[0].hdc_vector.as_ref().expect("persisted hdc vector");
        assert_eq!(vector.len(), HDC_VECTOR_BYTES);
    }

    // ── Confirmation detection tests ─────────────────────────────────

    #[test]
    fn entries_are_similar_detects_tag_and_keyword_overlap() {
        let now = Utc::now();
        let existing = entry(
            KnowledgeKind::Insight,
            "k1",
            "Rust async actors are useful for concurrent pipelines",
            &["rust", "async", "concurrency"],
            1.0,
            &["ep-a"],
            now,
        );
        let similar = entry(
            KnowledgeKind::Insight,
            "k2",
            "Rust async runtime handles concurrent execution well",
            &["rust", "async"],
            0.9,
            &["ep-b"],
            now,
        );
        let unrelated = entry(
            KnowledgeKind::Insight,
            "k3",
            "PostgreSQL requires VACUUM for dead tuple cleanup",
            &["postgres", "maintenance"],
            0.8,
            &["ep-c"],
            now,
        );

        assert!(entries_are_similar(&existing, &similar));
        assert!(!entries_are_similar(&existing, &unrelated));
    }

    #[test]
    fn entries_are_similar_requires_minimum_keyword_overlap() {
        let now = Utc::now();
        let existing = entry(
            KnowledgeKind::Insight,
            "k1",
            "Rust async actors are useful",
            &["rust"],
            1.0,
            &["ep-a"],
            now,
        );
        // Shares the tag "rust" but only one keyword overlap ("rust").
        let one_keyword = entry(
            KnowledgeKind::Insight,
            "k2",
            "Rust borrow checker prevents data races",
            &["rust"],
            0.9,
            &["ep-b"],
            now,
        );

        // Meets MIN_TAG_OVERLAP but not MIN_KEYWORD_OVERLAP.
        assert!(!entries_are_similar(&existing, &one_keyword));
    }

    #[test]
    fn entries_are_similar_skips_antiknowledge() {
        let now = Utc::now();
        let existing = entry(
            KnowledgeKind::Insight,
            "k1",
            "Rust async actors are useful for concurrent pipelines",
            &["rust", "async"],
            1.0,
            &["ep-a"],
            now,
        );
        let anti = KnowledgeEntry {
            id: "anti-1".to_owned(),
            kind: KnowledgeKind::AntiKnowledge,
            source: None,
            content: "Rust async actors are not suitable for all concurrent pipelines".to_owned(),
            confidence: 0.9,
            confidence_weight: -0.9,
            refuted_insight_id: Some("k1".to_owned()),
            refutation_evidence: Some("test".to_owned()),
            source_episodes: vec!["ep-b".to_owned()],
            tags: vec!["rust".to_owned(), "async".to_owned()],
            source_model: None,
            model_generality: 1.0,
            created_at: now,
            half_life_days: KnowledgeKind::AntiKnowledge.default_half_life_days(),
            tier: KnowledgeTier::Working,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,

            confirmation_count: 0,

            distinct_contexts: Vec::new(),

            deprecated: false,
        };

        assert!(!entries_are_similar(&existing, &anti));
    }

    #[test]
    fn detect_confirmations_finds_similar_entries() {
        let now = Utc::now();
        let existing = vec![entry(
            KnowledgeKind::Insight,
            "k1",
            "Rust async actors are useful for concurrent pipelines",
            &["rust", "async"],
            1.0,
            &["ep-a"],
            now,
        )];
        let new_entries = vec![entry(
            KnowledgeKind::Insight,
            "k2",
            "Rust async runtime handles concurrent execution well",
            &["rust", "async"],
            0.9,
            &["ep-b"],
            now,
        )];

        let confirmations = detect_confirmations(&existing, &new_entries);
        assert_eq!(confirmations.len(), 1);
        assert_eq!(confirmations[0].confirmed_entry_id, "k1");
        assert_eq!(confirmations[0].confirming_entry_id, "k2");
        assert!(
            confirmations[0]
                .source_episodes
                .contains(&"ep-a".to_owned())
        );
        assert!(
            confirmations[0]
                .source_episodes
                .contains(&"ep-b".to_owned())
        );
    }

    #[test]
    fn detect_confirmations_skips_unrelated_entries() {
        let now = Utc::now();
        let existing = vec![entry(
            KnowledgeKind::Insight,
            "k1",
            "Rust async actors are useful for concurrent pipelines",
            &["rust", "async"],
            1.0,
            &["ep-a"],
            now,
        )];
        let new_entries = vec![entry(
            KnowledgeKind::Insight,
            "k3",
            "PostgreSQL requires VACUUM for dead tuple cleanup",
            &["postgres", "maintenance"],
            0.8,
            &["ep-c"],
            now,
        )];

        let confirmations = detect_confirmations(&existing, &new_entries);
        assert!(confirmations.is_empty());
    }

    #[test]
    fn ingest_writes_confirmation_records_for_similar_entries() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        // Add first entry.
        store
            .add(entry(
                KnowledgeKind::Insight,
                "k1",
                "Rust async actors are useful for concurrent pipelines",
                &["rust", "async"],
                1.0,
                &["ep-a"],
                now,
            ))
            .expect("add first");

        // No confirmations after first entry.
        let records = store.read_confirmations().expect("read confirmations");
        assert!(records.is_empty());

        // Add a similar entry.
        store
            .add(entry(
                KnowledgeKind::Insight,
                "k2",
                "Rust async runtime handles concurrent execution well",
                &["rust", "async"],
                0.9,
                &["ep-b"],
                now,
            ))
            .expect("add similar");

        // Now there should be a confirmation record.
        let records = store.read_confirmations().expect("read confirmations");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].confirmed_entry_id, "k1");
        assert_eq!(records[0].confirming_entry_id, "k2");
        assert!(records[0].source_episodes.contains(&"ep-a".to_owned()));
        assert!(records[0].source_episodes.contains(&"ep-b".to_owned()));
    }

    #[test]
    fn ingest_does_not_write_confirmations_for_unrelated_entries() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(entry(
                KnowledgeKind::Insight,
                "k1",
                "Rust async actors are useful for concurrent pipelines",
                &["rust", "async"],
                1.0,
                &["ep-a"],
                now,
            ))
            .expect("add first");

        store
            .add(entry(
                KnowledgeKind::Insight,
                "k3",
                "PostgreSQL requires VACUUM for dead tuple cleanup",
                &["postgres", "maintenance"],
                0.8,
                &["ep-c"],
                now,
            ))
            .expect("add unrelated");

        let records = store.read_confirmations().expect("read confirmations");
        assert!(records.is_empty());
    }

    #[test]
    fn confirmations_path_is_sibling_of_knowledge_path() {
        let store = KnowledgeStore::new("/some/path/neuro/knowledge.jsonl");
        assert_eq!(
            store.confirmations_path(),
            Path::new("/some/path/neuro/knowledge-confirmations.jsonl")
        );
    }

    #[test]
    fn ingest_promotes_high_support_entries_to_longer_tiers() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(KnowledgeEntry {
                id: "tiered".to_owned(),
                kind: KnowledgeKind::Insight,
                source: None,
                content: "Repeatedly validated insight".to_owned(),
                confidence: 0.92,
                confidence_weight: 0.92,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-a".to_owned(), "ep-b".to_owned(), "ep-c".to_owned()],
                tags: vec!["tier".to_owned()],
                source_model: None,
                model_generality: 1.0,
                created_at: now,
                half_life_days: KnowledgeKind::Insight.default_half_life_days(),
                tier: KnowledgeTier::Transient,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
            })
            .expect("add tiered");

        let all = store.read_all().expect("read");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].tier, KnowledgeTier::Consolidated);
    }

    #[test]
    fn ingest_keeps_stronger_explicit_tiers() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(KnowledgeEntry {
                id: "persistent".to_owned(),
                kind: KnowledgeKind::StrategyFragment,
                source: None,
                content: "A durable playbook fragment".to_owned(),
                confidence: 0.6,
                confidence_weight: 0.6,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-a".to_owned()],
                tags: vec!["strategy".to_owned()],
                source_model: None,
                model_generality: 1.0,
                created_at: now,
                half_life_days: KnowledgeKind::StrategyFragment.default_half_life_days(),
                tier: KnowledgeTier::Persistent,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
            })
            .expect("add persistent");

        let all = store.read_all().expect("read");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].tier, KnowledgeTier::Persistent);
    }

    #[test]
    fn stats_includes_tier_and_source_counts() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        // Use low confidence so normalize_entry_tier does not auto-promote.
        let mut e1 = entry(
            KnowledgeKind::Insight,
            "k1",
            "something useful",
            &["rust"],
            0.5,
            &["ep-a"],
            now,
        );
        e1.tier = KnowledgeTier::Working;
        e1.source = Some("local".to_owned());

        let mut e2 = entry(
            KnowledgeKind::AntiKnowledge,
            "k2",
            "do not retry on 5xx",
            &["http"],
            0.5,
            &["ep-b"],
            now,
        );
        e2.tier = KnowledgeTier::Working;

        store.add(e1).expect("add");
        store.add(e2).expect("add anti");

        let stats = store.stats().expect("stats");
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.anti_knowledge_count, 1);
        assert_eq!(stats.tier_counts.get("working"), Some(&2));
        assert_eq!(stats.source_counts.get("local"), Some(&1));
    }

    #[test]
    fn export_import_roundtrip_with_confidence_discount() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        let mut e = entry(
            KnowledgeKind::Insight,
            "k1",
            "important heuristic",
            &["rust"],
            0.5,
            &["ep-a"],
            now,
        );
        e.tier = KnowledgeTier::Consolidated;
        store.add(e).expect("add");

        // Export.
        let backup_path = tmp.path().join("backup.jsonl");
        let filter = ExportFilter::default();
        let count = store.export(&backup_path, &filter).expect("export");
        assert_eq!(count, 1);
        assert!(backup_path.exists());

        // Import into a fresh store.
        let store2 = KnowledgeStore::new(tmp.path().join("neuro2").join("knowledge.jsonl"));
        let options = ImportOptions {
            confidence_discount: 0.85,
            reset_tier: true,
            source_label: "backup-test".to_owned(),
        };
        let imported = store2.import(&backup_path, &options).expect("import");
        assert_eq!(imported, 1);

        let all = store2.read_all().expect("read");
        assert_eq!(all.len(), 1);
        // Confidence should be discounted: 0.5 * 0.85 = 0.425.
        assert!((all[0].confidence - 0.425).abs() < 0.01);
        // Tier should be reset to Transient (low confidence won't trigger promotion).
        assert_eq!(all[0].tier, KnowledgeTier::Transient);
        // Source label should be recorded.
        assert_eq!(all[0].source.as_deref(), Some("backup-test"));
    }

    #[test]
    fn export_filter_by_kind_and_confidence() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));
        let now = Utc::now();

        store
            .add(entry(
                KnowledgeKind::Insight,
                "k1",
                "high confidence insight",
                &["rust"],
                0.9,
                &["ep-a"],
                now,
            ))
            .expect("add");
        store
            .add(entry(
                KnowledgeKind::Warning,
                "k2",
                "low confidence warning",
                &["rust"],
                0.2,
                &["ep-b"],
                now,
            ))
            .expect("add");

        let backup_path = tmp.path().join("filtered.jsonl");
        let filter = ExportFilter {
            kinds: Some(vec![KnowledgeKind::Insight]),
            min_confidence: Some(0.5),
            ..Default::default()
        };
        let count = store.export(&backup_path, &filter).expect("export");
        assert_eq!(count, 1);
    }

    #[test]
    fn import_rejects_unsupported_version() {
        let tmp = TempDir::new().expect("tempdir");
        let store = KnowledgeStore::new(tmp.path().join("neuro").join("knowledge.jsonl"));

        let bad_backup = tmp.path().join("bad.jsonl");
        let header = BackupHeader {
            version: 99,
            created_at: Utc::now(),
            entry_count: 0,
            source_path: "test".to_owned(),
        };
        std::fs::write(
            &bad_backup,
            serde_json::to_string(&header).unwrap() + "\n",
        )
        .unwrap();

        let result = store.import(&bad_backup, &ImportOptions::default());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unsupported backup version")
        );
    }

    #[cfg(feature = "hdc")]
    #[test]
    fn anti_knowledge_check_rejects_near_duplicate() {
        // When a new entry has very high HDC similarity to an existing
        // AntiKnowledge entry, it should be filtered out.
        let anti = KnowledgeEntry {
            id: "anti-1".to_owned(),
            kind: KnowledgeKind::AntiKnowledge,
            source: None,
            content: "Never retry failed HTTP 5xx requests without backoff".to_owned(),
            confidence: 0.9,
            confidence_weight: 1.0,
            refuted_insight_id: Some("old-insight".to_owned()),
            refutation_evidence: Some("caused cascading failures".to_owned()),
            source_episodes: vec!["ep-a".to_owned()],
            tags: vec!["http".to_owned(), "retry".to_owned()],
            source_model: None,
            model_generality: 1.0,
            created_at: Utc::now(),
            half_life_days: KnowledgeKind::AntiKnowledge.default_half_life_days(),
            tier: KnowledgeTier::Working,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
        };

        // A near-identical entry that should be rejected.
        let duplicate = KnowledgeEntry {
            id: "new-1".to_owned(),
            kind: KnowledgeKind::Insight,
            source: None,
            content: "Never retry failed HTTP 5xx requests without backoff".to_owned(),
            confidence: 0.8,
            confidence_weight: 1.0,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: vec!["ep-b".to_owned()],
            tags: vec!["http".to_owned(), "retry".to_owned()],
            source_model: None,
            model_generality: 1.0,
            created_at: Utc::now(),
            half_life_days: KnowledgeKind::Insight.default_half_life_days(),
            tier: KnowledgeTier::Transient,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
        };

        // An unrelated entry that should pass through.
        let unrelated = KnowledgeEntry {
            id: "new-2".to_owned(),
            kind: KnowledgeKind::Insight,
            source: None,
            content: "PostgreSQL requires regular VACUUM for performance".to_owned(),
            confidence: 0.9,
            confidence_weight: 1.0,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: vec!["ep-c".to_owned()],
            tags: vec!["postgres".to_owned(), "maintenance".to_owned()],
            source_model: None,
            model_generality: 1.0,
            created_at: Utc::now(),
            half_life_days: KnowledgeKind::Insight.default_half_life_days(),
            tier: KnowledgeTier::Transient,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
        };

        let existing = vec![anti];
        let new_entries = prepare_entries_for_ingest(vec![duplicate, unrelated]);

        let result = check_against_anti_knowledge(new_entries, &existing);
        // The near-duplicate should be rejected, leaving only the unrelated entry.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "new-2");
    }

    #[cfg(feature = "hdc")]
    #[test]
    fn anti_knowledge_check_passes_antiknowledge_entries_through() {
        // AntiKnowledge entries themselves should not be blocked by existing
        // AntiKnowledge.
        let existing_anti = KnowledgeEntry {
            id: "anti-1".to_owned(),
            kind: KnowledgeKind::AntiKnowledge,
            source: None,
            content: "Never retry failed HTTP 5xx requests without backoff".to_owned(),
            confidence: 0.9,
            confidence_weight: 1.0,
            refuted_insight_id: Some("old".to_owned()),
            refutation_evidence: None,
            source_episodes: vec!["ep-a".to_owned()],
            tags: vec!["http".to_owned()],
            source_model: None,
            model_generality: 1.0,
            created_at: Utc::now(),
            half_life_days: KnowledgeKind::AntiKnowledge.default_half_life_days(),
            tier: KnowledgeTier::Working,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
        };

        let new_anti = KnowledgeEntry {
            id: "anti-2".to_owned(),
            kind: KnowledgeKind::AntiKnowledge,
            source: None,
            content: "Never retry failed HTTP 5xx requests without backoff -- updated".to_owned(),
            confidence: 0.95,
            confidence_weight: 1.0,
            refuted_insight_id: Some("other".to_owned()),
            refutation_evidence: None,
            source_episodes: vec!["ep-b".to_owned()],
            tags: vec!["http".to_owned()],
            source_model: None,
            model_generality: 1.0,
            created_at: Utc::now(),
            half_life_days: KnowledgeKind::AntiKnowledge.default_half_life_days(),
            tier: KnowledgeTier::Working,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
        };

        let existing = vec![existing_anti];
        let new_entries = prepare_entries_for_ingest(vec![new_anti]);
        let result = check_against_anti_knowledge(new_entries, &existing);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "anti-2");
    }
}
