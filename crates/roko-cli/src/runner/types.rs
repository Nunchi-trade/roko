//! Core types for the runner v2 event-driven plan executor.
//!
//! These types form the protocol between the agent stream parser, the event
//! loop, and the TUI bridge.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ─── Agent Events ───────────────────────────────────────────────────────

/// Events emitted by the agent stream parser.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Claude CLI sent a `system` init message.
    SystemInit {
        session_id: String,
        model: String,
    },
    /// A chunk of assistant text output.
    MessageDelta {
        text: String,
    },
    /// The agent is invoking a tool.
    ToolCall {
        id: String,
        name: String,
    },
    /// Result of a tool invocation.
    ToolOutput {
        id: String,
        output: String,
    },
    /// Token usage update from a turn.
    TokenUsage {
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    },
    /// An entire turn has completed.
    TurnCompleted {
        session_id: Option<String>,
        total_cost_usd: Option<f64>,
        num_turns: Option<u32>,
        is_error: bool,
    },
    /// An error from the agent process.
    Error {
        message: String,
    },
    /// The agent process has exited.
    Exited {
        exit_code: Option<i32>,
    },
}

// ─── Verify Completion ────────────────────────────────────────────────────

/// Result of a gate run, sent back through the gate channel.
#[derive(Debug, Clone)]
pub struct GateCompletion {
    pub plan_id: String,
    pub task_id: String,
    pub rung: u32,
    pub passed: bool,
    pub verdicts: Vec<GateVerdictSummary>,
    pub output: String,
    pub duration_ms: u64,
}

/// Minimal gate verdict for reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateVerdictSummary {
    pub gate_name: String,
    pub passed: bool,
    pub summary: String,
}

// ─── Run Config ─────────────────────────────────────────────────────────

/// Configuration for a runner v2 execution.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Working directory for the plan execution.
    pub workdir: PathBuf,
    /// Directory containing plan(s).
    pub plan_dir: PathBuf,
    /// Default model to use when task has no model_hint.
    pub model: String,
    /// Per-task timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum auto-fix retries per task.
    pub max_retries: u32,
    /// Whether to require approval before each task.
    pub approval: bool,
    /// Whether to dangerously skip permissions in the agent.
    pub dangerously_skip_permissions: bool,
    /// Optional MCP config path.
    pub mcp_config: Option<PathBuf>,
    /// Optional session ID to resume from.
    pub resume_session: Option<String>,
    /// Maximum gate rung to run (0=compile, 1=clippy, 2=test, ...).
    pub max_gate_rung: u32,
    /// Claude CLI binary path.
    pub claude_program: PathBuf,
    /// Maximum USD spend per plan (0 = unlimited). From `[budget]`.
    pub max_plan_usd: f64,
    /// Maximum USD spend per single agent turn (0 = unlimited). From `[budget]`.
    pub max_turn_usd: f64,
    /// Whether clippy gate is enabled. From `[gates]` / gate config.
    pub clippy_enabled: bool,
    /// Whether to skip the test gate. From `[gates]` / gate config.
    pub skip_tests: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            workdir: PathBuf::from("."),
            plan_dir: PathBuf::from("plans"),
            model: "claude-sonnet-4-6".to_string(),
            timeout_secs: 600,
            max_retries: 5,
            approval: false,
            dangerously_skip_permissions: true,
            mcp_config: None,
            resume_session: None,
            max_gate_rung: 2,
            claude_program: PathBuf::from("claude"),
            max_plan_usd: 25.0,
            max_turn_usd: 3.0,
            clippy_enabled: true,
            skip_tests: false,
        }
    }
}

// ─── Claude Stream JSON Protocol ────────────────────────────────────────

/// Top-level stream event from `claude --output-format stream-json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeStreamEvent {
    System(ClaudeSystemEvent),
    Assistant(ClaudeAssistantEvent),
    Tool(ClaudeToolEvent),
    Result(ClaudeResultEvent),
}

/// The `system` init event.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeSystemEvent {
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub tools: Vec<serde_json::Value>,
    // mcp_servers, cwd, etc. — we ignore them
}

/// An assistant message event.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeAssistantEvent {
    #[serde(default)]
    pub subtype: String,
    pub message: ClaudeMessage,
}

/// The message body inside an assistant event.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeMessage {
    #[serde(default)]
    pub content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    pub usage: Option<ClaudeUsage>,
}

/// Content block — either text or tool_use.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
}

/// A tool result event.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeToolEvent {
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub content: String,
}

/// The final result event.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeResultEvent {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub total_cost_usd: Option<f64>,
    #[serde(default)]
    pub num_turns: Option<u32>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub duration_ms: Option<f64>,
    #[serde(default)]
    pub duration_api_ms: Option<f64>,
}

/// Token usage from a message.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}
