//! ACP session state management.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::{DateTime, Utc};
use roko_compose::SystemPromptBuilder;
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

/// Default provider for serde deserialization of old sessions missing the field.
fn default_provider() -> String {
    "anthropic".to_owned()
}

/// Maximum number of conversation turns to retain.
const MAX_HISTORY_TURNS: usize = 40;
/// Maximum total characters across all history turns.
const MAX_HISTORY_CHARS: usize = 64_000;

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
    /// Selected provider key (maps to `[providers.*]` in roko.toml).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Selected model key (maps to `[models.*]` in roko.toml).
    pub model: String,
    /// Effort level: low, medium, high, max.
    pub effort: String,
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
            provider: String::new(),
            model: String::new(),
            effort: "medium".to_owned(),
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
        Self::from_roko_config_with_warnings(config).0
    }

    /// Create config state and return non-fatal fallback warnings.
    pub fn from_roko_config_with_warnings(
        config: &roko_core::config::schema::RokoConfig,
    ) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let configured_default = config.agent.default_model.trim();
        let configured_model = (!configured_default.is_empty())
            .then(|| {
                config
                    .models
                    .get(configured_default)
                    .map(|profile| (configured_default, profile))
                    .or_else(|| {
                        let message = format!(
                            "agent.default_model '{}' is not declared in [models], using the first ready model",
                            configured_default
                        );
                        tracing::warn!("{message}");
                        warnings.push(message);
                        None
                    })
            })
            .flatten();

        let ready_configured = configured_model.and_then(|(key, profile)| {
            config
                .providers
                .get(&profile.provider)
                .filter(|provider| config.is_provider_available(provider))
                .map(|_| (key, profile))
                .or_else(|| {
                    let message = format!(
                        "agent.default_model '{}' uses provider '{}' which is not ready, using the first ready model",
                        key, profile.provider
                    );
                    tracing::warn!("{message}");
                    warnings.push(message);
                    None
                })
        });

        let selected_model = ready_configured
            .or_else(|| {
                config.models.iter().find_map(|(key, profile)| {
                    config
                        .providers
                        .get(&profile.provider)
                        .filter(|provider| config.is_provider_available(provider))
                        .map(|_| (key.as_str(), profile))
                })
            })
            .or_else(|| {
                config
                    .models
                    .iter()
                    .next()
                    .map(|(key, profile)| (key.as_str(), profile))
            });

        let default_model = selected_model.map(|(key, _)| key).unwrap_or_default();
        let default_provider = selected_model
            .map(|(_, profile)| profile.provider.clone())
            .or_else(|| config.providers.keys().next().cloned())
            .unwrap_or_default();
        let state = Self {
            agent_mode: "code".to_owned(),
            provider: default_provider,
            model: default_model.to_owned(),
            effort: config.agent.default_effort.clone(),
            clippy_enabled: config.gates.clippy_enabled,
            tests_enabled: !config.gates.skip_tests,
            workflow: "none".to_owned(),
            review_strictness: "none".to_owned(),
            max_iterations: 2,
        };
        (state, warnings)
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
    /// Non-fatal warnings from session creation or config fallback.
    #[serde(default)]
    pub warnings: Vec<String>,
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
    /// Workspace CLAUDE.md content loaded once per session.
    #[serde(skip)]
    pub cached_conventions: Option<String>,
    /// Actions pre-granted via "always allow" for this session.
    /// Loaded from workspace trust at session creation; updated on "always allow" decisions.
    #[serde(skip, default)]
    pub always_allowed: HashSet<crate::types::PermissionAction>,
}

impl AcpSession {
    /// Creates a new in-memory ACP session from client parameters and workspace config.
    #[must_use]
    pub fn new(params: SessionNewParams) -> Self {
        let config_state = SessionConfigState::default();
        let config_options = build_config_options(&config_state, &Default::default());
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
            warnings: Vec::new(),
            conversation_history: Vec::new(),
            active_run: None,
            shared_run: new_shared_run(),
            cached_conventions: None,
            always_allowed: HashSet::new(),
        }
    }

    /// Creates a new session with config options derived from roko.toml.
    #[must_use]
    pub fn new_with_config(
        params: SessionNewParams,
        roko_config: &roko_core::config::schema::RokoConfig,
    ) -> Self {
        let SessionNewParams {
            session_name,
            client_capabilities,
            model,
            provider,
            effort,
            mcp_servers,
        } = params;
        let (mut config_state, mut warnings) =
            SessionConfigState::from_roko_config_with_warnings(roko_config);
        apply_session_new_overrides(
            &mut config_state,
            &mut warnings,
            roko_config,
            model.as_deref(),
            provider.as_deref(),
            effort.as_deref(),
        );
        validate_mcp_servers(&mcp_servers, &mut warnings);
        let config_options = build_config_options(&config_state, roko_config);
        Self {
            session_id: format!("sess_{}", Uuid::new_v4()),
            session_name,
            created_at: Utc::now(),
            config_state,
            client_capabilities: client_capabilities.unwrap_or_default(),
            cancel_token: CancelToken::new(),
            busy: new_atomic_flag(),
            mcp_servers,
            config_options,
            warnings,
            conversation_history: Vec::new(),
            active_run: None,
            shared_run: new_shared_run(),
            cached_conventions: None,
            always_allowed: HashSet::new(),
        }
    }

    /// Load workspace CLAUDE.md content, truncated to 4096 characters.
    ///
    /// Returns `None` if the file does not exist or cannot be read.
    #[must_use]
    pub fn load_conventions(workdir: &std::path::Path) -> Option<String> {
        let claude_md = workdir.join("CLAUDE.md");
        std::fs::read_to_string(&claude_md)
            .ok()
            .map(|content| content.chars().take(4096).collect())
    }

    /// Check if an action has already been pre-granted for this session.
    #[must_use]
    pub fn is_pre_granted(&self, action: &crate::types::PermissionAction) -> bool {
        self.always_allowed.contains(action)
    }

    /// Record an "always allow" decision for this session.
    pub fn grant_always_allow(&mut self, action: crate::types::PermissionAction) {
        self.always_allowed.insert(action);
    }

    /// Load workspace-level trust from `.roko/trust/permissions.json`.
    #[must_use]
    pub fn load_workspace_trust(
        workdir: &std::path::Path,
    ) -> HashSet<crate::types::PermissionAction> {
        let path = workdir.join(".roko/trust/permissions.json");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    /// Persist workspace-level trust to `.roko/trust/permissions.json`.
    pub fn save_workspace_trust(
        workdir: &std::path::Path,
        trust: &HashSet<crate::types::PermissionAction>,
    ) {
        let path = workdir.join(".roko/trust/permissions.json");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_string_pretty(trust) {
            let _ = std::fs::write(&path, data);
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
            warnings: self.warnings.clone(),
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

    /// Build a context-rich system prompt using the 9-layer prompt builder.
    #[must_use]
    pub fn build_system_prompt(
        &self,
        workdir: &std::path::Path,
        gate_feedback_text: &[String],
        conventions: Option<&str>,
    ) -> String {
        let role_identity = match self.config_state.agent_mode.as_str() {
            "plan" => {
                "You are a software architect and strategist. Your role is to plan, not implement. Decompose tasks into clear actionable steps, identify files that need changes, and produce structured plans with numbered steps. Do NOT write implementation code directly."
            }
            "research" => {
                "You are a technical researcher. Your role is to gather context and analyze options. Search broadly, cite specific files and line numbers, compare alternatives with tradeoffs. Do NOT make changes. Report what you find."
            }
            _ => {
                "You are an expert code implementer. Your role is to write and edit code directly. Make minimal targeted changes, read existing code before modifying it, follow existing patterns, and write correct working code."
            }
        };

        let mut builder = SystemPromptBuilder::new(role_identity);

        let conventions = conventions
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                self.cached_conventions
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
            });
        if let Some(conventions) = conventions {
            builder = builder.with_conventions(conventions);
        }

        let domain = format!(
            "Working directory: {}\nSession: {} (mode: {}, model: {})",
            workdir.display(),
            self.session_id,
            self.config_state.agent_mode,
            self.config_state.model,
        );
        builder = builder.with_domain(domain);

        for feedback in gate_feedback_text
            .iter()
            .filter(|text| !text.trim().is_empty())
        {
            builder = builder.with_gate_feedback_text(feedback.as_str());
        }

        builder.build()
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
    ) -> Result<(), String> {
        match option_id {
            "provider" => {
                if let Some(s) = new_value.as_str() {
                    if !roko_config.providers.contains_key(s) {
                        self.config_options = build_config_options(&self.config_state, roko_config);
                        return Err(format!("unknown provider '{s}'"));
                    }
                    self.config_state.provider = s.to_owned();
                    // If the current model doesn't belong to the new provider,
                    // pick the first model for that provider.
                    let model_belongs = roko_config
                        .models
                        .get(&self.config_state.model)
                        .is_some_and(|p| p.provider == s);
                    if !model_belongs {
                        self.config_state.model =
                            first_model_for_provider(roko_config, s).unwrap_or_default();
                    }
                }
            }
            "model" => {
                if let Some(s) = new_value.as_str() {
                    let model_valid = roko_config
                        .models
                        .get(s)
                        .is_some_and(|profile| profile.provider == self.config_state.provider);
                    if !model_valid {
                        self.config_options = build_config_options(&self.config_state, roko_config);
                        return Err(format!(
                            "model '{}' is not declared for provider '{}'",
                            s, self.config_state.provider
                        ));
                    }
                    self.config_state.model = s.to_owned();
                }
            }
            "effort" => {
                if let Some(s) = new_value.as_str() {
                    if !matches!(s, "low" | "medium" | "high" | "max") {
                        self.config_options = build_config_options(&self.config_state, roko_config);
                        return Err(format!("invalid effort '{s}'"));
                    }
                    self.config_state.effort = s.to_owned();
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
                    if !matches!(s, "none" | "express" | "standard" | "full" | "auto") {
                        self.config_options = build_config_options(&self.config_state, roko_config);
                        return Err(format!("invalid workflow '{s}'"));
                    }
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
            _ => {
                self.config_options = build_config_options(&self.config_state, roko_config);
                return Err(format!("unknown config option '{option_id}'"));
            }
        }
        self.config_options = build_config_options(&self.config_state, roko_config);
        Ok(())
    }

    /// Reconcile persisted provider/model selections with the current config.
    pub fn revalidate_config_state(&mut self, roko_config: &roko_core::config::schema::RokoConfig) {
        let provider_valid = !self.config_state.provider.is_empty()
            && roko_config
                .providers
                .contains_key(&self.config_state.provider);
        if !provider_valid {
            let replacement = SessionConfigState::from_roko_config(roko_config);
            if self.config_state.provider != replacement.provider
                || self.config_state.model != replacement.model
            {
                tracing::info!(
                    old_provider = %self.config_state.provider,
                    old_model = %self.config_state.model,
                    new_provider = %replacement.provider,
                    new_model = %replacement.model,
                    "persisted ACP provider/model no longer valid, resetting to config defaults"
                );
            }
            self.config_state.provider = replacement.provider;
            self.config_state.model = replacement.model;
            self.config_options = build_config_options(&self.config_state, roko_config);
            return;
        }

        let model_valid = roko_config
            .models
            .get(&self.config_state.model)
            .is_some_and(|profile| profile.provider == self.config_state.provider);
        if !model_valid {
            let replacement_model =
                first_model_for_provider(roko_config, &self.config_state.provider)
                    .unwrap_or_default();
            if self.config_state.model != replacement_model {
                tracing::info!(
                    provider = %self.config_state.provider,
                    old_model = %self.config_state.model,
                    new_model = %replacement_model,
                    "persisted ACP model no longer valid, resetting for provider"
                );
            }
            self.config_state.model = replacement_model;
        }

        self.config_options = build_config_options(&self.config_state, roko_config);
    }
}

fn apply_session_new_overrides(
    state: &mut SessionConfigState,
    warnings: &mut Vec<String>,
    roko_config: &roko_core::config::schema::RokoConfig,
    model: Option<&str>,
    provider: Option<&str>,
    effort: Option<&str>,
) {
    if let Some(model_key) = model.map(str::trim).filter(|value| !value.is_empty()) {
        match roko_config.models.get(model_key) {
            Some(profile) => {
                state.model = model_key.to_owned();
                state.provider = profile.provider.clone();
            }
            None => warnings.push(format!(
                "requested model '{}' is not declared in [models], using '{}'",
                model_key, state.model
            )),
        }
    }

    if let Some(provider_key) = provider.map(str::trim).filter(|value| !value.is_empty()) {
        if roko_config.providers.contains_key(provider_key) {
            state.provider = provider_key.to_owned();
            let model_belongs = roko_config
                .models
                .get(&state.model)
                .is_some_and(|profile| profile.provider == provider_key);
            if !model_belongs {
                if let Some(model_key) = first_model_for_provider(roko_config, provider_key) {
                    if !state.model.is_empty() {
                        warnings.push(format!(
                            "requested provider '{}' does not serve model '{}', using '{}'",
                            provider_key, state.model, model_key
                        ));
                    }
                    state.model = model_key;
                } else {
                    warnings.push(format!(
                        "requested provider '{}' has no declared models",
                        provider_key
                    ));
                    state.model.clear();
                }
            }
        } else {
            warnings.push(format!(
                "requested provider '{}' is not declared in [providers], using '{}'",
                provider_key, state.provider
            ));
        }
    }

    if let Some(effort) = effort.map(str::trim).filter(|value| !value.is_empty()) {
        match effort {
            "low" | "medium" | "high" | "max" => state.effort = effort.to_owned(),
            _ => warnings.push(format!(
                "requested effort '{}' is invalid, using '{}'",
                effort, state.effort
            )),
        }
    }
}

/// Validate MCP server configurations at session creation time.
///
/// For stdio transports, checks that the command binary exists on `$PATH` or
/// as an absolute path. Pushes a warning for each server that will fail to
/// spawn so the IDE can surface the issue immediately rather than silently
/// dropping tools later.
fn validate_mcp_servers(
    servers: &[crate::types::McpServerConfig],
    warnings: &mut Vec<String>,
) {
    use crate::types::McpTransport;
    for server in servers {
        match &server.transport {
            McpTransport::Stdio { command, .. } => {
                let found = if command.contains('/') {
                    std::path::Path::new(command).exists()
                } else {
                    resolve_on_path(command)
                };
                if !found {
                    warnings.push(format!(
                        "MCP server '{}': command '{}' not found",
                        server.name, command,
                    ));
                }
            }
            McpTransport::Http { .. } => {}
        }
    }
}

/// Check whether `name` exists as an executable on `$PATH`.
fn resolve_on_path(name: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .any(|p| p.is_file())
}

fn first_model_for_provider(
    roko_config: &roko_core::config::schema::RokoConfig,
    provider_key: &str,
) -> Option<String> {
    roko_config
        .models
        .iter()
        .find(|(_, profile)| profile.provider == provider_key)
        .map(|(key, _)| key.clone())
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
    /// Config source paths surfaced in the initialize response.
    pub config_sources: Vec<String>,
}

impl SessionManager {
    /// Creates an empty session manager.
    #[must_use]
    pub fn new(workdir: PathBuf, roko_config: roko_core::config::schema::RokoConfig) -> Self {
        Self {
            sessions: HashMap::new(),
            workdir,
            roko_config,
            config_sources: Vec::new(),
        }
    }

    /// Replace the loaded config used for new sessions and prompt dispatch.
    pub fn replace_roko_config(&mut self, roko_config: roko_core::config::schema::RokoConfig) {
        self.roko_config = roko_config;
        for session in self.sessions.values_mut() {
            session.revalidate_config_state(&self.roko_config);
        }
    }

    /// Re-validate all active sessions against the current roko config.
    ///
    /// Call this periodically or on config-change notification to bring all
    /// sessions in sync with the current config. This is the poll-on-request
    /// companion to the file-watcher push model (which will use an async
    /// channel in a follow-on task).
    pub fn revalidate_all_sessions(&mut self) {
        for session in self.sessions.values_mut() {
            session.revalidate_config_state(&self.roko_config);
        }
    }

    /// Snapshot each active session's current config options for notification.
    ///
    /// Returns `(session_id, config_options)` pairs. Used by the handler to
    /// push `config_option_update` notifications after a live config reload.
    #[must_use]
    pub fn active_session_config_options(
        &self,
    ) -> Vec<(String, Vec<crate::types::ConfigOption>)> {
        self.sessions
            .iter()
            .map(|(id, session)| (id.clone(), session.config_options()))
            .collect()
    }

    /// Creates and stores a new ACP session.
    pub fn create_session(&mut self, params: SessionNewParams) -> SessionNewResult {
        let mut session = AcpSession::new_with_config(params, &self.roko_config);
        session.cached_conventions = AcpSession::load_conventions(&self.workdir);
        session.always_allowed = AcpSession::load_workspace_trust(&self.workdir);
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
        let mut session: AcpSession = serde_json::from_str(&data).ok()?;
        session.revalidate_config_state(&self.roko_config);
        session.cached_conventions = AcpSession::load_conventions(&self.workdir);
        session.always_allowed = AcpSession::load_workspace_trust(&self.workdir);
        Some(session)
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

    /// Closes a session: cancels any active work, persists, and removes from memory.
    pub fn close_session(&mut self, session_id: &str) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.cancel();
        }
        self.persist_session(session_id);
        self.sessions.remove(session_id);
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

/// Capitalize a model key for display: split on `-`, capitalize each segment, join with space.
/// Numbers are kept as-is. `"gemini-2-5-pro"` → `"Gemini 2 5 Pro"`.
fn capitalize_model_key(key: &str) -> String {
    key.split('-')
        .map(|seg| {
            let mut chars = seg.chars();
            match chars.next() {
                Some(c) if c.is_ascii_alphabetic() => {
                    let mut s = c.to_uppercase().to_string();
                    s.extend(chars);
                    s
                }
                _ => seg.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn provider_option_description(
    roko_config: &roko_core::config::schema::RokoConfig,
    provider: &roko_core::config::schema::ProviderConfig,
) -> Option<String> {
    if roko_config.is_provider_available(provider) {
        return Some("Ready".to_owned());
    }

    match provider.api_key_env.as_deref().map(str::trim) {
        Some(env_name) if !env_name.is_empty() => {
            Some(format!("API key env {env_name} is not set"))
        }
        None => Some("API key env is not configured".to_owned()),
        Some(_) => Some("Unavailable".to_owned()),
    }
}

fn build_config_options(
    state: &SessionConfigState,
    roko_config: &roko_core::config::schema::RokoConfig,
) -> Vec<ConfigOption> {
    // ── Provider options from [providers.*] in roko.toml, with availability status ──
    let mut provider_options: Vec<ConfigOptionValue> = roko_config
        .providers
        .iter()
        .map(|(key, provider)| ConfigOptionValue {
            value: key.clone(),
            name: capitalize_model_key(key),
            description: provider_option_description(roko_config, provider),
            ready: roko_config.is_provider_available(provider),
        })
        .collect();
    provider_options.sort_by(|a, b| a.value.cmp(&b.value));

    // ── Model options filtered by selected provider ──
    let mut model_options: Vec<ConfigOptionValue> = roko_config
        .models
        .iter()
        .filter(|(_, profile)| profile.provider == state.provider)
        .map(|(key, profile)| ConfigOptionValue {
            value: key.clone(),
            name: capitalize_model_key(key),
            description: Some(format!(
                "{} (max output: {})",
                profile.slug,
                profile.effective_max_output()
            )),
            ready: roko_config
                .providers
                .get(&profile.provider)
                .is_some_and(|provider| roko_config.is_provider_available(provider)),
        })
        .collect();
    model_options.sort_by(|a, b| a.value.cmp(&b.value));

    vec![
        // 1. Provider
        ConfigOption {
            id: "provider".to_owned(),
            name: "Provider".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "model".to_owned(),
            current_value: serde_json::Value::String(state.provider.clone()),
            description: Some("LLM provider".to_owned()),
            options: Some(provider_options),
        },
        // 2. Model (filtered by provider)
        ConfigOption {
            id: "model".to_owned(),
            name: "Model".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "model".to_owned(),
            current_value: serde_json::Value::String(state.model.clone()),
            description: Some("Language model".to_owned()),
            options: Some(model_options),
        },
        // 3. Thinking (effort)
        ConfigOption {
            id: "effort".to_owned(),
            name: "Thinking".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "thought_level".to_owned(),
            current_value: serde_json::Value::String(state.effort.clone()),
            description: Some("Reasoning depth".to_owned()),
            options: Some(vec![
                ConfigOptionValue {
                    value: "low".to_owned(),
                    name: "Quick".to_owned(),
                    description: Some("Minimal reasoning".to_owned()),
                    ready: true,
                },
                ConfigOptionValue {
                    value: "medium".to_owned(),
                    name: "Standard".to_owned(),
                    description: Some("Balanced reasoning".to_owned()),
                    ready: true,
                },
                ConfigOptionValue {
                    value: "high".to_owned(),
                    name: "Deep".to_owned(),
                    description: Some("Extended reasoning".to_owned()),
                    ready: true,
                },
                ConfigOptionValue {
                    value: "max".to_owned(),
                    name: "Max".to_owned(),
                    description: Some("Full reasoning depth".to_owned()),
                    ready: true,
                },
            ]),
        },
        // 4. Workflow
        ConfigOption {
            id: "workflow".to_owned(),
            name: "Workflow".to_owned(),
            option_type: ConfigOptionType::Select,
            category: "workflow".to_owned(),
            current_value: serde_json::Value::String(state.workflow.clone()),
            description: Some("Pipeline workflow for prompts".to_owned()),
            options: Some(vec![
                ConfigOptionValue {
                    value: "none".to_owned(),
                    name: "None".to_owned(),
                    description: Some("Single agent, no pipeline".to_owned()),
                    ready: true,
                },
                ConfigOptionValue {
                    value: "express".to_owned(),
                    name: "Express".to_owned(),
                    description: Some("Implement → gate → commit (fastest)".to_owned()),
                    ready: true,
                },
                ConfigOptionValue {
                    value: "standard".to_owned(),
                    name: "Standard".to_owned(),
                    description: Some("Implement → gate → review → commit".to_owned()),
                    ready: true,
                },
                ConfigOptionValue {
                    value: "full".to_owned(),
                    name: "Full".to_owned(),
                    description: Some(
                        "Strategy → implement → gate → multi-review → commit".to_owned(),
                    ),
                    ready: true,
                },
                ConfigOptionValue {
                    value: "auto".to_owned(),
                    name: "Auto".to_owned(),
                    description: Some("Select pipeline based on complexity".to_owned()),
                    ready: true,
                },
            ]),
        },
        // 5. Clippy
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
                    ready: true,
                },
                ConfigOptionValue {
                    value: "off".to_owned(),
                    name: "Off".to_owned(),
                    description: Some("Skip clippy".to_owned()),
                    ready: true,
                },
            ]),
        },
        // 6. Tests
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
                    ready: true,
                },
                ConfigOptionValue {
                    value: "off".to_owned(),
                    name: "Off".to_owned(),
                    description: Some("Skip tests".to_owned()),
                    ready: true,
                },
            ]),
        },
    ]
}

fn slash_command(
    name: &str,
    description: &str,
    category: &str,
    hint: Option<&str>,
) -> SlashCommand {
    SlashCommand {
        name: name.to_owned(),
        description: description.to_owned(),
        category: Some(category.to_owned()),
        input: hint.map(|hint| CommandInput {
            hint: Some(hint.to_owned()),
        }),
    }
}

fn bare_mode_allows_category(category: &str) -> bool {
    matches!(
        category,
        "system" | "research" | "implementation" | "verification" | "workflow" | "help"
    )
}

/// Build the list of available slash commands.
///
/// In bare mode, commands that depend on Roko workspace state such as PRDs,
/// plans, knowledge, dreams, and learning stores are hidden from IDE clients.
pub fn build_slash_commands(bare_mode: bool) -> Vec<SlashCommand> {
    let commands = vec![
        slash_command(
            "status",
            "Workspace status: signals, agents, runs, knowledge",
            "system",
            None,
        ),
        slash_command(
            "doctor",
            "Diagnose workspace bootstrap state",
            "system",
            None,
        ),
        slash_command("config", "Show roko.toml configuration", "system", None),
        slash_command(
            "learn",
            "Learning state: episodes, routing, experiments, efficiency",
            "learning",
            None,
        ),
        slash_command(
            "research",
            "Deep research a topic with citations (Perplexity)",
            "research",
            Some("topic to research"),
        ),
        slash_command(
            "search",
            "Quick web search",
            "research",
            Some("search query"),
        ),
        slash_command(
            "enhance-prd",
            "Enrich a PRD with web research",
            "specification",
            Some("PRD slug"),
        ),
        slash_command("analyze", "Analyze execution data", "research", None),
        slash_command(
            "prd-idea",
            "Capture a new work item idea",
            "specification",
            Some("idea description"),
        ),
        slash_command(
            "prd-draft",
            "Draft a new PRD from an idea",
            "specification",
            Some("slug for the new PRD"),
        ),
        slash_command(
            "prd-list",
            "List all PRDs and their status",
            "specification",
            None,
        ),
        slash_command(
            "prd-status",
            "PRD pipeline coverage report",
            "specification",
            None,
        ),
        slash_command(
            "prd-plan",
            "Generate implementation plan from a published PRD",
            "specification",
            Some("PRD slug"),
        ),
        slash_command(
            "prd-consolidate",
            "Scan PRDs for gaps and duplicates",
            "specification",
            None,
        ),
        slash_command(
            "plan-list",
            "List all plans in the workspace",
            "planning",
            None,
        ),
        slash_command(
            "plan-show",
            "Show a specific plan",
            "planning",
            Some("plan name"),
        ),
        slash_command(
            "plan-generate",
            "Generate a plan from a prompt",
            "planning",
            Some("describe what to build..."),
        ),
        slash_command(
            "plan-validate",
            "Lint tasks.toml without executing",
            "planning",
            Some("path to plan dir"),
        ),
        slash_command(
            "plan-run",
            "Execute a plan (orchestrate agents, gates, persistence)",
            "planning",
            Some("path to plan dir"),
        ),
        slash_command(
            "plan-resume",
            "Resume an interrupted plan run",
            "planning",
            Some("path to executor state"),
        ),
        slash_command(
            "run",
            "Single prompt -> universal loop (compose->agent->gate->persist)",
            "implementation",
            Some("prompt text"),
        ),
        slash_command(
            "agents",
            "List agents and their status",
            "implementation",
            None,
        ),
        slash_command(
            "agent-chat",
            "Interactive chat REPL with a specific agent",
            "implementation",
            Some("agent name"),
        ),
        slash_command(
            "agent-start",
            "Start a named agent",
            "implementation",
            Some("agent name"),
        ),
        slash_command(
            "agent-stop",
            "Stop a running agent",
            "implementation",
            Some("agent name"),
        ),
        slash_command(
            "review",
            "Review recent changes (git diff)",
            "verification",
            Some("focus area or 'all'"),
        ),
        slash_command("build", "cargo build --workspace", "verification", None),
        slash_command("test", "cargo test --workspace", "verification", None),
        slash_command(
            "clippy",
            "cargo clippy --workspace --no-deps -- -D warnings",
            "verification",
            None,
        ),
        slash_command(
            "fmt",
            "cargo +nightly fmt --all --check",
            "verification",
            None,
        ),
        slash_command(
            "gate",
            "Run full gate pipeline (compile + test + clippy + diff)",
            "verification",
            None,
        ),
        slash_command(
            "knowledge",
            "Query the durable knowledge store",
            "knowledge",
            Some("topic to search"),
        ),
        slash_command(
            "knowledge-stats",
            "Knowledge store statistics and health",
            "knowledge",
            None,
        ),
        slash_command(
            "knowledge-gc",
            "Garbage collect knowledge store",
            "knowledge",
            None,
        ),
        slash_command(
            "knowledge-backup",
            "Backup knowledge store",
            "knowledge",
            None,
        ),
        slash_command(
            "dream",
            "Run dream consolidation cycle (NREM -> REM -> integration)",
            "knowledge",
            None,
        ),
        slash_command(
            "index",
            "Build or search code intelligence index",
            "implementation",
            Some("build | search <query> | stats"),
        ),
        slash_command(
            "explain",
            "Explain a codebase concept at 3 depth levels",
            "research",
            Some("topic"),
        ),
        slash_command(
            "replay",
            "Walk signal DAG by hash (episode replay)",
            "knowledge",
            Some("signal hash"),
        ),
        slash_command(
            "learn-router",
            "Inspect cascade router state and model routing",
            "learning",
            None,
        ),
        slash_command(
            "learn-episodes",
            "Recent episode log (agent turns + gate results)",
            "learning",
            None,
        ),
        slash_command(
            "learn-tune",
            "Tune adaptive thresholds (gates, routing, budget)",
            "learning",
            Some("gates | routing | budget"),
        ),
        slash_command("audit", "Plugin security audit", "system", None),
        slash_command(
            "workflow",
            "Workflow management: list/status/cancel/resume",
            "workflow",
            Some("list | status | cancel | resume"),
        ),
        slash_command(
            "express",
            "Run express pipeline: implement -> gate -> commit",
            "workflow",
            Some("prompt text"),
        ),
        slash_command(
            "full",
            "Run full pipeline: strategy -> implement -> gate -> multi-review -> commit",
            "workflow",
            Some("prompt text"),
        ),
        slash_command(
            "review-this",
            "Run review pipeline on current uncommitted changes",
            "workflow",
            None,
        ),
        slash_command(
            "pipeline",
            "Run a named workflow pipeline",
            "workflow",
            Some("pipeline name"),
        ),
        slash_command("help", "List all available commands", "help", None),
    ];

    if bare_mode {
        commands
            .into_iter()
            .filter(|command| {
                command
                    .category
                    .as_deref()
                    .is_some_and(bare_mode_allows_category)
            })
            .collect()
    } else {
        commands
    }
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
            model: None,
            provider: None,
            effort: None,
            mcp_servers: Vec::new(),
        }
    }

    fn option_values(options: &[ConfigOption], id: &str) -> Vec<String> {
        options
            .iter()
            .find(|option| option.id == id)
            .and_then(|option| option.options.as_ref())
            .map(|values| values.iter().map(|value| value.value.clone()).collect())
            .unwrap_or_default()
    }

    fn option_description(options: &[ConfigOption], id: &str, value: &str) -> Option<String> {
        options
            .iter()
            .find(|option| option.id == id)
            .and_then(|option| option.options.as_ref())
            .and_then(|values| values.iter().find(|item| item.value == value))
            .and_then(|item| item.description.clone())
    }

    fn config_with_provider_model(
        provider_key: &str,
        model_key: &str,
    ) -> roko_core::config::schema::RokoConfig {
        roko_core::config::schema::RokoConfig::from_toml(&format!(
            r#"
config_version = 2
schema_version = 2

[providers.{provider_key}]
kind = "openai_compat"
base_url = "https://example.test/v1"
api_key_env = ""

[models.{model_key}]
provider = "{provider_key}"
slug = "{model_key}-slug"
context_window = 8192
"#
        ))
        .expect("test config should parse")
    }

    fn config_with_missing_key_provider() -> roko_core::config::schema::RokoConfig {
        roko_core::config::schema::RokoConfig::from_toml(
            r#"
config_version = 2
schema_version = 2

[providers.missing-key-provider]
kind = "openai_compat"
base_url = "https://example.test/v1"
api_key_env = "ROKO_ACP_TEST_UNSET_PROVIDER_KEY"

[models.missing-key-model]
provider = "missing-key-provider"
slug = "missing-key-model-slug"
context_window = 8192
"#,
        )
        .expect("test config should parse")
    }

    fn config_with_two_providers() -> roko_core::config::schema::RokoConfig {
        roko_core::config::schema::RokoConfig::from_toml(
            r#"
config_version = 2
schema_version = 2

[agent]
default_model = "model-a"

[providers.provider-a]
kind = "openai_compat"
base_url = "https://a.example.test/v1"
api_key_env = ""

[providers.provider-b]
kind = "openai_compat"
base_url = "https://b.example.test/v1"
api_key_env = ""

[models.model-a]
provider = "provider-a"
slug = "model-a-slug"
context_window = 8192

[models.model-b]
provider = "provider-b"
slug = "model-b-slug"
context_window = 8192
"#,
        )
        .expect("test config should parse")
    }

    fn write_persisted_session(workdir: &std::path::Path, session: &AcpSession) {
        let sessions_dir = workdir.join(".roko").join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let path = sessions_dir.join(format!("{}.json", session.session_id));
        let json = serde_json::to_string_pretty(session).expect("serialize session");
        std::fs::write(path, json).expect("write session");
    }

    #[test]
    fn empty_config_does_not_offer_static_provider_or_model() {
        let session =
            AcpSession::new_with_config(session_params("empty-config"), &Default::default());
        let options = session.config_options();

        assert!(session.config_state.provider.is_empty());
        assert!(session.config_state.model.is_empty());
        assert!(option_values(&options, "provider").is_empty());
        assert!(option_values(&options, "model").is_empty());
        assert!(
            !options.iter().any(|option| {
                option
                    .options
                    .as_ref()
                    .is_some_and(|values| values.iter().any(|value| value.value == "anthropic"))
            }),
            "empty config must not invent an Anthropic provider"
        );
        assert!(
            !options.iter().any(|option| {
                option
                    .options
                    .as_ref()
                    .is_some_and(|values| values.iter().any(|value| value.value == "sonnet"))
            }),
            "empty config must not invent a Sonnet model"
        );
    }

    #[test]
    fn legacy_new_session_does_not_offer_static_provider_or_model() {
        let session = AcpSession::new(session_params("legacy-empty-config"));
        let options = session.config_options();

        assert!(session.config_state.provider.is_empty());
        assert!(session.config_state.model.is_empty());
        assert!(option_values(&options, "provider").is_empty());
        assert!(option_values(&options, "model").is_empty());
    }

    #[test]
    fn config_options_include_unavailable_configured_providers_with_status() {
        let config = config_with_missing_key_provider();
        let session = AcpSession::new_with_config(session_params("missing-key-provider"), &config);
        let options = session.config_options();

        assert_eq!(
            option_values(&options, "provider"),
            vec!["missing-key-provider".to_owned()]
        );
        assert_eq!(
            option_description(&options, "provider", "missing-key-provider").as_deref(),
            Some("API key env ROKO_ACP_TEST_UNSET_PROVIDER_KEY is not set")
        );
        assert_eq!(
            option_values(&options, "model"),
            vec!["missing-key-model".to_owned()]
        );
    }

    #[test]
    fn update_config_rejects_unknown_provider_selection() {
        let config = config_with_two_providers();
        let mut session = AcpSession::new_with_config(session_params("provider-update"), &config);

        let err = session
            .update_config(
                "provider",
                &serde_json::Value::String("missing-provider".to_owned()),
                &config,
            )
            .unwrap_err();

        assert!(err.contains("unknown provider"));
        assert_eq!(session.config_state.provider, "provider-a");
        assert_eq!(session.config_state.model, "model-a");
    }

    #[test]
    fn update_config_rejects_unknown_or_cross_provider_model_selection() {
        let config = config_with_two_providers();
        let mut session = AcpSession::new_with_config(session_params("model-update"), &config);

        let err = session
            .update_config(
                "model",
                &serde_json::Value::String("missing-model".to_owned()),
                &config,
            )
            .unwrap_err();
        assert!(err.contains("model 'missing-model'"));
        assert_eq!(session.config_state.model, "model-a");

        let err = session
            .update_config(
                "model",
                &serde_json::Value::String("model-b".to_owned()),
                &config,
            )
            .unwrap_err();
        assert!(err.contains("provider 'provider-a'"));
        assert_eq!(session.config_state.provider, "provider-a");
        assert_eq!(session.config_state.model, "model-a");
    }

    #[test]
    fn load_session_resets_stale_persisted_provider_and_model() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let current_config = config_with_provider_model("current-provider", "current-model");
        let mut stale_session = AcpSession::new(session_params("stale-provider"));
        stale_session.session_id = "sess_stale_provider".to_owned();
        stale_session.config_state.provider = "removed-provider".to_owned();
        stale_session.config_state.model = "removed-model".to_owned();
        write_persisted_session(tmp.path(), &stale_session);

        let mut manager = SessionManager::new(tmp.path().to_path_buf(), current_config);
        manager
            .load_session("sess_stale_provider")
            .expect("session should load");
        let loaded = manager
            .get_session("sess_stale_provider")
            .expect("session should be in memory");

        assert_eq!(loaded.config_state.provider, "current-provider");
        assert_eq!(loaded.config_state.model, "current-model");
        assert_eq!(
            option_values(&loaded.config_options(), "provider"),
            vec!["current-provider".to_owned()]
        );
        assert_eq!(
            option_values(&loaded.config_options(), "model"),
            vec!["current-model".to_owned()]
        );
    }

    #[test]
    fn load_session_resets_stale_persisted_model_for_valid_provider() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let current_config = config_with_provider_model("current-provider", "current-model");
        let mut stale_session = AcpSession::new(session_params("stale-model"));
        stale_session.session_id = "sess_stale_model".to_owned();
        stale_session.config_state.provider = "current-provider".to_owned();
        stale_session.config_state.model = "removed-model".to_owned();
        write_persisted_session(tmp.path(), &stale_session);

        let mut manager = SessionManager::new(tmp.path().to_path_buf(), current_config);
        manager
            .load_session("sess_stale_model")
            .expect("session should load");
        let loaded = manager
            .get_session("sess_stale_model")
            .expect("session should be in memory");

        assert_eq!(loaded.config_state.provider, "current-provider");
        assert_eq!(loaded.config_state.model, "current-model");
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
    fn build_system_prompt_uses_mode_specific_role_identity() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path();

        let session = AcpSession::new(session_params("prompts"));
        let code_prompt = session.build_system_prompt(workdir, &[], None);
        assert!(code_prompt.contains("expert code implementer"));

        let mut plan_session = session.clone();
        plan_session.set_mode("plan".into());
        let plan_prompt = plan_session.build_system_prompt(workdir, &[], None);
        assert!(plan_prompt.contains("software architect and strategist"));

        let mut research_session = session;
        research_session.set_mode("research".into());
        let research_prompt = research_session.build_system_prompt(workdir, &[], None);
        assert!(research_prompt.contains("technical researcher"));

        assert_ne!(code_prompt, plan_prompt);
        assert_ne!(plan_prompt, research_prompt);
    }

    #[test]
    fn build_system_prompt_includes_conventions_and_gate_feedback() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path();
        let mut session = AcpSession::new(session_params("prompts"));
        session.cached_conventions = Some("Use snake_case.\nKeep changes minimal.".to_string());

        let prompt =
            session.build_system_prompt(workdir, &["Prior gate failed on tests".to_string()], None);

        assert!(prompt.contains("## Project Conventions"));
        assert!(prompt.contains("Use snake_case."));
        assert!(prompt.contains("## Gate Feedback"));
        assert!(prompt.contains("Prior gate failed on tests"));
        assert!(prompt.contains("Working directory:"));
        assert!(prompt.contains("Session:"));
    }

    #[test]
    fn load_conventions_truncates_to_4096_characters() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path();
        std::fs::write(workdir.join("CLAUDE.md"), "a".repeat(5_000)).expect("write claude");

        let conventions = AcpSession::load_conventions(workdir).expect("load conventions");
        assert_eq!(conventions.len(), 4_096);
        assert!(conventions.chars().all(|c| c == 'a'));
    }

    #[test]
    fn create_session_loads_cached_conventions() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path().to_path_buf();
        std::fs::write(workdir.join("CLAUDE.md"), "Use snake_case.").expect("write claude");

        let mut manager = SessionManager::new(workdir, Default::default());
        let result = manager.create_session(session_params("alpha"));
        let session = manager
            .get_session(&result.session_id)
            .expect("session should exist");

        assert_eq!(
            session.cached_conventions.as_deref(),
            Some("Use snake_case.")
        );
    }

    #[test]
    fn create_session_loads_workspace_trust() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let workdir = tmp.path().to_path_buf();
        let mut trust = HashSet::new();
        trust.insert(crate::types::PermissionAction::FileEdit);
        trust.insert(crate::types::PermissionAction::GitOperation);

        AcpSession::save_workspace_trust(&workdir, &trust);
        assert!(workdir.join(".roko/trust/permissions.json").exists());

        let mut manager = SessionManager::new(workdir, Default::default());
        let result = manager.create_session(session_params("workspace-trust"));
        let session = manager
            .get_session(&result.session_id)
            .expect("session should exist");

        assert!(
            session
                .always_allowed
                .contains(&crate::types::PermissionAction::FileEdit)
        );
        assert!(
            session
                .always_allowed
                .contains(&crate::types::PermissionAction::GitOperation)
        );
    }

    #[test]
    fn load_workspace_trust_defaults_to_empty_when_missing() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        let trust = AcpSession::load_workspace_trust(tmp.path());
        assert!(trust.is_empty());
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
        let commands = build_slash_commands(false);
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
