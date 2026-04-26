//! ACP session state management.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{
    ClientCapabilities, ConfigOption, McpServerConfig, ModeInfo, ModesInfo, SessionInfo,
    SessionNewParams, SessionNewResult,
};

/// ACP server-side session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpSession {
    /// Server-generated session identifier.
    pub session_id: String,
    /// Optional client-provided session name.
    pub session_name: Option<String>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// Session-scoped client capabilities.
    pub client_capabilities: ClientCapabilities,
    /// Session-scoped MCP server attachments.
    pub mcp_servers: Vec<McpServerConfig>,
    /// Current ACP configuration options.
    pub config_options: Vec<ConfigOption>,
    /// Current mode identifier.
    pub current_mode_id: String,
    /// Cooperative cancellation flag for the active prompt.
    pub cancelled: bool,
    /// Whether a prompt is currently in flight for this session.
    pub busy: bool,
}

impl AcpSession {
    /// Creates a new in-memory ACP session from client parameters.
    pub fn new(params: SessionNewParams) -> Self {
        Self {
            session_id: format!("sess_{}", Uuid::new_v4()),
            session_name: params.session_name,
            created_at: Utc::now().to_rfc3339(),
            client_capabilities: params.client_capabilities.unwrap_or_default(),
            mcp_servers: params.mcp_servers,
            config_options: Vec::new(),
            current_mode_id: "default".to_owned(),
            cancelled: false,
            busy: false,
        }
    }

    /// Returns the session metadata used by `session/list`.
    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            session_id: self.session_id.clone(),
            session_name: self.session_name.clone(),
            created_at: self.created_at.clone(),
        }
    }

    /// Returns the session payload used by `session/new` and `session/load`.
    pub fn new_result(&self) -> SessionNewResult {
        SessionNewResult {
            session_id: self.session_id.clone(),
            config_options: self.config_options.clone(),
            modes: Some(ModesInfo {
                current_mode_id: self.current_mode_id.clone(),
                available_modes: vec![ModeInfo {
                    id: "default".to_owned(),
                    name: "Default".to_owned(),
                    description: "Standard ACP interaction mode.".to_owned(),
                }],
            }),
        }
    }

    /// Marks the session prompt loop as cancelled.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.busy = false;
    }

    /// Marks the session prompt loop as running.
    pub fn begin_prompt(&mut self) {
        self.cancelled = false;
        self.busy = true;
    }

    /// Marks the session prompt loop as completed.
    pub fn finish_prompt(&mut self) {
        self.busy = false;
    }

    /// Updates the current legacy mode identifier.
    pub fn set_mode(&mut self, mode_id: String) {
        self.current_mode_id = mode_id;
    }
}
