//! ACP session state management.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::types::{
    ClientCapabilities, ConfigOption, McpServerConfig, ModeInfo, ModesInfo, SESSION_NOT_FOUND,
    SessionInfo, SessionListResult, SessionNewParams, SessionNewResult,
};

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

/// Session-scoped ACP configuration state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigState {
    /// Active agent interaction mode.
    pub agent_mode: String,
    /// Preferred model tier.
    pub model_tier: String,
    /// Thinking verbosity policy.
    pub thinking: String,
    /// Whether the gate pipeline is enabled.
    pub gate_pipeline: bool,
    /// Whether automatic correction is enabled.
    pub auto_correct: bool,
    /// Whether the knowledge store is enabled.
    pub knowledge_store: bool,
    /// Whether the daimon subsystem is enabled.
    pub daimon_enabled: bool,
}

impl Default for SessionConfigState {
    fn default() -> Self {
        Self {
            agent_mode: "code".to_owned(),
            model_tier: "auto".to_owned(),
            thinking: "auto".to_owned(),
            gate_pipeline: true,
            auto_correct: true,
            knowledge_store: true,
            daimon_enabled: false,
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
}

impl AcpSession {
    /// Creates a new in-memory ACP session from client parameters.
    #[must_use]
    pub fn new(params: SessionNewParams) -> Self {
        Self {
            session_id: format!("sess_{}", Uuid::new_v4()),
            session_name: params.session_name,
            created_at: Utc::now(),
            config_state: SessionConfigState::default(),
            client_capabilities: params.client_capabilities.unwrap_or_default(),
            cancel_token: CancelToken::new(),
            busy: new_atomic_flag(),
            mcp_servers: params.mcp_servers,
            config_options: Vec::new(),
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
        SessionNewResult {
            session_id: self.session_id.clone(),
            config_options: self.config_options(),
            modes: Some(default_modes(&self.config_state.agent_mode)),
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

    /// Updates the current legacy mode identifier.
    pub fn set_mode(&mut self, mode_id: String) {
        self.config_state.agent_mode = mode_id;
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
    fn into_rpc_error(self) -> (i32, String) {
        match self {
            Self::NotFound(session_id) => (
                SESSION_NOT_FOUND,
                format!("ACP session '{session_id}' was not found"),
            ),
        }
    }
}

/// Result type used by ACP session management APIs.
pub type Result<T> = std::result::Result<T, (i32, String)>;

/// In-memory store for ACP sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionManager {
    sessions: HashMap<String, AcpSession>,
}

impl SessionManager {
    /// Creates an empty session manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Creates and stores a new ACP session.
    pub fn create_session(&mut self, params: SessionNewParams) -> SessionNewResult {
        let session = AcpSession::new(params);
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
    pub fn load_session(&mut self, id: &str) -> Result<SessionNewResult> {
        self.sessions
            .get(id)
            .map(AcpSession::new_result)
            .ok_or_else(|| SessionError::NotFound(id.to_owned()).into_rpc_error())
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
            ModeInfo {
                id: "review".to_owned(),
                name: "Review".to_owned(),
                description: "Inspect changes for bugs and regressions.".to_owned(),
            },
            ModeInfo {
                id: "auto".to_owned(),
                name: "Auto".to_owned(),
                description: "Let the agent pick the best mode.".to_owned(),
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
        let mut manager = SessionManager::new();
        let result = manager.create_session(session_params("alpha"));

        assert!(result.session_id.starts_with("sess_"));
        let uuid_part = &result.session_id["sess_".len()..];
        let parsed = Uuid::parse_str(uuid_part).expect("session id should contain a valid UUID");
        assert_eq!(parsed.to_string(), uuid_part);
    }

    #[test]
    fn list_sessions_returns_expected_count() {
        let mut manager = SessionManager::new();
        manager.create_session(session_params("alpha"));
        manager.create_session(session_params("beta"));

        let sessions = manager.list_sessions();
        assert_eq!(sessions.sessions.len(), 2);
    }

    #[test]
    fn missing_session_lookup_returns_none() {
        let manager = SessionManager::new();

        assert!(manager.get_session("sess_missing").is_none());
    }

    #[test]
    fn session_config_defaults_match_acp05() {
        let session = AcpSession::new(SessionNewParams {
            session_name: None,
            client_capabilities: None,
            mcp_servers: Vec::new(),
        });

        assert_eq!(session.config_state.agent_mode, "code");
        assert_eq!(session.config_state.model_tier, "auto");
        assert_eq!(session.config_state.thinking, "auto");
        assert!(session.config_state.gate_pipeline);
        assert!(session.config_state.auto_correct);
        assert!(session.config_state.knowledge_store);
        assert!(!session.config_state.daimon_enabled);
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
}
