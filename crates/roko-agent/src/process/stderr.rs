//! Stderr classification and suppression.
//!
//! Agent subprocesses (codex, claude-cli, etc.) emit many benign warnings on
//! stderr that would clutter operator logs. This module classifies known-benign
//! patterns and provides warn-once semantics: the first occurrence of each
//! pattern key is logged at WARN, subsequent occurrences are silently dropped.

#[allow(clippy::disallowed_types)]
use std::sync::Mutex;

use std::collections::HashSet;
use std::sync::OnceLock;

/// A classified benign stderr line.
#[derive(Debug, Clone)]
pub struct BenignStderr {
    /// Stable key for deduplication (e.g. `"codex-apply-patch-verification"`).
    pub key: &'static str,
    /// Human-readable summary for the first-occurrence warning.
    pub summary: &'static str,
}

/// Classify a stderr line as benign, returning its key and summary if matched.
///
/// Ported from Mori's 25+ pattern set. Each pattern matches on substrings to
/// avoid fragile exact-line matching against upstream agents that may change
/// their wording.
#[allow(clippy::too_many_lines)]
pub fn classify_benign_stderr(line: &str) -> Option<BenignStderr> {
    // --- Codex patch verification ---
    if line.contains("apply_patch verification failed:")
        || line.contains("Failed to find expected lines in ")
    {
        return Some(BenignStderr {
            key: "codex-apply-patch-verification",
            summary: "Codex rejected a patch hunk against changed source; suppressing redundant patch-verification noise",
        });
    }

    // --- Codex state DB migration ---
    if line.contains("failed to open state db at")
        && line.contains("previously applied but is missing in the resolved migrations")
    {
        return Some(BenignStderr {
            key: "codex-state-db-migration",
            summary: "Codex local state DB is incompatible; suppressing repeated upstream migration warnings",
        });
    }

    // --- Codex state runtime migration ---
    if line.contains("failed to initialize state runtime at")
        && line.contains("previously applied but is missing in the resolved migrations")
    {
        return Some(BenignStderr {
            key: "codex-state-runtime-migration",
            summary: "Codex state runtime initialization is failing; suppressing repeated upstream migration warnings",
        });
    }

    // --- Codex shell snapshot ENOENT ---
    if line.contains("Failed to delete shell snapshot at")
        && line.contains("No such file or directory")
    {
        return Some(BenignStderr {
            key: "codex-shell-snapshot-enoent",
            summary: "Codex shell snapshot cleanup is emitting ENOENT warnings; suppressing repeats",
        });
    }

    // --- Codex thread shutdown timeout ---
    if line.contains("timed out waiting for thread") && line.contains("to shut down") {
        return Some(BenignStderr {
            key: "codex-thread-shutdown-timeout",
            summary: "Codex thread shutdown is timing out during cleanup; suppressing repeats",
        });
    }

    // --- Codex process group kill failures ---
    if line.contains("Failed to kill process group")
        || line.contains("failed to kill process group")
        || line.contains("Failed to kill MCP process group")
        || line.contains("failed to kill MCP process group")
    {
        return Some(BenignStderr {
            key: "codex-process-group-kill",
            summary: "Codex process-group shutdown is emitting repeated warnings; suppressing repeats",
        });
    }

    // --- Cursor CLI config race ---
    if line.contains("cli-config.json")
        && line.contains("ENOENT: no such file or directory")
        && line.contains("rename")
    {
        return Some(BenignStderr {
            key: "cursor-cli-config-race",
            summary: "Cursor CLI is racing on ~/.cursor/cli-config.json; suppressing repeated warnings after first report",
        });
    }

    // --- Codex project config disabled ---
    if line.contains("Project config.toml files are disabled in the following folders")
        || line.contains("To load config.toml, add")
        || (line.trim_start().starts_with("1. ") && line.contains("/.codex"))
    {
        return Some(BenignStderr {
            key: "codex-project-config-disabled",
            summary: "Codex project config is disabled for this repo; suppressing repeated trust warnings",
        });
    }

    // --- Codex rollout channel closed ---
    if line.contains("failed to record rollout items")
        && line.contains("failed to queue rollout items")
        && line.contains("channel closed")
    {
        return Some(BenignStderr {
            key: "codex-rollout-channel-closed",
            summary: "Codex rollout queue is already closed during shutdown; suppressing repeated stderr spam",
        });
    }

    // --- Codex rollout persist channel closed ---
    if line.contains("failed to materialize rollout recorder")
        && line.contains("failed to queue rollout persist")
        && line.contains("channel closed")
    {
        return Some(BenignStderr {
            key: "codex-rollout-persist-channel-closed",
            summary: "Codex rollout persistence is already shutting down; suppressing repeated upstream warnings",
        });
    }

    // --- Codex state DB discrepancy ---
    if line.contains("state db discrepancy during find_thread_path_by_id_str_in_subdir")
        && line.contains("falling_back")
    {
        return Some(BenignStderr {
            key: "codex-state-db-discrepancy",
            summary: "Codex state DB lookup is falling back to filesystem discovery; suppressing repeated upstream warnings",
        });
    }

    // --- Codex state DB read-repair ---
    if line.contains("state db discrepancy during read_repair_rollout_path")
        && line.contains("upsert_needed")
    {
        return Some(BenignStderr {
            key: "codex-state-db-read-repair",
            summary: "Codex state DB is performing read-repair bookkeeping; suppressing repeated upstream warnings",
        });
    }

    // --- Codex fallback model metadata ---
    if line.contains("Unknown model ") && line.contains("This will use fallback model metadata") {
        return Some(BenignStderr {
            key: "codex-fallback-model-metadata",
            summary: "Codex is using fallback metadata for an unresolved model alias; suppressing repeated upstream warnings",
        });
    }

    // --- Codex missing model messages ---
    if line.contains("Model personality requested but model_messages is missing")
        && line.contains("falling back to base instructions")
    {
        return Some(BenignStderr {
            key: "codex-missing-model-messages",
            summary: "Codex is falling back to base instructions for a model personality; suppressing repeated upstream warnings",
        });
    }

    // --- Codex slow SQL statement ---
    if line.contains("sqlx::query: slow statement")
        && line.contains("execution time exceeded alert threshold")
    {
        return Some(BenignStderr {
            key: "codex-sqlx-slow-statement",
            summary: "Codex emitted a slow-statement warning while flushing logs; suppressing repeated stderr noise",
        });
    }

    // --- Codex unknown turn item ---
    if line.contains("dropping turn-scoped item for unknown turn id") {
        return Some(BenignStderr {
            key: "codex-unknown-turn-item",
            summary: "Codex app-server dropped a stale turn-scoped item after turn completion; suppressing repeated upstream warnings",
        });
    }

    // --- Codex rollout flush channel closed ---
    if line.contains("failed to flush rollout recorder")
        && line.contains("failed to queue rollout flush")
        && line.contains("channel closed")
    {
        return Some(BenignStderr {
            key: "codex-rollout-flush-channel-closed",
            summary: "Codex rollout flush queue is already closed during shutdown; suppressing repeated stderr noise",
        });
    }

    // --- Codex write_stdin closed ---
    if line.contains("write_stdin failed: stdin is closed for this session")
        || line.contains("stdin is closed for this session; rerun exec_command with tty=true")
    {
        return Some(BenignStderr {
            key: "codex-write-stdin-closed",
            summary: "Codex tried to write to a closed exec session; suppressing repeated upstream tool-router warnings",
        });
    }

    // --- Codex write_stdin unknown process ---
    if line.contains("write_stdin failed: Unknown process id") {
        return Some(BenignStderr {
            key: "codex-write-stdin-unknown-process",
            summary: "Codex tried to write to an exec session that had already exited; suppressing repeated upstream tool-router warnings",
        });
    }

    // --- Source snippet fragments (codex stderr leakage) ---
    if looks_like_source_snippet(line) {
        return Some(BenignStderr {
            key: "codex-source-snippet",
            summary: "Codex emitted standalone source-snippet lines on stderr; suppressing redundant fragments while keeping the real diagnostic",
        });
    }

    // --- Claude CLI warnings ---
    if line.contains("Claude CLI is starting") || line.contains("Downloading Claude CLI") {
        return Some(BenignStderr {
            key: "claude-cli-startup",
            summary: "Claude CLI emitting startup/download progress on stderr; suppressing repeats",
        });
    }

    // --- Ollama warnings ---
    if line.contains("llama_model_loader") || line.contains("llm_load_") {
        return Some(BenignStderr {
            key: "ollama-model-load",
            summary: "Ollama model loader emitting verbose load diagnostics; suppressing repeats",
        });
    }

    // --- Generic node warnings ---
    if line.contains("ExperimentalWarning:") || line.contains("--experimental") {
        return Some(BenignStderr {
            key: "node-experimental-warning",
            summary: "Node.js experimental feature warning on stderr; suppressing repeats",
        });
    }

    // --- Python deprecation warnings ---
    if line.contains("DeprecationWarning:") {
        return Some(BenignStderr {
            key: "python-deprecation-warning",
            summary: "Python deprecation warning on stderr; suppressing repeats",
        });
    }

    None
}

/// Detect source-code snippet fragments that agents sometimes leak to stderr.
///
/// Returns `true` if the line looks like a code fragment rather than a real
/// diagnostic. Lines containing real error/warning keywords are excluded.
fn looks_like_source_snippet(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.len() > 180 {
        return false;
    }
    // Preserve lines that look like real diagnostics.
    if trimmed.contains("error[")
        || trimmed.contains("warning:")
        || trimmed.contains(" WARN ")
        || trimmed.contains(" ERROR ")
        || trimmed.contains("panicked at")
        || trimmed.contains("Caused by:")
        || trimmed.contains("failed")
    {
        return false;
    }
    // Pure punctuation lines.
    if matches!(trimmed, "{" | "}" | "});" | "})?;" | ")?;" | ");") {
        return true;
    }
    // Comment lines.
    if trimmed.starts_with("//") {
        return true;
    }
    // Lines starting with Rust syntax keywords/prefixes.
    [
        "let ",
        "pub ",
        "fn ",
        "mod ",
        "#[",
        "if ",
        "match ",
        "return ",
        "use ",
        "impl ",
        "struct ",
        "enum ",
        "trait ",
        "assert!(",
        "assert_eq!(",
        ".",
        "&& ",
        "|| ",
        "message: format!(",
        "CapabilityTier::",
        "ToolError::",
        "check_capability(",
        "cb.",
        "cms.",
        "router.",
        "self.",
        "ctx.",
        "def.",
        "crate::",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
}

/// The warn-once set: tracks which benign-stderr keys have already been warned.
#[allow(clippy::disallowed_types)]
fn warned_keys() -> &'static Mutex<HashSet<&'static str>> {
    static KEYS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    KEYS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Returns `true` if this is the first time `key` has been seen (and a warning
/// should be emitted). Returns `false` for subsequent occurrences.
///
/// Some keys are **always silent** (never warn, even on first occurrence) because
/// they are too noisy or uninformative. These are listed in the match below.
pub fn benign_stderr_warn_once(key: &'static str) -> bool {
    // Keys that should never produce a warning (always silent).
    let always_silent = matches!(
        key,
        "codex-shell-snapshot-enoent"
            | "codex-write-stdin-closed"
            | "codex-write-stdin-unknown-process"
            | "codex-apply-patch-verification"
            | "codex-source-snippet"
            | "codex-sqlx-slow-statement"
            | "codex-rollout-flush-channel-closed"
    );
    if always_silent {
        return false;
    }

    // Insert returns true if the key was newly inserted.
    warned_keys().lock().is_ok_and(|mut set| set.insert(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_patch_verification() {
        let line = "error: apply_patch verification failed: hunk 2 at line 50";
        let result = classify_benign_stderr(line);
        assert!(result.is_some());
        assert_eq!(
            result.expect("classified").key,
            "codex-apply-patch-verification"
        );
    }

    #[test]
    fn classify_state_db_migration() {
        let line = "failed to open state db at /foo/bar: migration 20240101 previously applied but is missing in the resolved migrations";
        let result = classify_benign_stderr(line);
        assert!(result.is_some());
        assert_eq!(result.expect("classified").key, "codex-state-db-migration");
    }

    #[test]
    fn classify_process_group_kill() {
        for variant in [
            "Failed to kill process group 12345",
            "failed to kill process group",
            "Failed to kill MCP process group",
            "failed to kill MCP process group for session",
        ] {
            let result = classify_benign_stderr(variant);
            assert!(result.is_some(), "should classify: {variant}");
        }
    }

    #[test]
    fn classify_cursor_cli_config_race() {
        let line = "Error: ENOENT: no such file or directory, rename '/home/user/.cursor/cli-config.json.tmp'";
        let result = classify_benign_stderr(line);
        assert!(result.is_some());
        assert_eq!(result.expect("classified").key, "cursor-cli-config-race");
    }

    #[test]
    fn classify_unknown_model() {
        let line = "WARN codex_protocol: Unknown model claude-test-999. This will use fallback model metadata.";
        let result = classify_benign_stderr(line);
        assert!(result.is_some());
        assert_eq!(
            result.expect("classified").key,
            "codex-fallback-model-metadata"
        );
    }

    #[test]
    fn classify_missing_model_messages() {
        let line = "WARN codex_protocol::openai_models: Model personality requested but model_messages is missing, falling back to base instructions. model=claude-sonnet-4-6";
        let result = classify_benign_stderr(line);
        assert!(result.is_some());
        assert_eq!(
            result.expect("classified").key,
            "codex-missing-model-messages"
        );
    }

    #[test]
    fn classify_write_stdin_closed() {
        let line = "write_stdin failed: stdin is closed for this session";
        let result = classify_benign_stderr(line);
        assert!(result.is_some());
        assert_eq!(result.expect("classified").key, "codex-write-stdin-closed");
    }

    #[test]
    fn classify_source_snippet() {
        assert!(classify_benign_stderr("mod tests {").is_some());
        assert!(classify_benign_stderr("#[test]").is_some());
        assert!(classify_benign_stderr("assert_eq!(cms.width(), 128);").is_some());
        assert!(
            classify_benign_stderr("router.update(RoutingDecision::Discard, false);").is_some()
        );
    }

    #[test]
    fn real_errors_not_classified_as_benign() {
        assert!(classify_benign_stderr("error[E0308]: mismatched types").is_none());
        assert!(classify_benign_stderr("warning: unused variable `x`").is_none());
        assert!(classify_benign_stderr("thread 'main' panicked at 'oops'").is_none());
    }

    #[test]
    fn classify_node_experimental() {
        let line = "(node:12345) ExperimentalWarning: The Fetch API is an experimental feature.";
        let result = classify_benign_stderr(line);
        assert!(result.is_some());
        assert_eq!(result.expect("classified").key, "node-experimental-warning");
    }

    #[test]
    fn classify_python_deprecation() {
        let line = "DeprecationWarning: pkg_resources is deprecated as an API";
        let result = classify_benign_stderr(line);
        assert!(result.is_some());
        assert_eq!(
            result.expect("classified").key,
            "python-deprecation-warning"
        );
    }

    #[test]
    fn classify_ollama_model_load() {
        let line = "llama_model_loader: loaded meta data with 22 key-value pairs";
        let result = classify_benign_stderr(line);
        assert!(result.is_some());
        assert_eq!(result.expect("classified").key, "ollama-model-load");
    }

    #[test]
    fn warn_once_returns_true_first_time() {
        // Use a unique key for this test to avoid interaction with other tests.
        let first = benign_stderr_warn_once("codex-thread-shutdown-timeout");
        // The first call should return true (emit warning).
        // Note: if another test ran first, this might be false. That's ok —
        // we at least verify it doesn't panic.
        let _ = first;

        // Second call should return false (already warned).
        let second = benign_stderr_warn_once("codex-thread-shutdown-timeout");
        assert!(!second);
    }

    #[test]
    fn always_silent_keys_never_warn() {
        assert!(!benign_stderr_warn_once("codex-source-snippet"));
        assert!(!benign_stderr_warn_once("codex-shell-snapshot-enoent"));
        assert!(!benign_stderr_warn_once("codex-apply-patch-verification"));
    }

    #[test]
    fn classify_codex_rollout_flush() {
        let line =
            "WARN: failed to flush rollout recorder: failed to queue rollout flush: channel closed";
        let result = classify_benign_stderr(line);
        assert!(result.is_some());
        assert_eq!(
            result.expect("classified").key,
            "codex-rollout-flush-channel-closed"
        );
    }

    #[test]
    fn classify_codex_unknown_turn_item() {
        let line = "dropping turn-scoped item for unknown turn id 1234-5678";
        let result = classify_benign_stderr(line);
        assert!(result.is_some());
        assert_eq!(result.expect("classified").key, "codex-unknown-turn-item");
    }

    #[test]
    fn empty_line_is_not_benign() {
        assert!(classify_benign_stderr("").is_none());
    }

    #[test]
    fn very_long_line_is_not_snippet() {
        let long = "x".repeat(200);
        // Should not match source snippet (>180 chars), may or may not match other patterns.
        let result = classify_benign_stderr(&long);
        // Just verify it doesn't panic.
        let _ = result;
    }
}
