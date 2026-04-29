//! Unified agent session for interactive and one-shot CLI modes.
//!
//! This module owns the session state that will later be passed to the Claude
//! CLI adapter or to API-backed provider adapters.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;
use roko_core::foundation::ChatMessage;

use crate::config::Config;
use crate::model_selection::EffectiveModelSelection;

/// Unified agent session for interactive and one-shot CLI modes.
///
/// Delegates to `ClaudeCliAgent` for Claude CLI turns and to provider
/// adapters for API turns, instead of duplicating command construction.
pub struct ChatAgentSession {
    /// Working directory for the agent.
    pub workdir: PathBuf,
    /// Resolved model identity (provider + slug + source).
    pub model_selection: EffectiveModelSelection,
    /// Reasoning effort level: `"low"`, `"medium"`, `"high"`, `"max"`.
    pub effort: String,
    /// System prompt built by `SystemPromptBuilder`.
    pub system_prompt: String,
    /// Tool allowlist as comma-separated names for `--tools`.
    pub allowed_tools_csv: String,
    /// Path to MCP config file, if discovered.
    pub mcp_config: Option<PathBuf>,
    /// Session ID from previous turn, reused via `--resume`.
    pub session_id: Option<String>,
    /// API message history for non-CLI providers.
    pub api_history: Vec<ChatMessage>,
    /// Shared HTTP client for API providers.
    pub http_client: reqwest::Client,
    /// Path to Claude CLI settings JSON file.
    pub settings_json: Option<PathBuf>,
    /// Per-turn timeout.
    pub timeout: Option<Duration>,
}

impl ChatAgentSession {
    /// Create a new session from CLI config and working directory.
    ///
    /// Resolves system prompt via `SystemPromptBuilder`, tool policy from
    /// safety contracts, and MCP config from discovery paths. Creates one
    /// shared `reqwest::Client`.
    #[must_use]
    pub fn new(
        config: &Config,
        workdir: PathBuf,
        model_selection: EffectiveModelSelection,
    ) -> Result<Self> {
        let system_prompt = resolve_system_prompt_stub(config, &workdir);
        let allowed_tools_csv = resolve_allowed_tools_csv_stub(config, &workdir);
        let mcp_config = discover_mcp_config_stub(config, &workdir);
        let effort = config.agent.effort.clone();
        let timeout = (config.agent.timeout_ms > 0)
            .then(|| Duration::from_millis(config.agent.timeout_ms));

        Ok(Self {
            workdir,
            model_selection,
            effort,
            system_prompt,
            allowed_tools_csv,
            mcp_config,
            session_id: None,
            api_history: Vec::new(),
            http_client: shared_http_client(),
            settings_json: None,
            timeout,
        })
    }
}

fn resolve_system_prompt_stub(_config: &Config, _workdir: &Path) -> String {
    // Placeholder until R3_A02 wires SystemPromptBuilder into session creation.
    String::new()
}

fn resolve_allowed_tools_csv_stub(_config: &Config, _workdir: &Path) -> String {
    // Placeholder until R3_A03 wires the safety/tool policy contract.
    String::new()
}

fn discover_mcp_config_stub(_config: &Config, _workdir: &Path) -> Option<PathBuf> {
    // Placeholder until R3_A04 wires MCP discovery into session creation.
    None
}

fn shared_http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new).clone()
}
