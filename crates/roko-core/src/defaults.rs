//! Central constants for the Roko workspace.
//!
//! Every numeric default that was previously hardcoded across multiple crates
//! lives here.  Import from `roko_core::defaults` instead of duplicating.

// ── Timeouts (milliseconds) ─────────────────────────────────────────────

/// TTFT (time-to-first-token) timeout.  Shared by all LLM providers.
pub const DEFAULT_TTFT_TIMEOUT_MS: u64 = 15_000;

/// Hard request / subprocess timeout for LLM calls.
pub const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 120_000;

/// TCP connection timeout for provider HTTP clients.
pub const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 5_000;

/// Embedding / search request timeout (shorter than full LLM calls).
pub const DEFAULT_EMBED_TIMEOUT_MS: u64 = 30_000;

/// Grace period before force-killing child processes on shutdown.
pub const DEFAULT_SHUTDOWN_DRAIN_SECS: u64 = 15;

/// Grace period for stdin close during process kill sequence.
pub const DEFAULT_GRACE_STDIN_CLOSE_MS: u64 = 1_200;

/// Grace period for SIGTERM during process kill sequence.
pub const DEFAULT_GRACE_SIGTERM_MS: u64 = 800;

// ── Token budgets ───────────────────────────────────────────────────────

/// Default max output tokens when not specified per-model.
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 16_384;

/// Fallback max output tokens for models with no profile.
pub const DEFAULT_FALLBACK_MAX_OUTPUT_TOKENS: u32 = 2_048;

/// Default max tool-loop iterations (unified across all providers).
pub const DEFAULT_MAX_TOOL_ITERATIONS: usize = 50;

/// Token limit for message pruning / context management.
pub const DEFAULT_CONTEXT_TOKEN_LIMIT: usize = 102_400;

// ── Retry ───────────────────────────────────────────────────────────────

/// Default retry attempts for LLM calls.
pub const DEFAULT_RETRY_ATTEMPTS: u32 = 3;

/// Base backoff delay for retries (milliseconds).
pub const DEFAULT_RETRY_BASE_DELAY_MS: u64 = 1_000;

/// Maximum backoff delay for retries (milliseconds).
pub const DEFAULT_RETRY_MAX_BACKOFF_MS: u64 = 60_000;

/// Retry attempts for rate-limited operations.
pub const DEFAULT_RATE_LIMIT_RETRY_ATTEMPTS: u32 = 5;

/// Base backoff delay for rate-limited operations (milliseconds).
pub const DEFAULT_RATE_LIMIT_RETRY_BASE_DELAY_MS: u64 = 2_000;

/// Maximum backoff delay for rate-limited operations (milliseconds).
pub const DEFAULT_RATE_LIMIT_RETRY_MAX_BACKOFF_MS: u64 = DEFAULT_RETRY_MAX_BACKOFF_MS;

/// Retry attempts for timeout failures.
pub const DEFAULT_TIMEOUT_RETRY_ATTEMPTS: u32 = DEFAULT_RETRY_ATTEMPTS;

/// Base backoff delay for timeout failures (milliseconds).
pub const DEFAULT_TIMEOUT_RETRY_BASE_DELAY_MS: u64 = DEFAULT_RETRY_BASE_DELAY_MS;

/// Maximum backoff delay for timeout failures (milliseconds).
pub const DEFAULT_TIMEOUT_RETRY_MAX_BACKOFF_MS: u64 = 30_000;

/// Retry attempts for generic transient failures.
pub const DEFAULT_TRANSIENT_RETRY_ATTEMPTS: u32 = DEFAULT_RETRY_ATTEMPTS;

/// Base backoff delay for generic transient failures (milliseconds).
pub const DEFAULT_TRANSIENT_RETRY_BASE_DELAY_MS: u64 = 500;

/// Maximum backoff delay for generic transient failures (milliseconds).
pub const DEFAULT_TRANSIENT_RETRY_MAX_BACKOFF_MS: u64 = 15_000;

/// Default max merge retries in the orchestrator.
pub const DEFAULT_MAX_MERGE_RETRIES: u32 = 5;

/// Default max auto-fix iterations in executor state machine.
pub const DEFAULT_MAX_AUTO_FIX_ITERATIONS: u32 = 5;

/// Default wall-clock timeout for runner plan execution (seconds).
pub const DEFAULT_PLAN_TIMEOUT_SECS: u64 = 3_600;

/// Base retry delay for runner plan backoff (seconds).
pub const DEFAULT_PLAN_RETRY_BASE_SECS: u64 = 1;

/// Maximum retry delay for runner plan backoff (seconds).
pub const DEFAULT_PLAN_RETRY_MAX_SECS: u64 = 30;

/// Maximum exponent shift used while computing runner plan backoff.
pub const DEFAULT_PLAN_RETRY_BACKOFF_SHIFT_CAP: u32 = 16;

// ── Resource limits ─────────────────────────────────────────────────────

/// Maximum bytes a tool result may return before truncation.
pub const DEFAULT_MAX_RESULT_BYTES: usize = 65_536;

/// Truncation point for tool output in Claude CLI stream.
pub const DEFAULT_TOOL_OUTPUT_TRUNCATE_AT: usize = 4_096;

/// Maximum response bytes from safety result filter.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 100 * 1024;

/// Maximum file read size (10 MB).
pub const DEFAULT_MAX_FILE_READ_BYTES: usize = 10 * 1024 * 1024;

/// Maximum file write size (5 MB).
pub const DEFAULT_MAX_FILE_WRITE_BYTES: usize = 5 * 1024 * 1024;

/// Maximum glob results before truncation.
pub const DEFAULT_MAX_GLOB_RESULTS: usize = 1_000;

/// Maximum concurrent tool dispatches.
pub const DEFAULT_MAX_CONCURRENT_TOOLS: usize = 8;

/// Maximum concurrent requests per provider.
pub const DEFAULT_PROVIDER_MAX_CONCURRENT: usize = 10;

/// Maximum diff bytes for LLM judge gate.
pub const DEFAULT_MAX_DIFF_BYTES: usize = 30 * 1024;

/// Maximum file path length (safety check).
pub const DEFAULT_MAX_PATH_LEN: usize = 4_096;

// ── Cache & GC ──────────────────────────────────────────────────────────

/// Response cache TTL (milliseconds).
pub const DEFAULT_RESPONSE_CACHE_TTL_MS: u64 = 30_000;

/// Dedup cache TTL (seconds).
pub const DEFAULT_DEDUP_CACHE_TTL_SECS: u64 = 600;

/// Result cache TTL (seconds).
pub const DEFAULT_RESULT_CACHE_TTL_SECS: u64 = 300;

/// Max entries in dedup cache.
pub const DEFAULT_MAX_DEDUP_ENTRIES: usize = 512;

/// Max entries in result cache.
pub const DEFAULT_MAX_CACHE_ENTRIES: usize = 256;

/// Workspace GC interval (seconds). 5 minutes for dev, configurable for prod.
pub const DEFAULT_WORKSPACE_GC_INTERVAL_SECS: u64 = 300;

/// Pointer GC: max age in turns before eviction.
pub const DEFAULT_POINTER_MAX_AGE_TURNS: u32 = 10;

/// Pointer GC: max total bytes before eviction.
pub const DEFAULT_POINTER_MAX_TOTAL_BYTES: u64 = 10 * 1024 * 1024;

// ── Message pruning ─────────────────────────────────────────────────────

/// Number of messages to keep at the head during pruning.
pub const DEFAULT_HEAD_KEEP: usize = 2;

/// Number of messages to keep at the tail during pruning.
pub const DEFAULT_TAIL_KEEP: usize = 3;

/// Recent tool groups to keep during compaction.
pub const DEFAULT_RECENT_TOOL_GROUPS_TO_KEEP: usize = 2;

/// Character threshold for tool result compaction.
pub const DEFAULT_TOOL_RESULT_COMPACTION_THRESHOLD_CHARS: usize = 500;

/// Character count for tool result preview after compaction.
pub const DEFAULT_TOOL_RESULT_PREVIEW_CHARS: usize = 200;

// ── Server ──────────────────────────────────────────────────────────────

/// Default HTTP serve port.
pub const DEFAULT_SERVE_PORT: u16 = 6677;

/// Default staleness threshold for relay-sourced data (seconds).
pub const DEFAULT_RELAY_STALE_THRESHOLD_SECS: u64 = 30;

/// Number of consecutive relay heartbeat failures before backoff starts.
pub const DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD: u32 = 3;

/// Base relay circuit-breaker backoff duration (seconds).
pub const DEFAULT_RELAY_CIRCUIT_BREAKER_BASE_BACKOFF_SECS: u64 = 2;

/// Maximum relay circuit-breaker backoff duration (seconds).
pub const DEFAULT_RELAY_CIRCUIT_BREAKER_MAX_BACKOFF_SECS: u64 = 60;

// ── Alerting ────────────────────────────────────────────────────────────

/// Default failure rate threshold for anomaly alerts (25%).
pub const DEFAULT_FAILURE_THRESHOLD: f64 = 0.25;

/// Minimum calls before anomaly alerting kicks in.
pub const DEFAULT_ALERT_MIN_CALLS: u64 = 50;

// ── Event bus ───────────────────────────────────────────────────────────

/// Default event bus channel capacity.
pub const DEFAULT_EVENT_BUS_CAPACITY: usize = 32;

/// Default bounded channel buffer for per-subscriber and streaming channels.
pub const DEFAULT_CHANNEL_BUFFER: usize = 256;

/// Default bounded channel buffer for per-agent streaming multiplexers.
pub const DEFAULT_MUX_CHANNEL_BUFFER: usize = 512;

/// MCP discovery timeout (seconds).
pub const DEFAULT_MCP_DISCOVERY_TIMEOUT_SECS: u64 = 5;

/// Timeout for writing a JSON-RPC request to an MCP server's stdin.
pub const DEFAULT_MCP_STDIN_WRITE_TIMEOUT_SECS: u64 = 5;

/// Timeout for reading a JSON-RPC response from an MCP server's stdout.
pub const DEFAULT_MCP_RESPONSE_TIMEOUT_SECS: u64 = 30;

// ── Output tail limits ──────────────────────────────────────────────────

/// Default tail lines for log endpoints.
pub const DEFAULT_LOG_TAIL: usize = 200;

/// Maximum tail lines for log endpoints.
pub const DEFAULT_LOG_MAX_TAIL: usize = 2_000;

/// Tail lines for watcher signal output.
pub const DEFAULT_WATCHER_SIGNAL_TAIL: usize = 200;

/// Tail lines for task output display.
pub const DEFAULT_TASK_OUTPUT_TAIL_CAP: usize = 400;

/// Tail lines for task failure output.
pub const DEFAULT_TASK_FAILURE_OUTPUT_TAIL_LINES: usize = 20;

/// Tail for efficiency signal.
pub const DEFAULT_EFFICIENCY_SIGNAL_TAIL: usize = 256;

/// Tail characters for pre-agent remediation command output.
pub const DEFAULT_PRE_AGENT_REMEDIATION_OUTPUT_TAIL: usize = 4_000;

// ── Gate & verification ─────────────────────────────────────────────────

/// Default proptest cases.
pub const DEFAULT_PROPTEST_CASES: u32 = 256;

/// Default max shrink iterations for proptest.
pub const DEFAULT_MAX_SHRINK_ITERS: u32 = 2_048;

/// Default minimum confidence for fact checking.
pub const DEFAULT_MIN_CONFIDENCE: f64 = 0.7;

/// Stale lock timeout (seconds) for worktree operations.
pub const DEFAULT_STALE_LOCK_SECS: u64 = 60;

// ── Runner workflow ────────────────────────────────────────────────────

/// Maximum agent turns before the runner gives up on a single task.
pub const DEFAULT_AGENT_TURN_LIMIT: u32 = 50;

/// Maximum concurrent plans the runner DAG allows in-flight at once.
pub const DEFAULT_RUNNER_MAX_CONCURRENT_PLANS: usize = 4;

/// Maximum concurrent tasks the runner allows in-flight at once.
pub const DEFAULT_RUNNER_MAX_CONCURRENT_TASKS: usize = 4;

/// Default gate concurrency (matches task concurrency).
pub const DEFAULT_RUNNER_GATE_CONCURRENCY: usize = 4;

/// Default max retries for runner task config.
pub const DEFAULT_RUNNER_CONFIG_MAX_RETRIES: u32 = 2;

/// After this many attempts, the runner pivots retry strategy.
pub const DEFAULT_RUNNER_RETRY_STRATEGY_PIVOT_ATTEMPT: u32 = 3;

/// Minimum gate retries (adaptive lower bound).
pub const DEFAULT_GATE_RETRY_MIN: u32 = 1;

/// Maximum gate retries (adaptive upper bound).
pub const DEFAULT_GATE_RETRY_MAX: u32 = 5;

/// Cold-start gate retry count before observations accumulate.
pub const DEFAULT_GATE_RETRY_COLD_START: u32 = 3;

/// Minimum observations before adaptive gate thresholds activate.
pub const DEFAULT_GATE_RETRY_MIN_OBSERVATIONS: u64 = 5;

/// Maximum retry backoff shift cap (prevents exponential overflow).
pub const DEFAULT_RUNNER_RETRY_BACKOFF_SHIFT_CAP: u32 = 5;

/// Fallback multiplier for retry backoff when no history available.
pub const DEFAULT_RUNNER_RETRY_BACKOFF_MULTIPLIER_FALLBACK: u64 = 32;

/// Maximum seconds for retry backoff delay.
pub const DEFAULT_RUNNER_RETRY_BACKOFF_MAX_SECS: u64 = 45;

// ── Model slugs ────────────────────────────────────────────────────────

/// Default model for the "deep" / architectural tier.
pub const MODEL_DEEP: &str = "claude-opus-4-6";

/// Default model for the "focused" / standard implementation tier.
pub const MODEL_FOCUSED: &str = "claude-sonnet-4-6";

/// Default model for the "mechanical" / fast tier.
pub const MODEL_FAST: &str = "claude-haiku-4-5";

/// Escalation ladder (mechanical -> focused -> deep).
pub const MODEL_ESCALATION_LADDER: [&str; 3] = [MODEL_FAST, MODEL_FOCUSED, MODEL_DEEP];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_drain_is_reasonable() {
        assert!(DEFAULT_SHUTDOWN_DRAIN_SECS >= 5);
        assert!(DEFAULT_SHUTDOWN_DRAIN_SECS <= 60);
    }

    #[test]
    fn relay_backoff_defaults_are_ordered() {
        assert!(DEFAULT_RELAY_CIRCUIT_BREAKER_THRESHOLD > 0);
        assert!(
            DEFAULT_RELAY_CIRCUIT_BREAKER_BASE_BACKOFF_SECS
                < DEFAULT_RELAY_CIRCUIT_BREAKER_MAX_BACKOFF_SECS
        );
        assert!(
            DEFAULT_RELAY_STALE_THRESHOLD_SECS < DEFAULT_RELAY_CIRCUIT_BREAKER_MAX_BACKOFF_SECS
        );
    }

    #[test]
    fn ttft_less_than_request_timeout() {
        assert!(DEFAULT_TTFT_TIMEOUT_MS < DEFAULT_REQUEST_TIMEOUT_MS);
    }

    #[test]
    fn retry_backoff_ordering() {
        assert!(DEFAULT_RETRY_BASE_DELAY_MS < DEFAULT_RETRY_MAX_BACKOFF_MS);
        assert!(DEFAULT_RATE_LIMIT_RETRY_ATTEMPTS > DEFAULT_RETRY_ATTEMPTS);
        assert!(DEFAULT_TRANSIENT_RETRY_BASE_DELAY_MS < DEFAULT_RETRY_BASE_DELAY_MS);
        assert!(DEFAULT_TIMEOUT_RETRY_MAX_BACKOFF_MS < DEFAULT_RETRY_MAX_BACKOFF_MS);
        assert_eq!(
            DEFAULT_RATE_LIMIT_RETRY_MAX_BACKOFF_MS,
            DEFAULT_RETRY_MAX_BACKOFF_MS
        );
        assert!(DEFAULT_PLAN_RETRY_BASE_SECS < DEFAULT_PLAN_RETRY_MAX_SECS);
        assert!(DEFAULT_PLAN_TIMEOUT_SECS > DEFAULT_PLAN_RETRY_MAX_SECS);
        assert!(DEFAULT_PLAN_RETRY_BACKOFF_SHIFT_CAP < u64::BITS);
    }

    #[test]
    fn max_output_tokens_nonzero() {
        assert!(DEFAULT_MAX_OUTPUT_TOKENS > 0);
        assert!(DEFAULT_FALLBACK_MAX_OUTPUT_TOKENS > 0);
    }

    #[test]
    fn runner_limits_are_sane() {
        assert!(DEFAULT_RUNNER_MAX_CONCURRENT_TASKS >= 1);
        assert!(DEFAULT_GATE_RETRY_MIN <= DEFAULT_GATE_RETRY_MAX);
        assert!(DEFAULT_GATE_RETRY_COLD_START >= DEFAULT_GATE_RETRY_MIN);
        assert!(DEFAULT_GATE_RETRY_COLD_START <= DEFAULT_GATE_RETRY_MAX);
        assert!(DEFAULT_RUNNER_RETRY_BACKOFF_MAX_SECS > 0);
    }

    #[test]
    fn file_limits_are_sane() {
        assert!(DEFAULT_MAX_FILE_READ_BYTES >= 1024 * 1024); // at least 1MB
        assert!(DEFAULT_MAX_RESULT_BYTES >= 1024); // at least 1KB
    }
}
