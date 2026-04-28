//! PTY-backed terminal sessions for the web UI.
//!
//! Each session spawns a real shell process via `portable-pty` and bridges
//! it to a WebSocket connection. Multiple sessions can run concurrently.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::thread;

use axum::{
    Json,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use parking_lot::Mutex;
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A managed terminal session. Keeps the master PTY handle for resizing.
pub(crate) struct PtySession {
    /// PTY writer (send keystrokes to the shell).
    writer: Box<dyn Write + Send>,
    /// Master PTY handle — kept alive for resize support.
    master: Box<dyn MasterPty + Send>,
    /// Child process handle.
    _child: Box<dyn portable_pty::Child + Send>,
}

/// Session metadata returned by the REST API.
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub created_at: String,
    pub cols: u16,
    pub rows: u16,
}

/// Request to create a new terminal session.
#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    #[serde(default = "default_cols")]
    pub cols: u16,
    #[serde(default = "default_rows")]
    pub rows: u16,
    pub command: Option<String>,
    pub workdir: Option<String>,
}

fn default_cols() -> u16 {
    80
}
fn default_rows() -> u16 {
    24
}

/// Request to send input to a terminal session.
#[derive(Debug, Deserialize)]
pub struct SendInputRequest {
    pub input: String,
}

/// Manages all active PTY sessions.
pub struct SessionManager {
    pub(crate) sessions: Mutex<HashMap<String, PtySession>>,
    pub(crate) session_info: Mutex<HashMap<String, SessionInfo>>,
    workdir: std::path::PathBuf,
}

impl SessionManager {
    pub fn new(workdir: std::path::PathBuf) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            session_info: Mutex::new(HashMap::new()),
            workdir,
        }
    }

    /// Create a new PTY session. Returns (id, reader).
    pub fn create_session(
        &self,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        workdir: Option<&str>,
    ) -> anyhow::Result<(String, Box<dyn Read + Send>)> {
        let pty_system = NativePtySystem::default();
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(size)
            .map_err(|e| anyhow::anyhow!("open pty: {e}"))?;

        let wd = workdir
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.workdir.clone());

        let mut cmd = if let Some(command) = command {
            let parts: Vec<&str> = command.split_whitespace().collect();
            let mut cmd = CommandBuilder::new(parts[0]);
            for arg in &parts[1..] {
                cmd.arg(arg);
            }
            cmd
        } else {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            CommandBuilder::new(shell)
        };
        cmd.cwd(&wd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| anyhow::anyhow!("spawn: {e}"))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| anyhow::anyhow!("clone reader: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| anyhow::anyhow!("take writer: {e}"))?;

        let id = Uuid::new_v4().to_string()[..8].to_string();

        self.session_info.lock().insert(
            id.clone(),
            SessionInfo {
                id: id.clone(),
                created_at: chrono::Utc::now().to_rfc3339(),
                cols,
                rows,
            },
        );

        self.sessions.lock().insert(
            id.clone(),
            PtySession {
                writer,
                master: pair.master,
                _child: child,
            },
        );

        Ok((id, reader))
    }

    /// Send input to a session's PTY stdin.
    pub fn send_input(&self, id: &str, input: &[u8]) -> anyhow::Result<()> {
        let mut sessions = self.sessions.lock();
        let session = sessions
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {id}"))?;
        session.writer.write_all(input)?;
        session.writer.flush()?;
        Ok(())
    }

    /// Resize a session's PTY.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> anyhow::Result<()> {
        let sessions = self.sessions.lock();
        let session = sessions
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {id}"))?;
        session
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| anyhow::anyhow!("resize: {e}"))?;
        // Update stored info
        drop(sessions);
        if let Some(info) = self.session_info.lock().get_mut(id) {
            info.cols = cols;
            info.rows = rows;
        }
        Ok(())
    }

    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        self.session_info.lock().values().cloned().collect()
    }

    pub fn destroy_session(&self, id: &str) {
        self.sessions.lock().remove(id);
        self.session_info.lock().remove(id);
    }
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateSessionRequest>,
) -> impl IntoResponse {
    match state.terminal_sessions.create_session(
        req.cols,
        req.rows,
        req.command.as_deref(),
        req.workdir.as_deref(),
    ) {
        Ok((id, _reader)) => {
            let info = state
                .terminal_sessions
                .session_info
                .lock()
                .get(&id)
                .cloned();
            Json(serde_json::json!({ "id": id, "session": info })).into_response()
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn list_sessions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({"sessions": state.terminal_sessions.list_sessions()}))
}

pub async fn destroy_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    state.terminal_sessions.destroy_session(&id);
    Json(serde_json::json!({"ok": true}))
}

pub async fn send_input(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SendInputRequest>,
) -> impl IntoResponse {
    match state
        .terminal_sessions
        .send_input(&id, req.input.as_bytes())
    {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// WebSocket bridge to a PTY session. Auto-creates the session.
pub async fn ws_terminal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    // Destroy any stale session, then create fresh.
    state.terminal_sessions.destroy_session(&id);

    let reader = match state.terminal_sessions.create_session(80, 24, None, None) {
        Ok((new_id, reader)) => {
            // Remap to the requested ID if different
            if new_id != id {
                let mut info_map = state.terminal_sessions.session_info.lock();
                if let Some(mut info) = info_map.remove(&new_id) {
                    info.id = id.clone();
                    info_map.insert(id.clone(), info);
                }
                drop(info_map);
                let mut sessions = state.terminal_sessions.sessions.lock();
                if let Some(session) = sessions.remove(&new_id) {
                    sessions.insert(id.clone(), session);
                }
            }
            Some(reader)
        }
        Err(e) => {
            tracing::error!("failed to create PTY: {e}");
            None
        }
    };

    ws.on_upgrade(move |socket| handle_ws(socket, id, state, reader))
}

async fn handle_ws(
    socket: WebSocket,
    id: String,
    state: Arc<AppState>,
    reader: Option<Box<dyn Read + Send>>,
) {
    let (mut sink, mut stream) = socket.split();
    let (pty_tx, mut pty_rx) = mpsc::channel::<Vec<u8>>(256);

    // Spawn reader thread: PTY stdout → WebSocket
    if let Some(mut reader) = reader {
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if pty_tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Bridge loop
    loop {
        tokio::select! {
            Some(data) = pty_rx.recv() => {
                if sink.send(Message::Binary(data.into())).await.is_err() {
                    break;
                }
            }
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Check for resize JSON: {"type":"resize","cols":N,"rows":N}
                        if text.starts_with("{\"type\":\"resize\"") || text.starts_with("{\"type\": \"resize\"") {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                                if let (Some(cols), Some(rows)) = (
                                    v.get("cols").and_then(|c| c.as_u64()),
                                    v.get("rows").and_then(|r| r.as_u64()),
                                ) {
                                    let _ = state.terminal_sessions.resize(&id, cols as u16, rows as u16);
                                    continue;
                                }
                            }
                        }
                        // Regular input
                        let _ = state.terminal_sessions.send_input(&id, text.as_bytes());
                    }
                    Some(Ok(Message::Binary(data))) => {
                        let _ = state.terminal_sessions.send_input(&id, &data);
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    state.terminal_sessions.destroy_session(&id);
}

pub fn routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/api/terminal/sessions",
            axum::routing::post(create_session),
        )
        .route("/api/terminal/sessions", axum::routing::get(list_sessions))
        .route(
            "/api/terminal/sessions/{id}",
            axum::routing::delete(destroy_session),
        )
        .route(
            "/api/terminal/sessions/{id}/input",
            axum::routing::post(send_input),
        )
        .route("/ws/terminal/{id}", axum::routing::get(ws_terminal))
}
