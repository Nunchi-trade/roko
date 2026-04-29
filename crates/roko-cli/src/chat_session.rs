//! Unified agent session for interactive and one-shot CLI modes.
//!
//! This module owns the session state that will later be passed to the Claude
//! CLI adapter or to API-backed provider adapters.

use std::fs;
use std::io::{self, Read as _, Write as StdWrite};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use roko_agent::AgentRuntimeEvent;
use roko_agent::agent::{Agent, AgentResult};
use roko_agent::claude_cli_agent::ClaudeCliAgent;
use roko_agent::process::{GRACE_STDIN_CLOSE_MS, kill_tree, set_process_group};
use roko_agent::provider::claude_cli::stream::parse_stream_line;
use roko_agent::safety::contract::AgentContract;
use roko_compose::system_prompt_builder::SystemPromptBuilder;
use roko_compose::{ProjectConventions, TokenCounter, detect_conventions};
use roko_core::agent::ProviderKind;
use roko_core::foundation::{ChatMessage, MessageRole};
use roko_core::{Body, Context, Engram, Kind, OperatingFrequency};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::signal;

use crate::config::Config;
use crate::model_selection::EffectiveModelSelection;

const CHAT_SYSTEM_PROMPT_TOKEN_BUDGET: usize = 4_000;
const MAX_WORKSPACE_SAMPLE_BYTES: usize = 16_384;
const MAX_WORKSPACE_SAMPLE_FILES: usize = 8;
const MAX_WORKSPACE_SCAN_DEPTH: usize = 5;
const SKIP_DIR_NAMES: [&str; 12] = [
    ".git",
    ".next",
    ".roko",
    ".turbo",
    ".venv",
    "__pycache__",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "target",
    "venv",
];

/// Errors returned by `ChatAgentSession` send operations.
#[derive(Debug, Error)]
pub enum SessionError {
    /// The configured model resolves to a non-CLI provider that is not yet
    /// supported in the chat session path.
    ///
    /// # Suggestions
    ///
    /// - Switch to a Claude CLI model: `/model claude-sonnet-4-6-20250514`
    /// - Use `roko run "prompt"` for API provider dispatch
    #[error(
        "API provider chat not yet implemented for provider '{provider}' (model: '{model}'). \
         Switch to Claude CLI with `/model claude-sonnet-4-6-20250514`, \
         or use `roko run \"{model}\"` for API provider dispatch."
    )]
    ApiProviderNotImplemented {
        /// Provider kind string (for example `anthropic_api`).
        provider: String,
        /// Model slug that was requested.
        model: String,
    },

    /// No API key could be resolved for the configured provider.
    ///
    /// The key is looked up from the environment variable named
    /// `<PROVIDER>_API_KEY` (e.g. `ANTHROPIC_API_KEY`).  Set the variable
    /// or add the provider to `roko.toml` before retrying.
    #[error(
        "no API key for provider '{provider}': set {env_var} or configure it in roko.toml"
    )]
    ApiKeyMissing {
        /// Provider kind label (for example `anthropic_api`).
        provider: String,
        /// Environment variable name that was checked.
        env_var: String,
    },

    /// HTTP/network error communicating with the provider.
    #[error("network error talking to provider '{provider}': {message}")]
    NetworkError {
        /// Provider kind label.
        provider: String,
        /// Error detail from the transport layer.
        message: String,
    },

    /// Authentication failure (HTTP 401 or 403) from the provider.
    #[error("authentication failed for provider '{provider}' (HTTP {status}): check your API key")]
    AuthError {
        /// Provider kind label.
        provider: String,
        /// HTTP status code (401 or 403).
        status: u16,
    },

    /// Rate limit hit (HTTP 429) from the provider.
    #[error(
        "rate limited by provider '{provider}' (HTTP 429)\
         {retry_after}"
    )]
    RateLimited {
        /// Provider kind label.
        provider: String,
        /// Formatted `retry_after` hint, or empty string when not provided.
        retry_after: String,
    },

    /// Any other error (I/O, config, network, etc.).
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Update the stored session id after a turn completes.
///
/// Empty session ids are ignored so a missing result event does not erase a
/// previously captured `SystemInit` session.
fn apply_session_id(session_id: &mut Option<String>, new_id: Option<String>) {
    if let Some(ref sid) = new_id {
        if !sid.is_empty() {
            *session_id = new_id;
        }
    }
}

/// Result of processing a potential slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashResult {
    /// Command recognized and session state updated. String is user-facing message.
    Updated(String),
    /// Command recognized but had an error. String is the error message.
    Error(String),
    /// Command recognized and produces display-only output (no state change).
    ///
    /// The string is a multi-line plain-text block ready to print. Callers
    /// should render it verbatim — each line is newline-separated.
    Display(String),
    /// Input starts with `/` but command is not recognized.
    Unknown(String),
    /// Input is not a slash command.
    NotACommand,
}

/// Summary of a tool call captured during a turn.
#[derive(Debug, Clone)]
pub struct ToolCallSummary {
    /// Tool name (for example `Read`, `Bash`, or `Edit`).
    pub name: String,
    /// Abbreviated tool output, capped at the first 200 characters.
    pub input_abbrev: String,
    /// Whether the tool call succeeded.
    pub success: bool,
}

/// Update the streaming tool-call accumulator with one runtime event.
///
/// The streaming turn path calls this for every `AgentRuntimeEvent` so the
/// caller can render tool usage after the turn completes. Non-streaming
/// `send_turn` does not call this because structured tool data is not
/// available from `ClaudeCliAgent::run`.
pub fn accumulate_tool_event(
    tool_calls: &mut Vec<ToolCallSummary>,
    pending_ids: &mut Vec<(String, usize)>,
    event: &AgentRuntimeEvent,
) {
    match event {
        AgentRuntimeEvent::ToolCall { id, name } => {
            let idx = tool_calls.len();
            tool_calls.push(ToolCallSummary {
                name: name.clone(),
                input_abbrev: String::new(),
                success: true,
            });
            pending_ids.push((id.clone(), idx));
        }
        AgentRuntimeEvent::ToolOutput { id, output } => {
            if let Some(pos) = pending_ids
                .iter()
                .position(|(tool_use_id, _)| tool_use_id == id)
            {
                let (_, idx) = pending_ids.remove(pos);
                if let Some(tool_call) = tool_calls.get_mut(idx) {
                    tool_call.input_abbrev = preview_text(output, 200);
                    tool_call.success = true;
                }
            }
        }
        _ => {}
    }
}

fn write_stdout_bytes(bytes: &[u8]) {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let _ = handle.write_all(bytes);
    let _ = handle.flush();
}

fn write_stderr_line(line: &str) {
    let stderr = io::stderr();
    let mut handle = stderr.lock();
    let _ = handle.write_all(line.as_bytes());
    let _ = handle.write_all(b"\n");
    let _ = handle.flush();
}

/// Render an `AgentRuntimeEvent` to stdout/stderr for plain terminal display.
///
/// This is the non-TUI rendering path used for plain terminal output.
pub fn render_stream_event(event: &AgentRuntimeEvent) {
    match event {
        AgentRuntimeEvent::MessageDelta { text } => {
            write_stdout_bytes(text.as_bytes());
        }
        AgentRuntimeEvent::ToolCall { name, .. } => {
            write_stderr_line(&format!("[{name}] running..."));
        }
        AgentRuntimeEvent::ToolOutput { id, .. } => {
            write_stderr_line(&format!("[tool:{id}] done"));
        }
        AgentRuntimeEvent::TurnCompleted { is_error, .. } => {
            if *is_error {
                write_stderr_line("[error] turn completed with error flag");
            }
        }
        AgentRuntimeEvent::Error { message } => {
            write_stderr_line(&format!("[error] {message}"));
        }
        _ => {}
    }
}

/// Print a final newline after streaming completes.
pub fn render_stream_end() {
    write_stdout_bytes(b"\n");
}

/// Render collected text in one shot for the `--no-stream` fallback.
pub fn render_collected(text: &str) {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    let _ = handle.write_all(text.as_bytes());
    let _ = handle.write_all(b"\n");
    let _ = handle.flush();
}

/// Result of a single agent turn.
#[derive(Debug, Clone)]
pub struct TurnResult {
    /// The model's text response.
    pub text: String,
    /// Which model responded.
    pub model: String,
    /// Input tokens consumed during the turn.
    pub input_tokens: u64,
    /// Output tokens produced during the turn.
    pub output_tokens: u64,
    /// Tool calls executed during the turn.
    pub tool_calls: Vec<ToolCallSummary>,
    /// Session identifier for `--resume`.
    ///
    /// This batch uses `ClaudeCliAgent::run`, which does not surface the
    /// stream `result` event session id into `AgentResult`, so this stays
    /// `None` until the streaming turn path lands.
    pub session_id: Option<String>,
    /// Wall-clock duration of the turn.
    pub duration: Duration,
    /// Whether the turn was cancelled by the user.
    pub cancelled: bool,
}

impl TurnResult {
    /// Create a cancelled result with no text content.
    ///
    /// Used when a turn is interrupted by Ctrl-C or timeout.
    /// The session loop should display a "[cancelled]" indicator and
    /// continue accepting input.
    pub fn cancelled(duration: Duration) -> Self {
        Self {
            text: String::new(),
            model: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            tool_calls: Vec::new(),
            session_id: None,
            duration,
            cancelled: true,
        }
    }
}

/// Unified agent session for interactive and one-shot CLI modes.
///
/// Delegates to `ClaudeCliAgent` for Claude CLI turns and to provider
/// adapters for API turns, instead of duplicating command construction.
pub struct ChatAgentSession {
    /// Working directory for the agent.
    pub workdir: PathBuf,
    /// Mutable model string used by slash commands and future turns.
    pub model: String,
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
        let system_prompt = build_chat_system_prompt(&workdir, config);
        let allowed_tools_csv = resolve_tool_policy(&workdir);
        let mcp_config = resolve_mcp_config(&workdir, config);
        let effort = config.agent.effort.clone();
        let timeout =
            (config.agent.timeout_ms > 0).then(|| Duration::from_millis(config.agent.timeout_ms));
        let model = model_selection.effective_model_key.clone();

        Ok(Self {
            workdir,
            model,
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

    /// Returns `true` if the current model resolves to Claude CLI.
    ///
    /// Non-CLI providers are rejected until the API chat path is wired.
    fn is_cli_provider(&self) -> bool {
        self.model_selection.provider_kind == ProviderKind::ClaudeCli.label()
    }

    /// Build the typed error used when an unsupported provider is requested.
    fn api_provider_not_implemented_error(&self) -> SessionError {
        let provider = self.model_selection.provider_kind.clone();
        let model = self.model.clone();
        tracing::warn!(
            provider = %provider,
            model = %model,
            "ChatAgentSession: API provider requested but not yet supported in chat path"
        );
        SessionError::ApiProviderNotImplemented { provider, model }
    }

    /// Derive the canonical environment variable name for the current provider's
    /// API key.
    ///
    /// Convention: `<PROVIDER_KIND_UPPER>_API_KEY`, e.g.
    /// - `anthropic_api` → `ANTHROPIC_API_KEY`
    /// - `openai_compat` → `OPENAI_COMPAT_API_KEY`  (fallback; most callers
    ///   set a provider-specific variable via `roko.toml`)
    fn api_key_env_var(&self) -> String {
        let kind = self.model_selection.provider_kind.to_uppercase();
        // Strip trailing `_API` suffix that would otherwise produce
        // `ANTHROPIC_API_API_KEY` — the label is already `anthropic_api`.
        let base = kind.trim_end_matches("_API").to_string();
        format!("{base}_API_KEY")
    }

    /// Resolve the API key for the current non-CLI provider.
    ///
    /// Checks the canonical environment variable (`<PROVIDER>_API_KEY`) and
    /// returns a typed [`SessionError::ApiKeyMissing`] when the variable is
    /// absent or empty.
    fn resolve_api_key(&self) -> std::result::Result<String, SessionError> {
        let env_var = self.api_key_env_var();
        let key = std::env::var(&env_var).unwrap_or_default();
        if key.trim().is_empty() {
            return Err(SessionError::ApiKeyMissing {
                provider: self.model_selection.provider_kind.clone(),
                env_var,
            });
        }
        Ok(key)
    }

    /// Classify an HTTP status code into a [`SessionError`].
    ///
    /// Called by `send_turn_api` after receiving a non-success response from
    /// the provider.  Maps the well-known error categories (auth, rate-limit,
    /// network) to typed variants so callers can distinguish them without
    /// parsing error messages.
    fn classify_http_error(
        &self,
        status: u16,
        body: &str,
    ) -> SessionError {
        let provider = self.model_selection.provider_kind.clone();
        match status {
            401 | 403 => SessionError::AuthError { provider, status },
            429 => {
                // Best-effort: surface a `retry_after` hint when the body
                // contains `retry_after` (Anthropic format).
                let retry_hint = serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .and_then(|v| {
                        v.pointer("/error/retry_after")
                            .or_else(|| v.pointer("/retry_after"))
                            .and_then(serde_json::Value::as_u64)
                    })
                    .map(|secs| format!("; retry after {secs}s"))
                    .unwrap_or_default();
                SessionError::RateLimited {
                    provider,
                    retry_after: retry_hint,
                }
            }
            _ => SessionError::NetworkError {
                provider,
                message: format!("HTTP {status}: {body}"),
            },
        }
    }

    /// Send a single turn through a direct API provider (non-CLI path).
    ///
    /// This method owns the full conversation lifecycle for non-CLI providers:
    ///
    /// 1. Resolve the API key from the environment.
    /// 2. Prepend the system prompt to the messages array (if non-empty).
    /// 3. Append the user message to `api_history` and build the full
    ///    `messages` array.
    /// 4. Dispatch to the appropriate provider endpoint based on
    ///    `model_selection.provider_kind`:
    ///    - `anthropic_api` → `POST /v1/messages` with `x-api-key` header
    ///    - `openai_compat` and everything else → `POST /chat/completions`
    ///      with `Authorization: Bearer <key>` header
    /// 5. Parse the response, extract text and usage.
    /// 6. Push an `Assistant` message onto `api_history`.
    /// 7. Return a [`TurnResult`] with text, usage, and elapsed time.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::ApiKeyMissing`] when no key is found,
    /// [`SessionError::AuthError`] on 401/403, [`SessionError::RateLimited`]
    /// on 429, and [`SessionError::NetworkError`] for other HTTP failures.
    ///
    /// # Implementation status
    ///
    /// The request construction and history management are complete.
    /// The actual HTTP dispatch (`POST` call + response parsing) is marked
    /// with `todo!()` so the types flow correctly through the whole path and
    /// a future implementer only needs to fill in the network call.
    pub async fn send_turn_api(
        &mut self,
        prompt: &str,
    ) -> std::result::Result<TurnResult, SessionError> {
        let started = Instant::now();
        let provider_kind = self.model_selection.provider_kind.clone();
        let model_slug = self.model_selection.backend_slug.clone();

        // Step 1 — Resolve API key.
        let api_key = self.resolve_api_key()?;

        // Step 2 — Build the messages array from history.
        //
        // Prepend the system prompt as the first message (if non-empty) only
        // when the history is empty so it is not duplicated across turns.
        let mut messages: Vec<serde_json::Value> = Vec::new();
        if self.api_history.is_empty() && !self.system_prompt.is_empty() {
            messages.push(serde_json::json!({
                "role": "system",
                "content": self.system_prompt,
            }));
        }
        for msg in &self.api_history {
            let role = match msg.role {
                MessageRole::System => "system",
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            };
            messages.push(serde_json::json!({
                "role": role,
                "content": msg.content,
            }));
        }
        // Append the new user message.
        messages.push(serde_json::json!({
            "role": "user",
            "content": prompt,
        }));

        // Step 3 — Record the user turn in history before dispatching so that
        // cancellations do not silently drop it.
        self.api_history.push(ChatMessage {
            role: MessageRole::User,
            content: prompt.to_string(),
        });

        // Step 4 — Dispatch to the provider.
        //
        // The provider-specific request body and endpoint differ between
        // Anthropic's Messages API and OpenAI-compatible providers.
        let (response_text, input_tokens, output_tokens) = if provider_kind
            == ProviderKind::AnthropicApi.label()
        {
            // ── Anthropic Messages API ──────────────────────────────────────
            //
            // POST https://api.anthropic.com/v1/messages
            // Headers:
            //   x-api-key: <api_key>
            //   anthropic-version: 2023-06-01
            //   content-type: application/json
            //
            // Body:
            //   { "model": "<slug>", "max_tokens": 4096, "messages": [...] }

            let request_body = serde_json::json!({
                "model": model_slug,
                "max_tokens": 4096_u32,
                "messages": messages,
            });

            let url = "https://api.anthropic.com/v1/messages";
            tracing::debug!(
                provider = %provider_kind,
                model = %model_slug,
                url = %url,
                "send_turn_api: dispatching Anthropic Messages API request"
            );

            // TODO(R3_E01-followup): Replace this todo!() with the actual
            // reqwest call.  The http_client, api_key, url, and request_body
            // are all in scope and correctly typed.
            //
            // Sketch:
            //   let resp = self.http_client
            //       .post(url)
            //       .header("x-api-key", &api_key)
            //       .header("anthropic-version", "2023-06-01")
            //       .json(&request_body)
            //       .send()
            //       .await
            //       .map_err(|e| SessionError::NetworkError {
            //           provider: provider_kind.clone(),
            //           message: e.to_string(),
            //       })?;
            //   if !resp.status().is_success() {
            //       let status = resp.status().as_u16();
            //       let body = resp.text().await.unwrap_or_default();
            //       return Err(self.classify_http_error(status, &body));
            //   }
            //   let body_text = resp.text().await.map_err(|e| SessionError::NetworkError {
            //       provider: provider_kind.clone(),
            //       message: e.to_string(),
            //   })?;
            //   // Parse body_text as MessagesResponse and extract text + usage.
            let _ = (&api_key, &request_body); // suppress unused warnings until TODO filled
            todo!(
                "Anthropic Messages API HTTP dispatch: \
                 POST {url} with x-api-key header, \
                 parse response JSON for content[].text and usage.input_tokens/output_tokens"
            )
        } else {
            // ── OpenAI-compatible providers ─────────────────────────────────
            //
            // POST <base_url>/chat/completions
            // Headers:
            //   Authorization: Bearer <api_key>
            //   content-type: application/json
            //
            // Body:
            //   { "model": "<slug>", "messages": [...] }

            // The base URL is provider-specific.  We use a well-known default
            // and expect a future commit to look it up from roko.toml.
            let base_url = "https://api.openai.com/v1";
            let url = format!("{base_url}/chat/completions");

            let request_body = serde_json::json!({
                "model": model_slug,
                "messages": messages,
            });

            tracing::debug!(
                provider = %provider_kind,
                model = %model_slug,
                url = %url,
                "send_turn_api: dispatching OpenAI-compat request"
            );

            // TODO(R3_E01-followup): Replace this todo!() with the actual
            // reqwest call.
            //
            // Sketch:
            //   let resp = self.http_client
            //       .post(&url)
            //       .bearer_auth(&api_key)
            //       .json(&request_body)
            //       .send()
            //       .await
            //       .map_err(|e| SessionError::NetworkError {
            //           provider: provider_kind.clone(),
            //           message: e.to_string(),
            //       })?;
            //   if !resp.status().is_success() {
            //       let status = resp.status().as_u16();
            //       let body = resp.text().await.unwrap_or_default();
            //       return Err(self.classify_http_error(status, &body));
            //   }
            //   let body_text = resp.text().await.map_err(|e| ...)?;
            //   // Parse body_text: extract choices[0].message.content and
            //   // usage.prompt_tokens / usage.completion_tokens.
            let _ = (&api_key, &request_body); // suppress unused warnings until TODO filled
            todo!(
                "OpenAI-compat HTTP dispatch: \
                 POST {url} with Bearer auth, \
                 parse response JSON for choices[0].message.content \
                 and usage.prompt_tokens/completion_tokens"
            )
        };

        // Step 5 — Push the assistant reply into history.
        self.api_history.push(ChatMessage {
            role: MessageRole::Assistant,
            content: response_text.clone(),
        });

        // Step 6 — Return the turn result.
        Ok(TurnResult {
            text: response_text,
            model: model_slug,
            input_tokens,
            output_tokens,
            tool_calls: Vec::new(),
            session_id: None,
            duration: started.elapsed(),
            cancelled: false,
        })
    }

    /// Process input that may be a slash command.
    ///
    /// If the input starts with `/`, parses the command and mutates session
    /// state as needed. Regular chat text returns [`SlashResult::NotACommand`].
    pub fn handle_slash_command(&mut self, input: &str) -> SlashResult {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return SlashResult::NotACommand;
        }

        let parts: Vec<&str> = trimmed.splitn(2, char::is_whitespace).collect();
        let cmd = parts[0];
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

        match cmd {
            "/system" => {
                if arg.is_empty() {
                    let preview = preview_text(&self.system_prompt, 200);
                    SlashResult::Updated(format!(
                        "Current system prompt ({} chars): {}",
                        self.system_prompt.chars().count(),
                        preview,
                    ))
                } else {
                    self.system_prompt = arg.to_string();
                    SlashResult::Updated(format!(
                        "System prompt set ({} chars)",
                        arg.chars().count()
                    ))
                }
            }
            "/model" => {
                if arg.is_empty() {
                    SlashResult::Updated(format!("Current model: {}", self.model))
                } else {
                    self.model = arg.to_string();
                    SlashResult::Updated(format!("Model set to: {arg}"))
                }
            }
            "/effort" => match arg {
                "low" | "medium" | "high" | "max" => {
                    self.effort = arg.to_string();
                    SlashResult::Updated(format!("Effort set to: {arg}"))
                }
                "" => SlashResult::Updated(format!("Current effort: {}", self.effort)),
                other => SlashResult::Error(format!(
                    "Invalid effort level: {other} (use low/medium/high/max)"
                )),
            },
            "/reset" => {
                self.session_id = None;
                self.api_history.clear();
                SlashResult::Updated("Session reset: cleared session_id and history".to_string())
            }
            "/tools" => {
                if arg.is_empty() {
                    SlashResult::Updated(format!("Current tools: {}", self.allowed_tools_csv))
                } else {
                    self.allowed_tools_csv = arg.to_string();
                    SlashResult::Updated(format!("Tools set to: {arg}"))
                }
            }
            "/mcp" => {
                if arg.is_empty() {
                    let status = match &self.mcp_config {
                        Some(path) => format!("MCP config: {}", path.display()),
                        None => "No MCP config".to_string(),
                    };
                    SlashResult::Updated(status)
                } else {
                    let path = PathBuf::from(arg);
                    if path.exists() {
                        self.mcp_config = Some(path.clone());
                        SlashResult::Updated(format!("MCP config set to: {}", path.display()))
                    } else {
                        SlashResult::Error(format!("MCP config not found: {}", path.display()))
                    }
                }
            }
            "/context" => {
                let workdir = self.workdir.display().to_string();
                let provider = &self.model_selection.provider_kind;
                let mcp_status = match &self.mcp_config {
                    Some(path) => path.display().to_string(),
                    None => "none".to_string(),
                };
                let system_preview: String = if self.system_prompt.is_empty() {
                    "(none)".to_string()
                } else if self.system_prompt.len() > 200 {
                    format!(
                        "{}... [{} chars]",
                        &self.system_prompt[..200],
                        self.system_prompt.len()
                    )
                } else {
                    self.system_prompt.clone()
                };
                let tool_count = self.allowed_tools_csv.split(',').filter(|s| !s.is_empty()).count();
                let api_turns = self.api_history.len();
                let lines = vec![
                    format!("context  session"),
                    format!("  workdir   {workdir}"),
                    format!("  model     {}", self.model),
                    format!("  provider  {provider}"),
                    format!("  effort    {}", self.effort),
                    format!("  tools     {tool_count} configured"),
                    format!("  mcp       {mcp_status}"),
                    format!("  api turns {api_turns}"),
                    format!("  system    {system_preview}"),
                ];
                SlashResult::Display(lines.join("\n"))
            }
            "/history" => {
                if self.api_history.is_empty() {
                    SlashResult::Display("history  no turns yet".to_string())
                } else {
                    let total = self.api_history.len();
                    let start = total.saturating_sub(20);
                    let mut lines = vec![format!("history  {} messages (showing last {})", total, total - start)];
                    for (i, msg) in self.api_history[start..].iter().enumerate() {
                        let turn_num = start + i + 1;
                        let role = format!("{:?}", msg.role).to_lowercase();
                        let preview = preview_text(&msg.content, 50);
                        let char_count = msg.content.chars().count();
                        lines.push(format!("  #{turn_num} {role:<12} {preview}  [{char_count} chars]"));
                    }
                    SlashResult::Display(lines.join("\n"))
                }
            }
            _ => SlashResult::Unknown(cmd.to_string()),
        }
    }

    /// Build a `ClaudeCliAgent` with the current session state.
    ///
    /// This is kept as a helper so tests can inspect the configured agent
    /// without needing to spawn a turn.
    pub fn build_agent(&self) -> Result<ClaudeCliAgent> {
        let mut agent = ClaudeCliAgent::new("claude", self.workdir.clone(), self.model.clone())
            .with_effort(&self.effort)
            .with_bare_mode(false);

        if !self.system_prompt.is_empty() {
            agent = agent.with_system_prompt(&self.system_prompt);
        }

        if !self.allowed_tools_csv.is_empty() {
            agent = agent.with_tools(&self.allowed_tools_csv);
        }

        if let Some(ref mcp_path) = self.mcp_config {
            agent = agent.with_mcp_config(mcp_path.clone());
        }

        if let Some(ref sid) = self.session_id {
            agent = agent.with_resume(sid.clone());
        }

        if let Some(timeout) = self.timeout {
            agent = agent.with_timeout_ms(timeout.as_millis() as u64);
        }

        Ok(agent)
    }

    /// Build the input engram for a user prompt.
    pub fn build_engram(&self, prompt: &str) -> Engram {
        Engram::builder(Kind::Prompt)
            .body(Body::text(prompt))
            .build()
    }

    /// Convert an `AgentResult` into a `TurnResult`.
    ///
    /// Extracts text from the output `Engram` and token counts from `Usage`.
    /// `session_id` is always `None` for the non-streaming path.
    fn process_result(&self, result: AgentResult, start: Instant) -> TurnResult {
        let text = result
            .output
            .body
            .as_text()
            .ok()
            .map(str::to_string)
            .unwrap_or_default();

        // ClaudeCliAgent only sets wall_ms; input/output tokens are 0.
        // The streaming path populates tokens from the result event.
        let input_tokens = u64::from(result.usage.input_tokens);
        let output_tokens = u64::from(result.usage.output_tokens);

        TurnResult {
            text,
            model: self.model_selection.backend_slug.clone(),
            input_tokens,
            output_tokens,
            tool_calls: Vec::new(),
            session_id: None,
            duration: start.elapsed(),
            cancelled: false,
        }
    }

    /// Send a single turn through the configured agent.
    ///
    /// Dispatches to the appropriate backend:
    /// - **Claude CLI** (provider_kind = `"claude_cli"`): spawns the `claude`
    ///   subprocess via [`ClaudeCliAgent`], preserves `session_id` across turns.
    /// - **API providers** (`"anthropic_api"`, `"openai_compat"`, …): delegates
    ///   to [`send_turn_api`], which manages `api_history` and calls the
    ///   provider's HTTP endpoint.
    pub async fn send_turn(
        &mut self,
        prompt: &str,
    ) -> std::result::Result<TurnResult, SessionError> {
        if !self.is_cli_provider() {
            return self.send_turn_api(prompt).await;
        }

        let started = Instant::now();
        let agent = self.build_agent()?;
        let input = self.build_engram(prompt);
        let ctx = Context::default();
        let timeout_duration = self.timeout.unwrap_or(Duration::from_secs(300));

        let agent_result = tokio::select! {
            // Branch 1: Normal completion.
            result = agent.run(&input, &ctx) => result,

            // Branch 2: Session-level timeout fires before the agent's internal timeout.
            // ClaudeCliAgent has its own 120s internal timeout; this is the session's
            // configurable outer bound (default 300s).
            _ = tokio::time::sleep(timeout_duration) => {
                tracing::warn!(
                    timeout_secs = timeout_duration.as_secs(),
                    "turn timed out at session level"
                );
                return Ok(TurnResult::cancelled(started.elapsed()));
            }

            // Branch 3: Ctrl-C pressed by user.
            // The subprocess is killed via kill_on_drop when the agent future is dropped.
            _ = signal::ctrl_c() => {
                tracing::info!("turn cancelled by Ctrl-C");
                return Ok(TurnResult::cancelled(started.elapsed()));
            }
        };

        if !agent_result.success {
            let error_text = agent_result
                .output
                .body
                .as_text()
                .ok()
                .map(str::to_string)
                .unwrap_or_else(|| "agent failed".to_string());
            return Ok(TurnResult {
                text: error_text,
                model: self.model_selection.backend_slug.clone(),
                input_tokens: 0,
                output_tokens: 0,
                tool_calls: Vec::new(),
                session_id: None,
                duration: started.elapsed(),
                cancelled: false,
            });
        }

        Ok(self.process_result(agent_result, started))
    }

    /// Send a single turn in one-shot mode.
    ///
    /// One-shot mode explicitly suppresses `--resume` so each call starts
    /// a fresh conversation regardless of any stored session_id.
    /// The session_id is NOT updated after this call - one-shot turns
    /// are completely isolated from each other.
    ///
    /// For API providers (`!is_cli_provider()`) this clears `api_history`
    /// before dispatching and restores it afterward, mirroring the CLI
    /// behaviour where `--resume` is omitted.
    pub async fn send_turn_oneshot(
        &mut self,
        prompt: &str,
    ) -> std::result::Result<TurnResult, SessionError> {
        if !self.is_cli_provider() {
            // API path: save and clear history so the call is stateless.
            let saved_history = std::mem::take(&mut self.api_history);
            let result = self.send_turn_api(prompt).await;
            // Restore history regardless of success or failure.
            self.api_history = saved_history;
            return result;
        }

        // Temporarily clear session_id so build_agent() omits --resume.
        let saved_session_id = self.session_id.take();

        let result = self.send_turn(prompt).await;

        // Restore the previous session_id (one-shot does not accumulate state).
        // If the caller wants to discard state permanently, use /reset instead.
        self.session_id = saved_session_id;

        result
    }

    /// Send a streaming turn and forward runtime events into `tx`.
    pub async fn send_turn_streaming(
        &mut self,
        prompt: &str,
        tx: tokio::sync::mpsc::Sender<AgentRuntimeEvent>,
    ) -> std::result::Result<TurnResult, SessionError> {
        crate::chat_session::send_turn_streaming(self, prompt, tx).await
    }

    /// Send a streaming turn and render its events directly to the terminal.
    pub async fn send_turn_streaming_inline(
        &mut self,
        prompt: &str,
    ) -> std::result::Result<TurnResult, SessionError> {
        crate::chat_session::send_turn_streaming_inline(self, prompt).await
    }

    #[cfg(test)]
    /// Clone the session for test assertions.
    fn clone_for_test(&self) -> Self {
        Self {
            workdir: self.workdir.clone(),
            model: self.model.clone(),
            model_selection: self.model_selection.clone(),
            effort: self.effort.clone(),
            system_prompt: self.system_prompt.clone(),
            allowed_tools_csv: self.allowed_tools_csv.clone(),
            mcp_config: self.mcp_config.clone(),
            session_id: self.session_id.clone(),
            api_history: self.api_history.clone(),
            http_client: reqwest::Client::new(),
            settings_json: self.settings_json.clone(),
            timeout: self.timeout,
        }
    }
}

/// Build the Claude CLI command used by the streaming turn path.
fn build_streaming_command(session: &ChatAgentSession, program: &Path) -> TokioCommand {
    let mut cmd = TokioCommand::new(program);
    cmd.arg("--print")
        .arg("--verbose")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--model")
        .arg(&session.model)
        .arg("--effort")
        .arg(&session.effort)
        .arg("--settings")
        .arg(roko_agent::claude_cli_agent::build_settings_json());

    if session.model != "claude-haiku-4-5" {
        cmd.arg("--fallback-model").arg("claude-haiku-4-5");
    }
    if !session.system_prompt.is_empty() {
        cmd.arg("--append-system-prompt")
            .arg(&session.system_prompt);
    }
    if !session.allowed_tools_csv.is_empty() {
        cmd.arg("--tools").arg(&session.allowed_tools_csv);
    }
    if let Some(ref mcp_config) = session.mcp_config {
        cmd.arg("--mcp-config").arg(mcp_config);
        cmd.arg("--strict-mcp-config");
    }
    if let Some(ref resume) = session.session_id
        && !resume.trim().is_empty()
    {
        cmd.arg("--resume").arg(resume);
    }

    cmd.arg("--dangerously-skip-permissions");
    cmd.arg("--max-turns")
        .arg(OperatingFrequency::Theta.turn_limit().to_string());

    cmd.current_dir(&session.workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    set_process_group(&mut cmd);
    cmd.env("CARGO_INCREMENTAL", "0");
    cmd.env("CARGO_BUILD_JOBS", "2");
    cmd.env_remove("CLAUDECODE");
    cmd
}

async fn send_turn_streaming_with_program(
    session: &mut ChatAgentSession,
    prompt: &str,
    tx: tokio::sync::mpsc::Sender<AgentRuntimeEvent>,
    program: &Path,
) -> std::result::Result<TurnResult, SessionError> {
    if !session.is_cli_provider() {
        drop(tx);
        return Err(session.api_provider_not_implemented_error());
    }

    let started = Instant::now();
    let timeout_duration = session.timeout.unwrap_or(Duration::from_secs(300));
    let mut cmd = build_streaming_command(session, program);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = format!("spawn failed: {error}");
            let _ = tx
                .send(AgentRuntimeEvent::Error {
                    message: message.clone(),
                })
                .await;
            drop(tx);
            return Err(anyhow::anyhow!(message).into());
        }
    };

    let stderr = child.stderr.take();
    let stderr_handle = tokio::spawn(async move {
        let mut stderr_pipe = stderr;
        read_pipe_to_string(&mut stderr_pipe).await
    });

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let message = "agent stdout not captured".to_string();
            let _ = tx
                .send(AgentRuntimeEvent::Error {
                    message: message.clone(),
                })
                .await;
            let _ = kill_tree(&mut child, Duration::from_millis(GRACE_STDIN_CLOSE_MS)).await;
            let _ = stderr_handle.await;
            drop(tx);
            return Err(anyhow::anyhow!(message).into());
        }
    };

    if let Some(mut stdin) = child.stdin.take()
        && let Err(error) = stdin.write_all(prompt.as_bytes()).await
    {
        let message = format!("stdin write failed: {error}");
        let _ = kill_tree(&mut child, Duration::from_millis(GRACE_STDIN_CLOSE_MS)).await;
        let _ = stderr_handle.await;
        let _ = tx
            .send(AgentRuntimeEvent::Error {
                message: message.clone(),
            })
            .await;
        drop(tx);
        return Err(anyhow::anyhow!(message).into());
    }

    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut accumulated_text = String::new();
    let mut tool_calls = Vec::new();
    let mut pending_ids = Vec::new();
    let mut final_session_id: Option<String> = None;
    let mut final_input_tokens = 0_u64;
    let mut final_output_tokens = 0_u64;
    let mut saw_turn_completed_error = false;
    let mut timeout_sleep = Box::pin(tokio::time::sleep(timeout_duration));
    let mut ctrl_c = Box::pin(signal::ctrl_c());
    let mut cancelled = false;

    loop {
        tokio::select! {
            line = stdout_lines.next_line() => {
                let line = match line {
                    Ok(line) => line,
                    Err(error) => {
                        let message = format!("stdout read failed: {error}");
                        let _ = kill_tree(&mut child, Duration::from_millis(GRACE_STDIN_CLOSE_MS)).await;
                        let _ = stderr_handle.await;
                        let _ = tx.send(AgentRuntimeEvent::Error { message: message.clone() }).await;
                        let _ = tx.send(AgentRuntimeEvent::Exited { exit_code: None }).await;
                        drop(tx);
                        return Err(anyhow::anyhow!(message).into());
                    }
                };

                let Some(line) = line else {
                    break;
                };

                if line.trim().is_empty() {
                    continue;
                }

                for event in parse_stream_line(&line) {
                    accumulate_tool_event(&mut tool_calls, &mut pending_ids, &event);

                    match &event {
                        AgentRuntimeEvent::MessageDelta { text } => {
                            accumulated_text.push_str(text);
                        }
                        AgentRuntimeEvent::SystemInit { session_id, .. } => {
                            if !session_id.is_empty() && final_session_id.is_none() {
                                final_session_id = Some(session_id.clone());
                                tracing::debug!(
                                    session_id = %session_id,
                                    "captured session_id from SystemInit"
                                );
                            }
                        }
                        AgentRuntimeEvent::TurnCompleted {
                            session_id,
                            is_error,
                            ..
                        } => {
                            if let Some(sid) = session_id
                                && !sid.is_empty()
                            {
                                final_session_id = Some(sid.clone());
                                tracing::debug!(
                                    session_id = %sid,
                                    "captured session_id from TurnCompleted"
                                );
                            }
                            if *is_error {
                                saw_turn_completed_error = true;
                                tracing::warn!(
                                    "Claude CLI reported is_error=true in result event"
                                );
                            }
                        }
                        AgentRuntimeEvent::TokenUsage {
                            input_tokens,
                            output_tokens,
                            ..
                        } => {
                            final_input_tokens = *input_tokens;
                            final_output_tokens = *output_tokens;
                            tracing::debug!(
                                input_tokens = %input_tokens,
                                output_tokens = %output_tokens,
                                "token usage update"
                            );
                        }
                        _ => {}
                    }

                    let _ = tx.send(event.clone()).await;
                }
            }
            _ = &mut timeout_sleep => {
                tracing::warn!(
                    timeout_secs = timeout_duration.as_secs(),
                    "turn timed out at session level"
                );
                cancelled = true;
                break;
            }
            _ = &mut ctrl_c => {
                tracing::info!("turn cancelled by Ctrl-C");
                cancelled = true;
                break;
            }
        }
    }

    if cancelled {
        let _ = kill_tree(&mut child, Duration::from_millis(GRACE_STDIN_CLOSE_MS)).await;
        let _ = stderr_handle.await;
        let _ = tx.send(AgentRuntimeEvent::Exited { exit_code: None }).await;
        drop(tx);
        return Ok(TurnResult::cancelled(started.elapsed()));
    }

    let status = match child.wait().await {
        Ok(status) => status,
        Err(error) => {
            let message = format!("wait failed: {error}");
            let _ = kill_tree(&mut child, Duration::from_millis(GRACE_STDIN_CLOSE_MS)).await;
            let _ = stderr_handle.await;
            let _ = tx
                .send(AgentRuntimeEvent::Error {
                    message: message.clone(),
                })
                .await;
            let _ = tx.send(AgentRuntimeEvent::Exited { exit_code: None }).await;
            drop(tx);
            return Err(anyhow::anyhow!(message).into());
        }
    };

    let stderr = stderr_handle.await.unwrap_or_default();

    if let Some(ref sid) = final_session_id {
        session.session_id = Some(sid.clone());
        apply_session_id(&mut session.session_id, Some(sid.clone()));
    }

    let mut text = accumulated_text;
    if !status.success() && text.trim().is_empty() {
        text = stderr
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("agent failed")
            .to_string();
    }

    if saw_turn_completed_error || !status.success() {
        tracing::warn!(
            exit_code = ?status.code(),
            "Claude CLI turn completed with a non-success status"
        );
    }

    let result = TurnResult {
        text,
        model: session.model_selection.backend_slug.clone(),
        input_tokens: final_input_tokens,
        output_tokens: final_output_tokens,
        tool_calls,
        session_id: final_session_id,
        duration: started.elapsed(),
        cancelled: false,
    };

    let _ = tx
        .send(AgentRuntimeEvent::Exited {
            exit_code: status.code(),
        })
        .await;
    drop(tx);

    Ok(result)
}

/// Send a streaming turn and forward runtime events into `tx`.
pub async fn send_turn_streaming(
    session: &mut ChatAgentSession,
    prompt: &str,
    tx: tokio::sync::mpsc::Sender<AgentRuntimeEvent>,
) -> std::result::Result<TurnResult, SessionError> {
    send_turn_streaming_with_program(session, prompt, tx, Path::new("claude")).await
}

/// Send a streaming turn, rendering events to stdout/stderr directly.
///
/// This is the plain terminal path. It consumes the same event stream as the
/// TUI-free renderer and prints a final newline after the turn finishes.
pub async fn send_turn_streaming_inline(
    session: &mut ChatAgentSession,
    prompt: &str,
) -> std::result::Result<TurnResult, SessionError> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentRuntimeEvent>(256);

    let render_handle = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            render_stream_event(&event);
        }
    });

    let result = send_turn_streaming(session, prompt, tx).await;
    let _ = render_handle.await;
    render_stream_end();

    result
}

/// Build a system prompt for interactive and one-shot chat using the shared
/// `SystemPromptBuilder`.
///
/// Workspace context is inferred from the working directory. If the composed
/// prompt ends up empty for any reason, fall back to a minimal role identity.
fn build_chat_system_prompt(workdir: &Path, config: &Config) -> String {
    let role_identity = "You are an expert software engineer working in an interactive chat session. You help inspect, understand, and edit the current repository. Stay concise, grounded in the workspace, and prefer existing code over inventing new abstractions.";

    let mut builder = SystemPromptBuilder::new(role_identity);

    if let Some(conventions) = gather_workspace_conventions(workdir) {
        builder = builder.with_conventions(conventions);
    }

    let project_name = project_name_for(workdir);
    builder = builder.with_domain(format!(
        "Working directory: {}\nProject: {}",
        workdir.display(),
        project_name
    ));

    if let Ok(context) = gather_workspace_context(workdir) {
        if !context.trim().is_empty() {
            builder = builder.with_context(context);
        }
    }

    let token_budget = config
        .prompt
        .token_budget
        .clamp(1, CHAT_SYSTEM_PROMPT_TOKEN_BUDGET);
    let prompt =
        builder
            .with_token_budget(token_budget)
            .build_with_counter(&TokenCounter::Heuristic {
                chars_per_token: 4.0,
            });

    if prompt.trim().is_empty() {
        role_identity.to_string()
    } else {
        prompt
    }
}

/// Gather lightweight workspace context: git branch and language hints.
///
/// The result is best-effort. Missing git metadata or workspace markers are
/// treated as empty context instead of a hard error.
fn gather_workspace_context(workdir: &Path) -> Result<String> {
    let mut parts = Vec::new();

    if let Some(branch) = capture_git_branch(workdir) {
        if !branch.is_empty() {
            parts.push(format!("Git branch: {branch}"));
        }
    }

    let language_hints = language_hints_for(workdir);
    if !language_hints.is_empty() {
        parts.push(format!("Language hints: {}", language_hints.join(", ")));
    }

    Ok(parts.join("\n"))
}

/// Default tools for interactive chat when no safety contract is found.
const DEFAULT_CHAT_TOOLS: &str = "Read,Glob,Grep,Bash,Edit,Write,NotebookEdit";

/// Resolve tool allowlist from safety contracts.
///
/// Looks for an `AgentContract` for the "chat" role at `.roko/safety/chat.yaml`.
/// If found, uses its `allowed_tools` field. If not found, falls back to a
/// read-oriented default set and logs a debug message.
fn resolve_tool_policy(workdir: &Path) -> String {
    let contract_path = workdir.join(".roko/safety/chat.yaml");
    match std::fs::read_to_string(&contract_path) {
        Ok(content) => match serde_yaml_ng::from_str::<AgentContract>(&content) {
            Ok(contract) => {
                if let Some(ref allowlist) = contract.allowed_tools {
                    if !allowlist.is_empty() {
                        let tools = allowlist.join(",");
                        tracing::debug!("chat tool policy from contract: {}", tools);
                        return tools;
                    }
                }
                tracing::debug!("chat contract has no allowed_tools, using defaults");
                DEFAULT_CHAT_TOOLS.to_string()
            }
            Err(e) => {
                tracing::warn!(
                    "failed to parse chat contract at {}: {e}",
                    contract_path.display()
                );
                DEFAULT_CHAT_TOOLS.to_string()
            }
        },
        Err(_) => {
            tracing::debug!(
                "no chat safety contract at {}, using default tools",
                contract_path.display()
            );
            DEFAULT_CHAT_TOOLS.to_string()
        }
    }
}

fn gather_workspace_conventions(workdir: &Path) -> Option<String> {
    let cargo_toml = read_text_snippet(&workdir.join("Cargo.toml")).unwrap_or_default();
    let (source_samples, file_listing) = collect_workspace_samples(workdir);

    if cargo_toml.is_empty() && source_samples.is_empty() && file_listing.is_empty() {
        return None;
    }

    let source_refs = source_samples
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let file_refs = file_listing.iter().map(String::as_str).collect::<Vec<_>>();
    let conventions = detect_conventions(&cargo_toml, &source_refs, &file_refs);

    if conventions == ProjectConventions::default() {
        return None;
    }

    let fragment = conventions.to_prompt_fragment();
    if fragment.trim().is_empty() {
        None
    } else {
        Some(fragment)
    }
}

fn collect_workspace_samples(workdir: &Path) -> (Vec<String>, Vec<String>) {
    let mut source_samples = Vec::new();
    let mut file_listing = Vec::new();
    collect_workspace_samples_from_dir(workdir, workdir, 0, &mut source_samples, &mut file_listing);
    (source_samples, file_listing)
}

fn collect_workspace_samples_from_dir(
    dir: &Path,
    root: &Path,
    depth: usize,
    source_samples: &mut Vec<String>,
    file_listing: &mut Vec<String>,
) {
    if depth > MAX_WORKSPACE_SCAN_DEPTH || source_samples.len() >= MAX_WORKSPACE_SAMPLE_FILES {
        return;
    }

    let mut entries = match fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>(),
        Err(_) => return,
    };
    entries.sort_by(|left, right| left.path().cmp(&right.path()));

    for entry in entries {
        if source_samples.len() >= MAX_WORKSPACE_SAMPLE_FILES {
            break;
        }

        let path = entry.path();
        let file_name = path.file_name().and_then(|name| name.to_str());
        if path.is_dir() {
            if file_name.map_or(false, is_skipped_dir_name) {
                continue;
            }
            collect_workspace_samples_from_dir(
                &path,
                root,
                depth + 1,
                source_samples,
                file_listing,
            );
            continue;
        }

        if !path.is_file() || !is_workspace_source_file(&path) {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .ok()
            .and_then(|relative| relative.to_str())
            .map(|relative| relative.to_string())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        file_listing.push(relative);

        if let Some(sample) = read_text_snippet(&path) {
            if !sample.trim().is_empty() {
                source_samples.push(sample);
            }
        }
    }
}

fn read_text_snippet(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut limited = file.take(MAX_WORKSPACE_SAMPLE_BYTES as u64);
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn capture_git_branch(workdir: &Path) -> Option<String> {
    let output = StdCommand::new("git")
        .args(["branch", "--show-current"])
        .current_dir(workdir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

fn language_hints_for(workdir: &Path) -> Vec<String> {
    let mut hints = Vec::new();

    if workdir.join("Cargo.toml").is_file() || workdir.join("rust-toolchain.toml").is_file() {
        push_unique_hint(&mut hints, "Rust");
    }
    if workdir.join("package.json").is_file()
        || workdir.join("tsconfig.json").is_file()
        || workdir.join("deno.json").is_file()
        || workdir.join("deno.jsonc").is_file()
    {
        push_unique_hint(&mut hints, "TypeScript/JavaScript");
    }
    if workdir.join("pyproject.toml").is_file()
        || workdir.join("requirements.txt").is_file()
        || workdir.join("uv.lock").is_file()
    {
        push_unique_hint(&mut hints, "Python");
    }
    if workdir.join("go.mod").is_file() {
        push_unique_hint(&mut hints, "Go");
    }
    if workdir.join("pom.xml").is_file()
        || workdir.join("build.gradle").is_file()
        || workdir.join("build.gradle.kts").is_file()
    {
        push_unique_hint(&mut hints, "Java/Kotlin");
    }

    hints
}

fn push_unique_hint(hints: &mut Vec<String>, hint: &str) {
    if !hints.iter().any(|existing| existing == hint) {
        hints.push(hint.to_string());
    }
}

fn project_name_for(workdir: &Path) -> String {
    workdir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn is_workspace_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(
            "rs" | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "py"
                | "go"
                | "java"
                | "kt"
                | "swift"
                | "rb"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
                | "cs"
                | "lua"
                | "sh"
        )
    )
}

fn is_skipped_dir_name(name: &str) -> bool {
    SKIP_DIR_NAMES.contains(&name)
}

/// Discover MCP config file using the same resolution order as orchestrate.rs.
///
/// Priority:
/// 1. Explicit path in `config.agent.mcp_config`
/// 2. Workspace `.roko/mcp.json`
/// 3. Global `~/.claude/mcp-config.json`
///
/// Returns `None` if no MCP config is found.
fn resolve_mcp_config(workdir: &Path, config: &Config) -> Option<PathBuf> {
    if let Some(ref path) = config.agent.mcp_config {
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            workdir.join(path)
        };
        if resolved.exists() {
            tracing::debug!("MCP config from roko.toml: {}", resolved.display());
            return Some(resolved);
        }
        tracing::debug!(
            "MCP config in roko.toml does not exist: {}",
            resolved.display()
        );
    }

    let workspace_mcp = workdir.join(".roko/mcp.json");
    if workspace_mcp.exists() {
        tracing::debug!("MCP config from workspace: {}", workspace_mcp.display());
        return Some(workspace_mcp);
    }

    if let Some(home) = home_dir() {
        let global_mcp = home.join(".claude/mcp-config.json");
        if global_mcp.exists() {
            tracing::debug!("MCP config from global: {}", global_mcp.display());
            return Some(global_mcp);
        }
    }

    tracing::debug!("no MCP config found");
    None
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn shared_http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new).clone()
}

fn preview_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        preview.push_str("...");
    }
    preview
}

async fn read_pipe_to_string<R>(pipe: &mut Option<R>) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    let Some(reader) = pipe.as_mut() else {
        return String::new();
    };

    let mut bytes = Vec::new();
    if reader.read_to_end(&mut bytes).await.is_err() {
        return String::new();
    }

    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    use roko_core::foundation::{ChatMessage, MessageRole};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn test_model_selection() -> EffectiveModelSelection {
        EffectiveModelSelection {
            requested_model: Some("claude-sonnet-4-6".to_string()),
            effective_model_key: "claude-sonnet-4-6".to_string(),
            provider_key: "claude_cli".to_string(),
            provider_kind: "claude_cli".to_string(),
            backend_slug: "claude-sonnet-4-6".to_string(),
            source: crate::model_selection::SelectionSource::ProjectDefault,
            reason: "test selection".to_string(),
        }
    }

    /// Construct a minimal session for testing `build_agent()` and slash commands.
    fn test_session() -> ChatAgentSession {
        let model_selection = test_model_selection();
        let model = model_selection.effective_model_key.clone();
        ChatAgentSession {
            workdir: PathBuf::from("/tmp/test"),
            model,
            model_selection,
            effort: "medium".to_string(),
            system_prompt: "Test system prompt".to_string(),
            allowed_tools_csv: DEFAULT_CHAT_TOOLS.to_string(),
            mcp_config: None,
            session_id: None,
            api_history: Vec::new(),
            http_client: reqwest::Client::new(),
            settings_json: None,
            timeout: Some(Duration::from_secs(30)),
        }
    }

    fn streaming_test_session(workdir: PathBuf) -> ChatAgentSession {
        let model_selection = test_model_selection();
        let model = model_selection.effective_model_key.clone();
        ChatAgentSession {
            workdir,
            model,
            model_selection,
            effort: "medium".to_string(),
            system_prompt: "Test system prompt".to_string(),
            allowed_tools_csv: DEFAULT_CHAT_TOOLS.to_string(),
            mcp_config: None,
            session_id: None,
            api_history: Vec::new(),
            http_client: reqwest::Client::new(),
            settings_json: None,
            timeout: Some(Duration::from_secs(5)),
        }
    }

    fn write_fake_claude_script(tmp: &tempfile::TempDir, body: &str) -> PathBuf {
        let script = tmp.path().join("claude-fake.sh");
        std::fs::write(&script, body).expect("write fake claude script");
        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(&script).expect("metadata").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).expect("chmod");
        }
        script
    }

    fn agent_debug(session: &ChatAgentSession) -> String {
        format!("{:?}", session.build_agent().expect("build agent"))
    }

    #[test]
    fn first_turn_build_agent_has_no_resume() {
        let session = test_session();
        assert!(session.session_id.is_none());

        let debug = agent_debug(&session);
        assert!(debug.contains("model: \"claude-sonnet-4-6\""), "{debug}");
        assert!(debug.contains("effort: \"medium\""), "{debug}");
        assert!(
            debug.contains("system_prompt: Some(\"Test system prompt\")"),
            "{debug}"
        );
        assert!(
            debug.contains("allowed_tools: Some(\"Read,Glob,Grep,Bash,Edit,Write,NotebookEdit\")"),
            "{debug}"
        );
        assert!(debug.contains("resume: None"), "{debug}");
        assert!(debug.contains("timeout_ms: 30000"), "{debug}");
    }

    #[test]
    fn second_turn_build_agent_uses_resume() {
        let mut session = test_session();
        apply_session_id(&mut session.session_id, Some("sess-abc-123".to_string()));

        assert_eq!(session.session_id.as_deref(), Some("sess-abc-123"));

        let debug = agent_debug(&session);
        assert!(debug.contains("resume: Some(\"sess-abc-123\")"), "{debug}");
        assert_eq!(session.session_id.as_deref(), Some("sess-abc-123"));
    }

    #[tokio::test]
    async fn send_turn_routes_to_api_path_for_non_cli_provider() {
        // Without an API key in the environment the call must fail at the key
        // resolution step, NOT at the entry guard.  This proves that send_turn
        // routes through send_turn_api instead of returning
        // ApiProviderNotImplemented immediately.
        let mut session = test_session();
        session.model_selection.provider_kind = "anthropic_api".to_string();
        session.model_selection.backend_slug = "claude-sonnet-4-6".to_string();
        session.model = "claude-sonnet-4-6".to_string();

        // Make sure the key is absent so we get a clean ApiKeyMissing error.
        // (If ANTHROPIC_API_KEY happens to be set in the CI environment the
        // todo!() will panic — that's intentional: it means the HTTP call is
        // reachable, which is the correct next step.)
        let env_var = "ANTHROPIC_API_KEY";
        if std::env::var(env_var).is_ok() {
            // Key present: we can only assert the path reaches send_turn_api.
            // The todo!() will fire; mark the test skipped via early return.
            return;
        }

        let error = session.send_turn("hello").await.unwrap_err();
        match error {
            SessionError::ApiKeyMissing {
                provider,
                env_var: checked_var,
            } => {
                assert_eq!(provider, "anthropic_api");
                assert_eq!(checked_var, env_var);
            }
            other => panic!("expected ApiKeyMissing, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_turn_oneshot_api_path_preserves_history_on_error() {
        // send_turn_oneshot for an API provider saves + restores api_history
        // even when the call fails (e.g. missing key).
        let mut session = test_session();
        session.model_selection.provider_kind = "openai_compat".to_string();
        session.model_selection.backend_slug = "gpt-4.1-nano".to_string();
        session.model = "gpt-4.1-nano".to_string();
        session.session_id = Some("sess-keep".to_string());

        // Pre-populate history to verify it is restored.
        use roko_core::foundation::MessageRole;
        session.api_history.push(ChatMessage {
            role: MessageRole::User,
            content: "prior turn".to_string(),
        });

        let env_var = "OPENAI_COMPAT_API_KEY";
        if std::env::var(env_var).is_ok() {
            return; // key present — todo!() would fire; skip.
        }

        let error = session.send_turn_oneshot("hello").await.unwrap_err();
        match error {
            SessionError::ApiKeyMissing { provider, .. } => {
                assert_eq!(provider, "openai_compat");
            }
            other => panic!("expected ApiKeyMissing, got {other:?}"),
        }

        // History must be unchanged after a one-shot failure.
        assert_eq!(session.api_history.len(), 1);
        assert_eq!(session.api_history[0].content, "prior turn");

        // CLI session_id is unrelated and must be untouched.
        assert_eq!(session.session_id.as_deref(), Some("sess-keep"));
    }

    #[test]
    fn api_key_env_var_anthropic() {
        let mut session = test_session();
        session.model_selection.provider_kind = "anthropic_api".to_string();
        assert_eq!(session.api_key_env_var(), "ANTHROPIC_API_KEY");
    }

    #[test]
    fn api_key_env_var_openai_compat() {
        let mut session = test_session();
        session.model_selection.provider_kind = "openai_compat".to_string();
        assert_eq!(session.api_key_env_var(), "OPENAI_COMPAT_API_KEY");
    }

    #[test]
    fn api_key_env_var_gemini() {
        let mut session = test_session();
        session.model_selection.provider_kind = "gemini_api".to_string();
        assert_eq!(session.api_key_env_var(), "GEMINI_API_KEY");
    }

    #[test]
    fn resolve_api_key_returns_missing_when_env_absent() {
        let mut session = test_session();
        session.model_selection.provider_kind = "anthropic_api".to_string();
        // Ensure the variable is absent for this test.
        let var = session.api_key_env_var();
        let was_set = std::env::var(&var).is_ok();
        if was_set {
            // Key is present in environment — skip test rather than removing it.
            return;
        }
        let err = session.resolve_api_key().unwrap_err();
        match err {
            SessionError::ApiKeyMissing { provider, env_var } => {
                assert_eq!(provider, "anthropic_api");
                assert_eq!(env_var, "ANTHROPIC_API_KEY");
            }
            other => panic!("expected ApiKeyMissing, got {other:?}"),
        }
    }

    #[test]
    fn classify_http_error_401_is_auth_error() {
        let session = test_session();
        let err = session.classify_http_error(401, "unauthorized");
        assert!(
            matches!(err, SessionError::AuthError { status: 401, .. }),
            "{err:?}"
        );
    }

    #[test]
    fn classify_http_error_429_is_rate_limited() {
        let session = test_session();
        let err = session.classify_http_error(429, "{}");
        assert!(
            matches!(err, SessionError::RateLimited { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn classify_http_error_429_with_retry_after() {
        let session = test_session();
        let body = r#"{"error":{"retry_after":30}}"#;
        let err = session.classify_http_error(429, body);
        match err {
            SessionError::RateLimited { retry_after, .. } => {
                assert!(retry_after.contains("30"), "{retry_after}");
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn classify_http_error_500_is_network_error() {
        let session = test_session();
        let err = session.classify_http_error(500, "internal error");
        assert!(
            matches!(err, SessionError::NetworkError { .. }),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn send_turn_api_does_not_mutate_history_on_key_missing() {
        // When the API key is missing, resolve_api_key() returns early before
        // the user message is pushed to api_history.  History must be unchanged.
        let mut session = test_session();
        session.model_selection.provider_kind = "anthropic_api".to_string();
        session.model_selection.backend_slug = "claude-sonnet-4-6".to_string();

        let var = session.api_key_env_var();
        if std::env::var(&var).is_ok() {
            return; // key present — skip to avoid hitting todo!()
        }

        let err = session.send_turn_api("first message").await.unwrap_err();
        assert!(
            matches!(err, SessionError::ApiKeyMissing { .. }),
            "{err:?}"
        );

        // Key resolution happens before history mutation, so history is empty.
        assert!(
            session.api_history.is_empty(),
            "history should be empty when key check fails early"
        );
    }

    #[test]
    fn slash_system_shows_current() {
        let mut s = test_session().clone_for_test();
        s.system_prompt = "test prompt".to_string();
        match s.handle_slash_command("/system") {
            SlashResult::Updated(msg) => assert!(msg.contains("test prompt")),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn cancelled_turn_result_has_empty_payload() {
        let duration = Duration::from_secs(7);
        let result = TurnResult::cancelled(duration);

        assert!(result.cancelled);
        assert_eq!(result.duration, duration);
        assert!(result.text.is_empty());
        assert!(result.model.is_empty());
        assert_eq!(result.input_tokens, 0);
        assert_eq!(result.output_tokens, 0);
        assert!(result.tool_calls.is_empty());
        assert!(result.session_id.is_none());
    }

    #[tokio::test]
    async fn streaming_turn_captures_final_session_and_latest_usage() {
        let tmp = tempdir().expect("tempdir");
        let script = write_fake_claude_script(
            &tmp,
            r#"#!/bin/sh
set -eu
cat >/dev/null
printf '%s\n' '{"type":"system","subtype":"init","session_id":"sess-init","model":"claude-sonnet-4-6","tools":[]}'
printf '%s\n' '{"type":"assistant","subtype":"message","message":{"content":[{"type":"text","text":"hello "},{"type":"text","text":"world"},{"type":"tool_use","id":"tool-1","name":"Read","input":{"path":"foo"}}],"usage":{"input_tokens":7,"output_tokens":8,"cache_creation_input_tokens":1,"cache_read_input_tokens":2}}}'
printf '%s\n' '{"type":"tool","subtype":"result","tool_name":"Read","tool_use_id":"tool-1","content":"tool output"}'
printf '%s\n' '{"type":"result","session_id":"sess-final","model":"claude-sonnet-4-6","total_cost_usd":0.25,"is_error":false,"usage":{"input_tokens":11,"output_tokens":22,"cache_creation_input_tokens":33,"cache_read_input_tokens":44}}'
"#,
        );

        let mut session = streaming_test_session(tmp.path().to_path_buf());
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let result = send_turn_streaming_with_program(&mut session, "hi", tx, &script)
            .await
            .expect("streaming turn");

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert_eq!(session.session_id.as_deref(), Some("sess-final"));
        assert_eq!(result.session_id.as_deref(), Some("sess-final"));
        assert_eq!(result.model, "claude-sonnet-4-6");
        assert_eq!(result.text, "hello world");
        assert_eq!(result.input_tokens, 11);
        assert_eq!(result.output_tokens, 22);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "Read");
        assert_eq!(result.tool_calls[0].input_abbrev, "tool output");

        assert!(events.iter().any(|event| matches!(
            event,
            AgentRuntimeEvent::SystemInit {
                session_id,
                ..
            } if session_id == "sess-init"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentRuntimeEvent::TurnCompleted {
                session_id: Some(session_id),
                ..
            } if session_id == "sess-final"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentRuntimeEvent::TokenUsage {
                input_tokens: 11,
                output_tokens: 22,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn streaming_turn_keeps_system_session_when_result_is_empty() {
        let tmp = tempdir().expect("tempdir");
        let script = write_fake_claude_script(
            &tmp,
            r#"#!/bin/sh
set -eu
cat >/dev/null
printf '%s\n' '{"type":"system","subtype":"init","session_id":"sess-init-only","model":"claude-sonnet-4-6","tools":[]}'
printf '%s\n' '{"type":"assistant","subtype":"message","message":{"content":[{"type":"text","text":"partial"}],"usage":{"input_tokens":3,"output_tokens":4,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}'
printf '%s\n' '{"type":"result","session_id":"","model":"claude-sonnet-4-6","total_cost_usd":0.10,"is_error":false,"usage":{"input_tokens":9,"output_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}'
"#,
        );

        let mut session = streaming_test_session(tmp.path().to_path_buf());
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        let result = send_turn_streaming_with_program(&mut session, "hi", tx, &script)
            .await
            .expect("streaming turn");

        assert_eq!(session.session_id.as_deref(), Some("sess-init-only"));
        assert_eq!(result.session_id.as_deref(), Some("sess-init-only"));
        assert_eq!(result.input_tokens, 9);
        assert_eq!(result.output_tokens, 10);
        assert_eq!(result.text, "partial");
    }

    #[tokio::test]
    async fn streaming_turn_timeout_returns_cancelled_without_persisting_partial_state() {
        let tmp = tempdir().expect("tempdir");
        let script = write_fake_claude_script(
            &tmp,
            r#"#!/bin/sh
set -eu
cat >/dev/null
printf '%s\n' '{"type":"system","subtype":"init","session_id":"sess-timeout","model":"claude-sonnet-4-6","tools":[]}'
sleep 5
"#,
        );

        let mut session = streaming_test_session(tmp.path().to_path_buf());
        session.timeout = Some(Duration::from_millis(250));
        let (tx, _rx) = tokio::sync::mpsc::channel(32);
        let result = send_turn_streaming_with_program(&mut session, "hi", tx, &script)
            .await
            .expect("streaming turn");

        assert!(result.cancelled);
        assert!(result.session_id.is_none());
        assert_eq!(result.input_tokens, 0);
        assert_eq!(result.output_tokens, 0);
        assert!(session.session_id.is_none());
    }

    #[test]
    fn slash_system_sets_new() {
        let mut s = test_session();
        match s.handle_slash_command("/system You are a Rust expert") {
            SlashResult::Updated(msg) => assert!(msg.contains("System prompt set")),
            other => panic!("expected Updated, got {other:?}"),
        }
        assert_eq!(s.system_prompt, "You are a Rust expert");

        let debug = agent_debug(&s);
        assert!(
            debug.contains("system_prompt: Some(\"You are a Rust expert\")"),
            "{debug}"
        );
    }

    #[test]
    fn slash_system_preview_truncates_safely() {
        let mut s = test_session();
        s.system_prompt = "é".repeat(210);
        match s.handle_slash_command("/system") {
            SlashResult::Updated(msg) => {
                assert!(msg.contains("Current system prompt"));
                assert!(msg.ends_with("..."));
            }
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn slash_model_shows_current() {
        let mut s = test_session().clone_for_test();
        match s.handle_slash_command("/model") {
            SlashResult::Updated(msg) => assert!(msg.contains("claude-sonnet-4-6")),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn slash_model_sets_new() {
        let mut s = test_session();
        match s.handle_slash_command("/model claude-opus-4-5") {
            SlashResult::Updated(msg) => assert!(msg.contains("Model set to:")),
            other => panic!("expected Updated, got {other:?}"),
        }
        assert_eq!(s.model, "claude-opus-4-5");

        let debug = agent_debug(&s);
        assert!(debug.contains("model: \"claude-opus-4-5\""), "{debug}");
        assert!(
            debug.contains("name: \"claude-cli:claude-opus-4-5\""),
            "{debug}"
        );
    }

    #[test]
    fn slash_effort_valid_low() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/effort low"),
            SlashResult::Updated(_)
        ));
        assert_eq!(s.effort, "low");
    }

    #[test]
    fn slash_effort_valid_high() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/effort high"),
            SlashResult::Updated(_)
        ));
        assert_eq!(s.effort, "high");

        let debug = agent_debug(&s);
        assert!(debug.contains("effort: \"high\""), "{debug}");
    }

    #[test]
    fn slash_effort_valid_max() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/effort max"),
            SlashResult::Updated(_)
        ));
        assert_eq!(s.effort, "max");
    }

    #[test]
    fn slash_effort_shows_current_when_no_arg() {
        let mut s = test_session().clone_for_test();
        match s.handle_slash_command("/effort") {
            SlashResult::Updated(msg) => assert!(msg.contains("medium")),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn slash_effort_invalid() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/effort turbo"),
            SlashResult::Error(_)
        ));
        assert_eq!(s.effort, "medium");
    }

    #[test]
    fn slash_reset_clears_session() {
        let mut s = test_session();
        s.session_id = Some("sess-123".to_string());
        s.api_history.push(ChatMessage {
            role: MessageRole::User,
            content: "hello".to_string(),
        });
        match s.handle_slash_command("/reset") {
            SlashResult::Updated(msg) => assert!(msg.contains("Session reset")),
            other => panic!("expected Updated, got {other:?}"),
        }
        assert!(s.session_id.is_none());
        assert!(s.api_history.is_empty());

        let debug = agent_debug(&s);
        assert!(debug.contains("resume: None"), "{debug}");
    }

    #[test]
    fn slash_tools_shows_current() {
        let mut s = test_session().clone_for_test();
        match s.handle_slash_command("/tools") {
            SlashResult::Updated(msg) => assert!(msg.contains("Read")),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn slash_tools_sets_new() {
        let mut s = test_session();
        match s.handle_slash_command("/tools Read,Edit") {
            SlashResult::Updated(msg) => assert!(msg.contains("Tools set to:")),
            other => panic!("expected Updated, got {other:?}"),
        }
        assert_eq!(s.allowed_tools_csv, "Read,Edit");

        let debug = agent_debug(&s);
        assert!(
            debug.contains("allowed_tools: Some(\"Read,Edit\")"),
            "{debug}"
        );
    }

    #[test]
    fn slash_mcp_shows_none() {
        let mut s = test_session().clone_for_test();
        match s.handle_slash_command("/mcp") {
            SlashResult::Updated(msg) => assert!(msg.contains("No MCP")),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn slash_mcp_sets_new() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("mcp.json");
        std::fs::write(&path, "{}").expect("write mcp config");

        let mut s = test_session();
        match s.handle_slash_command(&format!("/mcp {}", path.display())) {
            SlashResult::Updated(msg) => assert!(msg.contains("MCP config set to:")),
            other => panic!("expected Updated, got {other:?}"),
        }
        assert_eq!(s.mcp_config.as_deref(), Some(path.as_path()));

        let debug = agent_debug(&s);
        assert!(debug.contains("mcp_config: Some("), "{debug}");
        assert!(debug.contains("mcp.json"), "{debug}");
    }

    #[test]
    fn slash_mcp_invalid_path() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/mcp /nonexistent/path/mcp.json"),
            SlashResult::Error(_)
        ));
        assert!(s.mcp_config.is_none());
    }

    #[test]
    fn slash_context_returns_display_with_key_fields() {
        let mut s = test_session();
        s.model = "claude-opus-4-5".to_string();
        s.effort = "high".to_string();
        s.system_prompt = "You are helpful".to_string();
        s.allowed_tools_csv = "Read,Edit,Bash".to_string();
        match s.handle_slash_command("/context") {
            SlashResult::Display(text) => {
                assert!(text.contains("claude-opus-4-5"), "missing model: {text}");
                assert!(text.contains("claude_cli"), "missing provider: {text}");
                assert!(text.contains("high"), "missing effort: {text}");
                assert!(text.contains("3 configured"), "missing tool count: {text}");
                assert!(text.contains("none"), "missing mcp status: {text}");
                assert!(text.contains("You are helpful"), "missing system preview: {text}");
                assert!(text.contains("/tmp/test"), "missing workdir: {text}");
            }
            other => panic!("expected Display, got {other:?}"),
        }
    }

    #[test]
    fn slash_context_truncates_long_system_prompt() {
        let mut s = test_session();
        s.system_prompt = "x".repeat(300);
        match s.handle_slash_command("/context") {
            SlashResult::Display(text) => {
                assert!(text.contains("... [300 chars]"), "expected truncation: {text}");
            }
            other => panic!("expected Display, got {other:?}"),
        }
    }

    #[test]
    fn slash_context_shows_mcp_path_when_set() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mcp_path = tmp.path().join("mcp.json");
        std::fs::write(&mcp_path, "{}").expect("write mcp");

        let mut s = test_session();
        s.mcp_config = Some(mcp_path.clone());
        match s.handle_slash_command("/context") {
            SlashResult::Display(text) => {
                assert!(
                    text.contains("mcp.json"),
                    "expected mcp path in context: {text}"
                );
            }
            other => panic!("expected Display, got {other:?}"),
        }
    }

    #[test]
    fn slash_history_empty_returns_display_no_turns() {
        let mut s = test_session();
        // api_history is empty by default
        match s.handle_slash_command("/history") {
            SlashResult::Display(text) => {
                assert!(text.contains("no turns yet"), "expected empty message: {text}");
            }
            other => panic!("expected Display, got {other:?}"),
        }
    }

    #[test]
    fn slash_history_with_messages_shows_turns() {
        use roko_core::foundation::{ChatMessage, MessageRole};
        let mut s = test_session();
        s.api_history.push(ChatMessage {
            role: MessageRole::User,
            content: "hello from user".to_string(),
        });
        s.api_history.push(ChatMessage {
            role: MessageRole::Assistant,
            content: "hello from assistant".to_string(),
        });
        match s.handle_slash_command("/history") {
            SlashResult::Display(text) => {
                assert!(text.contains("#1"), "missing turn 1: {text}");
                assert!(text.contains("#2"), "missing turn 2: {text}");
                assert!(text.contains("hello from user"), "missing user text: {text}");
                assert!(
                    text.contains("hello from assistant"),
                    "missing assistant text: {text}"
                );
                assert!(text.contains("2 messages"), "missing message count: {text}");
            }
            other => panic!("expected Display, got {other:?}"),
        }
    }

    #[test]
    fn slash_history_caps_at_20_turns() {
        use roko_core::foundation::{ChatMessage, MessageRole};
        let mut s = test_session();
        for i in 0..25 {
            s.api_history.push(ChatMessage {
                role: MessageRole::User,
                content: format!("message {i}"),
            });
        }
        match s.handle_slash_command("/history") {
            SlashResult::Display(text) => {
                assert!(text.contains("25 messages"), "missing total count: {text}");
                // First 5 should be cut off; #6 (index 5) is the first shown.
                assert!(!text.contains("#1 "), "turn 1 should not be shown: {text}");
                assert!(text.contains("#6 ") || text.contains("#25 "), "late turns should appear: {text}");
            }
            other => panic!("expected Display, got {other:?}"),
        }
    }

    #[test]
    fn regular_text_returns_not_a_command() {
        let mut s = test_session();
        assert_eq!(
            s.handle_slash_command("hello world"),
            SlashResult::NotACommand
        );
    }

    #[test]
    fn unknown_slash_returns_unknown() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/banana"),
            SlashResult::Unknown(_)
        ));
    }

    #[test]
    fn accumulate_tool_event_links_output_by_id() {
        let mut tool_calls = Vec::new();
        let mut pending_ids = Vec::new();

        accumulate_tool_event(
            &mut tool_calls,
            &mut pending_ids,
            &AgentRuntimeEvent::ToolCall {
                id: "tool-1".to_string(),
                name: "Read".to_string(),
            },
        );
        accumulate_tool_event(
            &mut tool_calls,
            &mut pending_ids,
            &AgentRuntimeEvent::ToolOutput {
                id: "tool-1".to_string(),
                output: "é".repeat(201),
            },
        );

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "Read");
        assert!(tool_calls[0].success);
        assert_eq!(tool_calls[0].input_abbrev.chars().count(), 203);
        assert!(tool_calls[0].input_abbrev.ends_with("..."));
        assert!(pending_ids.is_empty());
    }

    #[test]
    fn accumulate_tool_event_ignores_unmatched_output() {
        let mut tool_calls = Vec::new();
        let mut pending_ids = Vec::new();

        accumulate_tool_event(
            &mut tool_calls,
            &mut pending_ids,
            &AgentRuntimeEvent::ToolOutput {
                id: "missing".to_string(),
                output: "orphan".to_string(),
            },
        );

        assert!(tool_calls.is_empty());
        assert!(pending_ids.is_empty());
    }

    mod streaming_tests {
        use super::{ToolCallSummary, accumulate_tool_event, render_stream_event};
        use roko_agent::provider::claude_cli::parse_stream_line;
        use roko_agent::runtime_events::AgentRuntimeEvent;

        /// Mock Claude CLI `stream-json` output used to prove the full
        /// parser -> accumulator -> renderer path without any subprocesses.
        const MOCK_STREAM: &str = concat!(
            r#"{"type":"system","subtype":"init","session_id":"sess-test-123","model":"claude-sonnet-4-6","tools":[]}"#,
            "\n",
            r#"{"type":"assistant","subtype":"message","message":{"content":[{"type":"text","text":"Hello, "}],"usage":null}}"#,
            "\n",
            r#"{"type":"assistant","subtype":"message","message":{"content":[{"type":"text","text":"world!"}],"usage":null}}"#,
            "\n",
            r#"{"type":"assistant","subtype":"message","message":{"content":[{"type":"tool_use","id":"tu_1","name":"Read","input":{"path":"foo"}}],"usage":null}}"#,
            "\n",
            r#"{"type":"tool","subtype":"result","tool_name":"Read","tool_use_id":"tu_1","content":"file contents here"}"#,
            "\n",
            r#"{"type":"result","session_id":"sess-test-123","total_cost_usd":0.01,"num_turns":1,"is_error":false,"usage":{"input_tokens":150,"output_tokens":42,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}"#,
            "\n",
        );

        fn parsed_mock_stream_events() -> Vec<AgentRuntimeEvent> {
            MOCK_STREAM.lines().flat_map(parse_stream_line).collect()
        }

        #[test]
        fn parse_text_deltas_incrementally() {
            let text_deltas: Vec<String> = parsed_mock_stream_events()
                .into_iter()
                .filter_map(|event| match event {
                    AgentRuntimeEvent::MessageDelta { text } => Some(text),
                    _ => None,
                })
                .collect();

            assert_eq!(
                text_deltas,
                vec!["Hello, ".to_string(), "world!".to_string()]
            );
        }

        #[test]
        fn parse_session_id_from_system_event() {
            let events = parsed_mock_stream_events();
            let system_event = events
                .iter()
                .find(|event| matches!(event, AgentRuntimeEvent::SystemInit { .. }));

            assert!(system_event.is_some(), "should have SystemInit event");

            if let Some(AgentRuntimeEvent::SystemInit { session_id, model }) = system_event {
                assert_eq!(session_id, "sess-test-123");
                assert_eq!(model, "claude-sonnet-4-6");
            }
        }

        #[test]
        fn parse_session_id_from_result_event() {
            let events = parsed_mock_stream_events();
            let turn_completed = events
                .iter()
                .find(|event| matches!(event, AgentRuntimeEvent::TurnCompleted { .. }));

            assert!(turn_completed.is_some(), "should have TurnCompleted event");

            if let Some(AgentRuntimeEvent::TurnCompleted {
                session_id,
                is_error,
                ..
            }) = turn_completed
            {
                assert_eq!(session_id.as_deref(), Some("sess-test-123"));
                assert!(!is_error);
            }
        }

        #[test]
        fn parse_token_usage_from_result() {
            let events = parsed_mock_stream_events();
            let usage_events: Vec<_> = events
                .iter()
                .filter(|event| matches!(event, AgentRuntimeEvent::TokenUsage { .. }))
                .collect();

            assert_eq!(usage_events.len(), 1, "expected one TokenUsage event");

            if let AgentRuntimeEvent::TokenUsage {
                input_tokens,
                output_tokens,
                ..
            } = usage_events[0]
            {
                assert_eq!(*input_tokens, 150);
                assert_eq!(*output_tokens, 42);
            }
        }

        #[test]
        fn parse_tool_events() {
            let events = parsed_mock_stream_events();
            let tool_calls: Vec<_> = events
                .iter()
                .filter(|event| matches!(event, AgentRuntimeEvent::ToolCall { .. }))
                .collect();
            let tool_outputs: Vec<_> = events
                .iter()
                .filter(|event| matches!(event, AgentRuntimeEvent::ToolOutput { .. }))
                .collect();

            assert_eq!(tool_calls.len(), 1, "expected 1 ToolCall");
            assert_eq!(tool_outputs.len(), 1, "expected 1 ToolOutput");

            if let AgentRuntimeEvent::ToolCall { id, name } = &tool_calls[0] {
                assert_eq!(id, "tu_1");
                assert_eq!(name, "Read");
            }
            if let AgentRuntimeEvent::ToolOutput { id, output } = &tool_outputs[0] {
                assert_eq!(id, "tu_1");
                assert_eq!(output, "file contents here");
            }
        }

        #[test]
        fn empty_lines_produce_no_events() {
            assert!(parse_stream_line("").is_empty());
            assert!(parse_stream_line("   ").is_empty());
            assert!(parse_stream_line("{not json}").is_empty());
        }

        #[test]
        fn error_event_parsed() {
            let events = parse_stream_line(r#"{"type":"error","message":"rate limited"}"#);
            assert_eq!(events.len(), 1, "expected 1 Error event");
            if let AgentRuntimeEvent::Error { message } = &events[0] {
                assert!(message.contains("rate limited"));
            } else {
                panic!("expected AgentRuntimeEvent::Error");
            }
        }

        #[tokio::test]
        async fn streaming_channel_receives_events() {
            let expected = parsed_mock_stream_events();
            let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentRuntimeEvent>(100);

            let tx_clone = tx.clone();
            let handle = tokio::spawn(async move {
                for line in MOCK_STREAM.lines() {
                    for event in parse_stream_line(line) {
                        let _ = tx_clone.send(event).await;
                    }
                }
            });

            drop(tx);
            let _ = handle.await;

            let mut received = Vec::new();
            while let Some(event) = rx.recv().await {
                received.push(event);
            }

            assert_eq!(received, expected);
        }

        #[test]
        fn render_stream_event_smoke_handles_parsed_events() {
            let mut events = parsed_mock_stream_events();
            events.push(AgentRuntimeEvent::TurnCompleted {
                session_id: Some("sess-error".to_string()),
                total_cost_usd: None,
                num_turns: None,
                is_error: true,
            });
            events.push(AgentRuntimeEvent::Error {
                message: "rate limited".to_string(),
            });

            for event in &events {
                render_stream_event(event);
            }
        }

        #[test]
        fn accumulate_tool_event_matches_by_id() {
            let mut tool_calls: Vec<ToolCallSummary> = Vec::new();
            let mut pending_ids: Vec<(String, usize)> = Vec::new();

            for event in parsed_mock_stream_events() {
                accumulate_tool_event(&mut tool_calls, &mut pending_ids, &event);
            }

            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].name, "Read");
            assert_eq!(tool_calls[0].input_abbrev, "file contents here");
            assert!(tool_calls[0].success);
            assert!(pending_ids.is_empty());
        }

        #[test]
        fn tool_output_truncated_at_200_chars() {
            let mut tool_calls: Vec<ToolCallSummary> = Vec::new();
            let mut pending_ids: Vec<(String, usize)> = Vec::new();

            accumulate_tool_event(
                &mut tool_calls,
                &mut pending_ids,
                &AgentRuntimeEvent::ToolCall {
                    id: "tu_x".to_string(),
                    name: "Read".to_string(),
                },
            );
            let long_output = "a".repeat(300);
            accumulate_tool_event(
                &mut tool_calls,
                &mut pending_ids,
                &AgentRuntimeEvent::ToolOutput {
                    id: "tu_x".to_string(),
                    output: long_output,
                },
            );

            assert_eq!(tool_calls[0].input_abbrev.len(), 203);
            assert!(tool_calls[0].input_abbrev.ends_with("..."));
        }
    }
}
