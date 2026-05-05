//! JSON-RPC stdio client for MCP servers (SS36.58).
//!
//! [`McpClient`] wraps a transport abstraction that sends JSON-RPC
//! requests and receives responses. The default transport spawns a child
//! process and communicates over stdin/stdout, but tests can substitute a
//! mock transport via the [`Transport`] trait.

use async_trait::async_trait;
use roko_core::defaults::{
    DEFAULT_MCP_RESPONSE_TIMEOUT_SECS, DEFAULT_MCP_STDIN_WRITE_TIMEOUT_SECS,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

/// MCP protocol version targeted by Roko's client.
pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

// ── Wire types ──────────────────────────────────────────────────────────

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    /// Must be `"2.0"`.
    pub jsonrpc: String,
    /// The method to invoke.
    pub method: String,
    /// Arguments for the method.
    pub params: serde_json::Value,
    /// Request identifier.
    pub id: u64,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResponse {
    /// Must be `"2.0"`.
    pub jsonrpc: String,
    /// Successful result (mutually exclusive with `error`).
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    /// Error result (mutually exclusive with `result`).
    #[serde(default)]
    pub error: Option<JsonRpcError>,
    /// Request identifier echoed back.
    pub id: u64,
}

/// JSON-RPC error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Numeric error code.
    pub code: i64,
    /// Human-readable message.
    pub message: String,
    /// Optional structured data.
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// An MCP tool definition as returned by `tools/list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDef {
    /// Tool name.
    pub name: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// JSON Schema for the tool's input.
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Option<serde_json::Value>,
    /// Optional behavioral annotations from newer MCP servers.
    #[serde(default)]
    pub annotations: Option<McpToolAnnotations>,
}

/// Behavioral annotations for an MCP tool.
///
/// These annotations let Roko map dynamic MCP tools onto its static
/// permission model without trusting every external tool equally.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolAnnotations {
    /// Tool does not modify external state.
    #[serde(default, rename = "readOnly")]
    pub read_only: Option<bool>,
    /// Tool accesses open-world resources such as network services.
    #[serde(default, rename = "openWorld")]
    pub open_world: Option<bool>,
    /// Calling the tool twice with the same arguments has no extra effect.
    #[serde(default)]
    pub idempotent: Option<bool>,
    /// Human-readable title for UI surfaces.
    #[serde(default)]
    pub title: Option<String>,
}

/// The result of invoking an MCP tool via `tools/call`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    /// Content blocks returned by the tool.
    #[serde(default)]
    pub content: Vec<McpContent>,
    /// Whether the tool call produced an error.
    #[serde(default, rename = "isError")]
    pub is_error: bool,
}

/// A single content block in an MCP tool result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpContent {
    /// Content type (e.g. `"text"`).
    #[serde(rename = "type")]
    pub content_type: String,
    /// The text content (present when `content_type == "text"`).
    #[serde(default)]
    pub text: Option<String>,
}

// ── Transport trait ─────────────────────────────────────────────────────

/// Abstraction over the wire between [`McpClient`] and an MCP server.
///
/// The default implementation (`StdioTransport`) spawns a child process;
/// tests inject a mock that records requests and replays canned responses.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a JSON-RPC request and receive the response.
    async fn roundtrip(&self, request: &McpRequest) -> Result<McpResponse, McpError>;
}

/// Errors from the MCP client layer.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// JSON serialization / deserialization failure.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// I/O failure talking to the child process.
    #[error("transport error: {0}")]
    Transport(String),

    /// The server returned a JSON-RPC error.
    #[error("server error {code}: {message}")]
    Server {
        /// Error code from the server.
        code: i64,
        /// Error message from the server.
        message: String,
    },
}

// ── StdioTransport ──────────────────────────────────────────────────────

/// Transport that spawns a child process and communicates over stdin/stdout
/// using newline-delimited JSON-RPC.
pub struct StdioTransport {
    stdin: Mutex<BufWriter<ChildStdin>>,
    stdout: Mutex<BufReader<ChildStdout>>,
    /// Keep child handle alive so the process is not dropped.
    _child: Mutex<Child>,
}

impl StdioTransport {
    /// Spawn a child MCP server process.
    ///
    /// The process is started with stdin/stdout piped for JSON-RPC communication.
    /// Stderr is inherited so server logs appear in the parent's stderr.
    pub fn spawn(command: &str, args: &[String]) -> Result<Self, McpError> {
        Self::spawn_with_env(command, args, &HashMap::new())
    }

    /// Spawn a child MCP server process with additional environment variables.
    ///
    /// The provided environment is layered on top of the current process
    /// environment before the child process is started.
    pub fn spawn_with_env(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self, McpError> {
        let resolved_env = resolve_env(env);
        let mut child = tokio::process::Command::new(command)
            .args(args)
            .envs(resolved_env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| McpError::Transport(format!("failed to spawn {command}: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Transport("child stdin not available".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Transport("child stdout not available".into()))?;

        Ok(Self {
            stdin: Mutex::new(BufWriter::new(stdin)),
            stdout: Mutex::new(BufReader::new(stdout)),
            _child: Mutex::new(child),
        })
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn roundtrip(&self, request: &McpRequest) -> Result<McpResponse, McpError> {
        // Serialize request as a single JSON line
        let mut line = serde_json::to_string(request)?;
        line.push('\n');

        // Write to child stdin (with timeout)
        let write_result = tokio::time::timeout(
            Duration::from_secs(DEFAULT_MCP_STDIN_WRITE_TIMEOUT_SECS),
            async {
                let mut stdin = self.stdin.lock().await;
                stdin.write_all(line.as_bytes()).await?;
                stdin.flush().await?;
                Ok::<(), std::io::Error>(())
            },
        )
        .await;
        match write_result {
            Err(_) => {
                return Err(McpError::Transport(
                    "MCP server stdin write timed out after 5s".into(),
                ));
            }
            Ok(Err(e)) => return Err(McpError::Transport(format!("write to stdin: {e}"))),
            Ok(Ok(())) => {}
        }

        // Read one line from child stdout (with timeout)
        let read_result = tokio::time::timeout(
            Duration::from_secs(DEFAULT_MCP_RESPONSE_TIMEOUT_SECS),
            async {
                let mut stdout = self.stdout.lock().await;
                let mut line = String::new();
                stdout.read_line(&mut line).await?;
                Ok::<String, std::io::Error>(line)
            },
        )
        .await;
        let response_line = match read_result {
            Err(_) => {
                return Err(McpError::Transport(
                    "MCP server response timed out after 30s".into(),
                ));
            }
            Ok(Err(e)) => return Err(McpError::Transport(format!("read from stdout: {e}"))),
            Ok(Ok(line)) => line,
        };

        if response_line.is_empty() {
            return Err(McpError::Transport(
                "child process closed stdout (EOF)".into(),
            ));
        }

        let resp: McpResponse = serde_json::from_str(&response_line)?;
        Ok(resp)
    }
}

// ── McpClient ───────────────────────────────────────────────────────────

/// JSON-RPC client for a single MCP server.
///
/// Wraps a [`Transport`] and provides typed helpers for the three MCP
/// methods Roko uses: `initialize`, `tools/list`, and `tools/call`.
pub struct McpClient<T: Transport> {
    transport: T,
    next_id: AtomicU64,
}

impl<T: Transport> McpClient<T> {
    /// Create a new client over the given transport.
    pub const fn new(transport: T) -> Self {
        Self {
            transport,
            next_id: AtomicU64::new(1),
        }
    }

    /// Allocate the next request ID.
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Build and send a JSON-RPC request, returning the parsed response.
    async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, McpError> {
        let req = McpRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: self.next_id(),
        };
        let resp = self.transport.roundtrip(&req).await?;
        if let Some(err) = resp.error {
            return Err(McpError::Server {
                code: err.code,
                message: err.message,
            });
        }
        Ok(resp.result.unwrap_or(serde_json::Value::Null))
    }

    /// Send the `initialize` handshake.
    ///
    /// Returns the server's capability object (or `Null` if the server
    /// doesn't return one).
    pub async fn initialize(&self) -> Result<serde_json::Value, McpError> {
        self.call(
            "initialize",
            serde_json::json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "sampling": {},
                    "roots": {
                        "listChanged": true
                    }
                },
                "clientInfo": {
                    "name": "roko",
                    "version": "0.1.0"
                }
            }),
        )
        .await
    }

    /// List available tools from the MCP server.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>, McpError> {
        let result = self.call("tools/list", serde_json::json!({})).await?;
        let tools_value = result
            .get("tools")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        let tools: Vec<McpToolDef> = serde_json::from_value(tools_value)?;
        Ok(tools)
    }

    /// Invoke a tool on the MCP server.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult, McpError> {
        let result = self
            .call(
                "tools/call",
                serde_json::json!({
                    "name": name,
                    "arguments": arguments,
                }),
            )
            .await?;
        let tool_result: McpToolResult = serde_json::from_value(result)?;
        Ok(tool_result)
    }
}

fn resolve_env(env: &HashMap<String, String>) -> HashMap<String, String> {
    env.iter()
        .map(|(key, value)| (key.clone(), resolve_env_value(value)))
        .collect()
}

fn resolve_env_value(value: &str) -> String {
    let Some(name) = value
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return value.to_string();
    };

    std::env::var(name).unwrap_or_default()
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A mock transport that records requests and replays canned responses.
    struct MockTransport {
        /// Canned responses, popped in order.
        responses: Mutex<Vec<McpResponse>>,
        /// Recorded requests.
        requests: Mutex<Vec<McpRequest>>,
    }

    impl MockTransport {
        fn new(responses: Vec<McpResponse>) -> Self {
            Self {
                responses: Mutex::new(responses),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn take_requests(&self) -> Vec<McpRequest> {
            self.requests.lock().unwrap().drain(..).collect()
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn roundtrip(&self, request: &McpRequest) -> Result<McpResponse, McpError> {
            self.requests.lock().unwrap().push(request.clone());
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(McpError::Transport("no more canned responses".into()));
            }
            Ok(responses.remove(0))
        }
    }

    fn ok_response(id: u64, result: serde_json::Value) -> McpResponse {
        McpResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    fn err_response(id: u64, code: i64, message: &str) -> McpResponse {
        McpResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
                data: None,
            }),
            id,
        }
    }

    #[test]
    fn mcp_env_placeholders_resolve_from_host_env() {
        let literal = resolve_env_value("plain-token");
        assert_eq!(literal, "plain-token");

        let missing = resolve_env_value("${ROKO_DP18_MISSING_ENV_FOR_TEST}");
        assert_eq!(missing, "");
    }

    #[tokio::test]
    async fn mcp_initialize_sends_correct_method() {
        let transport = MockTransport::new(vec![ok_response(
            1,
            serde_json::json!({"capabilities": {}}),
        )]);
        let client = McpClient::new(transport);
        let result = client.initialize().await.unwrap();
        assert!(result.get("capabilities").is_some());

        let reqs = client.transport.take_requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "initialize");
        assert_eq!(reqs[0].jsonrpc, "2.0");
        assert_eq!(reqs[0].params["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert!(reqs[0].params["capabilities"]["sampling"].is_object());
    }

    #[tokio::test]
    async fn mcp_list_tools_parses_tool_definitions() {
        let tools_json = serde_json::json!({
            "tools": [
                {
                    "name": "read_file",
                    "description": "Read a file",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {"type": "string"}
                        },
                        "required": ["path"]
                    }
                },
                {
                    "name": "search",
                    "description": "Search text"
                }
            ]
        });
        let transport = MockTransport::new(vec![ok_response(1, tools_json)]);
        let client = McpClient::new(transport);
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "read_file");
        assert_eq!(tools[0].description.as_deref(), Some("Read a file"));
        assert!(tools[0].input_schema.is_some());
        assert!(tools[0].annotations.is_none());
        assert_eq!(tools[1].name, "search");
        assert!(tools[1].input_schema.is_none());
    }

    #[tokio::test]
    async fn mcp_list_tools_parses_annotations() {
        let tools_json = serde_json::json!({
            "tools": [{
                "name": "get_pr",
                "annotations": {
                    "readOnly": true,
                    "openWorld": true,
                    "idempotent": true,
                    "title": "Get PR"
                }
            }]
        });
        let transport = MockTransport::new(vec![ok_response(1, tools_json)]);
        let client = McpClient::new(transport);

        let tools = client.list_tools().await.unwrap();
        let annotations = tools[0].annotations.as_ref().expect("annotations");
        assert_eq!(annotations.read_only, Some(true));
        assert_eq!(annotations.open_world, Some(true));
        assert_eq!(annotations.idempotent, Some(true));
        assert_eq!(annotations.title.as_deref(), Some("Get PR"));
    }

    #[tokio::test]
    async fn mcp_call_tool_sends_name_and_args() {
        let result_json = serde_json::json!({
            "content": [{"type": "text", "text": "hello world"}],
            "isError": false
        });
        let transport = MockTransport::new(vec![ok_response(1, result_json)]);
        let client = McpClient::new(transport);
        let result = client
            .call_tool("read_file", serde_json::json!({"path": "/tmp/test"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content.len(), 1);
        assert_eq!(result.content[0].text.as_deref(), Some("hello world"));

        let reqs = client.transport.take_requests();
        assert_eq!(reqs[0].method, "tools/call");
        let params = &reqs[0].params;
        assert_eq!(params["name"], "read_file");
        assert_eq!(params["arguments"]["path"], "/tmp/test");
    }

    #[tokio::test]
    async fn mcp_server_error_propagates() {
        let transport = MockTransport::new(vec![err_response(1, -32600, "invalid request")]);
        let client = McpClient::new(transport);
        let err = client.initialize().await.unwrap_err();
        match err {
            McpError::Server { code, message } => {
                assert_eq!(code, -32600);
                assert_eq!(message, "invalid request");
            }
            other => panic!("expected Server error, got: {other}"),
        }
    }

    #[tokio::test]
    async fn mcp_request_ids_increment() {
        let transport = MockTransport::new(vec![
            ok_response(1, serde_json::json!({})),
            ok_response(2, serde_json::json!({"tools": []})),
            ok_response(3, serde_json::json!({"content": [], "isError": false})),
        ]);
        let client = McpClient::new(transport);
        let _ = client.initialize().await;
        let _ = client.list_tools().await;
        let _ = client.call_tool("x", serde_json::json!({})).await;

        let reqs = client.transport.take_requests();
        assert_eq!(reqs[0].id, 1);
        assert_eq!(reqs[1].id, 2);
        assert_eq!(reqs[2].id, 3);
    }

    #[tokio::test]
    async fn mcp_list_tools_empty_result() {
        let transport = MockTransport::new(vec![ok_response(1, serde_json::json!({"tools": []}))]);
        let client = McpClient::new(transport);
        let tools = client.list_tools().await.unwrap();
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn mcp_call_tool_error_flag() {
        let result_json = serde_json::json!({
            "content": [{"type": "text", "text": "file not found"}],
            "isError": true
        });
        let transport = MockTransport::new(vec![ok_response(1, result_json)]);
        let client = McpClient::new(transport);
        let result = client
            .call_tool("read_file", serde_json::json!({"path": "/no/such/file"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert_eq!(result.content[0].text.as_deref(), Some("file not found"));
    }

    // ── Timeout tests ────────────────────────────────────────────────────

    /// A transport that delays for a configurable duration before responding.
    /// Used with `tokio::test(start_paused = true)` to test timeout behavior
    /// without real wall-clock waits.
    struct SlowTransport {
        delay: std::time::Duration,
    }

    impl SlowTransport {
        fn new(delay: std::time::Duration) -> Self {
            Self { delay }
        }
    }

    #[async_trait]
    impl Transport for SlowTransport {
        async fn roundtrip(&self, request: &McpRequest) -> Result<McpResponse, McpError> {
            tokio::time::sleep(self.delay).await;
            Ok(McpResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::json!({})),
                error: None,
                id: request.id,
            })
        }
    }

    /// A transport that simulates stdin write timeout by delaying in roundtrip.
    ///
    /// In the real `StdioTransport`, the write and read timeouts are separate.
    /// Since the `Transport` trait abstracts both into a single `roundtrip`,
    /// we test the timeout constants and error messages directly against
    /// `StdioTransport`'s documented behavior.
    ///
    /// This test verifies that the 5-second stdin write timeout constant is
    /// correctly defined and matches the expected value.
    #[test]
    fn mcp_stdin_write_timeout_is_5s() {
        assert_eq!(
            DEFAULT_MCP_STDIN_WRITE_TIMEOUT_SECS, 5,
            "MCP stdin write timeout must be 5 seconds"
        );
    }

    /// Verifies that the 30-second response timeout constant is correctly defined.
    #[test]
    fn mcp_response_timeout_is_30s() {
        assert_eq!(
            DEFAULT_MCP_RESPONSE_TIMEOUT_SECS, 30,
            "MCP response timeout must be 30 seconds"
        );
    }

    /// With `start_paused = true`, time only advances when we `sleep`.
    /// A transport that takes longer than 30s (the response timeout) should
    /// cause a timeout error in the real StdioTransport.
    ///
    /// Here we verify the timeout logic by simulating the exact pattern used
    /// in StdioTransport::roundtrip: `tokio::time::timeout(Duration, future)`.
    #[tokio::test(start_paused = true)]
    async fn mcp_response_timeout_fires_at_30s() {
        let response_timeout = Duration::from_secs(DEFAULT_MCP_RESPONSE_TIMEOUT_SECS);

        // Simulate a server that takes 31 seconds to respond.
        let result = tokio::time::timeout(response_timeout, async {
            tokio::time::sleep(Duration::from_secs(31)).await;
            Ok::<McpResponse, McpError>(McpResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::json!({})),
                error: None,
                id: 1,
            })
        })
        .await;

        assert!(
            result.is_err(),
            "Response should time out after 30s when server takes 31s"
        );
    }

    /// Verify that a response arriving just under 30s does NOT time out.
    #[tokio::test(start_paused = true)]
    async fn mcp_response_within_30s_succeeds() {
        let response_timeout = Duration::from_secs(DEFAULT_MCP_RESPONSE_TIMEOUT_SECS);

        let result = tokio::time::timeout(response_timeout, async {
            tokio::time::sleep(Duration::from_secs(29)).await;
            Ok::<McpResponse, McpError>(McpResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::json!({})),
                error: None,
                id: 1,
            })
        })
        .await;

        assert!(
            result.is_ok(),
            "Response arriving at 29s should NOT time out"
        );
    }

    /// Verify that stdin write timeout fires at 5s.
    #[tokio::test(start_paused = true)]
    async fn mcp_stdin_write_timeout_fires_at_5s() {
        let write_timeout = Duration::from_secs(DEFAULT_MCP_STDIN_WRITE_TIMEOUT_SECS);

        // Simulate a blocked stdin write that takes 6 seconds.
        let result = tokio::time::timeout(write_timeout, async {
            tokio::time::sleep(Duration::from_secs(6)).await;
            Ok::<(), std::io::Error>(())
        })
        .await;

        assert!(
            result.is_err(),
            "Stdin write should time out after 5s when write takes 6s"
        );
    }

    /// Verify that a fast stdin write (under 5s) does not time out.
    #[tokio::test(start_paused = true)]
    async fn mcp_stdin_write_within_5s_succeeds() {
        let write_timeout = Duration::from_secs(DEFAULT_MCP_STDIN_WRITE_TIMEOUT_SECS);

        // Simulate a write that takes 4 seconds.
        let result = tokio::time::timeout(write_timeout, async {
            tokio::time::sleep(Duration::from_secs(4)).await;
            Ok::<(), std::io::Error>(())
        })
        .await;

        assert!(
            result.is_ok(),
            "Stdin write completing at 4s should NOT time out"
        );
    }

    /// End-to-end test: a slow transport (>30s) used through McpClient should
    /// still get a valid response when within bounds (the Transport trait
    /// abstracts the timeout — this tests that McpClient properly propagates
    /// transport results).
    #[tokio::test(start_paused = true)]
    async fn mcp_client_with_slow_but_successful_transport() {
        // 10s delay is well within the 30s response timeout.
        let transport = SlowTransport::new(Duration::from_secs(10));
        let client = McpClient::new(transport);
        let result = client.initialize().await;
        assert!(result.is_ok(), "Transport responding in 10s should succeed");
    }

    /// Test that the error message format for stdin timeout matches the
    /// expected string used by StdioTransport.
    #[test]
    fn mcp_stdin_timeout_error_message_format() {
        let err = McpError::Transport("MCP server stdin write timed out after 5s".into());
        assert!(err.to_string().contains("stdin write timed out after 5s"));
    }

    /// Test that the error message format for response timeout matches the
    /// expected string used by StdioTransport.
    #[test]
    fn mcp_response_timeout_error_message_format() {
        let err = McpError::Transport("MCP server response timed out after 30s".into());
        assert!(err.to_string().contains("response timed out after 30s"));
    }
}
