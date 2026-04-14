# 13 — roko-mcp-stdio

> Generic MCP stdio scaffold: protocol handler, tool registration,
> base implementation for building custom MCP servers.


> **Implementation**: Scaffold

---

## Overview

`roko-mcp-stdio` is a scaffold crate that provides the base implementation for building
MCP servers in Rust. It handles the MCP protocol (JSON-RPC 2.0 over stdio), tool registration,
request routing, and error handling — so that custom MCP servers only need to implement their
tool-specific logic.

**Status:** Scaffold (base implementation, not feature-complete)

**Purpose:** Shared foundation for `roko-mcp-github`, `roko-mcp-slack`, `roko-mcp-scripts`,
and any custom MCP servers.

---

## Architecture

```
roko-mcp-stdio
    ├── protocol.rs     # JSON-RPC 2.0 message types and parsing
    ├── server.rs       # Main event loop (read stdin, dispatch, write stdout)
    ├── registry.rs     # Tool registration and schema generation
    ├── handler.rs      # Trait for tool implementations
    └── lib.rs          # Public API
```

### Core Trait: McpToolHandler

Custom MCP servers implement this trait for each tool:

```rust
#[async_trait]
pub trait McpToolHandler: Send + Sync {
    /// Tool name (returned in tools/list).
    fn name(&self) -> &str;

    /// Tool description (LLM-facing).
    fn description(&self) -> &str;

    /// JSON Schema for input parameters.
    fn input_schema(&self) -> serde_json::Value;

    /// Execute the tool with the given arguments.
    async fn execute(
        &self,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult>;
}

pub struct McpToolResult {
    pub content: Vec<McpContent>,
    pub is_error: bool,
}

pub enum McpContent {
    Text { text: String },
    Image { data: String, mime_type: String },
    Resource { uri: String, text: String },
}
```

### Server Builder

```rust
pub struct McpServerBuilder {
    tools: Vec<Box<dyn McpToolHandler>>,
    name: String,
    version: String,
}

impl McpServerBuilder {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            tools: Vec::new(),
            name: name.to_string(),
            version: version.to_string(),
        }
    }

    /// Register a tool handler.
    pub fn tool(mut self, handler: impl McpToolHandler + 'static) -> Self {
        self.tools.push(Box::new(handler));
        self
    }

    /// Start the MCP server (reads from stdin, writes to stdout).
    pub async fn serve(self) -> Result<()> {
        let server = McpServer::new(self.name, self.version, self.tools);
        server.run().await
    }
}
```

### Usage Example

Building a custom MCP server using `roko-mcp-stdio`:

```rust
use roko_mcp_stdio::{McpServerBuilder, McpToolHandler, McpToolResult, McpContent};

struct MyTool;

#[async_trait]
impl McpToolHandler for MyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Does something useful" }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "input": { "type": "string" }
            },
            "required": ["input"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<McpToolResult> {
        let input = arguments["input"].as_str().unwrap_or("");
        Ok(McpToolResult {
            content: vec![McpContent::Text {
                text: format!("Processed: {input}"),
            }],
            is_error: false,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    McpServerBuilder::new("my-mcp-server", "0.1.0")
        .tool(MyTool)
        .serve()
        .await
}
```

---

## Protocol Implementation

### JSON-RPC 2.0 Messages

```rust
#[derive(Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,    // Must be "2.0"
    pub method: String,
    pub params: Option<serde_json::Value>,
    pub id: serde_json::Value,
}

#[derive(Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,    // "2.0"
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
    pub id: serde_json::Value,
}

#[derive(Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
}
```

### Supported Methods

| Method | Description |
|---|---|
| `initialize` | Server handshake (capabilities exchange) |
| `tools/list` | List all registered tools with schemas |
| `tools/call` | Execute a tool with arguments |
| `notifications/initialized` | Client confirmation (no response) |

### Server Event Loop

```rust
pub struct McpServer {
    name: String,
    version: String,
    tools: HashMap<String, Box<dyn McpToolHandler>>,
}

impl McpServer {
    pub async fn run(&self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let mut reader = BufReader::new(stdin);
        let mut writer = BufWriter::new(stdout);

        loop {
            let mut line = String::new();
            let bytes_read = reader.read_line(&mut line).await?;
            if bytes_read == 0 { break; } // EOF

            let request: JsonRpcRequest = serde_json::from_str(&line)?;
            let response = self.dispatch(&request).await;
            let response_json = serde_json::to_string(&response)?;

            writer.write_all(response_json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }

        Ok(())
    }

    async fn dispatch(&self, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "initialize" => self.handle_initialize(request),
            "tools/list" => self.handle_tools_list(request),
            "tools/call" => self.handle_tools_call(request).await,
            "notifications/initialized" => return, // No response for notifications
            _ => self.method_not_found(request),
        }
    }
}
```

---

## Tool Registry

```rust
pub struct ToolRegistry {
    tools: Vec<Box<dyn McpToolHandler>>,
}

impl ToolRegistry {
    /// Generate the tools/list response.
    pub fn list(&self) -> serde_json::Value {
        json!({
            "tools": self.tools.iter().map(|t| json!({
                "name": t.name(),
                "description": t.description(),
                "inputSchema": t.input_schema(),
            })).collect::<Vec<_>>()
        })
    }

    /// Find a tool by name.
    pub fn get(&self, name: &str) -> Option<&dyn McpToolHandler> {
        self.tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
    }
}
```

---

## Error Handling

Standard JSON-RPC 2.0 error codes:

| Code | Meaning | When |
|---|---|---|
| -32700 | Parse error | Malformed JSON |
| -32600 | Invalid request | Missing jsonrpc/method |
| -32601 | Method not found | Unknown method |
| -32602 | Invalid params | Parameters don't match schema |
| -32603 | Internal error | Tool execution failure |

---

## Extension Points

### Custom Transports

While `roko-mcp-stdio` defaults to stdio, the protocol layer is transport-agnostic. Custom
transports (WebSocket, HTTP SSE) can be implemented by providing alternative read/write
implementations.

### Middleware

The server supports middleware for cross-cutting concerns:

```rust
pub trait McpMiddleware: Send + Sync {
    async fn before_tool_call(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<()>;

    async fn after_tool_call(
        &self,
        tool_name: &str,
        result: &McpToolResult,
        duration: Duration,
    );
}
```

Use cases for middleware:
- **Logging**: Record all tool calls for debugging
- **Metrics**: Track tool execution latency and error rates
- **Rate limiting**: Enforce per-tool rate limits
- **Caching**: Cache deterministic tool results

---

## Relationship to Other MCP Crates

```
roko-mcp-stdio (this crate)
    ↑ depends on
    ├── roko-mcp-github  (17 tools)
    ├── roko-mcp-slack   (8 tools)
    └── roko-mcp-scripts (N tools, config-driven)
```

Each MCP server crate depends on `roko-mcp-stdio` for the protocol handling and implements
`McpToolHandler` for its specific tools. This ensures consistent protocol behavior across all
MCP servers.
