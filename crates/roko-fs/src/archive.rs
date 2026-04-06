//! Archival format for old runs and episodes.
//!
//! The archiver compresses old data into compact summaries:
//!
//! - **Runs**: old run directories are summarized into a single
//!   [`ArchiveEntry`] per day (daily sampling).
//! - **Episodes**: excess episodes are compressed into iteration
//!   summaries with aggregate statistics.
//!
//! Archived data lives under `.roko/memory/archive/` in JSONL format,
//! organized by month (e.g. `2026-04.jsonl`).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::layout::RokoLayout;

/// A single archived record — the minimal summary of a run or episode
/// batch that is kept after the raw data is removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveEntry {
    /// What kind of data was archived.
    pub kind: ArchiveKind,
    /// The date this entry covers (daily sampling granularity).
    pub date: NaiveDate,
    /// Original source identifier (run ID, episode range, etc.).
    pub source_id: String,
    /// Number of raw items that were compressed into this entry.
    pub item_count: usize,
    /// Aggregate statistics.
    pub stats: ArchiveStats,
    /// When this archive entry was created.
    pub archived_at: DateTime<Utc>,
}

/// Discriminator for the kind of archived data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveKind {
    /// A run directory that was archived.
    Run,
    /// A batch of episodes that were archived.
    Episode,
}

/// Aggregate statistics kept in an archive entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveStats {
    /// Total size in bytes of the original data.
    pub total_bytes: u64,
    /// Total number of iterations (if applicable).
    pub total_iterations: u64,
    /// Number of gate passes (if applicable).
    pub gate_passes: u64,
    /// Number of gate failures (if applicable).
    pub gate_failures: u64,
    /// Total cost in USD (if applicable).
    pub total_cost_usd: f64,
}

impl Default for ArchiveStats {
    fn default() -> Self {
        Self {
            total_bytes: 0,
            total_iterations: 0,
            gate_passes: 0,
            gate_failures: 0,
            total_cost_usd: 0.0,
        }
    }
}

/// The archiver: compresses old runs and episodes into compact summaries.
#[derive(Debug, Clone)]
pub struct Archiver {
    layout: RokoLayout,
}

impl Archiver {
    /// Create an archiver for the given layout.
    #[must_use]
    pub const fn new(layout: RokoLayout) -> Self {
        Self { layout }
    }

    /// Path to the archive directory: `.roko/memory/archive/`.
    #[must_use]
    pub fn archive_dir(&self) -> PathBuf {
        self.layout.memory_dir().join("archive")
    }

    /// Path to the archive file for a given month (e.g. `2026-04.jsonl`).
    #[must_use]
    pub fn archive_file_for_month(&self, year: i32, month: u32) -> PathBuf {
        self.archive_dir().join(format!("{year:04}-{month:02}.jsonl"))
    }

    /// Archive a single run directory.
    ///
    /// Creates an [`ArchiveEntry`] summarizing the run, appends it to the
    /// appropriate monthly archive file, then removes the original run
    /// directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the archive directory cannot be created, the
    /// archive file cannot be written, or the run directory cannot be
    /// removed.
    pub async fn archive_run(
        &self,
        run_id: &str,
        date: NaiveDate,
        stats: ArchiveStats,
    ) -> std::io::Result<ArchiveEntry> {
        let entry = ArchiveEntry {
            kind: ArchiveKind::Run,
            date,
            source_id: run_id.to_string(),
            item_count: 1,
            stats,
            archived_at: Utc::now(),
        };

        self.append_entry(&entry).await?;

        // Remove the original run directory if it exists.
        let run_dir = self.layout.run_dir(run_id);
        if run_dir.is_dir() {
            tokio::fs::remove_dir_all(&run_dir).await?;
        }

        Ok(entry)
    }

    /// Archive a batch of episodes.
    ///
    /// Creates an [`ArchiveEntry`] summarizing the batch, appends it to
    /// the appropriate monthly archive file. The caller is responsible for
    /// removing the archived episodes from the source file.
    ///
    /// # Errors
    ///
    /// Returns an error if the archive directory cannot be created or the
    /// archive file cannot be written.
    pub async fn archive_episode(
        &self,
        source_id: &str,
        date: NaiveDate,
        item_count: usize,
        stats: ArchiveStats,
    ) -> std::io::Result<ArchiveEntry> {
        let entry = ArchiveEntry {
            kind: ArchiveKind::Episode,
            date,
            source_id: source_id.to_string(),
            item_count,
            stats,
            archived_at: Utc::now(),
        };

        self.append_entry(&entry).await?;
        Ok(entry)
    }

    /// Read all archive entries from a given monthly file.
    ///
    /// Returns an empty vec if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure (other than not-found).
    pub async fn read_month(
        &self,
        year: i32,
        month: u32,
    ) -> std::io::Result<Vec<ArchiveEntry>> {
        let path = self.archive_file_for_month(year, month);
        read_archive_file(&path).await
    }

    /// Read all archive entries across all monthly files.
    ///
    /// # Errors
    ///
    /// Returns an error if the archive directory cannot be read.
    pub async fn read_all(&self) -> std::io::Result<Vec<ArchiveEntry>> {
        let dir = self.archive_dir();
        if !dir.is_dir() {
            return Ok(Vec::new());
        }

        let mut all = Vec::new();
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "jsonl") {
                let mut records = read_archive_file(&path).await?;
                all.append(&mut records);
            }
        }

        all.sort_by_key(|e| e.date);
        Ok(all)
    }

    /// Perform daily sampling on a set of entries: keep only one entry per
    /// day (the last one for each day).
    #[must_use]
    pub fn daily_sample(entries: &[ArchiveEntry]) -> Vec<ArchiveEntry> {
        use std::collections::BTreeMap;
        let mut by_date: BTreeMap<NaiveDate, ArchiveEntry> = BTreeMap::new();
        for entry in entries {
            by_date.insert(entry.date, entry.clone());
        }
        by_date.into_values().collect()
    }

    /// Compress a slice of entries into a single iteration summary.
    ///
    /// Useful for reducing N individual episode entries into one aggregate
    /// record.
    #[must_use]
    pub fn iteration_summary(
        entries: &[ArchiveEntry],
        source_id: &str,
    ) -> Option<ArchiveEntry> {
        if entries.is_empty() {
            return None;
        }
        let date = entries
            .iter()
            .map(|e| e.date)
            .min()
            .unwrap_or_else(|| Utc::now().date_naive());
        let total_items: usize = entries.iter().map(|e| e.item_count).sum();
        let stats = ArchiveStats {
            total_bytes: entries.iter().map(|e| e.stats.total_bytes).sum(),
            total_iterations: entries.iter().map(|e| e.stats.total_iterations).sum(),
            gate_passes: entries.iter().map(|e| e.stats.gate_passes).sum(),
            gate_failures: entries.iter().map(|e| e.stats.gate_failures).sum(),
            total_cost_usd: entries.iter().map(|e| e.stats.total_cost_usd).sum(),
        };
        Some(ArchiveEntry {
            kind: entries[0].kind,
            date,
            source_id: source_id.to_string(),
            item_count: total_items,
            stats,
            archived_at: Utc::now(),
        })
    }

    // ── private ──────────────────────────────────────────────────────────

    /// Append an entry to the appropriate monthly archive file.
    async fn append_entry(&self, entry: &ArchiveEntry) -> std::io::Result<()> {
        let dir = self.archive_dir();
        tokio::fs::create_dir_all(&dir).await?;

        let year = entry.date.year();
        let month = entry.date.month();
        let path = self.archive_file_for_month(year, month);

        let mut line = serde_json::to_string(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        tokio::io::AsyncWriteExt::write_all(&mut file, line.as_bytes()).await?;
        tokio::io::AsyncWriteExt::flush(&mut file).await?;

        Ok(())
    }
}

/// Read all entries from a single archive JSONL file.
async fn read_archive_file(path: &Path) -> std::io::Result<Vec<ArchiveEntry>> {
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut entries = Vec::new();
    for line in contents.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<ArchiveEntry>(line) {
            entries.push(entry);
        }
        // Malformed lines are silently skipped — same crash-recovery
        // contract as FileSubstrate.
    }
    Ok(entries)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 4, day).expect("valid date")
    }

    fn sample_stats() -> ArchiveStats {
        ArchiveStats {
            total_bytes: 1024,
            total_iterations: 5,
            gate_passes: 3,
            gate_failures: 2,
            total_cost_usd: 0.50,
        }
    }

    fn sample_entry(kind: ArchiveKind, day: u32, source: &str) -> ArchiveEntry {
        ArchiveEntry {
            kind,
            date: sample_date(day),
            source_id: source.to_string(),
            item_count: 10,
            stats: sample_stats(),
            archived_at: Utc::now(),
        }
    }

    async fn setup() -> (TempDir, Archiver) {
        let tmp = TempDir::new().expect("tempdir");
        let layout = RokoLayout::for_project(tmp.path());
        layout.ensure_dirs().await.expect("ensure_dirs");
        let archiver = Archiver::new(layout);
        (tmp, archiver)
    }

    // ── ArchiveEntry serialization ───────────────────────────────────────

    #[test]
    fn archive_entry_round_trips_through_json() {
        let entry = sample_entry(ArchiveKind::Run, 5, "run-42");
        let json = serde_json::to_string(&entry).expect("serialize");
        let back: ArchiveEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.kind, entry.kind);
        assert_eq!(back.date, entry.date);
        assert_eq!(back.source_id, entry.source_id);
        assert_eq!(back.item_count, entry.item_count);
        assert_eq!(back.stats, entry.stats);
    }

    #[test]
    fn archive_stats_default_is_zeroed() {
        let stats = ArchiveStats::default();
        assert_eq!(stats.total_bytes, 0);
        assert_eq!(stats.total_iterations, 0);
        assert_eq!(stats.gate_passes, 0);
        assert_eq!(stats.gate_failures, 0);
        assert!((stats.total_cost_usd - 0.0).abs() < f64::EPSILON);
    }

    // ── Archiver path helpers ────────────────────────────────────────────

    #[test]
    fn archive_file_for_month_path() {
        let layout = RokoLayout::new("/p/.roko");
        let archiver = Archiver::new(layout);
        assert_eq!(
            archiver.archive_file_for_month(2026, 4),
            PathBuf::from("/p/.roko/memory/archive/2026-04.jsonl")
        );
    }

    // ── archive_run ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn archive_run_creates_entry_and_removes_dir() {
        let (_tmp, archiver) = setup().await;

        // Create a run directory.
        let run_dir = archiver.layout.run_dir("run-1");
        tokio::fs::create_dir_all(&run_dir).await.expect("create run dir");
        tokio::fs::write(run_dir.join("data.txt"), "hello")
            .await
            .expect("write");

        assert!(run_dir.is_dir());

        let entry = archiver
            .archive_run("run-1", sample_date(5), sample_stats())
            .await
            .expect("archive_run");

        assert_eq!(entry.kind, ArchiveKind::Run);
        assert_eq!(entry.source_id, "run-1");
        assert!(!run_dir.exists(), "run dir should be removed after archival");

        // Verify it was written to the archive file.
        let entries = archiver.read_month(2026, 4).await.expect("read month");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source_id, "run-1");
    }

    // ── archive_episode ──────────────────────────────────────────────────

    #[tokio::test]
    async fn archive_episode_creates_entry() {
        let (_tmp, archiver) = setup().await;

        let entry = archiver
            .archive_episode("batch-2026-04", sample_date(10), 50, sample_stats())
            .await
            .expect("archive_episode");

        assert_eq!(entry.kind, ArchiveKind::Episode);
        assert_eq!(entry.source_id, "batch-2026-04");
        assert_eq!(entry.item_count, 50);

        let entries = archiver.read_month(2026, 4).await.expect("read month");
        assert_eq!(entries.len(), 1);
    }

    // ── read_all ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn archive_read_all_aggregates_across_months() {
        let (_tmp, archiver) = setup().await;

        // April entry.
        archiver
            .archive_episode("batch-april", sample_date(5), 10, sample_stats())
            .await
            .expect("archive april");

        // March entry (different month file).
        let march_date = NaiveDate::from_ymd_opt(2026, 3, 15).expect("date");
        archiver
            .archive_episode("batch-march", march_date, 20, sample_stats())
            .await
            .expect("archive march");

        let all = archiver.read_all().await.expect("read all");
        assert_eq!(all.len(), 2);
        // Sorted by date — March should come first.
        assert_eq!(all[0].source_id, "batch-march");
        assert_eq!(all[1].source_id, "batch-april");
    }

    #[tokio::test]
    async fn archive_read_all_empty_when_no_archives() {
        let (_tmp, archiver) = setup().await;
        let all = archiver.read_all().await.expect("read all");
        assert!(all.is_empty());
    }

    // ── daily_sample ─────────────────────────────────────────────────────

    #[test]
    fn archive_daily_sample_keeps_one_per_day() {
        let entries = vec![
            sample_entry(ArchiveKind::Run, 5, "a"),
            sample_entry(ArchiveKind::Run, 5, "b"), // same day — should replace a
            sample_entry(ArchiveKind::Run, 6, "c"),
            sample_entry(ArchiveKind::Run, 6, "d"), // same day — should replace c
            sample_entry(ArchiveKind::Run, 7, "e"),
        ];
        let sampled = Archiver::daily_sample(&entries);
        assert_eq!(sampled.len(), 3);
        // For day 5, "b" replaces "a"; for day 6, "d" replaces "c".
        assert_eq!(sampled[0].source_id, "b");
        assert_eq!(sampled[1].source_id, "d");
        assert_eq!(sampled[2].source_id, "e");
    }

    #[test]
    fn archive_daily_sample_empty_input() {
        let sampled = Archiver::daily_sample(&[]);
        assert!(sampled.is_empty());
    }

    // ── iteration_summary ────────────────────────────────────────────────

    #[test]
    fn archive_iteration_summary_aggregates() {
        let entries = vec![
            {
                let mut e = sample_entry(ArchiveKind::Episode, 5, "a");
                e.item_count = 3;
                e.stats.gate_passes = 2;
                e.stats.gate_failures = 1;
                e.stats.total_cost_usd = 0.10;
                e
            },
            {
                let mut e = sample_entry(ArchiveKind::Episode, 6, "b");
                e.item_count = 7;
                e.stats.gate_passes = 5;
                e.stats.gate_failures = 2;
                e.stats.total_cost_usd = 0.25;
                e
            },
        ];

        let summary = Archiver::iteration_summary(&entries, "summary-1")
            .expect("should produce summary");
        assert_eq!(summary.source_id, "summary-1");
        assert_eq!(summary.item_count, 10);
        assert_eq!(summary.stats.gate_passes, 7);
        assert_eq!(summary.stats.gate_failures, 3);
        assert!((summary.stats.total_cost_usd - 0.35).abs() < f64::EPSILON);
        // Date should be the minimum (day 5).
        assert_eq!(summary.date, sample_date(5));
    }

    #[test]
    fn archive_iteration_summary_empty_returns_none() {
        let summary = Archiver::iteration_summary(&[], "empty");
        assert!(summary.is_none());
    }

    // ── Malformed line handling ──────────────────────────────────────────

    #[tokio::test]
    async fn archive_read_skips_malformed_lines() {
        let (_tmp, archiver) = setup().await;

        // Write a valid entry first.
        archiver
            .archive_episode("ok", sample_date(1), 1, sample_stats())
            .await
            .expect("archive");

        // Append garbage.
        let path = archiver.archive_file_for_month(2026, 4);
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .await
            .expect("open");
        tokio::io::AsyncWriteExt::write_all(&mut file, b"not valid json\n")
            .await
            .expect("write garbage");

        let entries = archiver.read_month(2026, 4).await.expect("read month");
        assert_eq!(entries.len(), 1, "should skip malformed line");
    }
}
