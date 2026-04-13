# 09 — MCP Integration Architecture

> Model Context Protocol (MCP) — JSON-RPC stdio transport, tool converter,
> dynamic registry, and how MCP extends the Synapse tool system.


> **Implementation**: Shipping

---

## Overview

The **Model Context Protocol (MCP)** is a JSON-RPC 2.0 protocol for extending LLM agents with
external tools at runtime. In Roko, MCP provides the mechanism for dynamically discovering and
loading tools from external servers — a complement to the statically compiled built-in tools
and domain plugin tools.

MCP integration lives in `roko-agent` (`crates/roko-agent/src/mcp/`). The current
implementation includes:
- MCP client (JSON-RPC over stdio transport)
- Tool converter (MCP tool schema → Roko ToolDef)
- Dynamic tool registry (merges MCP tools with built-in tools)
- Configuration passthrough (`agent.mcp_config` in `roko.toml` → `--mcp-config`)

---

## Architecture

### MCP in the Roko Tool Stack

```
Agent Cognitive Loop
       |
       v
+--- Tool Registry (merged) --------------------------------+
|  Static tools (16 built-in from roko-std)                  |
|  + Domain plugin tools (423+ chain tools, etc.)            |
|  + MCP tools (dynamically discovered)                      |
+--------+---------------------------------------------------+
         |
         v (for MCP tools)
+--- MCP Client (roko-agent) --------------------------------+
|  JSON-RPC 2.0 over stdio                                   |
|  Tool converter: MCP schema → ToolDef                      |
|  Auto-discovery from roko.toml                             |
+--------+---------------------------------------------------+
         |
         v
+--- MCP Servers ---------------------------------------------+
|  roko-mcp-github  (17 tools)  — GitHub API operations      |
|  roko-mcp-slack   (8 tools)   — Slack messaging            |
|  roko-mcp-scripts (N tools)   — Config-driven wrappers     |
|  roko-mcp-stdio   (scaffold)  — Generic stdio protocol     |
|  Third-party MCP servers                                    |
+-------------------------------------------------------------+
```

### Protocol: JSON-RPC 2.0 over stdio

MCP uses JSON-RPC 2.0 over standard input/output. The MCP client spawns the server process,
sends JSON-RPC requests over stdin, and reads responses from stdout.

```
Client (roko-agent)          Server (roko-mcp-github)
       |                              |
       |  {"jsonrpc":"2.0",           |
       |   "method":"tools/list",     |
       |   "id":1}                    |
       | ---------------------------→ |
       |                              |
       |  {"jsonrpc":"2.0",           |
       |   "result":{"tools":[...]},  |
       |   "id":1}                    |
       | ←--------------------------- |
       |                              |
       |  {"jsonrpc":"2.0",           |
       |   "method":"tools/call",     |
       |   "params":{"name":"...",    |
       |    "arguments":{...}},       |
       |   "id":2}                    |
       | ---------------------------→ |
       |                              |
       |  {"jsonrpc":"2.0",           |
       |   "result":{"content":[...]},|
       |   "id":2}                    |
       | ←--------------------------- |
```

### Transport Advantages

- **Process isolation**: MCP servers run in separate processes. A crashing server doesn't
  affect the agent.
- **Language agnostic**: MCP servers can be written in any language (Rust, TypeScript, Python).
- **Security boundary**: The stdio transport provides a natural security boundary. Untrusted
  MCP tools run in their own process with their own permissions.

---

## MCP Client Implementation

The MCP client in `roko-agent` handles:

### 1. Server Discovery

Servers are declared in `roko.toml`:

```toml
[agent.mcp_config]
# Path to MCP config file (alternative to inline config)
config_file = ".roko/mcp-config.json"

# Or inline server declarations
[[agent.mcp_servers]]
name = "github"
command = "roko-mcp-github"
args = ["--repo", "nunchi/roko"]
env = { GITHUB_TOKEN = "${GITHUB_TOKEN}" }

[[agent.mcp_servers]]
name = "slack"
command = "roko-mcp-slack"
env = { SLACK_BOT_TOKEN = "${SLACK_BOT_TOKEN}" }

[[agent.mcp_servers]]
name = "scripts"
command = "roko-mcp-scripts"
args = ["--config", ".roko/scripts.toml"]
```

The `mcp_config` is passed through to the agent dispatcher as `--mcp-config`, which the
Claude CLI or other LLM backends use to spawn MCP servers alongside the agent session.

### 2. Tool Discovery

On startup, the client calls `tools/list` on each configured server to discover available
tools:

```rust
pub async fn discover_tools(&self, server: &McpServer) -> Result<Vec<McpTool>> {
    let response = self.call(server, "tools/list", json!({})).await?;
    let tools: Vec<McpTool> = serde_json::from_value(response.result)?;
    Ok(tools)
}
```

### 3. Tool Conversion

MCP tool schemas are converted to Roko `ToolDef` format:

```rust
pub fn convert_mcp_tool(mcp_tool: &McpTool, server_name: &str) -> ToolDef {
    ToolDef {
        name: format!("{}.{}", server_name, mcp_tool.name),
        description: &mcp_tool.description,
        category: Category::Custom(server_name.to_string()),
        capability: CapabilityTier::Write, // MCP tools are treated as Write by default
        risk_tier: RiskTier::Layer2,       // Conservative default
        // ... other fields
    }
}
```

MCP tools are namespaced by server name to avoid collisions:
- `github.get_pr` (from roko-mcp-github)
- `slack.post_message` (from roko-mcp-slack)
- `scripts.pm_sync` (from roko-mcp-scripts)

### 4. Tool Execution

When the agent selects an MCP tool, the client dispatches the call to the appropriate server:

```rust
pub async fn call_tool(
    &self,
    server: &McpServer,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<ToolResult> {
    let response = self.call(server, "tools/call", json!({
        "name": tool_name,
        "arguments": arguments,
    })).await?;

    // Convert MCP response to ToolResult
    Ok(ToolResult {
        data: response.result,
        is_error: response.is_error.unwrap_or(false),
        schema_version: 1,
        expected_outcome: None,
        actual_outcome: None,
        ground_truth_source: None,
    })
}
```

---

## Dynamic Tool Registry

The merged tool registry combines static and MCP tools:

```rust
pub struct MergedToolRegistry {
    /// Static built-in tools (from roko-std).
    static_tools: &'static [ToolDef],
    /// Domain plugin tools (from domain crate).
    domain_tools: Vec<ToolDef>,
    /// MCP-discovered tools (from MCP servers).
    mcp_tools: Vec<ToolDef>,
}

impl ToolRegistry for MergedToolRegistry {
    fn get(&self, name: &str) -> Option<&ToolDef> {
        self.static_tools.iter().find(|t| t.name == name)
            .or_else(|| self.domain_tools.iter().find(|t| t.name == name))
            .or_else(|| self.mcp_tools.iter().find(|t| t.name == name))
    }

    fn all(&self) -> Vec<&ToolDef> {
        self.static_tools.iter()
            .chain(self.domain_tools.iter())
            .chain(self.mcp_tools.iter())
            .collect()
    }
}
```

Precedence order: static tools > domain tools > MCP tools. If a name collision occurs,
the higher-precedence tool wins.

---

## MCP Configuration in Agent Templates

Agent templates (see `15-16-agent-templates.md`) specify which MCP servers they need:

```toml
# Template example
name = "pr-review-agent"
mcp_servers = ["github"]
# ...
```

When the template is instantiated, the runtime:
1. Resolves `"github"` to the server configuration in `roko.toml`
2. Starts the MCP server process
3. Discovers tools via `tools/list`
4. Merges discovered tools into the agent's tool registry
5. Stops the MCP server when the agent session ends

---

## Security Considerations

### MCP Trust Model

MCP tools are treated as **untrusted by default**:

| Property | Built-in Tools | Domain Plugin Tools | MCP Tools |
|---|---|---|---|
| Compiled with Roko | Yes | Yes | No |
| Code reviewed | Yes | Yes | Depends on source |
| Process isolation | No (in-process) | No (in-process) | Yes (separate process) |
| Default trust tier | Per-tool | Per-tool | Write (conservative) |
| WASM sandbox | No | No | Optional (for extra isolation) |

MCP tools run in separate processes (stdio transport), which provides basic isolation. For
additional security, MCP tools can be wrapped in the WASM sandbox (see `04-safety-hooks.md`).

### Credential Handling

MCP server credentials are passed via environment variables, never via stdio:

```toml
[[agent.mcp_servers]]
name = "github"
command = "roko-mcp-github"
env = { GITHUB_TOKEN = "${GITHUB_TOKEN}" }  # Resolved from host env
```

The `${GITHUB_TOKEN}` syntax resolves from the host environment. The token is never passed
over the stdio channel and is not accessible to other MCP servers.

---

## MCP Protocol Specification (2025-11-25)

The MCP specification is maintained by the Linux Foundation's Agentic AI Foundation (donated
by Anthropic in December 2025). Roko's MCP client targets the **2025-11-25** spec version.

### Protocol Versioning

MCP uses a date-based version scheme. During `initialize`, client and server negotiate the
highest mutually supported version:

```rust
/// MCP protocol version negotiation.
pub struct InitializeRequest {
    /// Protocol version the client supports.
    pub protocol_version: String, // "2025-11-25"
    /// Client capabilities.
    pub capabilities: ClientCapabilities,
    /// Client info.
    pub client_info: ClientInfo,
}

pub struct InitializeResult {
    /// Protocol version the server supports (must match or be earlier).
    pub protocol_version: String,
    /// Server capabilities.
    pub capabilities: ServerCapabilities,
    /// Server info.
    pub server_info: ServerInfo,
}

pub struct ClientCapabilities {
    /// Client supports sampling requests from server.
    pub sampling: Option<SamplingCapability>,
    /// Client supports roots (filesystem access).
    pub roots: Option<RootsCapability>,
}

pub struct ServerCapabilities {
    /// Server exposes tools.
    pub tools: Option<ToolsCapability>,
    /// Server exposes resources.
    pub resources: Option<ResourcesCapability>,
    /// Server exposes prompt templates.
    pub prompts: Option<PromptsCapability>,
    /// Server supports logging.
    pub logging: Option<LoggingCapability>,
}
```

### Three MCP Primitives

MCP servers expose three primitive types:

| Primitive | Purpose | Methods | Roko Usage |
|---|---|---|---|
| **Tools** | Executable functions the model can invoke | `tools/list`, `tools/call` | Primary — all MCP integrations |
| **Resources** | Data the model can read (files, DB records) | `resources/list`, `resources/read` | Future — for context injection |
| **Prompts** | Reusable prompt templates | `prompts/list`, `prompts/get` | Future — for prompt experiments |

### Tool Annotations (Spec 2025-03-26+)

MCP tools carry behavioral annotations that help the client make safety decisions:

```rust
/// MCP tool annotations per spec 2025-03-26.
pub struct McpToolAnnotations {
    /// Tool does not modify external state.
    pub read_only: Option<bool>,
    /// Tool accesses the open world (network, external APIs).
    pub open_world: Option<bool>,
    /// Tool is idempotent — calling twice produces the same result.
    pub idempotent: Option<bool>,
    /// Human-readable title for the tool.
    pub title: Option<String>,
}
```

Roko maps these annotations to its own trust tiers:

| MCP Annotation | Roko Mapping |
|---|---|
| `readOnly: true` | `CapabilityTier::Read` — no capability token needed |
| `readOnly: false` or absent | `CapabilityTier::Write` — conservative default |
| `openWorld: true` | Additional safety check: network access flagged |
| `idempotent: true` | Safe to retry on failure |

### Sampling (Reverse Direction)

MCP's sampling capability lets servers request LLM inference from the client. This enables
server-side agent loops where the MCP server orchestrates multi-step reasoning:

```rust
/// MCP sampling request — server asks client to run LLM inference.
pub struct SamplingRequest {
    pub messages: Vec<SamplingMessage>,
    pub model_preferences: Option<ModelPreferences>,
    pub system_prompt: Option<String>,
    pub max_tokens: u32,
}

pub struct SamplingMessage {
    pub role: Role, // "user" or "assistant"
    pub content: McpContent,
}
```

Roko's MCP client supports sampling when the `ClientCapabilities.sampling` field is set. The
client routes sampling requests through the agent's configured LLM backend, applying the same
token budget and rate limits as direct agent inference.

---

## Transport Layers

### stdio (Primary)

The stdio transport is the default for local MCP servers. The client spawns the server as a
child process and communicates via stdin/stdout using newline-delimited JSON-RPC:

```rust
/// MCP client over stdio transport.
pub struct StdioMcpClient {
    /// Child process handle.
    child: tokio::process::Child,
    /// Writer to child's stdin.
    writer: BufWriter<ChildStdin>,
    /// Reader from child's stdout.
    reader: BufReader<ChildStdout>,
    /// Request ID counter.
    next_id: AtomicU64,
    /// Pending request handlers.
    pending: Arc<DashMap<u64, oneshot::Sender<JsonRpcResponse>>>,
}

impl StdioMcpClient {
    pub async fn spawn(config: &McpServerConfig) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set environment variables
        for (key, value) in &config.env {
            let resolved = resolve_env_var(value)?;
            cmd.env(key, resolved);
        }

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();

        let client = Self {
            child,
            writer: BufWriter::new(stdin),
            reader: BufReader::new(stdout),
            next_id: AtomicU64::new(1),
            pending: Arc::new(DashMap::new()),
        };

        // Run initialization handshake
        client.initialize().await?;
        Ok(client)
    }

    /// Send a JSON-RPC request and await response.
    pub async fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id,
        });

        let (tx, rx) = oneshot::channel();
        self.pending.insert(id, tx);

        // Write request
        let line = serde_json::to_string(&request)?;
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;

        // Await response
        let response = tokio::time::timeout(
            Duration::from_secs(30),
            rx,
        ).await??;

        if let Some(error) = response.error {
            return Err(anyhow!("MCP error {}: {}", error.code, error.message));
        }
        Ok(response.result.unwrap_or(json!(null)))
    }
}
```

### Streamable HTTP (Spec 2025-03-26+)

The Streamable HTTP transport replaces the deprecated HTTP+SSE transport. A single HTTP
endpoint supports both client-to-server (POST) and server-to-client (GET for streaming):

```rust
/// MCP client over Streamable HTTP transport.
pub struct HttpMcpClient {
    /// HTTP client.
    client: reqwest::Client,
    /// Server endpoint URL.
    endpoint: Url,
    /// Session ID for multi-request sessions.
    session_id: Option<String>,
}

impl HttpMcpClient {
    pub async fn connect(endpoint: Url) -> Result<Self> {
        let client = reqwest::Client::new();
        let mut mcp = Self {
            client,
            endpoint,
            session_id: None,
        };

        // Initialize — POST to endpoint
        let init_response = mcp.call("initialize", json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "roko", "version": env!("CARGO_PKG_VERSION") },
        })).await?;

        // Extract session ID from response header
        mcp.session_id = init_response.headers
            .get("mcp-session-id")
            .map(|v| v.to_str().unwrap().to_string());

        Ok(mcp)
    }

    pub async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<reqwest::Response> {
        let mut request = self.client.post(self.endpoint.clone())
            .json(&json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
                "id": 1,
            }));

        if let Some(ref session_id) = self.session_id {
            request = request.header("mcp-session-id", session_id);
        }

        Ok(request.send().await?)
    }
}
```

### Transport Selection

```toml
# roko.toml — MCP server transport configuration
[[agent.mcp_servers]]
name = "github"
transport = "stdio"         # Default: stdio
command = "roko-mcp-github"
args = ["--repo", "nunchi/roko"]

[[agent.mcp_servers]]
name = "remote-tools"
transport = "http"          # Remote server
endpoint = "https://tools.example.com/mcp"
auth = { type = "bearer", token = "${MCP_AUTH_TOKEN}" }
```

| Transport | Latency | Use Case | Auth |
|---|---|---|---|
| stdio | ~1ms | Local tools, co-located processes | Process isolation |
| Streamable HTTP | ~10-50ms | Remote tools, cloud services | Bearer token, OAuth |

---

## Capabilities Negotiation

The initialization handshake ensures client and server agree on supported features:

```rust
/// Full initialization sequence.
pub async fn initialize(&mut self) -> Result<ServerCapabilities> {
    // Step 1: Send initialize request
    let result: InitializeResult = self.call("initialize", json!({
        "protocolVersion": "2025-11-25",
        "capabilities": {
            "sampling": {},      // We support sampling
            "roots": {
                "listChanged": true  // We notify on root changes
            }
        },
        "clientInfo": {
            "name": "roko",
            "version": env!("CARGO_PKG_VERSION"),
        }
    })).await?;

    // Step 2: Validate protocol version
    if result.protocol_version != "2025-11-25" {
        warn!(
            server_version = %result.protocol_version,
            "MCP server uses older protocol version, falling back"
        );
    }

    // Step 3: Send initialized notification (no response expected)
    self.notify("notifications/initialized", json!({})).await?;

    // Step 4: Gate feature usage by server capabilities
    if result.capabilities.tools.is_none() {
        return Err(anyhow!("MCP server does not expose tools"));
    }

    Ok(result.capabilities)
}
```

### Capability-Gated Feature Usage

The client only uses features the server has advertised:

```rust
pub struct McpServerSession {
    capabilities: ServerCapabilities,
    client: Box<dyn McpTransport>,
}

impl McpServerSession {
    pub async fn list_resources(&self) -> Result<Vec<McpResource>> {
        if self.capabilities.resources.is_none() {
            return Ok(vec![]); // Server doesn't support resources
        }
        let result = self.client.call("resources/list", json!({})).await?;
        Ok(serde_json::from_value(result)?)
    }

    pub async fn request_sampling(&self, request: SamplingRequest) -> Result<SamplingResponse> {
        // Sampling is a reverse capability — server calls client
        // This is handled by the client's incoming message handler
        unimplemented!("Sampling handled by message router")
    }
}
```

---

## MCP Tool Change Notifications

When an MCP server's tool set changes at runtime, it sends a `notifications/tools/list_changed`
notification. The client re-discovers tools and updates the merged registry:

```rust
/// Handle tool change notifications from MCP servers.
pub async fn handle_tool_change(
    &mut self,
    server_name: &str,
    registry: &mut MergedToolRegistry,
) -> Result<()> {
    // Re-discover tools
    let tools = self.client.call("tools/list", json!({})).await?;
    let mcp_tools: Vec<McpTool> = serde_json::from_value(tools)?;

    // Replace server's tools in the registry
    registry.remove_mcp_server(server_name);
    for mcp_tool in mcp_tools {
        let tool_def = convert_mcp_tool(&mcp_tool, server_name);
        registry.add_mcp_tool(tool_def);
    }

    info!(server = server_name, count = mcp_tools.len(), "MCP tools refreshed");
    Ok(())
}
```

---

## Test Criteria

- stdio transport correctly frames JSON-RPC messages with newline delimiters.
- Initialization handshake completes within 5 seconds or times out.
- Tool discovery returns all server tools with correct schemas.
- Tool call timeout is enforced (default 30s).
- Capabilities negotiation: client does not call `resources/list` when server lacks capability.
- Tool annotations map correctly: `readOnly: true` → `CapabilityTier::Read`.
- Server crash is detected and reported within 10 seconds.
- Tool change notification triggers re-discovery and registry update.
- Streamable HTTP transport correctly passes `mcp-session-id` header.

---

## Current Implementation Status

| Component | Status | Location |
|---|---|---|
| MCP client (stdio transport) | **Built** | `crates/roko-agent/src/mcp/` |
| MCP client (HTTP transport) | **Planned** | Spec above |
| Tool converter (MCP → ToolDef) | **Built** | `crates/roko-agent/src/mcp/` |
| Config passthrough | **Wired** | `roko.toml` → `--mcp-config` |
| Capabilities negotiation | **Partial** | `initialize` sent, capabilities not fully gated |
| Tool annotations | **Planned** | Spec above |
| Tool change notifications | **Planned** | Spec above |
| Sampling support | **Planned** | Spec above |
| roko-mcp-github | **Planned** | See `10-mcp-github.md` |
| roko-mcp-slack | **Planned** | See `11-mcp-slack.md` |
| roko-mcp-scripts | **Planned** | See `12-mcp-scripts.md` |
| roko-mcp-stdio | **Scaffold** | See `13-mcp-stdio.md` |

---

## References

- **MCP Specification 2025-11-25** (Anthropic / Linux Foundation) — Current protocol version.
  [spec](https://modelcontextprotocol.io/specification/2025-11-25)
- **MCP Specification 2025-03-26** — Introduced Streamable HTTP, tool annotations.
  [changelog](https://modelcontextprotocol.io/specification/2025-03-26)
- **MCP GitHub Repository** — Reference implementations and SDKs.
  [repo](https://github.com/modelcontextprotocol/modelcontextprotocol)
