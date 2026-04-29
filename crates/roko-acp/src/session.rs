//! ACP session state management.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

use crate::types::{
    ClientCapabilities, CommandInput, ConfigOption, ConfigOptionType, ConfigOptionValue,
    McpServerConfig, ModeInfo, ModesInfo, SESSION_NOT_FOUND, SessionInfo, SessionListResult,
    SessionNewParams, SessionNewResult, SlashCommand,
};
use crate::workflow::WorkflowRun;

/// Shared handle to the active workflow run, updated by the runner in real time.
pub type SharedWorkflowRun = Arc<Mutex<Option<WorkflowRun>>>;

fn new_shared_run() -> SharedWorkflowRun {
    Arc::new(Mutex::new(None))
}

fn new_atomic_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn new_notify() -> Arc<Notify> {
    Arc::new(Notify::new())
}

/// A lightweight cooperative cancellation token for ACP session work.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelToken {
    #[serde(skip, default = "new_atomic_flag")]
    cancelled: Arc<AtomicBool>,
    #[serde(skip, default = "new_notify")]
    notify: Arc<Notify>,
}

impl CancelToken {
    /// Creates a new uncancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancelled: new_atomic_flag(),
            notify: new_notify(),
        }
    }

    /// Marks the token as cancelled and wakes any waiters.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    /// Returns whether the token has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Waits until the token is cancelled.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }

        loop {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Default model key when roko.toml has no models configured.
pub const FALLBACK_MODEL: &str = "sonnet";

/// Maximum number of conversation turns to retain.
const MAX_HISTORY_TURNS: usize = 40;
/// Maximum total characters across all history turns.
const MAX_HISTORY_CHARS: usize = 64_000;

// ── Mode-specific system prompts ─────────────────────────────────────

const CODE_MODE_SYSTEM_PROMPT: &str = "\
You are an expert code implementer. Your role is to write and edit code directly.

Rules:
- Make minimal, targeted changes. Don't refactor unrelated code.
- Read existing code before modifying it. Understand context first.
- Follow existing patterns and conventions in the codebase.
- Write correct, working code. Verify your changes compile.
- Be concise in explanations. Lead with the code change.";

const PLAN_MODE_SYSTEM_PROMPT: &str = "\
You are a software architect and strategist. Your role is to plan, not implement.

Rules:
- Decompose tasks into clear, actionable steps.
- Identify files that need changes and describe what changes are needed.
- Consider edge cases, dependencies, and ordering constraints.
- Do NOT write implementation code directly. Describe what to build.
- Output structured plans with numbered steps.";

const RESEARCH_MODE_SYSTEM_PROMPT: &str = "\
You are a technical researcher. Your role is to gather context and analyze options.

Rules:
- Search broadly before concluding. Check multiple sources of truth.
- Cite specific files, functions, and line numbers when referencing code.
- Compare alternatives with tradeoffs when multiple approaches exist.
- Summarize findings clearly with actionable recommendations.
- Do NOT make changes. Report what you find.";

/// A single turn in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTurn {
    /// The role of this turn (user or assistant).
    pub role: TurnRole,
    /// The text content of this turn.
    pub content: String,
}

/// Role identifier for conversation turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnRole {
    /// A user message.
    User,
    /// An assistant response.
    Assistant,
}

/// Session-scoped ACP configuration state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigState {
    /// Active agent interaction mode.
    pub agent_mode: String,
    /// Selected model key (maps to `[models.*]` in roko.toml).
    pub model: String,
    /// Effort level: low, medium, high, max.
    pub effort: String,
    /// Temperament: cautious, balanced, aggressive.
    pub temperament: String,
    /// Routing mode: auto_override, manual, cascade.
    pub routing_mode: String,
    /// Whether clippy gate is enabled.
    pub clippy_enabled: bool,
    /// Whether test gate is enabled.
    pub tests_enabled: bool,
    /// Workflow pipeline: none, express, standard, full, auto.
    pub workflow: String,
    /// Review strictness: none, quick, standard, thorough.
    pub review_strictness: String,
    /// Maximum pipeline retry iterations (1-3).
    pub max_iterations: u32,
}

impl Default for SessionConfigState {
    fn default() -> Self {
        Self {
            agent_mode: "code".to_owned(),
            model: FALLBACK_MODEL.to_owned(),
            effort: "medium".to_owned(),
            temperament: "balanced".to_owned(),
            routing_mode: "auto_override".to_owned(),
            clippy_enabled: true,
            tests_enabled: true,
            workflow: "none".to_owned(),
            review_strictness: "none".to_owned(),
            max_iterations: 2,
        }
    }
}

impl SessionConfigState {
    /// Create config state from roko.toml values.
    pub fn from_roko_config(config: &roko_core::config::schema::RokoConfig) -> Self {
        let default_model = if config.models.contains_key("glm51") {
            "glm51"
        } else if config.models.contains_key("glm4") {
            "glm4"
        } else if config.models.contains_key("kimi-k26") {
            "kimi-k26"
        } else if config.models.contains_key("sonnet") {
            "sonnet"
        } else {
            config
                .models
                .keys()
                .next()
                .map(|s| s.as_str())
                .unwrap_or(FALLBACK_MODEL)
        };
        Self {
            agent_mode: "code".to_owned(),
            model: default_model.to_owned(),
            effort: config.agent.default_effort.clone(),
            temperament: config.agent.temperament.label().to_owned(),
            routing_mode: config.routing.mode.clone(),
            clippy_enabled: config.gates.clippy_enabled,
            tests_enabled: !config.gates.skip_tests,
            workflow: "none".to_owned(),
            review_strictness: "none".to_owned(),
            max_iterations: 2,
        }
    }
}

/// ACP server-side session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpSession {
    /// Server-generated session identifier.
    pub session_id: String,
    /// Optional client-provided session name.
    pub session_name: Option<String>,
    /// Timestamp when the session was created.
    pub created_at: DateTime<Utc>,
    /// Session-scoped ACP configuration.
    pub config_state: SessionConfigState,
    /// Session-scoped client capabilities.
    pub client_capabilities: ClientCapabilities,
    /// Cooperative cancellation token for the active prompt.
    #[serde(default)]
    pub cancel_token: CancelToken,
    /// Whether a prompt is currently in flight for this session.
    #[serde(skip, default = "new_atomic_flag")]
    pub busy: Arc<AtomicBool>,
    /// Session-scoped MCP server attachments.
    pub mcp_servers: Vec<McpServerConfig>,
    /// Current ACP configuration options.
    #[serde(default)]
    pub config_options: Vec<ConfigOption>,
    /// Multi-turn conversation history for context.
    #[serde(default)]
    pub conversation_history: Vec<ConversationTurn>,
    /// Active workflow run (if any pipeline is in progress or recently completed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_run: Option<WorkflowRun>,
    /// Shared handle to the live workflow run, updated by the runner in real time.
    /// Slash commands and status queries read from this handle.
    #[serde(skip, default = "new_shared_run")]
    pub shared_run: SharedWorkflowRun,
}

impl AcpSession {
    /// Creates a new in-memory ACP session from client parameters and workspace config.
    #[must_use]
    pub fn new(params: SessionNewParams) -> Self {
        let config_state = SessionConfigState::default();
        let config_options = build_config_options_static(&config_state);
        Self {
            session_id: format!("sess_{}", Uuid::new_v4()),
            session_name: params.session_name,
            created_at: Utc::now(),
            config_state,
            client_capabilities: params.client_capabilities.unwrap_or_default(),
            cancel_token: CancelToken::new(),
            busy: new_atomic_flag(),
            mcp_servers: params.mcp_servers,
            config_options,
            conversation_history: Vec::new(),
            active_run: None,
            shared_run: new_shared_run(),
        }
    }

    /// Creates a new session with config options derived from roko.toml.
    #[must_use]
    pub fn new_with_config(
        params: SessionNewParams,
        roko_config: &roko_core::config::schema::RokoConfig,
    ) -> Self {
        let config_state = SessionConfigState::from_roko_config(roko_config);
        let config_options = build_config_options(&config_state, roko_config);
        Self {
            session_id: format!("sess_{}", Uuid::new_v4()),
            session_name: params.session_name,
            created_at: Utc::now(),
            config_state,
            client_capabilities: params.client_capabilities.unwrap_or_default(),
            cancel_token: CancelToken::new(),
            busy: new_atomic_flag(),
            mcp_servers: params.mcp_servers,
            config_options,
            conversation_history: Vec::new(),
            active_run: None,
            shared_run: new_shared_run(),
        }
    }

    /// Returns the session metadata used by `session/list`.
    #[must_use]
    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            session_id: self.session_id.clone(),
            session_name: self.session_name.clone(),
            created_at: self.created_at.to_rfc3339(),
        }
    }

    /// Returns the session payload used by `session/new` and `session/load`.
    #[must_use]
    pub fn new_result(&self) -> SessionNewResult {
        let options = if self.config_options.is_empty() {
            None
        } else {
            Some(self.config_options.clone())
        };
        SessionNewResult {
            session_id: self.session_id.clone(),
            modes: Some(default_modes(&self.config_state.agent_mode)),
            config_options: options,
        }
    }

    /// Returns the currently exposed ACP config options.
    #[must_use]
    pub fn config_options(&self) -> Vec<ConfigOption> {
        self.config_options.clone()
    }

    /// Marks the session prompt loop as cancelled.
    pub fn cancel(&mut self) {
        self.cancel_token.cancel();
        self.busy.store(false, Ordering::Release);
    }

    /// Marks the session prompt loop as running.
    pub fn begin_prompt(&mut self) {
        self.cancel_token = CancelToken::new();
        self.busy.store(true, Ordering::Release);
    }

    /// Marks the session prompt loop as completed.
    pub fn finish_prompt(&mut self) {
        self.busy.store(false, Ordering::Release);
    }

    /// Returns whether the session currently has in-flight work.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    /// Updates the current legacy mode identifier. Clears history on mode change.
    pub fn set_mode(&mut self, mode_id: String) {
        if self.config_state.agent_mode != mode_id {
            self.conversation_history.clear();
        }
        self.config_state.agent_mode = mode_id;
    }

    /// Returns the system prompt for the current agent mode.
    #[must_use]
    pub fn system_prompt_for_mode(&self) -> &'static str {
        match self.config_state.agent_mode.as_str() {
            "plan" => PLAN_MODE_SYSTEM_PROMPT,
            "research" => RESEARCH_MODE_SYSTEM_PROMPT,
            _ => CODE_MODE_SYSTEM_PROMPT,
        }
    }

    /// Pushes a user turn onto conversation history, then trims.
    pub fn push_user_turn(&mut self, content: String) {
        self.conversation_history.push(ConversationTurn {
            role: TurnRole::User,
            content,
        });
        self.trim_history();
    }

    /// Pushes an assistant turn onto conversation history, then trims.
    pub fn push_assistant_turn(&mut self, content: String) {
        self.conversation_history.push(ConversationTurn {
            role: TurnRole::Assistant,
            content,
        });
        self.trim_history();
    }

    /// Trims conversation history to stay within turn count and character limits (FIFO).
    fn trim_history(&mut self) {
        // Trim by count first.
        while self.conversation_history.len() > MAX_HISTORY_TURNS {
            self.conversation_history.remove(0);
        }
        // Then trim by total character count.
        loop {
            let total_chars: usize = self
                .conversation_history
                .iter()
                .map(|t| t.content.len())
                .sum();
            if total_chars <= MAX_HISTORY_CHARS || self.conversation_history.is_empty() {
                break;
            }
            self.conversation_history.remove(0);
        }
    }

    /// Builds a messages array for OpenAI-compatible APIs.
    #[must_use]
    pub fn build_messages_array(
        &self,
        system_prompt: &str,
        current_prompt: &str,
    ) -> Vec<serde_json::Value> {
        let mut messages = Vec::with_capacity(self.conversation_history.len() + 2);
        messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt
        }));
        for turn in &self.conversation_history {
            let role = match turn.role {
                TurnRole::User => "user",
                TurnRole::Assistant => "assistant",
            };
            messages.push(serde_json::json!({
                "role": role,
                "content": turn.content
            }));
        }
        messages.push(serde_json::json!({
            "role": "user",
            "content": current_prompt
        }));
        messages
    }

    /// Builds an XML-tagged history context string for CLI-based providers.
    #[must_use]
    pub fn build_history_context_for_cli(&self) -> String {
        if self.conversation_history.is_empty() {
            return String::new();
        }
        let mut ctx = String::from("<conversation_history>\n");
        for turn in &self.conversation_history {
            let tag = match turn.role {
                TurnRole::User => "user",
                TurnRole::Assistant => "assistant",
            };
            ctx.push_str(&format!("<{tag}>\n{}\n</{tag}>\n", turn.content));
        }
        ctx.push_str("</conversation_history>\n\n");
        ctx
    }

    /// Applies a config option update and refreshes the options list.
    pub fn update_config(
        &mut self,
        option_id: &str,
        new_value: &serde_json::Value,
        roko_config: &roko_core::config::schema::RokoConfig,
    ) {
        match option_id {
            "model" => {
                if let Some(s) = new_value.as_str() {
                    self.config_state.model = s.to_owned();
                }
            }
            "effort" => {
                if let Some(s) = new_value.as_str() {
                    self.config_state.effort = s.to_owned();
                }
            }
            "temperament" => {
                if let Some(s) = new_value.as_str() {
                    self.config_state.temperament = s.to_owned();
                }
            }
            "routing_mode" => {
                if let Some(s) = new_value.as_str() {
                    self.config_state.routing_mode = s.to_owned();
                }
            }
            "clippy" => {
                if let Some(b) = new_value.as_bool() {
                    self.config_state.clippy_enabled = b;
                } else if let Some(s) = new_value.as_str() {
                    self.config_state.clippy_enabled = s == "on";
                }
            }
            "tests" => {
                if let Some(b) = new_value.as_bool() {
                    self.config_state.tests_enabled = b;
                } else if let Some(s) = new_value.as_str() {
                    self.config_state.tests_enabled = s == "on";
                }
            }
            "workflow" => {
                if let Some(s) = new_value.as_str() {
                    self.config_state.workflow = s.to_owned();
                }
            }
            "review_strictness" => {
                if let Some(s) = new_value.as_str() {
                    self.config_state.review_strictness = s.to_owned();
                }
            }
            "max_iterations" => {
                if let Some(s) = new_value.as_str() {
                    if let Ok(n) = s.parse::<u32>() {
                        self.config_state.max_iterations = n.clamp(1, 3);
                    }
                } else if let Some(n) = new_value.as_u64() {
                    self.config_state.max_iterations = (n as u32).clamp(1, 3);
                }
            }
            _ => {}
        }
        self.config_options = build_config_options(&self.config_state, roko_config);
    }
}

/// Errors produced by [`SessionManager`].
#[derive(Debug, Error)]
pub enum SessionError {
    /// The requested ACP session is not present in memory.
    #[error("ACP session '{0}' was not found")]
    NotFound(String),
}

impl SessionError {
    fn _into_rpc_error(self) -> (i32, String) {
        match self {
            Self::NotFound(session_id) => (
                SESSION_NOT_FOUND,
                format!("ACP session '{session_id}' was not found"),
            ),
        }
    }
}

/// In-memory store for ACP sessions.
///
/// The ACP stdio handler currently owns this manager on a single request
/// loop. If ACP gains concurrent transports, wrap it at the call site in
/// `Arc<tokio::sync::RwLock<SessionManager>>` instead of splitting session
/// state across tasks here.
#[derive(Debug, Clone)]
pub struct SessionManager {
    sessions: HashMap<String, AcpSession>,
    /// Working directory inherited from AcpConfig.
    pub workdir: PathBuf,
    /// Loaded roko.toml configuration.
    pub roko_config: roko_core::config::schema::RokoConfig,
}

impl SessionManager {
    /// Creates an empty session manager.
    #[must_use]
    pub fn new(workdir: PathBuf, roko_config: roko_core::config::schema::RokoConfig) -> Self {
        Self {
            sessions: HashMap::new(),
            workdir,
            roko_config,
        }
    }

    /// Creates and stores a new ACP session.
    pub fn create_session(&mut self, params: SessionNewParams) -> SessionNewResult {
        let session = AcpSession::new_with_config(params, &self.roko_config);
        let result = session.new_result();
        self.sessions.insert(session.session_id.clone(), session);
        result
    }

    /// Returns an immutable reference to a known session.
    #[must_use]
    pub fn get_session(&self, id: &str) -> Option<&AcpSession> {
        self.sessions.get(id)
    }

    /// Returns a mutable reference to a known session.
    pub fn get_session_mut(&mut self, id: &str) -> Option<&mut AcpSession> {
        self.sessions.get_mut(id)
    }

    /// Lists all known sessions.
    #[must_use]
    pub fn list_sessions(&self) -> SessionListResult {
        let mut sessions: Vec<_> = self.sessions.values().map(AcpSession::info).collect();
        sessions.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        SessionListResult { sessions }
    }

    /// Loads an existing session into the `session/new` response shape.
    /// Falls back to disk if not in memory.
    pub fn load_session(&mut self, id: &str) -> Result<SessionNewResult, (i32, String)> {
        if let Some(session) = self.sessions.get(id) {
            return Ok(session.new_result());
        }
        // Try loading from disk.
        if let Some(session) = self.load_from_disk(id) {
            let result = session.new_result();
            self.sessions.insert(session.session_id.clone(), session);
            return Ok(result);
        }
        Err((
            SESSION_NOT_FOUND,
            format!("ACP session '{id}' was not found"),
        ))
    }

    /// Persists a session to `.roko/sessions/{id}.json`.
    pub fn persist_session(&self, session_id: &str) {
        let Some(session) = self.sessions.get(session_id) else {
            return;
        };
        let sessions_dir = self.workdir.join(".roko").join("sessions");
        if let Err(e) = std::fs::create_dir_all(&sessions_dir) {
            tracing::warn!(error = %e, "failed to create sessions directory");
            return;
        }
        let path = sessions_dir.join(format!("{session_id}.json"));
        match serde_json::to_string_pretty(session) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!(path = %path.display(), error = %e, "failed to persist session");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize session for persistence");
            }
        }
    }

    /// Attempts to load a session from disk.
    fn load_from_disk(&self, session_id: &str) -> Option<AcpSession> {
        let path = self
            .workdir
            .join(".roko")
            .join("sessions")
            .join(format!("{session_id}.json"));
        let data = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// Lists session IDs discovered on disk (not already in memory).
    fn discover_persisted_sessions(&self) -> Vec<AcpSession> {
        let sessions_dir = self.workdir.join(".roko").join("sessions");
        let entries = match std::fs::read_dir(&sessions_dir) {
            Ok(entries) => entries,
            Err(_) => return Vec::new(),
        };
        let mut discovered = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json")
                && let Ok(data) = std::fs::read_to_string(&path)
                && let Ok(session) = serde_json::from_str::<AcpSession>(&data)
                && !self.sessions.contains_key(&session.session_id)
            {
                discovered.push(session);
            }
        }
        discovered
    }

    /// Lists all known sessions (in-memory + persisted on disk).
    #[must_use]
    pub fn list_sessions_with_persisted(&self) -> SessionListResult {
        let mut sessions: Vec<_> = self.sessions.values().map(AcpSession::info).collect();
        // Include persisted sessions not in memory.
        for session in self.discover_persisted_sessions() {
            sessions.push(session.info());
        }
        sessions.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        SessionListResult { sessions }
    }

    /// Removes persisted sessions older than `max_age`.
    pub fn gc_old_sessions(&self, max_age: chrono::Duration) {
        let sessions_dir = self.workdir.join(".roko").join("sessions");
        let entries = match std::fs::read_dir(&sessions_dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        let cutoff = Utc::now() - max_age;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json")
                && let Ok(data) = std::fs::read_to_string(&path)
                && let Ok(session) = serde_json::from_str::<AcpSession>(&data)
                && session.created_at < cutoff
            {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// Build config options dynamically from roko.toml providers/models.
fn build_config_options(
    state: &SessionConfigState,
    roko_config: &roko_core::config::schema::RokoConfig,
) -> Vec<ConfigOption> {
    // Model options from [models.*] in roko.toml.
    let mut model_options: Vec<ConfigOptionValue> = roko_config
        .models
        .iter()
        .map(|(key, profile)| ConfigOptionValue {
            value: key.clone(),
            name: format!("{key} ({})", profile.provider),
            description: Some(profile.slug.clone()),
        })
        .collect();
    model_options.sort_by(|a, b| a.value.cmp(&b.value));

    vec![
        ConfigOption {
            id: "model".to_owned(),
            name: "Model".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "agent".to_owned(),
            current_value: serde_json::Value::String(state.model.clone()),
            description: Some("Language model".to_owned()),
            options: Some(model_options),
        },
        ConfigOption {
            id: "effort".to_owned(),
            name: "Effort".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "agent".to_owned(),
            current_value: serde_json::Value::String(state.effort.clone()),
            description: Some("Agent effort level".to_owned()),
            options: Some(vec![
                ConfigOptionValue {
                    value: "low".to_owned(),
                    name: "Low".to_owned(),
                    description: Some("Quick, minimal reasoning".to_owned()),
                },
                ConfigOptionValue {
                    value: "medium".to_owned(),
                    name: "Medium".to_owned(),
                    description: Some("Balanced quality/speed".to_owned()),
                },
                ConfigOptionValue {
                    value: "high".to_owned(),
                    name: "High".to_owned(),
                    description: Some("Thorough reasoning".to_owned()),
                },
                ConfigOptionValue {
                    value: "max".to_owned(),
                    name: "Max".to_owned(),
                    description: Some("Maximum depth, slowest".to_owned()),
                },
            ]),
        },
        ConfigOption {
            id: "temperament".to_owned(),
            name: "Temperament".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "agent".to_owned(),
            current_value: serde_json::Value::String(state.temperament.clone()),
            description: Some("Agent risk appetite".to_owned()),
            options: Some(vec![
                ConfigOptionValue {
                    value: "conservative".to_owned(),
                    name: "Conservative".to_owned(),
                    description: Some("Favor stronger models, safer routing".to_owned()),
                },
                ConfigOptionValue {
                    value: "balanced".to_owned(),
                    name: "Balanced".to_owned(),
                    description: Some("Default heuristics".to_owned()),
                },
                ConfigOptionValue {
                    value: "aggressive".to_owned(),
                    name: "Aggressive".to_owned(),
                    description: Some("Favor faster/cheaper execution".to_owned()),
                },
                ConfigOptionValue {
                    value: "exploratory".to_owned(),
                    name: "Exploratory".to_owned(),
                    description: Some("Explore more alternatives".to_owned()),
                },
            ]),
        },
        ConfigOption {
            id: "routing_mode".to_owned(),
            name: "Routing".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "routing".to_owned(),
            current_value: serde_json::Value::String(state.routing_mode.clone()),
            description: Some("Model routing strategy".to_owned()),
            options: Some(vec![
                ConfigOptionValue {
                    value: "auto_override".to_owned(),
                    name: "Auto".to_owned(),
                    description: Some("Cascade router picks model".to_owned()),
                },
                ConfigOptionValue {
                    value: "manual".to_owned(),
                    name: "Manual".to_owned(),
                    description: Some("Always use selected model".to_owned()),
                },
            ]),
        },
        ConfigOption {
            id: "clippy".to_owned(),
            name: "Clippy".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "gates".to_owned(),
            current_value: serde_json::Value::String(
                if state.clippy_enabled { "on" } else { "off" }.to_owned(),
            ),
            description: Some("Run clippy gate after changes".to_owned()),
            options: Some(vec![
                ConfigOptionValue {
                    value: "on".to_owned(),
                    name: "On".to_owned(),
                    description: Some("Clippy validation enabled".to_owned()),
                },
                ConfigOptionValue {
                    value: "off".to_owned(),
                    name: "Off".to_owned(),
                    description: Some("Skip clippy".to_owned()),
                },
            ]),
        },
        ConfigOption {
            id: "tests".to_owned(),
            name: "Tests".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "gates".to_owned(),
            current_value: serde_json::Value::String(
                if state.tests_enabled { "on" } else { "off" }.to_owned(),
            ),
            description: Some("Run test gate after changes".to_owned()),
            options: Some(vec![
                ConfigOptionValue {
                    value: "on".to_owned(),
                    name: "On".to_owned(),
                    description: Some("Test validation enabled".to_owned()),
                },
                ConfigOptionValue {
                    value: "off".to_owned(),
                    name: "Off".to_owned(),
                    description: Some("Skip tests".to_owned()),
                },
            ]),
        },
        // ── Workflow execution options ──
        ConfigOption {
            id: "workflow".to_owned(),
            name: "Workflow".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "execution".to_owned(),
            current_value: serde_json::Value::String(state.workflow.clone()),
            description: Some("Pipeline workflow for prompts".to_owned()),
            options: Some(vec![
                ConfigOptionValue {
                    value: "none".to_owned(),
                    name: "None".to_owned(),
                    description: Some("Single agent, no pipeline".to_owned()),
                },
                ConfigOptionValue {
                    value: "express".to_owned(),
                    name: "Express".to_owned(),
                    description: Some("Implement → gate → commit (fastest)".to_owned()),
                },
                ConfigOptionValue {
                    value: "standard".to_owned(),
                    name: "Standard".to_owned(),
                    description: Some("Implement → gate → review → commit".to_owned()),
                },
                ConfigOptionValue {
                    value: "full".to_owned(),
                    name: "Full".to_owned(),
                    description: Some(
                        "Strategy → implement → gate → multi-review → commit".to_owned(),
                    ),
                },
                ConfigOptionValue {
                    value: "auto".to_owned(),
                    name: "Auto".to_owned(),
                    description: Some("Select pipeline based on complexity".to_owned()),
                },
            ]),
        },
        ConfigOption {
            id: "review_strictness".to_owned(),
            name: "Review".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "execution".to_owned(),
            current_value: serde_json::Value::String(state.review_strictness.clone()),
            description: Some("Review strictness level".to_owned()),
            options: Some(vec![
                ConfigOptionValue {
                    value: "none".to_owned(),
                    name: "None".to_owned(),
                    description: Some("Skip all reviews".to_owned()),
                },
                ConfigOptionValue {
                    value: "quick".to_owned(),
                    name: "Quick".to_owned(),
                    description: Some("Single-pass quick review".to_owned()),
                },
                ConfigOptionValue {
                    value: "standard".to_owned(),
                    name: "Standard".to_owned(),
                    description: Some("Architecture + correctness review".to_owned()),
                },
                ConfigOptionValue {
                    value: "thorough".to_owned(),
                    name: "Thorough".to_owned(),
                    description: Some("Architecture + audit + docs review".to_owned()),
                },
            ]),
        },
        ConfigOption {
            id: "max_iterations".to_owned(),
            name: "Retries".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "execution".to_owned(),
            current_value: serde_json::Value::String(state.max_iterations.to_string()),
            description: Some("Max retry iterations on failure".to_owned()),
            options: Some(vec![
                ConfigOptionValue {
                    value: "1".to_owned(),
                    name: "1".to_owned(),
                    description: Some("No retries".to_owned()),
                },
                ConfigOptionValue {
                    value: "2".to_owned(),
                    name: "2".to_owned(),
                    description: Some("Standard (1 retry)".to_owned()),
                },
                ConfigOptionValue {
                    value: "3".to_owned(),
                    name: "3".to_owned(),
                    description: Some("Persistent (2 retries)".to_owned()),
                },
            ]),
        },
    ]
}

/// Fallback config options when no roko.toml is available.
fn build_config_options_static(state: &SessionConfigState) -> Vec<ConfigOption> {
    vec![ConfigOption {
        id: "model".to_owned(),
        name: "Model".to_owned(),
        option_type: ConfigOptionType::Select,
        category: "agent".to_owned(),
        current_value: serde_json::Value::String(state.model.clone()),
        description: Some("Language model".to_owned()),
        options: Some(vec![ConfigOptionValue {
            value: "sonnet".to_owned(),
            name: "Sonnet".to_owned(),
            description: Some("Claude Sonnet".to_owned()),
        }]),
    }]
}

/// Build the list of available slash commands.
///
/// Organized by Will's core loop (Research → Synthesize → Specify → Implement → Verify → Feedback)
/// plus system/diagnostic/knowledge categories from the workflow-v1 PRDs and UX refresh specs.
pub fn build_slash_commands() -> Vec<SlashCommand> {
    vec![
        // ── Status & Diagnostics ────────────────────────────────────
        SlashCommand {
            name: "status".to_owned(),
            description: "Workspace status: signals, agents, runs, knowledge".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "doctor".to_owned(),
            description: "Diagnose workspace bootstrap state".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "config".to_owned(),
            description: "Show roko.toml configuration".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "learn".to_owned(),
            description: "Learning state: episodes, routing, experiments, efficiency".to_owned(),
            input: None,
        },
        // ── Research (foraging phase) ────────────────────────────────
        SlashCommand {
            name: "research".to_owned(),
            description: "Deep research a topic with citations (Perplexity)".to_owned(),
            input: Some(CommandInput {
                hint: Some("topic to research".to_owned()),
            }),
        },
        SlashCommand {
            name: "search".to_owned(),
            description: "Quick web search".to_owned(),
            input: Some(CommandInput {
                hint: Some("search query".to_owned()),
            }),
        },
        SlashCommand {
            name: "enhance-prd".to_owned(),
            description: "Enrich a PRD with web research".to_owned(),
            input: Some(CommandInput {
                hint: Some("PRD slug".to_owned()),
            }),
        },
        SlashCommand {
            name: "analyze".to_owned(),
            description: "Analyze execution data".to_owned(),
            input: None,
        },
        // ── Specification (PRD lifecycle) ────────────────────────────
        SlashCommand {
            name: "prd-idea".to_owned(),
            description: "Capture a new work item idea".to_owned(),
            input: Some(CommandInput {
                hint: Some("idea description".to_owned()),
            }),
        },
        SlashCommand {
            name: "prd-draft".to_owned(),
            description: "Draft a new PRD from an idea".to_owned(),
            input: Some(CommandInput {
                hint: Some("slug for the new PRD".to_owned()),
            }),
        },
        SlashCommand {
            name: "prd-list".to_owned(),
            description: "List all PRDs and their status".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "prd-status".to_owned(),
            description: "PRD pipeline coverage report".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "prd-plan".to_owned(),
            description: "Generate implementation plan from a published PRD".to_owned(),
            input: Some(CommandInput {
                hint: Some("PRD slug".to_owned()),
            }),
        },
        SlashCommand {
            name: "prd-consolidate".to_owned(),
            description: "Scan PRDs for gaps and duplicates".to_owned(),
            input: None,
        },
        // ── Planning ─────────────────────────────────────────────────
        SlashCommand {
            name: "plan-list".to_owned(),
            description: "List all plans in the workspace".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "plan-show".to_owned(),
            description: "Show a specific plan".to_owned(),
            input: Some(CommandInput {
                hint: Some("plan name".to_owned()),
            }),
        },
        SlashCommand {
            name: "plan-generate".to_owned(),
            description: "Generate a plan from a prompt".to_owned(),
            input: Some(CommandInput {
                hint: Some("describe what to build".to_owned()),
            }),
        },
        SlashCommand {
            name: "plan-validate".to_owned(),
            description: "Lint tasks.toml without executing".to_owned(),
            input: Some(CommandInput {
                hint: Some("path to plan dir".to_owned()),
            }),
        },
        SlashCommand {
            name: "plan-run".to_owned(),
            description: "Execute a plan (orchestrate agents, gates, persistence)".to_owned(),
            input: Some(CommandInput {
                hint: Some("path to plan dir".to_owned()),
            }),
        },
        SlashCommand {
            name: "plan-resume".to_owned(),
            description: "Resume an interrupted plan run".to_owned(),
            input: Some(CommandInput {
                hint: Some("path to executor state".to_owned()),
            }),
        },
        // ── Implementation & Execution ───────────────────────────────
        SlashCommand {
            name: "run".to_owned(),
            description: "Single prompt → universal loop (compose→agent→gate→persist)".to_owned(),
            input: Some(CommandInput {
                hint: Some("prompt text".to_owned()),
            }),
        },
        SlashCommand {
            name: "agents".to_owned(),
            description: "List agents and their status".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "agent-chat".to_owned(),
            description: "Interactive chat REPL with a specific agent".to_owned(),
            input: Some(CommandInput {
                hint: Some("agent name".to_owned()),
            }),
        },
        SlashCommand {
            name: "agent-start".to_owned(),
            description: "Start a named agent".to_owned(),
            input: Some(CommandInput {
                hint: Some("agent name".to_owned()),
            }),
        },
        SlashCommand {
            name: "agent-stop".to_owned(),
            description: "Stop a running agent".to_owned(),
            input: Some(CommandInput {
                hint: Some("agent name".to_owned()),
            }),
        },
        // ── Verification & Gates ─────────────────────────────────────
        SlashCommand {
            name: "review".to_owned(),
            description: "Review recent changes (git diff)".to_owned(),
            input: Some(CommandInput {
                hint: Some("target ref (default HEAD~1)".to_owned()),
            }),
        },
        SlashCommand {
            name: "build".to_owned(),
            description: "cargo build --workspace".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "test".to_owned(),
            description: "cargo test --workspace".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "clippy".to_owned(),
            description: "cargo clippy --workspace --no-deps -- -D warnings".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "fmt".to_owned(),
            description: "cargo +nightly fmt --all --check".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "gate".to_owned(),
            description: "Run full gate pipeline (compile + test + clippy + diff)".to_owned(),
            input: None,
        },
        // ── Knowledge & Dreams ───────────────────────────────────────
        SlashCommand {
            name: "knowledge".to_owned(),
            description: "Query the durable knowledge store".to_owned(),
            input: Some(CommandInput {
                hint: Some("topic to search".to_owned()),
            }),
        },
        SlashCommand {
            name: "knowledge-stats".to_owned(),
            description: "Knowledge store statistics and health".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "knowledge-gc".to_owned(),
            description: "Garbage collect knowledge store".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "knowledge-backup".to_owned(),
            description: "Backup knowledge store".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "dream".to_owned(),
            description: "Run dream consolidation cycle (NREM → REM → integration)".to_owned(),
            input: None,
        },
        // ── Code Intelligence ────────────────────────────────────────
        SlashCommand {
            name: "index".to_owned(),
            description: "Build or search code intelligence index".to_owned(),
            input: Some(CommandInput {
                hint: Some("build | search <query> | stats".to_owned()),
            }),
        },
        SlashCommand {
            name: "explain".to_owned(),
            description: "Explain a codebase concept at 3 depth levels".to_owned(),
            input: Some(CommandInput {
                hint: Some("topic".to_owned()),
            }),
        },
        SlashCommand {
            name: "replay".to_owned(),
            description: "Walk signal DAG by hash (episode replay)".to_owned(),
            input: Some(CommandInput {
                hint: Some("signal hash".to_owned()),
            }),
        },
        // ── Feedback & Learning ──────────────────────────────────────
        SlashCommand {
            name: "learn-router".to_owned(),
            description: "Inspect cascade router state and model routing".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "learn-episodes".to_owned(),
            description: "Recent episode log (agent turns + gate results)".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "learn-tune".to_owned(),
            description: "Tune adaptive thresholds (gates, routing, budget)".to_owned(),
            input: Some(CommandInput {
                hint: Some("gates | routing | budget".to_owned()),
            }),
        },
        // ── System ────────────────────────────────────────────────────
        SlashCommand {
            name: "audit".to_owned(),
            description: "Plugin security audit".to_owned(),
            input: None,
        },
        // ── Workflow ──────────────────────────────────────────────────
        SlashCommand {
            name: "workflow".to_owned(),
            description: "Workflow management: list/status/cancel/resume".to_owned(),
            input: Some(CommandInput {
                hint: Some("list | status | cancel | resume".to_owned()),
            }),
        },
        SlashCommand {
            name: "express".to_owned(),
            description: "Run express pipeline: implement → gate → commit".to_owned(),
            input: Some(CommandInput {
                hint: Some("prompt text".to_owned()),
            }),
        },
        SlashCommand {
            name: "full".to_owned(),
            description: "Run full pipeline: strategy → implement → gate → multi-review → commit"
                .to_owned(),
            input: Some(CommandInput {
                hint: Some("prompt text".to_owned()),
            }),
        },
        SlashCommand {
            name: "review-this".to_owned(),
            description: "Run review pipeline on current uncommitted changes".to_owned(),
            input: None,
        },
        SlashCommand {
            name: "pipeline".to_owned(),
            description: "Run a named workflow pipeline".to_owned(),
            input: Some(CommandInput {
                hint: Some("pipeline name".to_owned()),
            }),
        },
        // ── Help ─────────────────────────────────────────────────────
        SlashCommand {
            name: "help".to_owned(),
            description: "List all available commands".to_owned(),
            input: None,
        },
    ]
}

fn default_modes(current_mode_id: &str) -> ModesInfo {
    ModesInfo {
        current_mode_id: current_mode_id.to_owned(),
        available_modes: vec![
            ModeInfo {
                id: "code".to_owned(),
                name: "Code".to_owned(),
                description: "Implement and edit code directly.".to_owned(),
            },
            ModeInfo {
                id: "plan".to_owned(),
                name: "Plan".to_owned(),
                description: "Focus on planning before execution.".to_owned(),
            },
            ModeInfo {
                id: "research".to_owned(),
                name: "Research".to_owned(),
                description: "Gather context and analyze options.".to_owned(),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_params(name: &str) -> SessionNewParams {
        SessionNewParams {
            session_name: Some(name.to_owned()),
            client_capabilities: None,
            mcp_servers: Vec::new(),
        }
    }

    #[test]
    fn create_session_uses_prefixed_uuid_identifier() {
        let mut manager = SessionManager::new(PathBuf::from("."), Default::default());
        let result = manager.create_session(session_params("alpha"));

        assert!(result.session_id.starts_with("sess_"));
        let uuid_part = &result.session_id["sess_".len()..];
        let parsed = Uuid::parse_str(uuid_part).expect("session id should contain a valid UUID");
        assert_eq!(parsed.to_string(), uuid_part);
    }

    #[test]
    fn session_manager_remains_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<SessionManager>();
    }

    #[test]
    fn list_sessions_returns_expected_count() {
        let mut manager = SessionManager::new(PathBuf::from("."), Default::default());
        manager.create_session(session_params("alpha"));
        manager.create_session(session_params("beta"));

        let sessions = manager.list_sessions();
        assert_eq!(sessions.sessions.len(), 2);
    }

    #[test]
    fn missing_session_lookup_returns_none() {
        let manager = SessionManager::new(PathBuf::from("."), Default::default());
        assert!(manager.get_session("sess_missing").is_none());
    }

    #[tokio::test]
    async fn cancel_token_wakes_waiters() {
        let token = CancelToken::new();
        let waiter = token.clone();

        let handle = tokio::spawn(async move {
            waiter.cancelled().await;
            waiter.is_cancelled()
        });

        tokio::task::yield_now().await;
        token.cancel();

        assert!(handle.await.expect("waiter should complete"));
    }

    #[test]
    fn conversation_history_push_and_trim() {
        let mut session = AcpSession::new(session_params("history"));
        session.push_user_turn("hello".into());
        session.push_assistant_turn("hi there".into());
        assert_eq!(session.conversation_history.len(), 2);
        assert_eq!(session.conversation_history[0].role, TurnRole::User);
        assert_eq!(session.conversation_history[1].role, TurnRole::Assistant);
    }

    #[test]
    fn conversation_history_trims_by_count() {
        let mut session = AcpSession::new(session_params("trim"));
        for i in 0..50 {
            session.push_user_turn(format!("msg {i}"));
        }
        assert!(session.conversation_history.len() <= MAX_HISTORY_TURNS);
    }

    #[test]
    fn conversation_history_trims_by_chars() {
        let mut session = AcpSession::new(session_params("trim-chars"));
        // Push a few large messages.
        for _ in 0..5 {
            session.push_user_turn("x".repeat(20_000));
        }
        let total: usize = session
            .conversation_history
            .iter()
            .map(|t| t.content.len())
            .sum();
        assert!(total <= MAX_HISTORY_CHARS);
    }

    #[test]
    fn mode_change_clears_history() {
        let mut session = AcpSession::new(session_params("mode"));
        session.push_user_turn("hello".into());
        session.push_assistant_turn("hi".into());
        assert_eq!(session.conversation_history.len(), 2);

        session.set_mode("plan".into());
        assert!(session.conversation_history.is_empty());
    }

    #[test]
    fn system_prompt_for_mode_returns_correct_prompts() {
        let mut session = AcpSession::new(session_params("prompts"));
        assert!(
            session
                .system_prompt_for_mode()
                .contains("code implementer")
        );

        session.set_mode("plan".into());
        assert!(session.system_prompt_for_mode().contains("architect"));

        session.set_mode("research".into());
        assert!(session.system_prompt_for_mode().contains("researcher"));
    }

    #[test]
    fn build_messages_array_includes_history() {
        let mut session = AcpSession::new(session_params("messages"));
        session.push_user_turn("first".into());
        session.push_assistant_turn("response".into());

        let messages = session.build_messages_array("system text", "current prompt");
        // system + 2 history turns + current user
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "first");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[3]["role"], "user");
        assert_eq!(messages[3]["content"], "current prompt");
    }

    #[test]
    fn build_history_context_for_cli_formats_xml() {
        let mut session = AcpSession::new(session_params("cli-ctx"));
        session.push_user_turn("hello".into());
        session.push_assistant_turn("world".into());

        let ctx = session.build_history_context_for_cli();
        assert!(ctx.contains("<conversation_history>"));
        assert!(ctx.contains("<user>\nhello\n</user>"));
        assert!(ctx.contains("<assistant>\nworld\n</assistant>"));
    }

    #[test]
    fn empty_history_produces_empty_cli_context() {
        let session = AcpSession::new(session_params("empty"));
        assert!(session.build_history_context_for_cli().is_empty());
    }

    #[test]
    fn persist_and_load_session_round_trips() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path().to_path_buf();

        let mut manager = SessionManager::new(workdir, Default::default());
        let result = manager.create_session(session_params("persist-test"));
        let session_id = result.session_id.clone();

        // Add some history.
        {
            let session = manager.get_session_mut(&session_id).unwrap();
            session.push_user_turn("hello".into());
            session.push_assistant_turn("hi there".into());
        }

        manager.persist_session(&session_id);

        // Create a fresh manager to verify disk loading.
        let mut manager2 = SessionManager::new(tmp.path().to_path_buf(), Default::default());
        let loaded = manager2.load_session(&session_id);
        assert!(loaded.is_ok(), "should load from disk");

        // Verify the session is now in memory with history.
        let loaded_session = manager2.get_session(&session_id).unwrap();
        assert_eq!(loaded_session.conversation_history.len(), 2);
    }

    #[test]
    fn list_sessions_includes_persisted() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path().to_path_buf();

        let mut manager = SessionManager::new(workdir.clone(), Default::default());
        let result = manager.create_session(session_params("in-memory"));

        // Persist it, then create a new manager without that session.
        manager.persist_session(&result.session_id);
        let manager2 = SessionManager::new(workdir, Default::default());
        let list = manager2.list_sessions_with_persisted();
        assert!(
            list.sessions
                .iter()
                .any(|s| s.session_id == result.session_id),
            "persisted session should appear in list"
        );
    }

    #[test]
    fn slash_commands_include_new_commands() {
        let commands = build_slash_commands();
        let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
        for expected in [
            "plan-show",
            "plan-resume",
            "analyze",
            "review",
            "agent-start",
            "agent-stop",
            "knowledge-gc",
            "knowledge-backup",
            "audit",
        ] {
            assert!(
                names.contains(&expected),
                "missing slash command: {expected}"
            );
        }
    }
}
