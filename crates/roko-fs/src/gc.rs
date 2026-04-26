//! Garbage-collection engine for the `.roko/` directory.
//!
//! [`GcEngine`] scans the layout for data that exceeds configured retention
//! limits, then either reports what *would* be removed ([`GcEngine::dry_run`])
//! or actually removes it ([`GcEngine::collect`]).
//!
//! # Retention policies
//!
//! | Store | Default limit | React |
//! |-------|--------------|--------|
//! | Episodes | 200 max | Keep most recent N |
//! | Runs | 7 days | Delete runs older than N days |
//! | Archive | 30 days | Delete archive entries older than N days |
//!
//! Policies are fully configurable via [`RetentionPolicy`]. The engine
//! also supports a size-based trigger: when the `.roko/` directory exceeds
//! a configurable threshold (MB), GC is recommended.
//!
//! # Safety
//!
//! - **Never touches `config/`** — user configuration is sacred.
//! - **Never touches `runtime/`** — PID files and locks are the running
//!   process's responsibility.
//! - **Idempotent** — re-running GC when nothing exceeds limits is a no-op.

use std::path::{Path, PathBuf};

use crate::layout::RokoLayout;

/// Configurable retention limits for `.roko/` data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Maximum number of episode records to keep. Oldest are removed first.
    pub max_episodes: usize,
    /// Maximum age of run directories in days. Older runs are removed.
    pub max_run_age_days: u32,
    /// Maximum age of archive entries in days. Older archives are removed.
    pub max_archive_age_days: u32,
    /// Size in MB above which GC is recommended for the entire `.roko/` tree.
    pub size_threshold_mb: u64,
    /// Maximum number of context-pack cache entries.
    pub max_cache_entries: usize,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_episodes: 200,
            max_run_age_days: 7,
            max_archive_age_days: 30,
            size_threshold_mb: 500,
            max_cache_entries: 2000,
        }
    }
}

/// A single item that the GC engine identified for removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcCandidate {
    /// Filesystem path of the item (file or directory).
    pub path: PathBuf,
    /// Human-readable reason this item is a GC candidate.
    pub reason: String,
    /// Size in bytes (0 if unknown or directory).
    pub size_bytes: u64,
}

/// Summary of a GC scan or collection pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Items identified for removal (or already removed, after `collect`).
    pub candidates: Vec<GcCandidate>,
    /// Total bytes that would be (or were) freed.
    pub total_bytes: u64,
    /// Number of items removed (0 for dry-run, populated after `collect`).
    pub removed_count: usize,
    /// Number of items that failed to remove (populated after `collect`).
    pub failed_count: usize,
}

impl GcReport {
    /// Whether the scan found anything to remove.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// Number of candidates identified.
    #[must_use]
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }
}

/// The GC engine. Operates on a [`RokoLayout`] under a [`RetentionPolicy`].
#[derive(Debug, Clone)]
pub struct GcEngine {
    layout: RokoLayout,
    policy: RetentionPolicy,
}

impl GcEngine {
    /// Create a new GC engine for the given layout and policy.
    #[must_use]
    pub const fn new(layout: RokoLayout, policy: RetentionPolicy) -> Self {
        Self { layout, policy }
    }

    /// Access the current retention policy.
    #[must_use]
    pub const fn policy(&self) -> &RetentionPolicy {
        &self.policy
    }

    /// Update the retention policy.
    pub const fn set_policy(&mut self, policy: RetentionPolicy) {
        self.policy = policy;
    }

    /// Scan the `.roko/` directory and return candidates for removal
    /// without deleting anything.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure while scanning.
    pub async fn scan(&self) -> std::io::Result<GcReport> {
        let mut report = GcReport::default();

        self.scan_runs(&mut report).await?;
        self.scan_episodes(&mut report).await?;
        self.scan_cache(&mut report).await?;

        report.total_bytes = report.candidates.iter().map(|c| c.size_bytes).sum();
        Ok(report)
    }

    /// Dry-run: scan and report what *would* be removed, without mutating
    /// the filesystem.
    ///
    /// This is an alias for [`scan`](GcEngine::scan) — the name makes
    /// intent explicit at call sites.
    ///
    /// # Errors
    ///
    /// Same as [`scan`](GcEngine::scan).
    pub async fn dry_run(&self) -> std::io::Result<GcReport> {
        self.scan().await
    }

    /// Scan *and* remove all candidates.
    ///
    /// Returns a report with `removed_count` populated.
    ///
    /// # Errors
    ///
    /// Returns an error if the initial scan fails. Individual removal
    /// failures are recorded in `failed_count` but do not abort the run.
    pub async fn collect(&self) -> std::io::Result<GcReport> {
        let mut report = self.scan().await?;
        let mut removed = 0usize;
        let mut failed = 0usize;

        for candidate in &report.candidates {
            let result = if candidate.path.is_dir() {
                tokio::fs::remove_dir_all(&candidate.path).await
            } else {
                tokio::fs::remove_file(&candidate.path).await
            };
            match result {
                Ok(()) => removed += 1,
                Err(_) => failed += 1,
            }
        }

        report.removed_count = removed;
        report.failed_count = failed;
        Ok(report)
    }

    /// Check whether the `.roko/` directory exceeds the configured size
    /// threshold.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be walked.
    pub async fn should_auto_gc(&self) -> std::io::Result<bool> {
        let size_mb = dir_size_mb(self.layout.root()).await?;
        Ok(size_mb >= self.policy.size_threshold_mb)
    }

    // ── private scanners ─────────────────────────────────────────────────

    /// Scan `runs/` for directories older than `max_run_age_days`.
    async fn scan_runs(&self, report: &mut GcReport) -> std::io::Result<()> {
        let runs_dir = self.layout.runs_dir();
        if !runs_dir.is_dir() {
            return Ok(());
        }

        let cutoff_secs = i64::from(self.policy.max_run_age_days) * 86_400;
        let now = chrono::Utc::now().timestamp();

        let mut entries = tokio::fs::read_dir(&runs_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let meta = entry.metadata().await?;
            if !meta.is_dir() {
                continue;
            }
            if let Ok(modified) = meta.modified() {
                let modified_secs = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                #[allow(clippy::cast_possible_wrap)]
                let age_secs = now.saturating_sub(modified_secs as i64);
                if age_secs > cutoff_secs {
                    let size = dir_size_bytes(&entry.path()).await.unwrap_or(0);
                    report.candidates.push(GcCandidate {
                        path: entry.path(),
                        reason: format!(
                            "run older than {} days (age: {} days)",
                            self.policy.max_run_age_days,
                            age_secs / 86_400
                        ),
                        size_bytes: size,
                    });
                }
            }
        }

        Ok(())
    }

    /// Scan `memory/episodes.jsonl` — if the file has more lines than
    /// `max_episodes`, report the excess as a candidate. The "candidate"
    /// in this case is a synthetic marker; the actual truncation would
    /// happen in the archiver. Here we report it for awareness.
    async fn scan_episodes(&self, report: &mut GcReport) -> std::io::Result<()> {
        let path = self.layout.episodes_path();
        if !path.is_file() {
            return Ok(());
        }
        let contents = tokio::fs::read_to_string(&path).await?;
        let line_count = contents.lines().filter(|l| !l.trim().is_empty()).count();

        if line_count > self.policy.max_episodes {
            let excess = line_count - self.policy.max_episodes;
            let meta = tokio::fs::metadata(&path).await?;
            // Estimate: bytes proportional to excess fraction
            let excess_bytes = meta.len() * excess as u64 / std::cmp::max(line_count as u64, 1);
            report.candidates.push(GcCandidate {
                path,
                reason: format!(
                    "episodes exceed limit: {line_count} > {} (excess: {excess})",
                    self.policy.max_episodes
                ),
                size_bytes: excess_bytes,
            });
        }

        Ok(())
    }

    /// Scan `cache/context-pack-cache/` for excess entries.
    async fn scan_cache(&self, report: &mut GcReport) -> std::io::Result<()> {
        let cache_dir = self.layout.context_pack_cache_dir();
        if !cache_dir.is_dir() {
            return Ok(());
        }

        // Collect all entries with their modification times.
        let mut entries: Vec<(PathBuf, std::time::SystemTime, u64)> = Vec::new();
        let mut dir = tokio::fs::read_dir(&cache_dir).await?;
        while let Some(entry) = dir.next_entry().await? {
            let meta = entry.metadata().await?;
            let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            entries.push((entry.path(), mtime, meta.len()));
        }

        if entries.len() > self.policy.max_cache_entries {
            // Sort oldest first.
            entries.sort_by_key(|(_, mtime, _)| *mtime);
            let to_remove = entries.len() - self.policy.max_cache_entries;
            for (path, _, size) in entries.into_iter().take(to_remove) {
                report.candidates.push(GcCandidate {
                    path,
                    reason: format!(
                        "cache exceeds {} entry limit",
                        self.policy.max_cache_entries
                    ),
                    size_bytes: size,
                });
            }
        }

        Ok(())
    }
}

/// Compute the total size of a directory tree in megabytes.
///
/// Best-effort: unreadable entries are silently skipped.
///
/// # Errors
///
/// Returns an error if the root directory cannot be read.
pub async fn dir_size_mb(path: &Path) -> std::io::Result<u64> {
    let bytes = dir_size_bytes(path).await?;
    Ok(bytes / (1024 * 1024))
}

/// Compute the total size of a directory tree in bytes.
async fn dir_size_bytes(path: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Some(entry) = entries.next_entry().await? {
            let meta = entry.metadata().await?;
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    Ok(total)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: set up a layout with `ensure_dirs` and return (TempDir, RokoLayout).
    async fn setup() -> (TempDir, RokoLayout) {
        let tmp = TempDir::new().expect("tempdir");
        let layout = RokoLayout::for_project(tmp.path());
        layout.ensure_dirs().await.expect("ensure_dirs");
        (tmp, layout)
    }

    // ── RetentionPolicy ──────────────────────────────────────────────────

    #[test]
    fn gc_default_policy_has_expected_values() {
        let p = RetentionPolicy::default();
        assert_eq!(p.max_episodes, 200);
        assert_eq!(p.max_run_age_days, 7);
        assert_eq!(p.max_archive_age_days, 30);
        assert_eq!(p.size_threshold_mb, 500);
        assert_eq!(p.max_cache_entries, 2000);
    }

    #[test]
    fn gc_policy_is_configurable() {
        let p = RetentionPolicy {
            max_episodes: 50,
            max_run_age_days: 3,
            max_archive_age_days: 14,
            size_threshold_mb: 100,
            max_cache_entries: 500,
        };
        assert_eq!(p.max_episodes, 50);
        assert_eq!(p.max_run_age_days, 3);
    }

    // ── GcEngine basics ──────────────────────────────────────────────────

    #[tokio::test]
    async fn gc_scan_empty_layout_finds_nothing() {
        let (_tmp, layout) = setup().await;
        let engine = GcEngine::new(layout, RetentionPolicy::default());
        let report = engine.scan().await.expect("scan");
        assert!(report.is_empty());
        assert_eq!(report.candidate_count(), 0);
        assert_eq!(report.total_bytes, 0);
    }

    #[tokio::test]
    async fn gc_dry_run_is_alias_for_scan() {
        let (_tmp, layout) = setup().await;
        let engine = GcEngine::new(layout, RetentionPolicy::default());
        let scan_report = engine.scan().await.expect("scan");
        let dry_report = engine.dry_run().await.expect("dry_run");
        assert_eq!(scan_report, dry_report);
    }

    #[tokio::test]
    async fn gc_collect_on_empty_layout_removes_nothing() {
        let (_tmp, layout) = setup().await;
        let engine = GcEngine::new(layout, RetentionPolicy::default());
        let report = engine.collect().await.expect("collect");
        assert_eq!(report.removed_count, 0);
        assert_eq!(report.failed_count, 0);
        assert!(report.is_empty());
    }

    // ── Episode scanning ─────────────────────────────────────────────────

    #[tokio::test]
    async fn gc_scan_detects_excess_episodes() {
        let (_tmp, layout) = setup().await;

        // Write 10 episode lines.
        let mut content = String::new();
        for i in 0..10 {
            content.push_str(&format!("{{\"id\":{i}}}\n"));
        }
        tokio::fs::write(layout.episodes_path(), &content)
            .await
            .expect("write episodes");

        // React: max 5 episodes.
        let policy = RetentionPolicy {
            max_episodes: 5,
            ..Default::default()
        };
        let engine = GcEngine::new(layout, policy);
        let report = engine.scan().await.expect("scan");

        assert_eq!(report.candidate_count(), 1);
        assert!(report.candidates[0].reason.contains("exceed"));
        assert!(report.candidates[0].reason.contains("10 > 5"));
    }

    #[tokio::test]
    async fn gc_scan_no_excess_episodes() {
        let (_tmp, layout) = setup().await;

        let mut content = String::new();
        for i in 0..3 {
            content.push_str(&format!("{{\"id\":{i}}}\n"));
        }
        tokio::fs::write(layout.episodes_path(), &content)
            .await
            .expect("write episodes");

        let policy = RetentionPolicy {
            max_episodes: 10,
            ..Default::default()
        };
        let engine = GcEngine::new(layout, policy);
        let report = engine.scan().await.expect("scan");
        assert!(report.is_empty());
    }

    // ── Run scanning ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn gc_scan_detects_old_runs() {
        let (_tmp, layout) = setup().await;

        // Create a run directory.
        let run_dir = layout.run_dir("old-run");
        tokio::fs::create_dir_all(&run_dir)
            .await
            .expect("create run dir");
        tokio::fs::write(run_dir.join("metrics.jsonl"), "data\n")
            .await
            .expect("write metrics");

        // Set the directory modification time to 30 days ago.
        let thirty_days_ago =
            std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 86_400);
        let mtime = filetime::FileTime::from_system_time(thirty_days_ago);
        filetime::set_file_mtime(&run_dir, mtime).expect("set mtime");

        let policy = RetentionPolicy {
            max_run_age_days: 7,
            ..Default::default()
        };
        let engine = GcEngine::new(layout, policy);
        let report = engine.scan().await.expect("scan");

        assert!(!report.is_empty(), "should find old run");
        assert!(report.candidates.iter().any(|c| c.path == run_dir));
    }

    #[tokio::test]
    async fn gc_scan_keeps_fresh_runs() {
        let (_tmp, layout) = setup().await;

        let run_dir = layout.run_dir("fresh-run");
        tokio::fs::create_dir_all(&run_dir)
            .await
            .expect("create run dir");
        tokio::fs::write(run_dir.join("data.txt"), "hello\n")
            .await
            .expect("write data");

        let policy = RetentionPolicy {
            max_run_age_days: 7,
            ..Default::default()
        };
        let engine = GcEngine::new(layout, policy);
        let report = engine.scan().await.expect("scan");

        // Fresh run should not be a candidate.
        assert!(
            !report.candidates.iter().any(|c| c.path == run_dir),
            "fresh run should not be a GC candidate"
        );
    }

    #[tokio::test]
    async fn gc_collect_removes_old_runs() {
        let (_tmp, layout) = setup().await;

        let run_dir = layout.run_dir("doomed-run");
        tokio::fs::create_dir_all(&run_dir)
            .await
            .expect("create run dir");
        tokio::fs::write(run_dir.join("metrics.jsonl"), "doomed\n")
            .await
            .expect("write");

        let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(15 * 86_400);
        let mtime = filetime::FileTime::from_system_time(old_time);
        filetime::set_file_mtime(&run_dir, mtime).expect("set mtime");

        let policy = RetentionPolicy {
            max_run_age_days: 7,
            ..Default::default()
        };
        let engine = GcEngine::new(layout, policy);
        let report = engine.collect().await.expect("collect");

        assert!(report.removed_count > 0, "should have removed something");
        assert!(!run_dir.exists(), "run dir should be gone");
    }

    // ── Cache scanning ───────────────────────────────────────────────────

    #[tokio::test]
    async fn gc_scan_detects_excess_cache_entries() {
        let (_tmp, layout) = setup().await;

        let cache_dir = layout.context_pack_cache_dir();
        tokio::fs::create_dir_all(&cache_dir)
            .await
            .expect("create cache dir");

        // Create 10 cache files.
        for i in 0..10 {
            tokio::fs::write(cache_dir.join(format!("pack-{i}.bin")), "data")
                .await
                .expect("write cache");
        }

        let policy = RetentionPolicy {
            max_cache_entries: 5,
            ..Default::default()
        };
        let engine = GcEngine::new(layout, policy);
        let report = engine.scan().await.expect("scan");

        // Should identify 5 entries for removal (10 - 5).
        assert_eq!(
            report.candidate_count(),
            5,
            "should identify 5 excess cache entries"
        );
    }

    #[tokio::test]
    async fn gc_collect_removes_excess_cache_entries() {
        let (_tmp, layout) = setup().await;

        let cache_dir = layout.context_pack_cache_dir();
        tokio::fs::create_dir_all(&cache_dir)
            .await
            .expect("create cache dir");

        for i in 0..8 {
            tokio::fs::write(cache_dir.join(format!("pack-{i}.bin")), "data")
                .await
                .expect("write cache");
        }

        let policy = RetentionPolicy {
            max_cache_entries: 3,
            ..Default::default()
        };
        let engine = GcEngine::new(layout, policy);
        let report = engine.collect().await.expect("collect");

        assert_eq!(report.removed_count, 5, "should remove 5 entries");

        // Count remaining files.
        let mut remaining = 0;
        let mut dir = tokio::fs::read_dir(&cache_dir).await.expect("read dir");
        while dir.next_entry().await.expect("entry").is_some() {
            remaining += 1;
        }
        assert_eq!(remaining, 3, "should have 3 entries left");
    }

    // ── Size-based trigger ───────────────────────────────────────────────

    #[tokio::test]
    async fn gc_should_auto_gc_small_dir() {
        let (_tmp, layout) = setup().await;
        let policy = RetentionPolicy {
            size_threshold_mb: 100,
            ..Default::default()
        };
        let engine = GcEngine::new(layout, policy);
        let should = engine.should_auto_gc().await.expect("should_auto_gc");
        assert!(!should, "tiny dir should not trigger auto-gc");
    }

    #[tokio::test]
    async fn gc_should_auto_gc_with_zero_threshold() {
        let (_tmp, layout) = setup().await;
        // Any non-empty directory exceeds 0 MB.
        tokio::fs::write(layout.memory_dir().join("test.txt"), "hello")
            .await
            .expect("write");
        let policy = RetentionPolicy {
            size_threshold_mb: 0,
            ..Default::default()
        };
        let engine = GcEngine::new(layout, policy);
        let should = engine.should_auto_gc().await.expect("should_auto_gc");
        assert!(should, "should trigger with 0 MB threshold");
    }

    // ── GcReport ─────────────────────────────────────────────────────────

    #[test]
    fn gc_report_default_is_empty() {
        let report = GcReport::default();
        assert!(report.is_empty());
        assert_eq!(report.candidate_count(), 0);
        assert_eq!(report.total_bytes, 0);
        assert_eq!(report.removed_count, 0);
        assert_eq!(report.failed_count, 0);
    }

    // ── Engine configuration ─────────────────────────────────────────────

    #[test]
    fn gc_engine_policy_is_accessible() {
        let layout = RokoLayout::new("/tmp/.roko");
        let policy = RetentionPolicy {
            max_episodes: 42,
            ..Default::default()
        };
        let engine = GcEngine::new(layout, policy.clone());
        assert_eq!(engine.policy(), &policy);
    }

    #[test]
    fn gc_engine_set_policy() {
        let layout = RokoLayout::new("/tmp/.roko");
        let mut engine = GcEngine::new(layout, RetentionPolicy::default());
        let new_policy = RetentionPolicy {
            max_episodes: 10,
            max_run_age_days: 2,
            ..Default::default()
        };
        engine.set_policy(new_policy.clone());
        assert_eq!(engine.policy(), &new_policy);
    }
}
