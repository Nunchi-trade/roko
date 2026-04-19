//! Confidence-ladder staging buffer for dream outputs.
//!
//! Dream-generated insights pass through a staging buffer before entering the
//! main [`KnowledgeStore`]. Each entry starts at `Raw` and progresses through
//! validation stages. Entries that do not promote within 7 days are garbage
//! collected, preventing dream hallucinations from corrupting durable knowledge.

use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use roko_neuro::{KnowledgeEntry, KnowledgeStore, KnowledgeTier};
use roko_primitives::hdc::HdcVector;
use serde::{Deserialize, Serialize};

/// Confidence thresholds for each stage.
const CONFIDENCE_RAW: f64 = 0.20;
const CONFIDENCE_REPLAYED: f64 = 0.30;
const CONFIDENCE_VALIDATED: f64 = 0.50;
const CONFIDENCE_PROMOTED: f64 = 0.70;

/// Default GC horizon: entries that haven't promoted past `Raw` in 7 days.
const GC_HORIZON_DAYS: i64 = 7;

/// HDC similarity threshold above which an entry is considered redundant.
const REDUNDANCY_THRESHOLD: f32 = 0.90;

/// Confidence ladder for dream candidate staging.
///
/// Candidates start at `Raw` and progress through validation stages
/// before being promoted to the knowledge store.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceStage {
    /// Just extracted, unvalidated.
    Raw,
    /// Successfully replayed in a subsequent dream cycle.
    Replayed,
    /// Cross-checked against existing knowledge (no contradiction, not redundant).
    Validated,
    /// Ready for knowledge store promotion.
    Promoted,
}

impl ConfidenceStage {
    /// The minimum confidence score for this stage.
    #[must_use]
    pub fn confidence_floor(&self) -> f64 {
        match self {
            Self::Raw => CONFIDENCE_RAW,
            Self::Replayed => CONFIDENCE_REPLAYED,
            Self::Validated => CONFIDENCE_VALIDATED,
            Self::Promoted => CONFIDENCE_PROMOTED,
        }
    }

    /// Return the next stage, or `None` if already at `Promoted`.
    #[must_use]
    pub fn next(&self) -> Option<Self> {
        match self {
            Self::Raw => Some(Self::Replayed),
            Self::Replayed => Some(Self::Validated),
            Self::Validated => Some(Self::Promoted),
            Self::Promoted => None,
        }
    }
}

/// A single entry in the staging buffer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StagingEntry {
    /// The knowledge entry being staged.
    pub entry: KnowledgeEntry,
    /// Source episode that produced this insight.
    pub source_episode_id: String,
    /// Current position on the confidence ladder.
    pub stage: ConfidenceStage,
    /// Current confidence score (starts at 0.20).
    pub confidence: f64,
    /// When this entry was first added.
    pub created_at: DateTime<Utc>,
    /// When this entry last advanced a stage.
    pub last_advanced_at: DateTime<Utc>,
    /// When this entry was promoted to the knowledge store, if ever.
    pub promoted_at: Option<DateTime<Utc>>,
}

/// In-memory staging buffer with optional file persistence.
///
/// Serializable so it can be saved/loaded across dream cycles.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StagingBuffer {
    entries: Vec<StagingEntry>,
}

impl StagingBuffer {
    /// Create an empty staging buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Load staging buffer from a JSON file, or return empty if missing/invalid.
    #[must_use]
    pub fn load_or_new(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|json| serde_json::from_str::<Self>(&json).ok())
            .unwrap_or_default()
    }

    /// Persist the staging buffer to a JSON file.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::File::create(path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, self)?;
        writer.flush()?;
        Ok(())
    }

    /// Add a new candidate at the `Raw` stage with confidence 0.20.
    pub fn add_candidate(&mut self, entry: KnowledgeEntry, source_episode_id: String) {
        let now = Utc::now();
        self.entries.push(StagingEntry {
            entry,
            source_episode_id,
            stage: ConfidenceStage::Raw,
            confidence: CONFIDENCE_RAW,
            created_at: now,
            last_advanced_at: now,
            promoted_at: None,
        });
    }

    /// Advance an entry to the next stage if validation passes.
    ///
    /// Returns `true` if the entry was advanced.
    pub fn advance_stage(&mut self, index: usize) -> bool {
        let Some(entry) = self.entries.get_mut(index) else {
            return false;
        };
        let Some(next_stage) = entry.stage.next() else {
            return false;
        };
        entry.stage = next_stage.clone();
        entry.confidence = next_stage.confidence_floor();
        entry.last_advanced_at = Utc::now();
        true
    }

    /// Return indices and references to all entries at a given stage.
    #[must_use]
    pub fn candidates_at_stage(&self, stage: &ConfidenceStage) -> Vec<(usize, &StagingEntry)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| &e.stage == stage)
            .collect()
    }

    /// All entries in the buffer.
    #[must_use]
    pub fn entries(&self) -> &[StagingEntry] {
        &self.entries
    }

    /// Number of entries in the buffer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Promote all `Validated` entries to the knowledge store at `Transient` tier.
    ///
    /// Returns the promoted entries.
    pub fn promote_validated(
        &mut self,
        store: &KnowledgeStore,
    ) -> anyhow::Result<Vec<KnowledgeEntry>> {
        let now = Utc::now();
        let mut promoted = Vec::new();

        for entry in &mut self.entries {
            if entry.stage != ConfidenceStage::Validated {
                continue;
            }
            // Final promotion: advance to Promoted and write to store.
            entry.stage = ConfidenceStage::Promoted;
            entry.confidence = CONFIDENCE_PROMOTED;
            entry.promoted_at = Some(now);
            entry.last_advanced_at = now;

            let mut knowledge = entry.entry.clone();
            knowledge.tier = KnowledgeTier::Transient;
            store.add(knowledge.clone())?;
            promoted.push(knowledge);
        }

        Ok(promoted)
    }

    /// Try to advance entries from `Raw` to `Replayed`.
    ///
    /// An entry can be replayed if it survived one dream cycle without
    /// contradiction. `replayed_ids` is the set of source episode IDs
    /// from the current replay batch.
    pub fn advance_replayed(&mut self, replayed_ids: &[String]) {
        for entry in &mut self.entries {
            if entry.stage != ConfidenceStage::Raw {
                continue;
            }
            if replayed_ids.contains(&entry.source_episode_id) {
                entry.stage = ConfidenceStage::Replayed;
                entry.confidence = CONFIDENCE_REPLAYED;
                entry.last_advanced_at = Utc::now();
            }
        }
    }

    /// Try to advance entries from `Replayed` to `Validated`.
    ///
    /// Validation checks:
    /// 1. Not redundant (HDC similarity < 0.90 against all existing store entries)
    /// 2. Not contradicted (placeholder: always passes for now)
    ///
    /// `existing_entries` is the current contents of the knowledge store,
    /// passed in so the caller controls when to read it.
    pub fn advance_validated(&mut self, existing_entries: &[KnowledgeEntry]) {
        for entry in &mut self.entries {
            if entry.stage != ConfidenceStage::Replayed {
                continue;
            }

            // Redundancy check via HDC similarity when both have vectors.
            let dominated = if let Some(candidate_hdc) = hdc_from_entry(&entry.entry) {
                existing_entries.iter().any(|existing| {
                    hdc_from_entry(existing)
                        .map(|existing_hdc| candidate_hdc.similarity(&existing_hdc))
                        .is_some_and(|sim| sim >= REDUNDANCY_THRESHOLD)
                })
            } else {
                false
            };

            if dominated {
                continue;
            }

            entry.stage = ConfidenceStage::Validated;
            entry.confidence = CONFIDENCE_VALIDATED;
            entry.last_advanced_at = Utc::now();
        }
    }

    /// Garbage-collect entries older than `GC_HORIZON_DAYS` that haven't
    /// progressed past `Raw`.
    pub fn gc(&mut self) {
        self.gc_at(Utc::now());
    }

    /// Garbage-collect with an explicit "now" timestamp (for testing).
    pub fn gc_at(&mut self, now: DateTime<Utc>) {
        let horizon = now - Duration::days(GC_HORIZON_DAYS);
        self.entries.retain(|entry| {
            // Keep promoted entries and anything past Raw.
            if entry.stage != ConfidenceStage::Raw {
                return true;
            }
            // Drop Raw entries older than the horizon.
            entry.created_at > horizon
        });
    }

    /// Remove all entries that have been promoted (cleanup after promotion).
    pub fn remove_promoted(&mut self) {
        self.entries
            .retain(|entry| entry.stage != ConfidenceStage::Promoted);
    }
}

/// Try to reconstruct an `HdcVector` from the serialized bytes in a `KnowledgeEntry`.
fn hdc_from_entry(entry: &KnowledgeEntry) -> Option<HdcVector> {
    let bytes = entry.hdc_vector.as_ref()?;
    let array: &[u8; 1280] = bytes.as_slice().try_into().ok()?;
    Some(HdcVector::from_bytes(array))
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_neuro::{KnowledgeEntry, KnowledgeKind};

    fn test_entry(id: &str, content: &str) -> KnowledgeEntry {
        KnowledgeEntry {
            id: id.to_string(),
            content: content.to_string(),
            kind: KnowledgeKind::Heuristic,
            tier: KnowledgeTier::Transient,
            confidence: 0.5,
            confidence_weight: 0.5,
            source: Some("dream-test".to_string()),
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: vec![],
            tags: vec![],
            source_model: None,
            model_generality: 1.0,
            created_at: Utc::now(),
            half_life_days: 7.0,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: vec![],
            deprecated: false,
        }
    }

    #[test]
    fn confidence_ladder_progression() {
        let mut buf = StagingBuffer::new();
        buf.add_candidate(test_entry("e1", "test insight"), "ep-1".to_string());

        assert_eq!(buf.entries[0].stage, ConfidenceStage::Raw);
        assert!((buf.entries[0].confidence - 0.20).abs() < 1e-9);

        buf.advance_stage(0);
        assert_eq!(buf.entries[0].stage, ConfidenceStage::Replayed);
        assert!((buf.entries[0].confidence - 0.30).abs() < 1e-9);

        buf.advance_stage(0);
        assert_eq!(buf.entries[0].stage, ConfidenceStage::Validated);
        assert!((buf.entries[0].confidence - 0.50).abs() < 1e-9);

        buf.advance_stage(0);
        assert_eq!(buf.entries[0].stage, ConfidenceStage::Promoted);
        assert!((buf.entries[0].confidence - 0.70).abs() < 1e-9);

        // Can't advance past Promoted.
        assert!(!buf.advance_stage(0));
    }

    #[test]
    fn candidates_at_stage_filters_correctly() {
        let mut buf = StagingBuffer::new();
        buf.add_candidate(test_entry("e1", "insight one"), "ep-1".to_string());
        buf.add_candidate(test_entry("e2", "insight two"), "ep-2".to_string());
        buf.advance_stage(0);

        let raw = buf.candidates_at_stage(&ConfidenceStage::Raw);
        assert_eq!(raw.len(), 1);
        assert_eq!(raw[0].1.entry.id, "e2");

        let replayed = buf.candidates_at_stage(&ConfidenceStage::Replayed);
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].1.entry.id, "e1");
    }

    #[test]
    fn gc_removes_old_raw_entries() {
        let mut buf = StagingBuffer::new();
        buf.add_candidate(test_entry("e1", "old insight"), "ep-1".to_string());
        // Manually backdate the entry.
        buf.entries[0].created_at = Utc::now() - Duration::days(10);

        buf.add_candidate(test_entry("e2", "fresh insight"), "ep-2".to_string());

        // Advance e2 so it's not Raw.
        buf.advance_stage(1);

        buf.gc();
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.entries[0].entry.id, "e2");
    }

    #[test]
    fn gc_keeps_non_raw_entries() {
        let mut buf = StagingBuffer::new();
        buf.add_candidate(test_entry("e1", "promoted insight"), "ep-1".to_string());
        buf.entries[0].created_at = Utc::now() - Duration::days(10);
        buf.advance_stage(0); // Raw -> Replayed

        buf.gc();
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn advance_replayed_by_episode_ids() {
        let mut buf = StagingBuffer::new();
        buf.add_candidate(test_entry("e1", "insight one"), "ep-1".to_string());
        buf.add_candidate(test_entry("e2", "insight two"), "ep-2".to_string());

        buf.advance_replayed(&["ep-1".to_string()]);

        assert_eq!(buf.entries[0].stage, ConfidenceStage::Replayed);
        assert_eq!(buf.entries[1].stage, ConfidenceStage::Raw);
    }

    #[test]
    fn remove_promoted_cleans_up() {
        let mut buf = StagingBuffer::new();
        buf.add_candidate(test_entry("e1", "insight"), "ep-1".to_string());
        buf.advance_stage(0); // Raw -> Replayed
        buf.advance_stage(0); // Replayed -> Validated
        buf.advance_stage(0); // Validated -> Promoted

        buf.add_candidate(test_entry("e2", "other"), "ep-2".to_string());

        buf.remove_promoted();
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.entries[0].entry.id, "e2");
    }

    #[test]
    fn serialize_roundtrip() {
        let mut buf = StagingBuffer::new();
        buf.add_candidate(test_entry("e1", "insight"), "ep-1".to_string());
        buf.advance_stage(0);

        let json = serde_json::to_string(&buf).unwrap();
        let loaded: StagingBuffer = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.entries[0].stage, ConfidenceStage::Replayed);
    }

    #[test]
    fn save_and_load() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("staging.json");

        let mut buf = StagingBuffer::new();
        buf.add_candidate(test_entry("e1", "insight"), "ep-1".to_string());
        buf.save(&path).unwrap();

        let loaded = StagingBuffer::load_or_new(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.entries[0].entry.id, "e1");
    }
}
