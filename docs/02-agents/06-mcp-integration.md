# 06 — MCP Integration

> Sub-doc 06 of **02-agents** · Roko Documentation
>
> This document describes Roko's Model Context Protocol (MCP) integration:
> the JSON-RPC stdio client, tool conversion, multi-server dedup, config
> discovery, dynamic registry, and passthrough to Claude CLI.


> **Implementation**: Shipping

---

## What Is MCP

The Model Context Protocol (MCP) is a standard for connecting LLM agents
to external tools and data sources via JSON-RPC over stdio. An MCP server
exposes a set of tools (with JSON schema definitions), and an MCP client
discovers and invokes them at runtime. This allows dynamic tool registration
without recompiling the agent.

Roko's MCP integration lives at `crates/roko-agent/src/mcp/` and provides
five submodules:

```rust
pub mod client;           // JSON-RPC stdio transport
pub mod config;           // .mcp.json discovery and parsing
pub mod dedup;            // Multi-server tool deduplication
pub mod dynamic_registry; // Composes static + MCP tools
pub mod to_tool_def;      // MCP schema → roko_core::ToolDef conversion
```

---

## MCP Client

The `McpClient` struct manages the JSON-RPC connection to an MCP server:

```rust
pub struct McpClient {
    transport: Box<dyn Transport>,
    // ...
}
```

The `Transport` trait abstracts the communication channel:

```rust
pub trait Transport: Send + Sync {
    fn send(&mut self, request: McpRequest) -> Result<McpResponse>;
    fn receive(&mut self) -> Result<McpResponse>;
}
```

The primary transport is `StdioTransport`, which spawns the MCP server as
a child process and communicates via stdin/stdout JSON-RPC messages.

### MCP message types

```rust
pub struct McpRequest {
    pub jsonrpc: String,  // "2.0"
    pub id: u64,
    pub method: String,
    pub params: Option<Value>,
}

pub struct McpResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<Value>,
    pub error: Option<McpError>,
}
```

### Tool discovery

At startup, the client sends a `tools/list` request and receives the
server's tool catalog:

```rust
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,  // JSON Schema
}
```

### Tool invocation

When the agent requests a tool call, the client sends a `tools/call` request:

```json
{
    "jsonrpc": "2.0",
    "id": 42,
    "method": "tools/call",
    "params": {
        "name": "read_file",
        "arguments": { "path": "/src/main.rs" }
    }
}
```

And receives a result:

```rust
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    pub is_error: bool,
}
```

---

## Tool Conversion: `mcp_to_tool_def`

The `to_tool_def` module converts MCP tool definitions into Roko's canonical
`ToolDef` format:

```rust
pub fn mcp_to_tool_def(mcp_tool: &McpToolDef) -> ToolDef {
    ToolDef::new(
        &mcp_tool.name,
        &mcp_tool.description,
        ToolCategory::Custom,      // MCP tools are always custom
        ToolPermission::read_only(), // Conservative default
    )
    .with_schema(mcp_tool.input_schema.clone())
}
```

The conversion preserves the JSON schema from the MCP definition, which is
used by the `ToolDispatcher`'s validation step (step 1: validate args against
the registry's JSON schema).

### Permission assignment

MCP tools default to `read_only()` permissions. The rationale: external tools
registered via MCP are untrusted by default. The `SafetyLayer` enforces
this — even if an MCP tool tries to access the filesystem or network, the
path policy and network policy will block it unless the tool has been
explicitly granted higher permissions in the config.

---

## Config Discovery

The `config` module discovers MCP server configurations from `.mcp.json`
files:

```rust
pub struct McpConfig {
    pub servers: Vec<McpServerConfig>,
}

pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}
```

The `find_mcp_config` function searches for `.mcp.json` in:

1. The current working directory
2. The project root (from `roko.toml`)
3. The user's home directory (`~/.mcp.json`)

Example `.mcp.json`:

```json
{
    "servers": [
        {
            "name": "filesystem",
            "command": "mcp-server-filesystem",
            "args": ["--root", "/project"],
            "env": {}
        },
        {
            "name": "github",
            "command": "mcp-server-github",
            "args": [],
            "env": { "GITHUB_TOKEN": "ghp_..." }
        }
    ]
}
```

---

## Multi-Server Deduplication

When multiple MCP servers expose tools with the same name, the `dedup` module
resolves conflicts:

```rust
pub fn dedup_tools(all_tools: Vec<(String, McpToolDef)>) -> Vec<McpToolDef> {
    // server_name is used as a prefix when names collide:
    // "read_file" from two servers → "filesystem:read_file", "github:read_file"
}
```

The dedup strategy:
1. If a tool name is unique across all servers, keep it as-is.
2. If a tool name appears in multiple servers, prefix with the server name
   (e.g., `filesystem:read_file` vs `github:read_file`).
3. If a tool name collides with a built-in Roko tool, the built-in takes
   precedence and the MCP tool is prefixed.

---

## DynamicToolRegistry

The `DynamicToolRegistry` composes static built-in tools with dynamically
discovered MCP tools:

```rust
pub struct DynamicToolRegistry {
    static_tools: Vec<ToolDef>,
    mcp_tools: Vec<ToolDef>,
}
```

It implements the `ToolRegistry` trait from `roko-core`, so the
`ToolDispatcher` can use it transparently — it doesn't know whether a tool
came from the built-in catalog or from an MCP server.

```rust
impl ToolRegistry for DynamicToolRegistry {
    fn get(&self, name: &str) -> Option<&ToolDef> {
        self.static_tools.iter().find(|t| t.name == name)
            .or_else(|| self.mcp_tools.iter().find(|t| t.name == name))
    }

    fn all(&self) -> &[ToolDef] {
        // Returns static + MCP tools combined
    }
}
```

---

## Claude CLI Passthrough

For the `ClaudeCliAgent` backend, MCP configuration is passed through
directly as a CLI flag rather than going through Roko's MCP client. The
`claude` CLI has its own MCP client built in.

In `orchestrate.rs` at line 469:

```rust
if let Some(mcp_path) = &cfg.mcp_config {
    agent = agent.with_mcp_config(mcp_path);
}
```

This passes the `--mcp-config <path>` flag to the `claude` CLI subprocess.
The CLI reads the same `.mcp.json` format and manages MCP server lifecycles
internally.

The passthrough approach means:
- **Claude CLI agents** use Claude's built-in MCP client (battle-tested,
  high-performance).
- **HTTP-based agents** (OpenAI, Ollama, etc.) use Roko's MCP client via
  the `DynamicToolRegistry` + `ToolLoop`.

Both paths produce the same observable behavior: the agent can call tools
from MCP servers. The difference is in the plumbing.

### Configuration in roko.toml

The MCP config path is specified in `roko.toml`:

```toml
[agent]
mcp_config = ".mcp.json"
```

The auto-discovery fallback searches for `.mcp.json` if no explicit path
is configured. This means MCP "just works" for projects that have an
`.mcp.json` file in their root.

---

## MCP in the ToolLoop

For HTTP-based agents that go through the `ToolLoop`, MCP tools are
registered in the `DynamicToolRegistry` before the loop starts:

```
1. discover_mcp_servers()        → Vec<McpServerConfig>
2. connect_and_list_tools()      → Vec<(server_name, McpToolDef)>
3. dedup_tools()                 → Vec<McpToolDef>
4. mcp_to_tool_def()             → Vec<ToolDef>
5. DynamicToolRegistry::new()    → merges built-in + MCP tools
6. ToolLoop::new(translator, dispatcher, backend)
7. loop.run(system, user, all_tools, ctx)
```

The `ToolDispatcher` handles MCP tool calls through the `HandlerResolver`:

```rust
// The MCP handler wraps the McpClient for tool execution
struct McpHandler {
    client: Arc<Mutex<McpClient>>,
    tool_name: String,
}

impl ToolHandler for McpHandler {
    fn name(&self) -> &str { &self.tool_name }

    async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
        let response = self.client.lock().send_tool_call(&call)?;
        ToolResult::text(response.content)
    }
}
```

---

## Citations

1. `crates/roko-agent/src/mcp/` — Full MCP module: client, config, dedup,
   dynamic_registry, to_tool_def.
2. `crates/roko-cli/src/orchestrate.rs:469` — MCP passthrough to Claude CLI.
3. Implementation plan `01-agent-wiring.md` — Phase B item: MCP config
   passthrough.
4. `roko.toml` agent.mcp_config — Configuration field.
5. Refactoring PRD §10-developer-guide — Plugin system including MCP.
