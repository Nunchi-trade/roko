# 09 — MCP Integration Architecture

> Model Context Protocol (MCP) — JSON-RPC stdio transport, tool converter,
> dynamic registry, and how MCP extends the Synapse tool system.

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

## Current Implementation Status

| Component | Status | Location |
|---|---|---|
| MCP client (stdio transport) | **Built** | `crates/roko-agent/src/mcp/` |
| Tool converter (MCP → ToolDef) | **Built** | `crates/roko-agent/src/mcp/` |
| Config passthrough | **Wired** | `roko.toml` → `--mcp-config` |
| roko-mcp-github | **Planned** | See `10-mcp-github.md` |
| roko-mcp-slack | **Planned** | See `11-mcp-slack.md` |
| roko-mcp-scripts | **Planned** | See `12-mcp-scripts.md` |
| roko-mcp-stdio | **Scaffold** | See `13-mcp-stdio.md` |
