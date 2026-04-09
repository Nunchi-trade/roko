//! JSON-RPC stdio client for MCP servers (SS36.58).
//!
//! [`McpClient`] wraps a transport abstraction that sends JSON-RPC
//! requests and receives responses. The default transport spawns a child
//! process and communicates over stdin/stdout, but tests can substitute a
//! mock transport via the [`Transport`] trait.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

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
        let mut child = tokio::process::Command::new(command)
            .args(args)
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

        // Write to child stdin
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| McpError::Transport(format!("write to stdin: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| McpError::Transport(format!("flush stdin: {e}")))?;
        drop(stdin);

        // Read one line from child stdout
        let mut response_line = String::new();
        self.stdout
            .lock()
            .await
            .read_line(&mut response_line)
            .await
            .map_err(|e| McpError::Transport(format!("read from stdout: {e}")))?;

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
                "protocolVersion": "2024-11-05",
                "capabilities": {},
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
        assert_eq!(tools[1].name, "search");
        assert!(tools[1].input_schema.is_none());
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
}
