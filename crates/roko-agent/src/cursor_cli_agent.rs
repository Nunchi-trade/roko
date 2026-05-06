//! `CursorCliAgent` — persistent subprocess agent using Cursor's ACP JSON-RPC protocol.
//!
//! Unlike `ClaudeCliAgent` which spawns a fresh process per turn, the Cursor agent
//! subprocess stays alive across turns. The lifecycle is:
//!
//! 1. Spawn `agent --force --approve-mcps --workspace <dir> --output-format json acp`
//! 2. Send `initialize` JSON-RPC request
//! 3. Send `session/new` to create a session
//! 4. Send `session/prompt` to start each turn
//! 5. Read `session/update` notifications for streaming output
//! 6. Receive `session/prompt` response with `stopReason` when turn completes
//!
//! The process is kept alive and reused. Cleanup happens on drop or explicit kill.

use crate::agent::{Agent, AgentResult};
use crate::process::{
    GRACE_STDIN_CLOSE_MS, kill_tree, register_spawned_pid, set_process_group, unregister_pid,
};
use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::defaults::DEFAULT_REQUEST_TIMEOUT_MS;
use roko_core::{Body, Context, Kind, Signal};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc};
use tokio::time::{Duration, timeout};

/// Global startup lock to serialize concurrent Cursor agent spawns.
/// Cursor's auth/model-load on first run can conflict when multiple agents start simultaneously.
fn cursor_startup_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Batch threshold for message text before flushing (4 KB).
const STREAM_MESSAGE_BATCH_BYTES: usize = 4096;

// ─── JSON-RPC protocol types ──────────────────────────────────────────────

/// Outgoing JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

impl JsonRpcRequest {
    fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

/// Raw incoming message from the Cursor ACP server.
#[derive(Debug, Clone, Deserialize)]
struct RawServerMessage {
    id: Option<Value>,
    method: Option<String>,
    result: Option<Value>,
    error: Option<ServerError>,
    params: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct ServerError {
    #[allow(dead_code)]
    code: i64,
    message: String,
    #[allow(dead_code)]
    data: Option<Value>,
}

impl RawServerMessage {
    fn is_response(&self) -> bool {
        self.id.is_some() && self.method.is_none()
    }

    fn is_notification(&self) -> bool {
        self.method.is_some() && self.id.is_none()
    }

    fn is_server_request(&self) -> bool {
        self.id.is_some() && self.method.is_some()
    }

    fn numeric_id(&self) -> Option<u64> {
        self.id.as_ref().and_then(|v| v.as_u64())
    }
}

// ─── ACP lifecycle params ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CursorInitializeParams {
    protocol_version: u32,
    client_info: ClientInfo,
    client_capabilities: Value,
}

#[derive(Debug, Clone, Serialize)]
struct ClientInfo {
    name: String,
    version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CursorSessionNewParams {
    cwd: String,
    mode: &'static str,
    mcp_servers: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CursorPromptParams {
    session_id: String,
    prompt: Vec<CursorPromptItem>,
}

#[derive(Debug, Clone, Serialize)]
struct CursorPromptItem {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
}

// ─── Notification event types ─────────────────────────────────────────────

/// Events parsed from Cursor ACP notifications.
#[derive(Debug)]
enum CursorEvent {
    /// Agent message text chunk.
    MessageDelta(String),
    /// Tool call started.
    ToolCall(String),
    /// Tool output (command result).
    CommandOutput(String),
}

/// Parse a `session/update` notification into a `CursorEvent`.
fn parse_cursor_notification(method: &str, params: Option<&Value>) -> Option<CursorEvent> {
    match method {
        "session/update" => {
            let params_val = params?;
            let update = params_val.get("update")?;
            let kind = update
                .get("sessionUpdate")
                .and_then(|k| k.as_str())
                .unwrap_or("");

            match kind {
                "agent_message_chunk" => {
                    let content = update
                        .get("content")
                        .and_then(|c| c.get("text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    if content.is_empty() {
                        return None;
                    }
                    Some(CursorEvent::MessageDelta(content))
                }
                "agent_thought_chunk" => None,
                "tool_call" => {
                    let title = update.get("title").and_then(|t| t.as_str()).unwrap_or("?");
                    tracing::debug!("[cursor-cli] tool '{title}'");
                    Some(CursorEvent::ToolCall(title.to_string()))
                }
                "tool_call_update" => {
                    let status = update.get("status").and_then(|s| s.as_str()).unwrap_or("");
                    if status == "completed" {
                        let content = update
                            .get("rawOutput")
                            .and_then(|o| o.get("content"))
                            .and_then(|c| c.as_str())
                            .unwrap_or("");
                        if !content.is_empty() {
                            return Some(CursorEvent::CommandOutput(content.to_string()));
                        }
                    }
                    None
                }
                "available_commands_update" => None,
                other => {
                    let content = update
                        .get("content")
                        .and_then(|c| c.get("text").or(Some(c)))
                        .and_then(|t| t.as_str())
                        .unwrap_or("");
                    if !content.is_empty() {
                        tracing::debug!("[cursor-cli] session/update kind='{other}'");
                        Some(CursorEvent::MessageDelta(content.to_string()))
                    } else {
                        None
                    }
                }
            }
        }
        "session/request_permission" => {
            tracing::warn!("[cursor-cli] received permission request despite --force; ignoring");
            None
        }
        "cursor/update_todos" | "cursor/task" | "cursor/create_plan" => None,
        _ => {
            tracing::debug!("[cursor-cli] unhandled method '{method}'");
            None
        }
    }
}

// ─── Inner connection state ───────────────────────────────────────────────

/// Holds the live subprocess and its communication channels.
struct CursorConnection {
    child: Child,
    stdin: tokio::io::BufWriter<tokio::process::ChildStdin>,
    next_id: AtomicU64,
    response_rx: mpsc::UnboundedReceiver<(u64, Value)>,
    session_id: Option<String>,
    reader_handle: tokio::task::JoinHandle<()>,
    stderr_handle: Option<tokio::task::JoinHandle<()>>,
}

impl CursorConnection {
    /// Spawn the `agent` subprocess and wire up readers.
    async fn spawn(
        command: &str,
        working_dir: &PathBuf,
        model: Option<&str>,
        event_tx: mpsc::UnboundedSender<CursorEvent>,
        turn_done_tx: mpsc::UnboundedSender<()>,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(command);
        cmd.arg("--force");
        cmd.arg("--approve-mcps");
        cmd.arg("--workspace").arg(working_dir);
        cmd.args(["--output-format", "json"]);
        if let Some(slug) = model {
            cmd.args(["--model", slug]);
        }
        cmd.arg("acp");
        cmd.current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Limit cargo parallelism for multi-agent scenarios.
        cmd.env("CARGO_INCREMENTAL", "0");
        cmd.env("CARGO_BUILD_JOBS", "2");

        set_process_group(&mut cmd);
        cmd.kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn cursor agent: {e}"))?;

        if let Some(pid) = child.id() {
            register_spawned_pid(pid);
        }

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "No stdin on cursor child".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "No stdout on cursor child".to_string())?;

        let (resp_tx, resp_rx) = mpsc::unbounded_channel::<(u64, Value)>();

        // Stderr reader — log and discard.
        let stderr_handle = child.stderr.take().map(|stderr| {
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        tracing::debug!("[cursor-cli/stderr] {line}");
                    }
                }
            })
        });

        // Stdout reader — parse JSON-RPC messages.
        let reader_handle = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut pending_message = String::new();

            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                tracing::trace!("[cursor-cli] ← {}", &line[..line.len().min(200)]);

                match serde_json::from_str::<RawServerMessage>(&line) {
                    Ok(msg) => {
                        // Server requests (has both id and method) or notifications
                        if msg.is_server_request() || msg.is_notification() {
                            let method = msg.method.as_deref().unwrap_or("");
                            let params = msg.params.as_ref();
                            if let Some(event) = parse_cursor_notification(method, params) {
                                match &event {
                                    CursorEvent::MessageDelta(text) => {
                                        pending_message.push_str(text);
                                        if pending_message.len() >= STREAM_MESSAGE_BATCH_BYTES
                                            || text.contains('\n')
                                        {
                                            let _ = event_tx.send(CursorEvent::MessageDelta(
                                                std::mem::take(&mut pending_message),
                                            ));
                                        }
                                    }
                                    _ => {
                                        // Flush pending text before other events.
                                        if !pending_message.is_empty() {
                                            let _ = event_tx.send(CursorEvent::MessageDelta(
                                                std::mem::take(&mut pending_message),
                                            ));
                                        }
                                        let _ = event_tx.send(event);
                                    }
                                }
                            }
                        } else if msg.is_response() {
                            let id = msg.numeric_id().unwrap_or(0);
                            // Detect turn completion: stopReason in the response.
                            if msg
                                .result
                                .as_ref()
                                .and_then(|r| r.get("stopReason"))
                                .and_then(|s| s.as_str())
                                .is_some()
                            {
                                // Flush remaining text.
                                if !pending_message.is_empty() {
                                    let _ = event_tx.send(CursorEvent::MessageDelta(
                                        std::mem::take(&mut pending_message),
                                    ));
                                }
                                let _ = turn_done_tx.send(());
                            }
                            let val = if let Some(err) = msg.error {
                                let error_msg = if let Some(data) = &err.data {
                                    if let Some(msg_val) =
                                        data.get("message").and_then(|v| v.as_str())
                                    {
                                        format!("{} ({})", err.message, msg_val)
                                    } else {
                                        err.message.clone()
                                    }
                                } else {
                                    err.message.clone()
                                };
                                serde_json::json!({"error": error_msg})
                            } else {
                                msg.result.unwrap_or(Value::Null)
                            };
                            let _ = resp_tx.send((id, val));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[cursor-cli] parse error: {e}: {}",
                            &line[..line.len().min(200)]
                        );
                    }
                }
            }

            // Process exited — flush remaining.
            if !pending_message.is_empty() {
                let _ = event_tx.send(CursorEvent::MessageDelta(pending_message));
            }
        });

        Ok(Self {
            child,
            stdin: tokio::io::BufWriter::new(stdin),
            next_id: AtomicU64::new(1),
            response_rx: resp_rx,
            session_id: None,
            reader_handle,
            stderr_handle,
        })
    }

    /// Send a JSON-RPC request and return the assigned ID.
    async fn send_request(&mut self, method: &str, params: Option<Value>) -> Result<u64, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new(id, method, params);
        let mut json = serde_json::to_string(&req).map_err(|e| format!("serialize: {e}"))?;
        tracing::debug!("[cursor-cli] → {}", &json[..json.len().min(500)]);
        json.push('\n');
        self.stdin
            .write_all(json.as_bytes())
            .await
            .map_err(|e| format!("stdin write: {e}"))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| format!("stdin flush: {e}"))?;
        Ok(id)
    }

    /// Wait for a response with the given ID.
    async fn recv_response(&mut self, expected_id: u64) -> Result<Value, String> {
        loop {
            match self.response_rx.recv().await {
                Some((id, val)) => {
                    if id != expected_id {
                        tracing::debug!(
                            "[cursor-cli] discarding stale response id={id} (want {expected_id})"
                        );
                        continue;
                    }
                    if val.get("error").is_some() {
                        return Err(format!("ACP error: {val}"));
                    }
                    return Ok(val);
                }
                None => {
                    return Err(
                        "cursor agent exited without responding — verify `agent --force acp` is available".to_string(),
                    );
                }
            }
        }
    }

    /// Run the ACP initialize handshake.
    async fn initialize(&mut self) -> Result<(), String> {
        let params = CursorInitializeParams {
            protocol_version: 1,
            client_info: ClientInfo {
                name: "roko".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            client_capabilities: serde_json::json!({}),
        };
        let id = self
            .send_request(
                "initialize",
                Some(serde_json::to_value(params).map_err(|e| e.to_string())?),
            )
            .await?;
        let resp = timeout(Duration::from_secs(90), self.recv_response(id))
            .await
            .map_err(|_| {
                "cursor `initialize` timed out after 90s — is `agent --force acp` responding?"
                    .to_string()
            })??;
        tracing::info!("[cursor-cli] initialize OK: {resp}");
        Ok(())
    }

    /// Create a new ACP session.
    async fn create_session(&mut self, working_dir: &str) -> Result<String, String> {
        let cwd = if working_dir.is_empty() {
            ".".to_string()
        } else {
            working_dir.to_string()
        };
        let params = CursorSessionNewParams {
            cwd,
            mode: "agent",
            mcp_servers: vec![],
        };
        let id = self
            .send_request(
                "session/new",
                Some(serde_json::to_value(params).map_err(|e| e.to_string())?),
            )
            .await?;
        let resp = timeout(Duration::from_secs(60), self.recv_response(id))
            .await
            .map_err(|_| "cursor `session/new` timed out after 60s".to_string())??;

        let session_id = resp
            .get("sessionId")
            .or_else(|| resp.get("session_id"))
            .or_else(|| resp.get("id"))
            .and_then(|s| s.as_str())
            .map(String::from)
            .ok_or_else(|| format!("No sessionId in session/new response: {resp}"))?;

        self.session_id = Some(session_id.clone());
        tracing::info!("[cursor-cli] session created: {session_id}");
        Ok(session_id)
    }

    /// Send a prompt to the current session.
    async fn prompt(&mut self, message: &str) -> Result<u64, String> {
        // Check liveness.
        if let Ok(Some(status)) = self.child.try_wait() {
            return Err(format!("cursor agent already exited with {status}"));
        }

        let session_id = match &self.session_id {
            Some(id) => id.clone(),
            None => return Err("No active session".to_string()),
        };

        let params = CursorPromptParams {
            session_id,
            prompt: vec![CursorPromptItem {
                kind: "text",
                text: message.to_string(),
            }],
        };
        self.send_request(
            "session/prompt",
            Some(serde_json::to_value(params).map_err(|e| e.to_string())?),
        )
        .await
    }

    /// Graceful shutdown: kill_tree handles stdin-close → SIGTERM → SIGKILL.
    async fn kill(&mut self) {
        let pid = self.child.id();
        let _ = kill_tree(
            &mut self.child,
            Duration::from_millis(GRACE_STDIN_CLOSE_MS),
        )
        .await;
        if let Some(pid) = pid {
            unregister_pid(pid);
        }
        self.reader_handle.abort();
        if let Some(h) = self.stderr_handle.take() {
            h.abort();
        }
    }
}

// ─── Public agent struct ──────────────────────────────────────────────────

/// Cursor ACP subprocess agent.
///
/// Spawns `agent --force --approve-mcps --workspace <dir> --output-format json acp`
/// and communicates via JSON-RPC 2.0 over stdio. The process is persistent — it
/// stays alive across turns and is reused for subsequent prompts.
pub struct CursorCliAgent {
    command: String,
    working_dir: PathBuf,
    model: Option<String>,
    timeout_ms: u64,
    name: String,
    /// Lazy-initialized connection (spawned on first `run()`).
    connection: Arc<Mutex<Option<CursorConnection>>>,
    /// Channel to receive parsed events from the reader task.
    event_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<CursorEvent>>>>,
    /// Channel to know when a turn completes.
    turn_done_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<()>>>>,
    /// Senders kept here so they can be passed to the connection on init.
    event_tx: mpsc::UnboundedSender<CursorEvent>,
    turn_done_tx: mpsc::UnboundedSender<()>,
}

impl CursorCliAgent {
    /// Create a new Cursor CLI agent.
    #[must_use]
    pub fn new(command: impl Into<String>, working_dir: impl Into<PathBuf>) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (turn_done_tx, turn_done_rx) = mpsc::unbounded_channel();
        Self {
            command: command.into(),
            working_dir: working_dir.into(),
            model: None,
            timeout_ms: DEFAULT_REQUEST_TIMEOUT_MS,
            name: "cursor-cli".to_string(),
            connection: Arc::new(Mutex::new(None)),
            event_rx: Arc::new(Mutex::new(Some(event_rx))),
            turn_done_rx: Arc::new(Mutex::new(Some(turn_done_rx))),
            event_tx,
            turn_done_tx,
        }
    }

    /// Set the model slug to pass to `--model`.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the per-turn timeout in milliseconds.
    #[must_use]
    pub const fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Set the display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Ensure the connection is initialized (spawn + initialize + create session).
    async fn ensure_connected(&self) -> Result<(), String> {
        let mut conn_guard = self.connection.lock().await;
        if conn_guard.is_some() {
            return Ok(());
        }

        // Acquire startup lock to serialize spawns.
        let _lock = cursor_startup_lock().lock().await;
        tracing::info!(
            "[cursor-cli] spawning agent: {} --force --approve-mcps --workspace {} acp",
            self.command,
            self.working_dir.display()
        );

        let mut conn = CursorConnection::spawn(
            &self.command,
            &self.working_dir,
            self.model.as_deref(),
            self.event_tx.clone(),
            self.turn_done_tx.clone(),
        )
        .await?;

        conn.initialize().await?;
        conn.create_session(&self.working_dir.to_string_lossy())
            .await?;

        *conn_guard = Some(conn);
        Ok(())
    }
}

#[async_trait]
impl Agent for CursorCliAgent {
    async fn run(&self, input: &Signal, _ctx: &Context) -> AgentResult {
        let start = Instant::now();
        let prompt_text = input.body.as_text().unwrap_or("").to_string();

        // Ensure connection is alive.
        if let Err(e) = self.ensure_connected().await {
            return AgentResult::fail(
                Signal::builder(Kind::AgentOutput)
                    .body(Body::text(format!("cursor-cli spawn failed: {e}")))
                    .build(),
            );
        }

        // Send the prompt.
        {
            let mut conn_guard = self.connection.lock().await;
            let conn = conn_guard.as_mut().unwrap();
            if let Err(e) = conn.prompt(&prompt_text).await {
                return AgentResult::fail(
                    Signal::builder(Kind::AgentOutput)
                        .body(Body::text(format!("cursor-cli prompt failed: {e}")))
                        .build(),
                );
            }
        }

        // Collect events until turn completes or timeout.
        let mut output_text = String::new();
        let mut tool_calls = Vec::new();
        let timeout_dur = Duration::from_millis(self.timeout_ms);

        // Take the receivers (we hold them for the duration of the turn).
        let mut event_rx = self.event_rx.lock().await.take().unwrap_or_else(|| {
            let (_tx, rx) = mpsc::unbounded_channel();
            rx
        });
        let mut turn_done_rx = self.turn_done_rx.lock().await.take().unwrap_or_else(|| {
            let (_tx, rx) = mpsc::unbounded_channel();
            rx
        });

        let result = timeout(timeout_dur, async {
            loop {
                tokio::select! {
                    event = event_rx.recv() => {
                        match event {
                            Some(CursorEvent::MessageDelta(text)) => {
                                output_text.push_str(&text);
                            }
                            Some(CursorEvent::ToolCall(name)) => {
                                tool_calls.push(name);
                            }
                            Some(CursorEvent::CommandOutput(text)) => {
                                output_text.push_str(&text);
                            }
                            None => break, // Channel closed (process exited).
                        }
                    }
                    _ = turn_done_rx.recv() => {
                        // Drain any remaining events.
                        while let Ok(event) = event_rx.try_recv() {
                            match event {
                                CursorEvent::MessageDelta(text) | CursorEvent::CommandOutput(text) => {
                                    output_text.push_str(&text);
                                }
                                CursorEvent::ToolCall(name) => {
                                    tool_calls.push(name);
                                }
                            }
                        }
                        break;
                    }
                }
            }
        })
        .await;

        // Put receivers back.
        *self.event_rx.lock().await = Some(event_rx);
        *self.turn_done_rx.lock().await = Some(turn_done_rx);

        let elapsed = start.elapsed();

        if result.is_err() {
            // Timeout — kill the process.
            let mut conn_guard = self.connection.lock().await;
            if let Some(mut conn) = conn_guard.take() {
                conn.kill().await;
            }
            let msg = format!(
                "cursor-cli timed out after {} ms (collected {} bytes)",
                self.timeout_ms,
                output_text.len()
            );
            return AgentResult::fail(
                Signal::builder(Kind::AgentOutput)
                    .body(Body::text(msg))
                    .build(),
            );
        }

        let output = Signal::builder(Kind::AgentOutput)
            .body(Body::text(&output_text))
            .build();

        let usage = Usage {
            wall_ms: elapsed.as_millis() as u64,
            ..Usage::zero()
        };

        AgentResult::ok(output).with_usage(usage)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn backend_id(&self) -> &'static str {
        "cursor_cli"
    }
}

impl Drop for CursorCliAgent {
    fn drop(&mut self) {
        // Best-effort kill on drop — spawn a task since drop is sync.
        let conn = self.connection.clone();
        tokio::spawn(async move {
            let mut guard = conn.lock().await;
            if let Some(mut c) = guard.take() {
                c.kill().await;
            }
        });
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn prompt(text: &str) -> Signal {
        Signal::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn write_script(path: &std::path::Path, body: &str) {
        fs::write(path, body).expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path).expect("script metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).expect("chmod script");
        }
    }

    /// Mock ACP server script that responds to initialize, session/new, and session/prompt.
    /// Uses a sed/grep-free approach with only bash builtins for maximum portability
    /// under heavy parallel test load.
    fn mock_acp_script() -> String {
        // Use bash with line-by-line JSON parsing via parameter expansion.
        // This avoids spawning python3/jq which can fail under FD exhaustion.
        r#"#!/bin/bash
set -u
req_num=0
while IFS= read -r line; do
    [ -z "$line" ] && continue
    req_num=$((req_num + 1))
    case "$req_num" in
        1)
            # initialize (id=1)
            id="${line##*\"id\":}"
            id="${id%%,*}"
            id="${id%%\}*}"
            printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"serverInfo":{"name":"mock-cursor"}}}\n' "$id"
            ;;
        2)
            # session/new (id=2)
            id="${line##*\"id\":}"
            id="${id%%,*}"
            id="${id%%\}*}"
            printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"test-session-001"}}\n' "$id"
            ;;
        *)
            # session/prompt (id=N)
            id="${line##*\"id\":}"
            id="${id%%,*}"
            id="${id%%\}*}"
            printf '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"hello from cursor"}}}}\n'
            printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id"
            ;;
    esac
done
"#
        .to_string()
    }

    #[tokio::test]
    async fn cursor_cli_agent_json_rpc_request_serialization() {
        let req = JsonRpcRequest::new(42, "session/prompt", Some(serde_json::json!({"test": true})));
        let json = serde_json::to_string(&req).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 42);
        assert_eq!(parsed["method"], "session/prompt");
        assert_eq!(parsed["params"]["test"], true);
    }

    #[tokio::test]
    async fn cursor_cli_agent_notification_parsing() {
        // agent_message_chunk
        let params = serde_json::json!({
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"text": "hello world"}
            }
        });
        let event = parse_cursor_notification("session/update", Some(&params));
        assert!(matches!(event, Some(CursorEvent::MessageDelta(ref t)) if t == "hello world"));

        // tool_call
        let params = serde_json::json!({
            "update": {
                "sessionUpdate": "tool_call",
                "title": "Read file",
                "kind": "read"
            }
        });
        let event = parse_cursor_notification("session/update", Some(&params));
        assert!(matches!(event, Some(CursorEvent::ToolCall(ref t)) if t == "Read file"));

        // tool_call_update completed
        let params = serde_json::json!({
            "update": {
                "sessionUpdate": "tool_call_update",
                "status": "completed",
                "rawOutput": {"content": "file contents here"}
            }
        });
        let event = parse_cursor_notification("session/update", Some(&params));
        assert!(
            matches!(event, Some(CursorEvent::CommandOutput(ref t)) if t == "file contents here")
        );

        // agent_thought_chunk is dropped
        let params = serde_json::json!({
            "update": {
                "sessionUpdate": "agent_thought_chunk",
                "content": {"text": "thinking..."}
            }
        });
        let event = parse_cursor_notification("session/update", Some(&params));
        assert!(event.is_none());
    }

    #[tokio::test]
    async fn cursor_cli_agent_raw_server_message_classification() {
        // Response (has id, no method)
        let msg: RawServerMessage =
            serde_json::from_str(r#"{"id":1,"result":{"ok":true}}"#).unwrap();
        assert!(msg.is_response());
        assert!(!msg.is_notification());
        assert!(!msg.is_server_request());

        // Notification (has method, no id)
        let msg: RawServerMessage = serde_json::from_str(
            r#"{"method":"session/update","params":{"update":{}}}"#,
        )
        .unwrap();
        assert!(msg.is_notification());
        assert!(!msg.is_response());
        assert!(!msg.is_server_request());

        // Server request (has both id and method)
        let msg: RawServerMessage =
            serde_json::from_str(r#"{"id":5,"method":"session/update","params":{}}"#).unwrap();
        assert!(msg.is_server_request());
        assert!(!msg.is_notification());
        assert!(!msg.is_response());
    }

    #[tokio::test]
    #[ignore = "flaky under high parallelism (process resource exhaustion); run with --ignored"]
    async fn cursor_cli_agent_integration_with_mock_script() {
        let tmp = tempdir().expect("tempdir");
        let script_path = tmp.path().join("mock-agent.sh");
        write_script(&script_path, &mock_acp_script());

        let agent = CursorCliAgent::new(script_path.to_str().unwrap(), tmp.path())
            .with_timeout_ms(10_000)
            .with_name("test-cursor");

        assert_eq!(agent.name(), "test-cursor");
        assert_eq!(agent.backend_id(), "cursor_cli");

        let result = agent.run(&prompt("test prompt"), &Context::now()).await;
        assert!(
            result.success,
            "agent failed: {}",
            result.output.body.as_text().unwrap_or("?")
        );
        assert_eq!(
            result.output.body.as_text().unwrap_or(""),
            "hello from cursor"
        );
    }
}
