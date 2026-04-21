//! Temporal knowledge topology -- Allen interval algebra over knowledge states.
//!
//! This module implements temporal reasoning over knowledge validity periods.
//! Each piece of knowledge has a validity interval, and relationships between
//! knowledge items are expressed using Allen's 13 interval relations.
//!
//! # Architecture
//!
//! ```text
//! KnowledgeEpoch ──────────────── temporal boundary of knowledge validity
//! TemporalInterval ────────────── [start, end) validity window
//! AllenRelation ───────────────── 13 interval relations (before, after, meets, ...)
//! TemporalRelation ────────────── relates two knowledge entries via AllenRelation
//! TemporalIndex ───────────────── indexes entries by epoch for temporal queries
//! ```

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A temporal boundary marking a phase of knowledge validity.
///
/// Epochs are monotonically increasing: epoch N+1 starts when epoch N ends.
/// Knowledge created during epoch N is valid within its interval unless
/// explicitly superseded.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KnowledgeEpoch {
    /// Epoch sequence number (monotonically increasing).
    pub seq: u64,
    /// Human-readable label (e.g., "sprint-42", "v2.1-release").
    pub label: String,
    /// Start of the epoch (inclusive).
    pub start: DateTime<Utc>,
    /// End of the epoch (exclusive). `None` means the epoch is still open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<DateTime<Utc>>,
}

impl KnowledgeEpoch {
    /// Create a new open epoch starting now.
    #[must_use]
    pub fn open(seq: u64, label: impl Into<String>) -> Self {
        Self {
            seq,
            label: label.into(),
            start: Utc::now(),
            end: None,
        }
    }

    /// Create an epoch with explicit start time.
    #[must_use]
    pub fn at(seq: u64, label: impl Into<String>, start: DateTime<Utc>) -> Self {
        Self {
            seq,
            label: label.into(),
            start,
            end: None,
        }
    }

    /// Close the epoch at the given time.
    pub fn close(&mut self, at: DateTime<Utc>) {
        self.end = Some(at);
    }

    /// Whether this epoch is still open (no end time set).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.end.is_none()
    }

    /// Duration of the epoch in seconds, or `None` if still open.
    #[must_use]
    pub fn duration_secs(&self) -> Option<i64> {
        self.end.map(|e| (e - self.start).num_seconds())
    }

    /// Convert to a temporal interval. Open epochs get `i64::MAX` as end.
    #[must_use]
    pub fn to_interval(&self) -> TemporalInterval {
        TemporalInterval {
            start: self.start.timestamp_millis(),
            end: self.end.map_or(i64::MAX, |e| e.timestamp_millis()),
        }
    }
}

/// A half-open time interval `[start, end)` in epoch milliseconds.
///
/// Used for Allen interval algebra computations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TemporalInterval {
    /// Start time (inclusive), in epoch milliseconds.
    pub start: i64,
    /// End time (exclusive), in epoch milliseconds.
    pub end: i64,
}

impl TemporalInterval {
    /// Create a new interval.
    #[must_use]
    pub const fn new(start: i64, end: i64) -> Self {
        Self { start, end }
    }

    /// Duration of the interval in milliseconds.
    #[must_use]
    pub const fn duration_ms(&self) -> i64 {
        self.end - self.start
    }

    /// Whether a timestamp falls within this interval.
    #[must_use]
    pub const fn contains(&self, t: i64) -> bool {
        t >= self.start && t < self.end
    }

    /// Whether two intervals overlap (share at least one point in time).
    #[must_use]
    pub const fn overlaps(&self, other: &Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// The intersection of two intervals, or `None` if they don't overlap.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        if start < end {
            Some(Self { start, end })
        } else {
            None
        }
    }

    /// The convex hull (smallest interval containing both).
    #[must_use]
    pub fn hull(&self, other: &Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Determine the Allen relation of `self` to `other`.
    #[must_use]
    pub fn allen_relation(&self, other: &Self) -> AllenRelation {
        AllenRelation::compute(self, other)
    }
}

/// The 13 Allen interval relations.
///
/// Given two intervals X and Y, exactly one of these relations holds.
/// See J.F. Allen, "Maintaining Knowledge about Temporal Intervals" (1983).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AllenRelation {
    /// X is entirely before Y (gap between them).
    Before,
    /// Y is entirely before X (inverse of Before).
    After,
    /// X ends exactly where Y starts (no gap, no overlap).
    Meets,
    /// Y ends exactly where X starts (inverse of Meets).
    MetBy,
    /// X starts before Y starts and ends before Y ends, with overlap.
    Overlaps,
    /// Y starts before X starts and ends before X ends, with overlap (inverse of Overlaps).
    OverlappedBy,
    /// X starts before Y and ends after Y (X contains Y).
    Contains,
    /// Y starts before X and ends after X (Y contains X, inverse of Contains).
    During,
    /// X and Y start at the same time, X ends before Y.
    Starts,
    /// X and Y start at the same time, Y ends before X (inverse of Starts).
    StartedBy,
    /// X and Y end at the same time, X starts after Y.
    Finishes,
    /// X and Y end at the same time, Y starts after X (inverse of Finishes).
    FinishedBy,
    /// X and Y have identical start and end times.
    Equal,
}

impl AllenRelation {
    /// Compute the Allen relation between two intervals.
    #[must_use]
    pub fn compute(x: &TemporalInterval, y: &TemporalInterval) -> Self {
        if x.start == y.start && x.end == y.end {
            Self::Equal
        } else if x.end < y.start {
            Self::Before
        } else if y.end < x.start {
            Self::After
        } else if x.end == y.start {
            Self::Meets
        } else if y.end == x.start {
            Self::MetBy
        } else if x.start < y.start && x.end > y.start && x.end < y.end {
            Self::Overlaps
        } else if y.start < x.start && y.end > x.start && y.end < x.end {
            Self::OverlappedBy
        } else if x.start < y.start && x.end > y.end {
            Self::Contains
        } else if y.start < x.start && y.end > x.end {
            Self::During
        } else if x.start == y.start && x.end < y.end {
            Self::Starts
        } else if x.start == y.start && x.end > y.end {
            Self::StartedBy
        } else if x.end == y.end && x.start > y.start {
            Self::Finishes
        } else if x.end == y.end && x.start < y.start {
            Self::FinishedBy
        } else {
            // Exhaustive -- should not reach here for valid intervals
            Self::Equal
        }
    }

    /// The inverse relation (swapping X and Y).
    #[must_use]
    pub const fn inverse(&self) -> Self {
        match self {
            Self::Before => Self::After,
            Self::After => Self::Before,
            Self::Meets => Self::MetBy,
            Self::MetBy => Self::Meets,
            Self::Overlaps => Self::OverlappedBy,
            Self::OverlappedBy => Self::Overlaps,
            Self::Contains => Self::During,
            Self::During => Self::Contains,
            Self::Starts => Self::StartedBy,
            Self::StartedBy => Self::Starts,
            Self::Finishes => Self::FinishedBy,
            Self::FinishedBy => Self::Finishes,
            Self::Equal => Self::Equal,
        }
    }

    /// Whether the two intervals are concurrent (overlapping in any way).
    #[must_use]
    pub const fn is_concurrent(&self) -> bool {
        matches!(
            self,
            Self::Overlaps
                | Self::OverlappedBy
                | Self::Contains
                | Self::During
                | Self::Starts
                | Self::StartedBy
                | Self::Finishes
                | Self::FinishedBy
                | Self::Equal
        )
    }

    /// Whether one interval is strictly before the other (no overlap).
    #[must_use]
    pub const fn is_sequential(&self) -> bool {
        matches!(self, Self::Before | Self::After | Self::Meets | Self::MetBy)
    }

    /// Human-readable label.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
            Self::Meets => "meets",
            Self::MetBy => "met_by",
            Self::Overlaps => "overlaps",
            Self::OverlappedBy => "overlapped_by",
            Self::Contains => "contains",
            Self::During => "during",
            Self::Starts => "starts",
            Self::StartedBy => "started_by",
            Self::Finishes => "finishes",
            Self::FinishedBy => "finished_by",
            Self::Equal => "equal",
        }
    }
}

/// A directed temporal relation between two knowledge entry IDs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TemporalRelation {
    /// Source knowledge entry identifier.
    pub source: String,
    /// Target knowledge entry identifier.
    pub target: String,
    /// The Allen relation from source to target.
    pub relation: AllenRelation,
    /// Optional annotation (e.g., "supersedes", "extends", "contradicts").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotation: Option<String>,
}

impl TemporalRelation {
    /// Create a new temporal relation.
    #[must_use]
    pub fn new(
        source: impl Into<String>,
        target: impl Into<String>,
        relation: AllenRelation,
    ) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            relation,
            annotation: None,
        }
    }

    /// Attach an annotation to the relation.
    #[must_use]
    pub fn with_annotation(mut self, annotation: impl Into<String>) -> Self {
        self.annotation = Some(annotation.into());
        self
    }

    /// Compute the inverse relation (swapping source and target).
    #[must_use]
    pub fn inverse(&self) -> Self {
        Self {
            source: self.target.clone(),
            target: self.source.clone(),
            relation: self.relation.inverse(),
            annotation: self.annotation.clone(),
        }
    }
}

/// An index of knowledge entries organized by temporal epoch.
///
/// Supports efficient lookup of entries within an epoch, finding the epoch
/// for a given timestamp, and computing Allen relations between entries.
pub struct TemporalIndex {
    /// Epochs ordered by sequence number.
    epochs: BTreeMap<u64, KnowledgeEpoch>,
    /// Entry-id -> interval mapping.
    entries: HashMap<String, TemporalInterval>,
    /// Cached relations between pairs.
    relations: Vec<TemporalRelation>,
}

impl TemporalIndex {
    /// Create a new empty temporal index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            epochs: BTreeMap::new(),
            entries: HashMap::new(),
            relations: Vec::new(),
        }
    }

    /// Register a new epoch.
    pub fn add_epoch(&mut self, epoch: KnowledgeEpoch) {
        self.epochs.insert(epoch.seq, epoch);
    }

    /// Register a knowledge entry with its validity interval.
    pub fn add_entry(&mut self, id: impl Into<String>, interval: TemporalInterval) {
        self.entries.insert(id.into(), interval);
    }

    /// Compute and cache the Allen relation between two entries.
    /// Returns `None` if either entry is not registered.
    pub fn relate(&mut self, source: &str, target: &str) -> Option<AllenRelation> {
        let src_iv = self.entries.get(source)?;
        let tgt_iv = self.entries.get(target)?;
        let relation = src_iv.allen_relation(tgt_iv);
        self.relations
            .push(TemporalRelation::new(source, target, relation));
        Some(relation)
    }

    /// Find all entries whose validity interval contains the given timestamp.
    #[must_use]
    pub fn entries_at(&self, timestamp: i64) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|(_, iv)| iv.contains(timestamp))
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Find all entries concurrent with the given entry.
    #[must_use]
    pub fn concurrent_with(&self, entry_id: &str) -> Vec<&str> {
        let Some(iv) = self.entries.get(entry_id) else {
            return Vec::new();
        };
        self.entries
            .iter()
            .filter(|(id, other_iv)| id.as_str() != entry_id && iv.overlaps(other_iv))
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Find the epoch containing the given timestamp.
    #[must_use]
    pub fn epoch_at(&self, timestamp: i64) -> Option<&KnowledgeEpoch> {
        self.epochs
            .values()
            .find(|e| e.to_interval().contains(timestamp))
    }

    /// Return all cached temporal relations.
    #[must_use]
    pub fn relations(&self) -> &[TemporalRelation] {
        &self.relations
    }

    /// Number of indexed entries.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Number of registered epochs.
    #[must_use]
    pub fn epoch_count(&self) -> usize {
        self.epochs.len()
    }
}

impl Default for TemporalIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iv(start: i64, end: i64) -> TemporalInterval {
        TemporalInterval::new(start, end)
    }

    #[test]
    fn allen_before() {
        assert_eq!(iv(1, 3).allen_relation(&iv(5, 8)), AllenRelation::Before);
    }

    #[test]
    fn allen_after() {
        assert_eq!(iv(5, 8).allen_relation(&iv(1, 3)), AllenRelation::After);
    }

    #[test]
    fn allen_meets() {
        assert_eq!(iv(1, 5).allen_relation(&iv(5, 8)), AllenRelation::Meets);
    }

    #[test]
    fn allen_met_by() {
        assert_eq!(iv(5, 8).allen_relation(&iv(1, 5)), AllenRelation::MetBy);
    }

    #[test]
    fn allen_overlaps() {
        assert_eq!(iv(1, 5).allen_relation(&iv(3, 8)), AllenRelation::Overlaps);
    }

    #[test]
    fn allen_overlapped_by() {
        assert_eq!(
            iv(3, 8).allen_relation(&iv(1, 5)),
            AllenRelation::OverlappedBy
        );
    }

    #[test]
    fn allen_contains() {
        assert_eq!(iv(1, 10).allen_relation(&iv(3, 7)), AllenRelation::Contains);
    }

    #[test]
    fn allen_during() {
        assert_eq!(iv(3, 7).allen_relation(&iv(1, 10)), AllenRelation::During);
    }

    #[test]
    fn allen_starts() {
        assert_eq!(iv(1, 5).allen_relation(&iv(1, 8)), AllenRelation::Starts);
    }

    #[test]
    fn allen_started_by() {
        assert_eq!(iv(1, 8).allen_relation(&iv(1, 5)), AllenRelation::StartedBy);
    }

    #[test]
    fn allen_finishes() {
        assert_eq!(iv(3, 8).allen_relation(&iv(1, 8)), AllenRelation::Finishes);
    }

    #[test]
    fn allen_finished_by() {
        assert_eq!(
            iv(1, 8).allen_relation(&iv(3, 8)),
            AllenRelation::FinishedBy
        );
    }

    #[test]
    fn allen_equal() {
        assert_eq!(iv(1, 8).allen_relation(&iv(1, 8)), AllenRelation::Equal);
    }

    #[test]
    fn allen_inverse_is_symmetric() {
        let all = [
            AllenRelation::Before,
            AllenRelation::After,
            AllenRelation::Meets,
            AllenRelation::MetBy,
            AllenRelation::Overlaps,
            AllenRelation::OverlappedBy,
            AllenRelation::Contains,
            AllenRelation::During,
            AllenRelation::Starts,
            AllenRelation::StartedBy,
            AllenRelation::Finishes,
            AllenRelation::FinishedBy,
            AllenRelation::Equal,
        ];
        for r in &all {
            assert_eq!(r.inverse().inverse(), *r);
        }
    }

    #[test]
    fn interval_contains_point() {
        let iv = TemporalInterval::new(10, 20);
        assert!(iv.contains(10));
        assert!(iv.contains(15));
        assert!(!iv.contains(20)); // half-open
        assert!(!iv.contains(5));
    }

    #[test]
    fn interval_overlap() {
        assert!(iv(1, 5).overlaps(&iv(3, 8)));
        assert!(!iv(1, 5).overlaps(&iv(5, 8))); // half-open, no overlap at boundary
        assert!(!iv(1, 3).overlaps(&iv(5, 8)));
    }

    #[test]
    fn interval_intersection() {
        assert_eq!(iv(1, 5).intersection(&iv(3, 8)), Some(iv(3, 5)));
        assert_eq!(iv(1, 3).intersection(&iv(5, 8)), None);
    }

    #[test]
    fn interval_hull() {
        assert_eq!(iv(3, 5).hull(&iv(1, 8)), iv(1, 8));
        assert_eq!(iv(1, 3).hull(&iv(5, 8)), iv(1, 8));
    }

    #[test]
    fn temporal_relation_inverse() {
        let rel =
            TemporalRelation::new("a", "b", AllenRelation::Before).with_annotation("supersedes");
        let inv = rel.inverse();
        assert_eq!(inv.source, "b");
        assert_eq!(inv.target, "a");
        assert_eq!(inv.relation, AllenRelation::After);
    }

    #[test]
    fn is_concurrent_vs_sequential() {
        assert!(!AllenRelation::Before.is_concurrent());
        assert!(AllenRelation::Before.is_sequential());
        assert!(AllenRelation::Overlaps.is_concurrent());
        assert!(!AllenRelation::Overlaps.is_sequential());
        assert!(AllenRelation::Equal.is_concurrent());
    }

    #[test]
    fn temporal_index_basic() {
        let mut idx = TemporalIndex::new();
        idx.add_entry("fact-1", iv(0, 100));
        idx.add_entry("fact-2", iv(50, 200));
        idx.add_entry("fact-3", iv(300, 400));

        // fact-1 and fact-2 overlap
        let rel = idx.relate("fact-1", "fact-2").unwrap();
        assert_eq!(rel, AllenRelation::Overlaps);

        // fact-1 is before fact-3
        let rel = idx.relate("fact-1", "fact-3").unwrap();
        assert_eq!(rel, AllenRelation::Before);

        // Entries at timestamp 75 should include fact-1 and fact-2
        let at_75 = idx.entries_at(75);
        assert_eq!(at_75.len(), 2);
        assert!(at_75.contains(&"fact-1"));
        assert!(at_75.contains(&"fact-2"));

        // fact-1 is concurrent with fact-2 but not fact-3
        let mut conc = idx.concurrent_with("fact-1");
        conc.sort();
        assert_eq!(conc, vec!["fact-2"]);
    }

    #[test]
    fn epoch_lifecycle() {
        let mut epoch = KnowledgeEpoch::at(1, "sprint-1", Utc::now());
        assert!(epoch.is_open());
        assert!(epoch.duration_secs().is_none());

        epoch.close(epoch.start + chrono::Duration::hours(2));
        assert!(!epoch.is_open());
        assert_eq!(epoch.duration_secs(), Some(7200));
    }

    #[test]
    fn temporal_index_epochs() {
        let mut idx = TemporalIndex::new();
        let e1 = KnowledgeEpoch {
            seq: 0,
            label: "epoch-0".into(),
            start: DateTime::from_timestamp_millis(0).unwrap(),
            end: Some(DateTime::from_timestamp_millis(100).unwrap()),
        };
        let e2 = KnowledgeEpoch {
            seq: 1,
            label: "epoch-1".into(),
            start: DateTime::from_timestamp_millis(100).unwrap(),
            end: Some(DateTime::from_timestamp_millis(200).unwrap()),
        };
        idx.add_epoch(e1);
        idx.add_epoch(e2);

        assert_eq!(idx.epoch_count(), 2);
        let found = idx.epoch_at(50).unwrap();
        assert_eq!(found.label, "epoch-0");
        let found = idx.epoch_at(150).unwrap();
        assert_eq!(found.label, "epoch-1");
        assert!(idx.epoch_at(250).is_none());
    }

    #[test]
    fn serde_roundtrip_allen_relation() {
        let rel = AllenRelation::OverlappedBy;
        let json = serde_json::to_string(&rel).unwrap();
        let back: AllenRelation = serde_json::from_str(&json).unwrap();
        assert_eq!(rel, back);
    }

    #[test]
    fn serde_roundtrip_temporal_relation() {
        let tr =
            TemporalRelation::new("a", "b", AllenRelation::Contains).with_annotation("extends");
        let json = serde_json::to_string(&tr).unwrap();
        let back: TemporalRelation = serde_json::from_str(&json).unwrap();
        assert_eq!(tr, back);
    }
}
