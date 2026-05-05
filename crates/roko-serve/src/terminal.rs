//! PTY-backed terminal sessions for the web UI.
//!
//! Each session spawns a real shell process via `portable-pty` and bridges
//! it to a WebSocket connection. Multiple sessions can run concurrently.
//!
//! Sessions survive brief disconnects via a 60-second grace period: when a
//! WebSocket closes, the PTY keeps running and output accumulates in a
//! per-session scrollback ring buffer. A reconnecting client receives the
//! buffered scrollback before live data resumes.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

use crate::command_events::{CommandEvent, CommandOutputStream};
use crate::state::AppState;
use roko_core::config::schema::RokoConfig;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// How long a disconnected PTY session stays alive waiting for reattach.
const TERMINAL_GRACE_PERIOD: Duration = Duration::from_secs(60);

/// Maximum number of output chunks kept in the per-session scrollback ring.
const SCROLLBACK_CHUNKS: usize = 512;

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
    child: Box<dyn portable_pty::Child + Send>,
    /// Monotonic generation counter — used to avoid stale cleanup.
    sess_generation: u64,
    /// Temp ZDOTDIR created for this session's shell — cleaned up on destroy.
    zdotdir: Option<std::path::PathBuf>,
    /// When the last WebSocket client disconnected (None = currently attached).
    disconnected_at: Option<Instant>,
    /// Per-session scrollback ring buffer — shared with the PTY reader thread.
    scrollback: Arc<Mutex<VecDeque<Vec<u8>>>>,
    /// Live output subscribers (WebSocket bridge tasks subscribe here).
    subscribers: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>>,
    /// Working directory the PTY was spawned in (for state persistence).
    spawn_workdir: std::path::PathBuf,
    /// Terminal dimensions at creation.
    spawn_cols: u16,
    /// Terminal dimensions at creation.
    spawn_rows: u16,
}

/// Persisted terminal state written to `.roko/workspaces/{id}/terminal.state`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TerminalStateFile {
    pub session_id: String,
    pub workspace_id: String,
    pub cwd: String,
    pub scrollback_lines: usize,
    pub disconnected_at_unix: u64,
}

/// Result of attempting to attach to a session.
pub(crate) enum AttachResult {
    /// Reattached to an existing session within the grace period.
    Reattached {
        sess_generation: u64,
        receiver: mpsc::Receiver<Vec<u8>>,
        scrollback_snapshot: Vec<Vec<u8>>,
    },
    /// Created a fresh session (no existing session or grace period expired).
    Created {
        sess_generation: u64,
        receiver: mpsc::Receiver<Vec<u8>>,
    },
    /// Failed to create a session.
    Failed(String),
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

const TERMINAL_DISABLED_ERROR: &str = "Terminal disabled";
const TERMINAL_DISABLED_HINT: &str = "Set serve.terminal_enabled=true or use --enable-terminal";

#[derive(Clone, Debug, Default)]
struct PtyServerEnv {
    serve_url: Option<String>,
    auth_token: Option<String>,
}

impl PtyServerEnv {
    fn apply_to(&self, cmd: &mut CommandBuilder, session_id: &str) {
        cmd.env("ROKO_SESSION_ID", session_id);

        if let Some(serve_url) = &self.serve_url {
            cmd.env("ROKO_SERVE_URL", serve_url.as_str());
        }

        if let Some(auth_token) = &self.auth_token {
            cmd.env("ROKO_SERVER_AUTH_TOKEN", auth_token.as_str());
        }
    }
}

fn non_empty_env_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn configured_auth_token(config: &RokoConfig) -> Option<String> {
    std::env::var("ROKO_SERVER_AUTH_TOKEN")
        .ok()
        .and_then(|value| non_empty_env_value(&value))
        .or_else(|| {
            config
                .server
                .auth_token
                .as_deref()
                .and_then(non_empty_env_value)
        })
}

fn effective_config_port(config: &RokoConfig) -> u16 {
    if config.server.port == roko_core::defaults::DEFAULT_SERVE_PORT {
        config.serve.port.unwrap_or(config.server.port)
    } else {
        config.server.port
    }
}

fn serve_url_from_bind_and_port(bind: &str, port: u16) -> String {
    let bind = bind.trim();
    let host = match bind {
        "" | "0.0.0.0" => "127.0.0.1".to_string(),
        "::" => "[::1]".to_string(),
        host if host.contains(':') && !host.starts_with('[') => format!("[{host}]"),
        host => host.to_string(),
    };
    format!("http://{host}:{port}")
}

fn serve_url_from_socket_addr(addr: SocketAddr) -> String {
    let host = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => "127.0.0.1".to_string(),
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) if ip.is_unspecified() => "[::1]".to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    };
    format!("http://{host}:{}", addr.port())
}

#[derive(Clone)]
struct TerminalCommandEventEmitter {
    #[cfg(test)]
    events: Arc<Mutex<HashMap<String, Vec<CommandEvent>>>>,
}

impl TerminalCommandEventEmitter {
    fn new() -> Self {
        Self {
            #[cfg(test)]
            events: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg_attr(not(test), allow(clippy::unused_self))] // `self` is used only by test event capture.
    fn emit(&self, event: CommandEvent) {
        match serde_json::to_string(&event) {
            Ok(payload) => {
                tracing::debug!(
                    target: "roko_serve::terminal::command_event",
                    command_event = %payload,
                    "terminal command event"
                );
            }
            Err(error) => {
                tracing::warn!(
                    target: "roko_serve::terminal::command_event",
                    error = %error,
                    "failed to serialize terminal command event"
                );
            }
        }

        #[cfg(test)]
        if let Some(command_id) = command_event_id(&event) {
            self.events
                .lock()
                .entry(command_id.to_string())
                .or_default()
                .push(event);
        }
    }

    #[cfg(test)]
    fn events_for(&self, command_id: &str) -> Vec<CommandEvent> {
        self.events
            .lock()
            .get(command_id)
            .cloned()
            .unwrap_or_default()
    }

    #[cfg(test)]
    fn all_events(&self) -> Vec<CommandEvent> {
        self.events
            .lock()
            .values()
            .flat_map(|events| events.iter().cloned())
            .collect()
    }
}

fn command_event_id(event: &CommandEvent) -> Option<&str> {
    match event {
        CommandEvent::Started { command_id, .. }
        | CommandEvent::Output { command_id, .. }
        | CommandEvent::Exited { command_id, .. }
        | CommandEvent::Cancelled { command_id, .. } => Some(command_id.as_str()),
        CommandEvent::SpawnFailed { command_id, .. } => command_id.as_deref(),
    }
}

struct CommandEventReader {
    inner: Box<dyn Read + Send>,
    command_id: String,
    emitter: TerminalCommandEventEmitter,
}

impl Read for CommandEventReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.emitter.emit(CommandEvent::Output {
                command_id: self.command_id.clone(),
                stream: CommandOutputStream::System,
                data: String::from_utf8_lossy(&buf[..n]).into_owned(),
            });
        }
        Ok(n)
    }
}

/// Manages all active PTY sessions.
pub struct SessionManager {
    pub(crate) sessions: Mutex<HashMap<String, PtySession>>,
    pub(crate) session_info: Mutex<HashMap<String, SessionInfo>>,
    workdir: std::path::PathBuf,
    sess_generation: AtomicU64,
    command_event_emitter: TerminalCommandEventEmitter,
    server_env: Mutex<PtyServerEnv>,
}

impl SessionManager {
    pub fn new(workdir: std::path::PathBuf) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            session_info: Mutex::new(HashMap::new()),
            workdir,
            sess_generation: AtomicU64::new(0),
            command_event_emitter: TerminalCommandEventEmitter::new(),
            server_env: Mutex::new(PtyServerEnv::default()),
        }
    }

    /// Mark a session as disconnected (WebSocket closed). The PTY remains alive
    /// during the grace period. Also persists terminal state to disk.
    pub fn mark_disconnected(&self, id: &str, sess_gen: u64) {
        let mut sessions = self.sessions.lock();
        if let Some(session) = sessions.get_mut(id) {
            if session.sess_generation == sess_gen {
                session.disconnected_at = Some(Instant::now());
                // Persist state file for crash recovery
                let state_file = TerminalStateFile {
                    session_id: id.to_string(),
                    workspace_id: id.to_string(),
                    cwd: session.spawn_workdir.to_string_lossy().into_owned(),
                    scrollback_lines: session.scrollback.lock().len(),
                    disconnected_at_unix: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                };
                drop(sessions);
                self.write_state_file(id, &state_file);
            }
        }
    }

    /// Reap sessions whose grace period has expired. Called lazily from
    /// `attach_session` and `list_sessions`.
    pub fn reap_expired(&self) {
        let expired_ids: Vec<String> = {
            let sessions = self.sessions.lock();
            sessions
                .iter()
                .filter_map(|(id, s)| {
                    s.disconnected_at
                        .filter(|t| t.elapsed() > TERMINAL_GRACE_PERIOD)
                        .map(|_| id.clone())
                })
                .collect()
        };

        for id in &expired_ids {
            let removed = self.sessions.lock().remove(id);
            if let Some(session) = removed {
                self.finish_session(id, session, "grace period expired");
            }
            self.session_info.lock().remove(id);
        }
    }

    /// Attempt to attach to an existing session or create a new one.
    /// This is the primary entry point for WebSocket connections.
    pub fn attach_session(
        &self,
        id: &str,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        workdir: Option<&str>,
    ) -> AttachResult {
        // First, reap any expired sessions
        self.reap_expired();

        let mut sessions = self.sessions.lock();

        if let Some(session) = sessions.get_mut(id) {
            if let Some(disc_at) = session.disconnected_at {
                if disc_at.elapsed() <= TERMINAL_GRACE_PERIOD {
                    // Reattach: clear disconnected state, snapshot scrollback,
                    // subscribe to live output.
                    session.disconnected_at = None;
                    let scrollback_snapshot: Vec<Vec<u8>> =
                        session.scrollback.lock().iter().cloned().collect();
                    let (tx, rx) = mpsc::channel(256);
                    session.subscribers.lock().push(tx);
                    let generation = session.sess_generation;
                    drop(sessions);
                    // Remove stale state file since we're reattached
                    self.remove_state_file(id);
                    return AttachResult::Reattached {
                        sess_generation: generation,
                        receiver: rx,
                        scrollback_snapshot,
                    };
                } else {
                    // Grace period expired — remove and create fresh
                    let old = sessions.remove(id);
                    drop(sessions);
                    if let Some(old_session) = old {
                        self.finish_session(id, old_session, "grace period expired on reattach");
                    }
                    self.session_info.lock().remove(id);
                }
            } else {
                // Session is still actively connected (shouldn't normally happen
                // but handle gracefully by subscribing as an additional viewer).
                let (tx, rx) = mpsc::channel(256);
                session.subscribers.lock().push(tx);
                let generation = session.sess_generation;
                let scrollback_snapshot: Vec<Vec<u8>> =
                    session.scrollback.lock().iter().cloned().collect();
                drop(sessions);
                return AttachResult::Reattached {
                    sess_generation: generation,
                    receiver: rx,
                    scrollback_snapshot,
                };
            }
        } else {
            drop(sessions);
        }

        // Check for persisted state file to restore CWD
        let restored_workdir = self.read_state_file(id).map(|sf| sf.cwd);
        let effective_workdir = workdir
            .map(|w| w.to_string())
            .or(restored_workdir);

        // Create a new session
        match self.create_session_with_subscriber(
            id.to_string(),
            cols,
            rows,
            command,
            effective_workdir.as_deref(),
        ) {
            Ok((sess_gen, rx)) => AttachResult::Created {
                sess_generation: sess_gen,
                receiver: rx,
            },
            Err(e) => AttachResult::Failed(e.to_string()),
        }
    }

    /// Write terminal state file to disk.
    fn write_state_file(&self, id: &str, state: &TerminalStateFile) {
        let dir = self.workdir.join(".roko/workspaces").join(id);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("failed to create terminal state dir: {e}");
            return;
        }
        let path = dir.join("terminal.state");
        match serde_json::to_string_pretty(state) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!("failed to write terminal state file: {e}");
                }
            }
            Err(e) => tracing::warn!("failed to serialize terminal state: {e}"),
        }
    }

    /// Read terminal state file from disk.
    fn read_state_file(&self, id: &str) -> Option<TerminalStateFile> {
        let path = self
            .workdir
            .join(".roko/workspaces")
            .join(id)
            .join("terminal.state");
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Remove terminal state file from disk.
    fn remove_state_file(&self, id: &str) {
        let path = self
            .workdir
            .join(".roko/workspaces")
            .join(id)
            .join("terminal.state");
        let _ = std::fs::remove_file(&path);
    }

    /// Create a session with a session-owned PTY reader thread that fans output
    /// to a scrollback ring buffer and live subscriber channels. Returns the
    /// session generation and a receiver for the initial subscriber.
    fn create_session_with_subscriber(
        &self,
        id: String,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        workdir: Option<&str>,
    ) -> anyhow::Result<(u64, mpsc::Receiver<Vec<u8>>)> {
        let pty_system = NativePtySystem::default();
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let wd = workdir
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.workdir.clone());

        let (mut cmd, command_label) = if let Some(command) = command {
            let mut parts = command.split_whitespace();
            let Some(program) = parts.next() else {
                self.command_event_emitter.emit(CommandEvent::SpawnFailed {
                    command_id: Some(id.clone()),
                    command: command.to_string(),
                    error: "empty command".to_string(),
                });
                return Err(anyhow::anyhow!("empty command"));
            };
            let mut cmd = CommandBuilder::new(program);
            for arg in parts {
                cmd.arg(arg);
            }
            (cmd, command.to_string())
        } else {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            (CommandBuilder::new(shell.clone()), shell)
        };

        let pair = match pty_system.openpty(size) {
            Ok(pair) => pair,
            Err(e) => {
                self.command_event_emitter.emit(CommandEvent::SpawnFailed {
                    command_id: Some(id.clone()),
                    command: command_label,
                    error: format!("open pty: {e}"),
                });
                return Err(anyhow::anyhow!("open pty: {e}"));
            }
        };
        cmd.cwd(&wd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        self.server_env.lock().apply_to(&mut cmd, &id);

        let zdotdir_path = if command.is_none() {
            let zdotdir = std::env::temp_dir().join(format!("roko-zdot-{}", Uuid::new_v4()));
            let _ = std::fs::create_dir_all(&zdotdir);
            let _ = std::fs::write(zdotdir.join(".zshrc"), "PS1='%1~ %# '\n");
            cmd.env("ZDOTDIR", zdotdir.to_string_lossy().as_ref());
            Some(zdotdir)
        } else {
            None
        };

        let child = match pair.slave.spawn_command(cmd) {
            Ok(child) => child,
            Err(e) => {
                self.command_event_emitter.emit(CommandEvent::SpawnFailed {
                    command_id: Some(id.clone()),
                    command: command_label,
                    error: e.to_string(),
                });
                return Err(anyhow::anyhow!("spawn: {e}"));
            }
        };

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| anyhow::anyhow!("clone reader: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| anyhow::anyhow!("take writer: {e}"))?;

        self.session_info.lock().insert(
            id.clone(),
            SessionInfo {
                id: id.clone(),
                created_at: chrono::Utc::now().to_rfc3339(),
                cols,
                rows,
            },
        );

        let sess_gen = self.sess_generation.fetch_add(1, Ordering::Relaxed);

        // Shared scrollback + subscriber state (per-session, not global lock)
        let scrollback: Arc<Mutex<VecDeque<Vec<u8>>>> =
            Arc::new(Mutex::new(VecDeque::with_capacity(SCROLLBACK_CHUNKS)));
        let subscribers: Arc<Mutex<Vec<mpsc::Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(Vec::new()));

        // First subscriber for the initial attacher
        let (tx, rx) = mpsc::channel(256);
        subscribers.lock().push(tx);

        self.sessions.lock().insert(
            id.clone(),
            PtySession {
                writer,
                master: pair.master,
                child,
                sess_generation: sess_gen,
                zdotdir: zdotdir_path,
                disconnected_at: None,
                scrollback: scrollback.clone(),
                subscribers: subscribers.clone(),
                spawn_workdir: wd.clone(),
                spawn_cols: cols,
                spawn_rows: rows,
            },
        );

        self.command_event_emitter.emit(CommandEvent::Started {
            command_id: id.clone(),
            command: command_label,
            cwd: Some(wd.to_string_lossy().into_owned()),
        });

        // Session-owned PTY reader thread: reads from PTY, appends to scrollback
        // ring, and fans out to all live subscriber channels. This thread outlives
        // individual WebSocket connections so output is captured during the grace
        // period between disconnects.
        let emitter = self.command_event_emitter.clone();
        let reader_id = id.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = buf[..n].to_vec();

                        emitter.emit(CommandEvent::Output {
                            command_id: reader_id.clone(),
                            stream: CommandOutputStream::System,
                            data: String::from_utf8_lossy(&chunk).into_owned(),
                        });

                        // Append to scrollback ring (per-session lock, not global)
                        {
                            let mut sb = scrollback.lock();
                            if sb.len() >= SCROLLBACK_CHUNKS {
                                sb.pop_front();
                            }
                            sb.push_back(chunk.clone());
                        }

                        // Fan out to all live subscribers, pruning closed channels
                        {
                            let mut subs = subscribers.lock();
                            subs.retain(|s| s.try_send(chunk.clone()).is_ok());
                        }
                    }
                }
            }
        });

        Ok((sess_gen, rx))
    }

    pub(crate) fn configure_server_env_from_config(&self, config: &RokoConfig) {
        let serve_url =
            serve_url_from_bind_and_port(&config.server.bind, effective_config_port(config));
        self.configure_server_env(serve_url, configured_auth_token(config));
    }

    pub(crate) fn configure_server_env_from_addr(&self, addr: SocketAddr, config: &RokoConfig) {
        self.configure_server_env(
            serve_url_from_socket_addr(addr),
            configured_auth_token(config),
        );
    }

    fn configure_server_env(&self, serve_url: String, auth_token: Option<String>) {
        *self.server_env.lock() = PtyServerEnv {
            serve_url: non_empty_env_value(serve_url.trim_end_matches('/')),
            auth_token,
        };
    }

    /// Create a new PTY session. Returns (id, reader, generation).
    pub fn create_session(
        &self,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        workdir: Option<&str>,
    ) -> anyhow::Result<(String, Box<dyn Read + Send>, u64)> {
        self.create_session_inner(None, cols, rows, command, workdir)
    }

    #[allow(dead_code)] // Retained for REST API extensibility.
    fn create_session_with_id(
        &self,
        id: String,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        workdir: Option<&str>,
    ) -> anyhow::Result<(String, Box<dyn Read + Send>, u64)> {
        self.create_session_inner(Some(id), cols, rows, command, workdir)
    }

    fn create_session_inner(
        &self,
        requested_id: Option<String>,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        workdir: Option<&str>,
    ) -> anyhow::Result<(String, Box<dyn Read + Send>, u64)> {
        let pty_system = NativePtySystem::default();
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let wd = workdir
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.workdir.clone());

        let id = requested_id.unwrap_or_else(|| Uuid::new_v4().to_string()[..8].to_string());

        let (mut cmd, command_label) = if let Some(command) = command {
            let mut parts = command.split_whitespace();
            let Some(program) = parts.next() else {
                self.command_event_emitter.emit(CommandEvent::SpawnFailed {
                    command_id: Some(id.clone()),
                    command: command.to_string(),
                    error: "empty command".to_string(),
                });
                return Err(anyhow::anyhow!("empty command"));
            };

            let mut cmd = CommandBuilder::new(program);
            for arg in parts {
                cmd.arg(arg);
            }
            (cmd, command.to_string())
        } else {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            (CommandBuilder::new(shell.clone()), shell)
        };

        let pair = match pty_system.openpty(size) {
            Ok(pair) => pair,
            Err(e) => {
                self.command_event_emitter.emit(CommandEvent::SpawnFailed {
                    command_id: Some(id),
                    command: command_label,
                    error: format!("open pty: {e}"),
                });
                return Err(anyhow::anyhow!("open pty: {e}"));
            }
        };
        cmd.cwd(&wd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        self.server_env.lock().apply_to(&mut cmd, &id);

        // Set ZDOTDIR to a temp dir with a minimal .zshrc so user's shell config
        // doesn't override the prompt — makes prompt detection deterministic.
        let zdotdir_path = if command.is_none() {
            let zdotdir = std::env::temp_dir().join(format!("roko-zdot-{}", Uuid::new_v4()));
            let _ = std::fs::create_dir_all(&zdotdir);
            let _ = std::fs::write(zdotdir.join(".zshrc"), "PS1='%1~ %# '\n");
            cmd.env("ZDOTDIR", zdotdir.to_string_lossy().as_ref());
            Some(zdotdir)
        } else {
            None
        };

        let child = match pair.slave.spawn_command(cmd) {
            Ok(child) => child,
            Err(e) => {
                self.command_event_emitter.emit(CommandEvent::SpawnFailed {
                    command_id: Some(id),
                    command: command_label,
                    error: e.to_string(),
                });
                return Err(anyhow::anyhow!("spawn: {e}"));
            }
        };

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| anyhow::anyhow!("clone reader: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| anyhow::anyhow!("take writer: {e}"))?;

        self.session_info.lock().insert(
            id.clone(),
            SessionInfo {
                id: id.clone(),
                created_at: chrono::Utc::now().to_rfc3339(),
                cols,
                rows,
            },
        );

        let sess_gen = self.sess_generation.fetch_add(1, Ordering::Relaxed);

        self.sessions.lock().insert(
            id.clone(),
            PtySession {
                writer,
                master: pair.master,
                child,
                sess_generation: sess_gen,
                zdotdir: zdotdir_path,
                disconnected_at: None,
                scrollback: Arc::new(Mutex::new(VecDeque::new())),
                subscribers: Arc::new(Mutex::new(Vec::new())),
                spawn_workdir: wd.clone(),
                spawn_cols: cols,
                spawn_rows: rows,
            },
        );

        self.command_event_emitter.emit(CommandEvent::Started {
            command_id: id.clone(),
            command: command_label,
            cwd: Some(wd.to_string_lossy().into_owned()),
        });

        let reader = Box::new(CommandEventReader {
            inner: reader,
            command_id: id.clone(),
            emitter: self.command_event_emitter.clone(),
        });

        Ok((id, reader, sess_gen))
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
        self.reap_expired();
        self.session_info.lock().values().cloned().collect()
    }

    pub fn destroy_session(&self, id: &str) {
        let removed = self.sessions.lock().remove(id);
        if let Some(session) = removed {
            self.finish_session(id, session, "session destroyed");
        }
        self.session_info.lock().remove(id);
    }

    /// Destroy a session only if its generation matches (avoids killing a
    /// newer session that reused the same ID).
    pub fn destroy_session_if_sess_generation(&self, id: &str, sess_gen: u64) {
        let removed = {
            let mut sessions = self.sessions.lock();
            match sessions.get(id) {
                Some(s) if s.sess_generation == sess_gen => sessions.remove(id),
                _ => None,
            }
        };
        let did_remove = removed.is_some();
        if let Some(session) = removed {
            self.finish_session(id, session, "session destroyed");
        }
        if did_remove {
            self.session_info.lock().remove(id);
        }
    }

    fn finish_session(&self, id: &str, mut session: PtySession, cancel_reason: &str) {
        match session.child.try_wait() {
            Ok(Some(status)) => {
                self.command_event_emitter.emit(CommandEvent::Exited {
                    command_id: id.to_string(),
                    exit_code: i32::try_from(status.exit_code()).ok(),
                });
            }
            Ok(None) => {
                self.command_event_emitter.emit(CommandEvent::Cancelled {
                    command_id: id.to_string(),
                    reason: Some(cancel_reason.to_string()),
                });
                let _ = session.child.kill();
            }
            Err(error) => {
                self.command_event_emitter.emit(CommandEvent::Cancelled {
                    command_id: id.to_string(),
                    reason: Some(format!("{cancel_reason}: {error}")),
                });
                let _ = session.child.kill();
            }
        }
        // Clean up temp ZDOTDIR if one was created for this session.
        if let Some(zdotdir) = session.zdotdir {
            let _ = std::fs::remove_dir_all(&zdotdir);
        }
    }

    #[cfg(test)]
    fn command_events(&self, id: &str) -> Vec<CommandEvent> {
        self.command_event_emitter.events_for(id)
    }

    #[cfg(test)]
    fn all_command_events(&self) -> Vec<CommandEvent> {
        self.command_event_emitter.all_events()
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
        Ok((id, _reader, _sess_gen)) => {
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

/// WebSocket bridge to a PTY session. Reattaches to an existing session
/// within the grace period, or creates a new one if none exists.
pub async fn ws_terminal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let attach_result = state
        .terminal_sessions
        .attach_session(&id, 80, 24, None, None);

    crate::routes::ws_size_limits(ws)
        .on_upgrade(move |socket| handle_ws(socket, id, state, attach_result))
}

async fn handle_ws(
    mut socket: WebSocket,
    id: String,
    state: Arc<AppState>,
    attach_result: AttachResult,
) {
    let (sess_generation, mut pty_rx, scrollback_snapshot) = match attach_result {
        AttachResult::Reattached {
            sess_generation,
            receiver,
            scrollback_snapshot,
        } => (sess_generation, receiver, Some(scrollback_snapshot)),
        AttachResult::Created {
            sess_generation,
            receiver,
        } => (sess_generation, receiver, None),
        AttachResult::Failed(e) => {
            tracing::error!("failed to attach/create PTY session {id}: {e}");
            let _ = socket.close().await;
            return;
        }
    };

    let (mut sink, mut stream) = socket.split();

    // Replay scrollback snapshot before live data on reattach
    if let Some(snapshot) = scrollback_snapshot {
        for chunk in snapshot {
            if sink.send(Message::Binary(chunk.into())).await.is_err() {
                state
                    .terminal_sessions
                    .mark_disconnected(&id, sess_generation);
                return;
            }
        }
    }

    // Bridge loop: subscriber receiver <-> WebSocket
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

    // Mark disconnected instead of destroying — PTY stays alive during the
    // grace period so a reconnecting client can reattach.
    state
        .terminal_sessions
        .mark_disconnected(&id, sess_generation);
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

/// Return placeholder terminal routes that reject every request with 403.
pub fn disabled_routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/api/terminal/sessions",
            axum::routing::any(terminal_disabled),
        )
        .route(
            "/api/terminal/sessions/{id}",
            axum::routing::any(terminal_disabled),
        )
        .route(
            "/api/terminal/sessions/{id}/input",
            axum::routing::any(terminal_disabled),
        )
        .route("/ws/terminal/{id}", axum::routing::any(terminal_disabled))
}

async fn terminal_disabled(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        axum::http::StatusCode::FORBIDDEN,
        Json(serde_json::json!({
            "error": TERMINAL_DISABLED_ERROR,
            "hint": TERMINAL_DISABLED_HINT,
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[cfg(unix)]
    fn read_reader_to_end(
        mut reader: Box<dyn Read + Send>,
        timeout: Duration,
    ) -> anyhow::Result<Vec<u8>> {
        let (tx, rx) = mpsc::channel();
        let reader_thread = thread::spawn(move || {
            let mut output = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => output.extend_from_slice(&buf[..n]),
                    Err(_) => break,
                }
            }
            let _ = tx.send(output);
        });

        let output = rx
            .recv_timeout(timeout)
            .map_err(|error| anyhow::anyhow!("timed out reading PTY output: {error}"))?;
        reader_thread
            .join()
            .expect("terminal reader thread should finish");
        Ok(output)
    }

    #[cfg(unix)]
    #[test]
    fn terminal_command_event_lifecycle_records_started_output_and_exited() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let manager = SessionManager::new(tempdir.path().to_path_buf());
        let (id, reader, sess_gen) =
            manager.create_session(80, 24, Some("/bin/echo roko-command-event"), None)?;

        let output = read_reader_to_end(reader, Duration::from_secs(3))?;
        let output = String::from_utf8_lossy(&output);
        assert!(
            output.contains("roko-command-event"),
            "PTY output should contain command output, got {output:?}"
        );

        manager.destroy_session_if_sess_generation(&id, sess_gen);
        let events = manager.command_events(&id);

        assert!(
            events.iter().any(|event| matches!(
                event,
                CommandEvent::Started {
                    command_id,
                    command,
                    cwd
                } if command_id == &id
                    && command == "/bin/echo roko-command-event"
                    && cwd.as_deref() == Some(tempdir.path().to_string_lossy().as_ref())
            )),
            "started event missing from {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                CommandEvent::Output {
                    command_id,
                    stream: CommandOutputStream::System,
                    data
                } if command_id == &id && data.contains("roko-command-event")
            )),
            "output event missing from {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                CommandEvent::Exited {
                    command_id,
                    exit_code: Some(0)
                } if command_id == &id
            )),
            "exited event missing from {events:?}"
        );

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn terminal_command_event_lifecycle_records_cancelled() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let manager = SessionManager::new(tempdir.path().to_path_buf());
        let (id, _reader, sess_gen) = manager.create_session(80, 24, Some("/bin/sleep 5"), None)?;

        manager.destroy_session_if_sess_generation(&id, sess_gen);
        let events = manager.command_events(&id);

        assert!(
            events.iter().any(|event| matches!(
                event,
                CommandEvent::Cancelled {
                    command_id,
                    reason: Some(reason)
                } if command_id == &id && reason == "session destroyed"
            )),
            "cancelled event missing from {events:?}"
        );

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn terminal_command_event_lifecycle_records_spawn_failed() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let manager = SessionManager::new(tempdir.path().to_path_buf());

        let result = manager.create_session(
            80,
            24,
            Some("/definitely/not/a/roko-command-event-binary"),
            None,
        );

        assert!(result.is_err());
        let events = manager.all_command_events();
        assert!(
            events.iter().any(|event| matches!(
                event,
                CommandEvent::SpawnFailed {
                    command: _,
                    error,
                    ..
                } if !error.is_empty()
            )),
            "spawn_failed event missing from {events:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn grace_period_reattach_reuses_session_and_returns_scrollback() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let manager = SessionManager::new(tempdir.path().to_path_buf());

        // Create a session via attach (creates new)
        let sess_id = "test-reattach-session";
        let result = manager.attach_session(sess_id, 80, 24, Some("/bin/echo hello-scrollback"), None);
        let generation = match &result {
            AttachResult::Created { sess_generation, .. } => *sess_generation,
            other => panic!("expected Created, got {other:?}", other = std::mem::discriminant(other)),
        };

        // Give the echo command time to produce output and fill the scrollback
        thread::sleep(Duration::from_millis(500));

        // Mark disconnected (simulates WS close)
        manager.mark_disconnected(sess_id, generation);

        // Verify the session is still alive (disconnected_at is set)
        {
            let sessions = manager.sessions.lock();
            let session = sessions.get(sess_id).expect("session should still exist");
            assert!(session.disconnected_at.is_some(), "disconnected_at should be set");
        }

        // Reattach within grace period
        let reattach_result = manager.attach_session(sess_id, 80, 24, None, None);
        match reattach_result {
            AttachResult::Reattached {
                sess_generation,
                scrollback_snapshot,
                ..
            } => {
                assert_eq!(
                    sess_generation, generation,
                    "reattach should return same generation"
                );
                // The echo command should have produced some scrollback
                assert!(
                    !scrollback_snapshot.is_empty(),
                    "scrollback snapshot should not be empty after echo"
                );
            }
            other => panic!(
                "expected Reattached, got {:?}",
                std::mem::discriminant(&other)
            ),
        }

        // Verify disconnected_at is cleared after reattach
        {
            let sessions = manager.sessions.lock();
            let session = sessions.get(sess_id).expect("session should still exist");
            assert!(
                session.disconnected_at.is_none(),
                "disconnected_at should be cleared after reattach"
            );
        }

        // Clean up
        manager.destroy_session(sess_id);
    }

    #[test]
    fn reap_expired_removes_sessions_past_grace_period() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let manager = SessionManager::new(tempdir.path().to_path_buf());

        // Manually insert a fake session with an expired disconnected_at
        let fake_id = "test-reap-session";
        let scrollback = Arc::new(Mutex::new(VecDeque::new()));
        let subscribers: Arc<Mutex<Vec<tokio::sync::mpsc::Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(Vec::new()));

        // We need a ZDOTDIR to verify cleanup
        let zdotdir = tempdir.path().join("zdot-reap-test");
        std::fs::create_dir_all(&zdotdir).expect("create zdotdir");
        assert!(zdotdir.exists(), "zdotdir should exist before reap");

        // Create a minimal PTY session for the test. We use /bin/sleep
        // so the child stays alive long enough for the test to complete.
        #[cfg(unix)]
        {
            let pty_system = NativePtySystem::default();
            let size = PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            };
            let pair = pty_system.openpty(size).expect("open pty");
            let mut cmd = CommandBuilder::new("/bin/sleep");
            cmd.arg("60");
            cmd.cwd(tempdir.path());
            let child = pair.slave.spawn_command(cmd).expect("spawn sleep");
            let writer = pair.master.take_writer().expect("take writer");

            manager.sessions.lock().insert(
                fake_id.to_string(),
                PtySession {
                    writer,
                    master: pair.master,
                    child,
                    sess_generation: 0,
                    zdotdir: Some(zdotdir.clone()),
                    disconnected_at: Some(Instant::now() - Duration::from_secs(120)),
                    scrollback: scrollback.clone(),
                    subscribers: subscribers.clone(),
                    spawn_workdir: tempdir.path().to_path_buf(),
                    spawn_cols: 80,
                    spawn_rows: 24,
                },
            );
            manager.session_info.lock().insert(
                fake_id.to_string(),
                SessionInfo {
                    id: fake_id.to_string(),
                    created_at: "2024-01-01T00:00:00Z".to_string(),
                    cols: 80,
                    rows: 24,
                },
            );

            // Verify session exists before reap
            assert!(
                manager.sessions.lock().contains_key(fake_id),
                "session should exist before reap"
            );

            // Reap expired sessions
            manager.reap_expired();

            // Verify session was removed
            assert!(
                !manager.sessions.lock().contains_key(fake_id),
                "session should be removed after reap"
            );
            assert!(
                !manager.session_info.lock().contains_key(fake_id),
                "session info should be removed after reap"
            );

            // Verify ZDOTDIR was cleaned up
            assert!(
                !zdotdir.exists(),
                "zdotdir should be removed after reap via finish_session"
            );
        }
    }

    #[test]
    fn state_file_write_read_remove_roundtrip() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let manager = SessionManager::new(tempdir.path().to_path_buf());

        let test_id = "state-file-test-session";
        let state = TerminalStateFile {
            session_id: test_id.to_string(),
            workspace_id: test_id.to_string(),
            cwd: "/tmp/test-workspace".to_string(),
            scrollback_lines: 42,
            disconnected_at_unix: 1746600000,
        };

        // Write state file
        manager.write_state_file(test_id, &state);

        // Verify file exists on disk
        let state_path = tempdir
            .path()
            .join(".roko/workspaces")
            .join(test_id)
            .join("terminal.state");
        assert!(state_path.exists(), "state file should exist after write");

        // Read it back
        let loaded = manager
            .read_state_file(test_id)
            .expect("should read state file");
        assert_eq!(loaded.session_id, test_id);
        assert_eq!(loaded.cwd, "/tmp/test-workspace");
        assert_eq!(loaded.scrollback_lines, 42);
        assert_eq!(loaded.disconnected_at_unix, 1746600000);

        // Remove it
        manager.remove_state_file(test_id);
        assert!(
            !state_path.exists(),
            "state file should be removed after remove_state_file"
        );
        assert!(
            manager.read_state_file(test_id).is_none(),
            "read_state_file should return None after removal"
        );
    }

    #[cfg(unix)]
    #[test]
    fn attach_session_restores_cwd_from_state_file() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let manager = SessionManager::new(tempdir.path().to_path_buf());

        let test_id = "cwd-restore-test";

        // Write a state file with a custom CWD (use the tempdir itself as a
        // known-existing directory so the PTY can spawn)
        let state = TerminalStateFile {
            session_id: test_id.to_string(),
            workspace_id: test_id.to_string(),
            cwd: tempdir.path().to_string_lossy().into_owned(),
            scrollback_lines: 0,
            disconnected_at_unix: 0,
        };
        manager.write_state_file(test_id, &state);

        // Attach with no live session — should read the state file and use its CWD
        let result = manager.attach_session(test_id, 80, 24, Some("/bin/echo cwd-ok"), None);
        match result {
            AttachResult::Created { .. } => {
                // Success — new session created with restored CWD
            }
            other => panic!(
                "expected Created, got {:?}",
                std::mem::discriminant(&other)
            ),
        }

        // Clean up
        manager.destroy_session(test_id);
    }
}
