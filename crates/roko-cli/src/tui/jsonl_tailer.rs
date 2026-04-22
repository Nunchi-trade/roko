//! Generic incremental JSONL tailer with deserialization.
//!
//! Builds on [`JsonlCursor`] to provide a typed, accumulating reader for
//! append-only JSONL files. The TUI's `DashboardData::tick()` currently
//! re-reads entire files on every tick (O(N) on file size). This module
//! provides infrastructure to replace those hot paths with an incremental
//! reader that only deserializes newly appended lines.
//!
//! # Usage
//!
//! ```ignore
//! use roko_cli::tui::jsonl_tailer::IncrementalTailer;
//!
//! let mut tailer: IncrementalTailer<EfficiencyEvent> =
//!     IncrementalTailer::new(".roko/learn/efficiency.jsonl");
//!
//! // On each TUI tick:
//! let new_count = tailer.tick()?;
//! if new_count > 0 {
//!     // Only process new items.
//!     for item in tailer.items().iter().rev().take(new_count) {
//!         // ...
//!     }
//! }
//! ```

use std::path::PathBuf;

use super::jsonl_cursor::JsonlCursor;

/// Accumulating, incremental reader for typed JSONL files.
///
/// Wraps a [`JsonlCursor`] and deserializes each new line into `T`,
/// accumulating all successfully parsed items in an internal `Vec`.
/// Malformed lines are silently skipped (logged at trace level).
pub struct IncrementalTailer<T> {
    cursor: JsonlCursor,
    items: Vec<T>,
    /// Number of lines that failed deserialization (cumulative).
    pub parse_errors: usize,
}

impl<T: serde::de::DeserializeOwned> IncrementalTailer<T> {
    /// Create a new tailer for the given JSONL file.
    ///
    /// No I/O happens until [`tick`](Self::tick) is called.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            cursor: JsonlCursor::new(path),
            items: Vec::new(),
            parse_errors: 0,
        }
    }

    /// Read and deserialize newly appended lines since the last tick.
    ///
    /// Returns the number of *successfully parsed* new items added.
    /// On file truncation, the cursor resets and previously accumulated
    /// items are cleared so the file is re-read from the beginning.
    pub fn tick(&mut self) -> std::io::Result<usize> {
        let prev_offset = self.cursor.offset();
        let raw_lines = self.cursor.read_new_lines()?;

        // Detect truncation: cursor reset its offset below our previous.
        if self.cursor.offset() < prev_offset && raw_lines.is_empty() {
            // File was truncated but no new lines yet — clear accumulator
            // and wait for the next tick to pick up fresh data.
            self.items.clear();
            self.parse_errors = 0;
            return Ok(0);
        }

        // If the cursor read lines starting from 0 and we had items,
        // it means a truncation happened and the cursor re-read from start.
        if prev_offset > 0 && self.cursor.offset() > 0 && !raw_lines.is_empty() {
            // Check if cursor internally reset (offset moved backward from
            // our perspective). The cursor handles the reset internally;
            // we detect it by checking if new offset < old offset + new bytes.
            let new_bytes: u64 = raw_lines.iter().map(|l| l.len() as u64 + 1).sum();
            if self.cursor.offset() == new_bytes && prev_offset > new_bytes {
                self.items.clear();
                self.parse_errors = 0;
            }
        }

        let mut added = 0;
        for line in &raw_lines {
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<T>(line) {
                Ok(item) => {
                    self.items.push(item);
                    added += 1;
                }
                Err(_e) => {
                    self.parse_errors += 1;
                    tracing::trace!(
                        path = %self.cursor_path(),
                        error = %_e,
                        "skipping malformed JSONL line"
                    );
                }
            }
        }

        Ok(added)
    }

    /// All successfully parsed items accumulated so far.
    pub fn items(&self) -> &[T] {
        &self.items
    }

    /// Number of accumulated items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether no items have been accumulated.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Current byte offset into the file.
    pub fn offset(&self) -> u64 {
        self.cursor.offset()
    }

    /// Path being tailed (for diagnostics).
    fn cursor_path(&self) -> String {
        // JsonlCursor doesn't expose path directly, so we reconstruct
        // from the debug repr. This is only used for trace logging.
        format!("{:?}", self.cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use tempfile::tempdir;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestEvent {
        kind: String,
        value: i64,
    }

    fn append(path: &std::path::Path, text: &str) {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open for append");
        file.write_all(text.as_bytes()).expect("append bytes");
    }

    #[test]
    fn reads_and_deserializes_incrementally() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("events.jsonl");
        append(&path, r#"{"kind":"a","value":1}"#);
        append(&path, "\n");

        let mut tailer: IncrementalTailer<TestEvent> = IncrementalTailer::new(&path);

        let n = tailer.tick().expect("first tick");
        assert_eq!(n, 1);
        assert_eq!(tailer.len(), 1);
        assert_eq!(tailer.items()[0].kind, "a");

        // Append more.
        append(&path, r#"{"kind":"b","value":2}"#);
        append(&path, "\n");
        append(&path, r#"{"kind":"c","value":3}"#);
        append(&path, "\n");

        let n = tailer.tick().expect("second tick");
        assert_eq!(n, 2);
        assert_eq!(tailer.len(), 3);

        // Idle tick.
        let n = tailer.tick().expect("idle tick");
        assert_eq!(n, 0);
        assert_eq!(tailer.len(), 3);
    }

    #[test]
    fn skips_malformed_lines() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("events.jsonl");
        append(&path, r#"{"kind":"ok","value":1}"#);
        append(&path, "\n");
        append(&path, "NOT VALID JSON\n");
        append(&path, r#"{"kind":"ok2","value":2}"#);
        append(&path, "\n");

        let mut tailer: IncrementalTailer<TestEvent> = IncrementalTailer::new(&path);
        let n = tailer.tick().expect("tick");
        assert_eq!(n, 2);
        assert_eq!(tailer.parse_errors, 1);
        assert_eq!(tailer.items()[0].kind, "ok");
        assert_eq!(tailer.items()[1].kind, "ok2");
    }

    #[test]
    fn handles_missing_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("nonexistent.jsonl");

        let mut tailer: IncrementalTailer<TestEvent> = IncrementalTailer::new(&path);
        let n = tailer.tick().expect("missing file tick");
        assert_eq!(n, 0);
        assert!(tailer.is_empty());
    }

    #[test]
    fn handles_truncation() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("events.jsonl");
        append(&path, r#"{"kind":"a","value":1}"#);
        append(&path, "\n");
        append(&path, r#"{"kind":"b","value":2}"#);
        append(&path, "\n");

        let mut tailer: IncrementalTailer<TestEvent> = IncrementalTailer::new(&path);
        let n = tailer.tick().expect("first tick");
        assert_eq!(n, 2);

        // Truncate and write fresh data.
        fs::write(&path, r#"{"kind":"fresh","value":99}"#.to_owned() + "\n").expect("truncate");

        let n = tailer.tick().expect("after truncation");
        assert_eq!(n, 1);
        // Old items should be cleared, only fresh remains.
        assert_eq!(tailer.len(), 1);
        assert_eq!(tailer.items()[0].kind, "fresh");
    }
}
