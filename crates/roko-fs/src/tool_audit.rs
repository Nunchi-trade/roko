//! Append-only JSONL audit log for tool dispatches (§36.52).
//!
//! Every admitted tool call emits one `{"kind":"admit",...}` line;
//! every terminal result emits one `{"kind":"result",...}` line. The
//! file is safe to `tail -f` for live observability and safe to replay
//! for post-hoc audit.
//!
//! # Design
//!
//! - Single file: `<root>/.roko/tool_audit.jsonl`.
//! - Append-only, create-if-missing, line-buffered via `BufWriter`.
//! - `tokio::sync::Mutex<BufWriter<File>>` — short critical section.
//! - Pure sink: records facts, never rejects or mutates the call.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;

use roko_core::tool::{ToolCall, ToolResult};

/// The default audit-log path relative to the worktree.
pub const DEFAULT_AUDIT_PATH: &str = ".roko/tool_audit.jsonl";

// ─── Wire types ───────────────────────────────────────────────────────────────

/// A single audit-log line. The `kind` discriminator is written as a JSON
/// field so consumers can distinguish admit lines from result lines while
/// `tail -f`-ing a single file.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AuditLine<'a> {
    Admit {
        ts_ms: i64,
        call: &'a ToolCall,
    },
    Result {
        ts_ms: i64,
        call_id: &'a str,
        call_name: &'a str,
        result: &'a ToolResult,
    },
}

// ─── ToolAuditLog ─────────────────────────────────────────────────────────────

/// Append-only JSONL audit log for tool dispatches.
///
/// Cheap to clone via [`std::sync::Arc`]; wrap in `Arc<ToolAuditLog>` to
/// share across tasks.
pub struct ToolAuditLog {
    path: PathBuf,
    writer: Mutex<BufWriter<tokio::fs::File>>,
}

impl std::fmt::Debug for ToolAuditLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolAuditLog")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl ToolAuditLog {
    /// Open (or create) the audit log under `<root>/.roko/tool_audit.jsonl`.
    ///
    /// Creates `.roko/` if it does not exist. Opens the file with
    /// `create(true).append(true)` so existing records are preserved.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file
    /// cannot be opened.
    pub async fn open(root: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = root.as_ref().join(DEFAULT_AUDIT_PATH);
        Self::open_at(path).await
    }

    /// Open the audit log at an explicit path (no `.roko/` suffix is added).
    ///
    /// Parent directories are created if missing.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file
    /// cannot be opened.
    pub async fn open_at(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        Ok(Self {
            path,
            writer: Mutex::new(BufWriter::new(file)),
        })
    }

    /// Path to the underlying JSONL file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Record that a call was admitted for dispatch.
    ///
    /// Writes one `{"kind":"admit","ts_ms":…,"call":…}` line and flushes.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or the write fails.
    pub async fn record_admit(&self, call: &ToolCall) -> std::io::Result<()> {
        let ts_ms = chrono::Utc::now().timestamp_millis();
        let line = AuditLine::Admit { ts_ms, call };
        // Serialize synchronously before the first .await so the future is Send.
        let bytes = Self::serialize_line(&line)?;
        self.write_line(bytes).await
    }

    /// Record the terminal result of a dispatched call.
    ///
    /// Writes one `{"kind":"result","ts_ms":…,"call_id":…,"call_name":…,"result":…}` line
    /// and flushes. The `call_id` and `call_name` are copied out of the
    /// `ToolCall` so consumers can correlate without loading the full call.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or the write fails.
    pub async fn record_result(&self, call: &ToolCall, result: &ToolResult) -> std::io::Result<()> {
        let ts_ms = chrono::Utc::now().timestamp_millis();
        let line = AuditLine::Result {
            ts_ms,
            call_id: &call.id,
            call_name: &call.name,
            result,
        };
        // Serialize synchronously before the first .await so the future is Send.
        let bytes = Self::serialize_line(&line)?;
        self.write_line(bytes).await
    }

    /// Flush the internal buffer to the OS.
    ///
    /// `record_admit` and `record_result` flush automatically after each
    /// write. This method is exposed for callers that need a guaranteed
    /// flush at a specific point (e.g. before process exit).
    ///
    /// # Errors
    ///
    /// Returns an error if the flush fails.
    pub async fn flush(&self) -> std::io::Result<()> {
        self.writer.lock().await.flush().await
    }

    // ─── private ─────────────────────────────────────────────────────────────

    /// Serialize `value` to JSONL bytes (with trailing `\n`) synchronously,
    /// then write + flush under the async mutex.
    ///
    /// Serialization happens before the `.await` point so the future holds
    /// only `Vec<u8>` across the await, which is `Send`.
    async fn write_line(&self, bytes: Vec<u8>) -> std::io::Result<()> {
        let mut guard = self.writer.lock().await;
        guard.write_all(&bytes).await?;
        guard.flush().await
    }

    fn serialize_line(value: &impl Serialize) -> std::io::Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    // ── helpers ────────────────────────────────────────────────────────────

    fn make_call(id: &str, name: &str) -> ToolCall {
        ToolCall::at(id, name, serde_json::json!({"x": 1}), 1_700_000_000_000)
    }

    fn make_result_ok() -> ToolResult {
        ToolResult::text("output text")
    }

    async fn read_lines(path: &Path) -> Vec<String> {
        let contents = tokio::fs::read_to_string(path).await.expect("read file");
        contents
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(String::from)
            .collect()
    }

    // ── 1 ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn open_creates_jsonl_file_under_roko_dir() {
        let dir = TempDir::new().expect("tempdir");
        let log = ToolAuditLog::open(dir.path()).await.expect("open");

        assert_eq!(log.path(), dir.path().join(".roko/tool_audit.jsonl"));
        // File is created eagerly (on open, not on first write).
        assert!(log.path().exists(), "file should exist after open");
    }

    // ── 2 ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn record_admit_writes_one_line() {
        let dir = TempDir::new().expect("tempdir");
        let log = ToolAuditLog::open(dir.path()).await.expect("open");
        let call = make_call("c1", "read_file");

        log.record_admit(&call).await.expect("record_admit");

        let lines = read_lines(log.path()).await;
        assert_eq!(lines.len(), 1, "expected exactly 1 line, got: {lines:?}");
    }

    // ── 3 ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn record_result_writes_one_line() {
        let dir = TempDir::new().expect("tempdir");
        let log = ToolAuditLog::open(dir.path()).await.expect("open");
        let call = make_call("c2", "write_file");
        let result = make_result_ok();

        log.record_result(&call, &result)
            .await
            .expect("record_result");

        let lines = read_lines(log.path()).await;
        assert_eq!(lines.len(), 1, "expected exactly 1 line, got: {lines:?}");
    }

    // ── 4 ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn admit_line_has_kind_admit_and_call_fields() {
        let dir = TempDir::new().expect("tempdir");
        let log = ToolAuditLog::open(dir.path()).await.expect("open");
        let call = make_call("c3", "bash");

        log.record_admit(&call).await.expect("record_admit");

        let lines = read_lines(log.path()).await;
        let json: serde_json::Value = serde_json::from_str(&lines[0]).expect("parse line as JSON");

        assert_eq!(json["kind"], "admit", "kind must be 'admit'");
        assert!(json["ts_ms"].is_i64(), "ts_ms must be an integer");
        assert_eq!(json["call"]["id"], "c3", "call.id must match");
        assert_eq!(json["call"]["name"], "bash", "call.name must match");
    }

    // ── 5 ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn result_line_has_kind_result_and_call_id() {
        let dir = TempDir::new().expect("tempdir");
        let log = ToolAuditLog::open(dir.path()).await.expect("open");
        let call = make_call("c4", "grep");
        let result = make_result_ok();

        log.record_result(&call, &result)
            .await
            .expect("record_result");

        let lines = read_lines(log.path()).await;
        let json: serde_json::Value = serde_json::from_str(&lines[0]).expect("parse line as JSON");

        assert_eq!(json["kind"], "result", "kind must be 'result'");
        assert_eq!(json["call_id"], "c4", "call_id must match");
        assert_eq!(json["call_name"], "grep", "call_name must match");
        assert!(json["ts_ms"].is_i64(), "ts_ms must be an integer");
        assert!(json["result"].is_object(), "result must be an object");
    }

    // ── 6 ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn interleaved_writes_preserve_order() {
        let dir = TempDir::new().expect("tempdir");
        let log = ToolAuditLog::open(dir.path()).await.expect("open");
        let call_a = make_call("a", "read_file");
        let call_b = make_call("b", "write_file");
        let result = make_result_ok();

        log.record_admit(&call_a).await.expect("admit a");
        log.record_result(&call_a, &result).await.expect("result a");
        log.record_admit(&call_b).await.expect("admit b");
        log.record_result(&call_b, &result).await.expect("result b");

        let lines = read_lines(log.path()).await;
        assert_eq!(lines.len(), 4, "expected 4 lines");

        let kinds: Vec<&str> = lines
            .iter()
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).expect("valid json");
                match v["kind"].as_str().expect("kind string") {
                    "admit" => "admit",
                    "result" => "result",
                    other => panic!("unexpected kind: {other}"),
                }
            })
            .collect();
        assert_eq!(kinds, ["admit", "result", "admit", "result"]);
    }

    // ── 7 ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn reopen_appends_not_truncates() {
        let dir = TempDir::new().expect("tempdir");

        // First session: 3 lines.
        {
            let log = ToolAuditLog::open(dir.path()).await.expect("open first");
            let call = make_call("x", "glob");
            let result = make_result_ok();
            log.record_admit(&call).await.expect("admit 1");
            log.record_admit(&call).await.expect("admit 2");
            log.record_result(&call, &result).await.expect("result 1");
        }

        // Second session: 2 more lines.
        {
            let log = ToolAuditLog::open(dir.path()).await.expect("open second");
            let call = make_call("y", "ls");
            let result = make_result_ok();
            log.record_admit(&call).await.expect("admit 3");
            log.record_result(&call, &result).await.expect("result 2");
        }

        // Must see all 5 lines.
        let path = dir.path().join(".roko/tool_audit.jsonl");
        let lines = read_lines(&path).await;
        assert_eq!(lines.len(), 5, "expected 5 total lines after reopen");
    }

    // ── 8 ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn nested_subdir_already_exists_is_ok() {
        let dir = TempDir::new().expect("tempdir");
        // Pre-create the `.roko/` directory.
        tokio::fs::create_dir_all(dir.path().join(".roko"))
            .await
            .expect("pre-create .roko");

        let log = ToolAuditLog::open(dir.path())
            .await
            .expect("open with pre-existing dir");
        let call = make_call("pre", "bash");
        log.record_admit(&call).await.expect("record_admit");

        let lines = read_lines(log.path()).await;
        assert_eq!(lines.len(), 1);
    }

    // ── 9 ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn open_at_uses_exact_path() {
        let dir = TempDir::new().expect("tempdir");
        let explicit = dir.path().join("my_custom_audit.jsonl");

        let log = ToolAuditLog::open_at(&explicit).await.expect("open_at");
        assert_eq!(
            log.path(),
            explicit,
            "path must match exactly — no .roko/ suffix"
        );

        let call = make_call("e", "edit_file");
        log.record_admit(&call).await.expect("record_admit");

        // The file lives at the explicit path, NOT under .roko/.
        assert!(explicit.exists(), "file must exist at explicit path");
        assert!(
            !dir.path().join(".roko/my_custom_audit.jsonl").exists(),
            "must not create .roko/ suffix"
        );
        let lines = read_lines(&explicit).await;
        assert_eq!(lines.len(), 1);
    }

    // ── 10 ─────────────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_writes_do_not_interleave_bytes() {
        let dir = TempDir::new().expect("tempdir");
        let log = Arc::new(ToolAuditLog::open(dir.path()).await.expect("open"));

        let tasks = 10usize;
        let admits_per_task = 5usize;

        let mut handles = Vec::with_capacity(tasks);
        for task_idx in 0..tasks {
            let log = Arc::clone(&log);
            handles.push(tokio::spawn(async move {
                for call_idx in 0..admits_per_task {
                    let call = make_call(&format!("t{task_idx}-c{call_idx}"), "bash");
                    log.record_admit(&call).await.expect("concurrent admit");
                }
            }));
        }
        for handle in handles {
            handle.await.expect("task panicked");
        }

        let lines = read_lines(log.path()).await;
        let expected = tasks * admits_per_task;
        assert_eq!(
            lines.len(),
            expected,
            "expected {expected} lines, got {}",
            lines.len()
        );

        // Every line must parse as valid JSON — no torn writes.
        for (i, line) in lines.iter().enumerate() {
            let _: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("line {i} is not valid JSON: {e}\nLine: {line}"));
        }
    }

    // ── 11 ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ts_ms_is_reasonable() {
        let dir = TempDir::new().expect("tempdir");
        let log = ToolAuditLog::open(dir.path()).await.expect("open");
        let call = make_call("ts", "read_file");

        let before = chrono::Utc::now().timestamp_millis();
        log.record_admit(&call).await.expect("record_admit");
        let after = chrono::Utc::now().timestamp_millis();

        let lines = read_lines(log.path()).await;
        let json: serde_json::Value = serde_json::from_str(&lines[0]).expect("parse JSON");
        let ts_ms = json["ts_ms"].as_i64().expect("ts_ms is i64");

        assert!(
            ts_ms >= before && ts_ms <= after + 1_000,
            "ts_ms {ts_ms} should be within 1s of now (before={before}, after={after})"
        );
    }

    // ── 12 ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn each_line_parses_as_valid_json() {
        let dir = TempDir::new().expect("tempdir");
        let log = ToolAuditLog::open(dir.path()).await.expect("open");
        let call_a = make_call("j1", "bash");
        let call_b = make_call("j2", "glob");
        let result = ToolResult::err(roko_core::tool::ToolError::Cancelled);

        log.record_admit(&call_a).await.expect("admit a");
        log.record_result(&call_a, &make_result_ok())
            .await
            .expect("result a");
        log.record_admit(&call_b).await.expect("admit b");
        log.record_result(&call_b, &result)
            .await
            .expect("result b (err)");

        let lines = read_lines(log.path()).await;
        assert_eq!(lines.len(), 4, "expected 4 lines");
        for (i, line) in lines.iter().enumerate() {
            let _: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("line {i} is invalid JSON: {e}\nLine: {line}"));
        }
    }
}
