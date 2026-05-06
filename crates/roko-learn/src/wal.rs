//! Synchronous Write-Ahead Log for learning state durability (S16.7, S19.1).
//!
//! WAL lives at `.roko/learn/wal.jsonl`. Each line is a JSON-serialized
//! [`WalEntry`]. Entries are appended with `sync_data()` for durability.
//! After a successful snapshot save, the WAL is truncated to zero.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A single durable learning event.
///
/// Each variant carries exactly the fields needed to replay the in-memory
/// update -- not the full snapshot. This keeps WAL entries small (~100-400
/// bytes each) while preserving full replay fidelity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WalEntry {
    /// A cascade router observation: confidence stats + LinUCB arm update.
    CascadeObservation {
        /// Model slug that was routed to.
        model_slug: String,
        /// Context feature vector passed to LinUCB at routing time.
        context_features: Vec<f64>,
        /// Index into the router's model slug list.
        model_idx: usize,
        /// Scalar reward computed from gate outcome.
        reward: f64,
        /// Whether the task gated successfully.
        success: bool,
        /// Unix timestamp in milliseconds.
        ts_ms: i64,
    },
    /// A prompt experiment trial outcome.
    ExperimentOutcome {
        /// Variant ID that was tested.
        variant_id: String,
        /// Whether the variant succeeded.
        success: bool,
        /// Unix timestamp in milliseconds.
        ts_ms: i64,
    },
    /// A gate rung adaptive threshold EMA update.
    GateThresholdUpdate {
        /// Gate rung index (0-based).
        rung: u32,
        /// Whether the rung passed.
        passed: bool,
        /// Unix timestamp in milliseconds.
        ts_ms: i64,
    },
}

/// Append-only WAL writer backed by a plain `File`.
///
/// Writes are synchronous and call `sync_data()` after each append to
/// guarantee durability. Do NOT wrap in `BufWriter` -- buffering would
/// defeat the crash-safety guarantee.
pub struct WalWriter {
    path: PathBuf,
    file: File,
    entry_count: usize,
}

impl WalWriter {
    /// Open the WAL at `path`, creating it if absent.
    ///
    /// Returns both the writer and the existing entry count (so the
    /// caller can decide whether to replay before proceeding).
    pub fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Count existing entries before opening in append mode.
        let entry_count = if path.exists() {
            BufReader::new(File::open(path)?).lines().count()
        } else {
            0
        };
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            entry_count,
        })
    }

    /// Append `entry` to the WAL, flushing to the OS page cache and
    /// syncing the data to durable storage before returning.
    pub fn append(&mut self, entry: &WalEntry) -> io::Result<()> {
        let mut line = serde_json::to_string(entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.sync_data()?;
        self.entry_count += 1;
        Ok(())
    }

    /// Number of entries written since the WAL was last truncated.
    pub fn entry_count(&self) -> usize {
        self.entry_count
    }

    /// Truncate the WAL to zero after a successful snapshot save.
    ///
    /// Reopens the file with `O_TRUNC` so that subsequent appends start
    /// from an empty file. `entry_count` resets to zero.
    pub fn truncate(&mut self) -> io::Result<()> {
        // Close the append-mode handle, open truncate, then reopen append.
        let _ = std::mem::replace(
            &mut self.file,
            OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.path)?,
        );
        self.file = OpenOptions::new().append(true).open(&self.path)?;
        self.entry_count = 0;
        Ok(())
    }
}

/// Read and deserialize all entries from `path`.
///
/// Malformed lines are logged as warnings and skipped rather than
/// returning an error, so that a partially-written tail entry (from a
/// crash during `write_all`) does not block replay.
///
/// Returns an empty `Vec` if the WAL file does not exist.
pub fn replay_wal(path: &Path) -> io::Result<Vec<WalEntry>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    let mut entries = Vec::new();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<WalEntry>(&line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                tracing::warn!(line = i, error = %e, "[wal] skipping malformed entry");
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn append_and_reopen_preserves_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.jsonl");

        {
            let mut w = WalWriter::open(&path).unwrap();
            assert_eq!(w.entry_count(), 0);

            w.append(&WalEntry::CascadeObservation {
                model_slug: "claude-sonnet-4-5".into(),
                context_features: vec![0.1, 0.2, 0.3],
                model_idx: 0,
                reward: 0.85,
                success: true,
                ts_ms: 1_000_000,
            })
            .unwrap();

            w.append(&WalEntry::GateThresholdUpdate {
                rung: 2,
                passed: false,
                ts_ms: 1_000_001,
            })
            .unwrap();

            assert_eq!(w.entry_count(), 2);
        }

        // Reopen and verify count survives.
        let w2 = WalWriter::open(&path).unwrap();
        assert_eq!(w2.entry_count(), 2);

        // Replay should produce both entries.
        let entries = replay_wal(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0], WalEntry::CascadeObservation { .. }));
        assert!(matches!(entries[1], WalEntry::GateThresholdUpdate { .. }));
    }

    #[test]
    fn truncate_resets_file_and_count() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.jsonl");

        let mut w = WalWriter::open(&path).unwrap();
        w.append(&WalEntry::ExperimentOutcome {
            variant_id: "v1".into(),
            success: true,
            ts_ms: 42,
        })
        .unwrap();
        assert_eq!(w.entry_count(), 1);

        w.truncate().unwrap();
        assert_eq!(w.entry_count(), 0);

        // File should be empty.
        let entries = replay_wal(&path).unwrap();
        assert!(entries.is_empty());

        // Can still append after truncation.
        w.append(&WalEntry::GateThresholdUpdate {
            rung: 0,
            passed: true,
            ts_ms: 43,
        })
        .unwrap();
        assert_eq!(w.entry_count(), 1);
        let entries = replay_wal(&path).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn malformed_tail_is_skipped() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.jsonl");

        // Write a valid entry followed by a malformed line.
        let mut w = WalWriter::open(&path).unwrap();
        w.append(&WalEntry::CascadeObservation {
            model_slug: "claude-haiku-4-5".into(),
            context_features: vec![1.0],
            model_idx: 1,
            reward: 0.5,
            success: false,
            ts_ms: 99,
        })
        .unwrap();
        drop(w);

        // Append a broken line directly.
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"kind\":\"cascade_observation\",\"broken\n")
            .unwrap();

        let entries = replay_wal(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0], WalEntry::CascadeObservation { .. }));
    }

    #[test]
    fn replay_missing_file_returns_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.jsonl");
        let entries = replay_wal(&path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn serde_roundtrip_all_variants() {
        let entries = vec![
            WalEntry::CascadeObservation {
                model_slug: "model-a".into(),
                context_features: vec![0.0; 18],
                model_idx: 0,
                reward: 1.0,
                success: true,
                ts_ms: 100,
            },
            WalEntry::ExperimentOutcome {
                variant_id: "variant-b".into(),
                success: false,
                ts_ms: 200,
            },
            WalEntry::GateThresholdUpdate {
                rung: 5,
                passed: true,
                ts_ms: 300,
            },
        ];
        for entry in &entries {
            let json = serde_json::to_string(entry).unwrap();
            let deserialized: WalEntry = serde_json::from_str(&json).unwrap();
            // Verify kind tag is present.
            let val: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert!(val.get("kind").is_some());
            // Verify roundtrip produces same JSON.
            let json2 = serde_json::to_string(&deserialized).unwrap();
            assert_eq!(json, json2);
        }
    }
}
