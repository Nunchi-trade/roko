# 02 — MCP as Connect Protocol

> MCP integration as dynamic Cell discovery via the Connect protocol. External processes
> expose tools as Cells, namespaced and merged into the runtime registry. Trust boundaries
> map to process isolation.

**Parent spec**: [14-TOOLS.md](../../unified/14-TOOLS.md), [11-CONNECTIVITY.md](../../unified/11-CONNECTIVITY.md)

---

## 1. Core Insight

The Model Context Protocol (MCP) is a JSON-RPC 2.0 standard for extending LLM agents with external
tools at runtime. In Roko's unified model, MCP is the **dynamic discovery mechanism for Connect
Cells**. Where built-in tools are statically compiled Cells known at compile time, MCP tools are
Cells discovered at runtime from external processes.

An MCP server is itself a **Connect Cell** — it implements the Connect protocol lifecycle
(`connect / query / execute / disconnect`) where:
- `connect` = spawn the server process and complete the initialization handshake
- `query` = call `tools/list` to discover available tools
- `execute` = call `tools/call` to invoke a specific tool
- `disconnect` = terminate the server process

Each tool exposed by an MCP server becomes a **child Connect Cell** with metadata derived from the
MCP tool schema. These child Cells are merged into the runtime registry alongside static Cells,
indistinguishable to the Route and Compose protocols that select them.

---

## 2. MCP Client as Connect Cell

The MCP client in `crates/roko-agent/src/mcp/` is a Connect Cell that manages the lifecycle of
external MCP server processes.

### Connect Protocol Mapping

```rust
/// MCP Client implements the Connect protocol.
/// Each configured MCP server is a managed connection.
impl ConnectProtocol for McpClient {
    /// Spawn the server process, perform JSON-RPC initialize handshake,
    /// negotiate capabilities, send initialized notification.
    async fn connect(&mut self, config: &McpServerConfig) -> Result<ConnectionHandle>;

    /// Call tools/list — discover available tool Cells from the server.
    /// Returns Cell metadata for each tool.
    async fn query(&self, handle: &ConnectionHandle) -> Result<Vec<CellMetadata>>;

    /// Call tools/call — execute a specific tool Cell.
    async fn execute(&self, handle: &ConnectionHandle,
                     tool_name: &str, params: Value) -> Result<Value>;

    /// Terminate the server process, clean up resources.
    async fn disconnect(&mut self, handle: ConnectionHandle) -> Result<()>;
}
```

### Initialization Handshake

The connect phase follows JSON-RPC 2.0 over stdio transport:

```
Client (roko-agent)              Server (e.g., roko-mcp-github)
       |                                    |
       | ── initialize ───────────────────→ |  (protocol version, capabilities)
       | ←── InitializeResult ──────────── |  (server capabilities, version)
       | ── notifications/initialized ───→ |  (client ready)
       |                                    |
       | ── tools/list ───────────────────→ |  (discover available Cells)
       | ←── { tools: [...] } ────────────  |  (Cell metadata array)
       |                                    |
```

---

## 3. Tool Schema to Cell Metadata Conversion

When an MCP server reports its tools via `tools/list`, each tool schema is converted to
Cell metadata and registered in the merged registry.

### Conversion Logic

```rust
/// Convert MCP tool schema to Cell metadata for the runtime registry.
pub fn mcp_tool_to_cell_metadata(tool: &McpTool, server_name: &str) -> CellMetadata {
    CellMetadata {
        // Namespace by server name to prevent collisions
        id: format!("{}.{}", server_name, tool.name),

        // MCP description becomes Cell description
        description: tool.description.clone(),

        // Input schema from MCP (JSON Schema draft 2020-12)
        input_schema: tool.input_schema.clone(),

        // Output schema not declared by MCP — inferred as Value
        output_schema: Schema::any(),

        // Capability tier from MCP annotations
        capability: match tool.annotations.as_ref() {
            Some(a) if a.read_only == Some(true) => CapabilityTier::Read,
            _ => CapabilityTier::Write,  // Conservative default
        },

        // Protocol conformance
        protocols: vec![Protocol::Connect],

        // Cost estimate from latency characteristics
        cost_estimate: CostEstimate {
            tick_budget: TickBudget::Medium,  // Cross-process = 10-50ms overhead
            token_cost: 0,  // No LLM cost
        },
    }
}
```

### Namespace Partitioning

MCP tool names are prefixed by server name to create Bus topic partitions:

| Server | Tool | Namespaced ID | Bus topic |
|---|---|---|---|
| github | get_pr | `github.get_pr` | `tool.github.get_pr.*` |
| github | create_issue | `github.create_issue` | `tool.github.create_issue.*` |
| slack | post_message | `slack.post_message` | `tool.slack.post_message.*` |
| slack | get_channel | `slack.get_channel` | `tool.slack.get_channel.*` |
| scripts | pm_sync | `scripts.pm_sync` | `tool.scripts.pm_sync.*` |

This namespace partitioning means:
- No collision between tools from different servers
- Bus subscriptions can filter by server (`tool.github.*`)
- Lens Cells can observe per-server health independently

---

## 4. Dynamic Registry Merge

The merged registry combines three sources of Connect Cells with clear precedence:

```rust
/// Three-layer tool registry with precedence ordering.
pub struct MergedToolRegistry {
    /// Layer 1: Static built-in Cells (highest trust, in-process)
    static_cells: &'static [ToolDef],

    /// Layer 2: Domain plugin Cells (in-process, compiled, reviewed)
    domain_cells: Vec<ToolDef>,

    /// Layer 3: MCP-discovered Cells (separate process, default Write trust)
    mcp_cells: Vec<CellMetadata>,
}

impl MergedToolRegistry {
    /// Lookup by name. Precedence: static > domain > MCP.
    pub fn get(&self, name: &str) -> Option<CellRef> {
        self.static_cells.iter().find(|t| t.name == name)
            .map(CellRef::Static)
            .or_else(|| self.domain_cells.iter().find(|t| t.name == name)
                .map(CellRef::Domain))
            .or_else(|| self.mcp_cells.iter().find(|t| t.id == name)
                .map(CellRef::Mcp))
    }

    /// All available Cells (union of all layers).
    pub fn all(&self) -> impl Iterator<Item = CellRef> {
        self.static_cells.iter().map(CellRef::Static)
            .chain(self.domain_cells.iter().map(CellRef::Domain))
            .chain(self.mcp_cells.iter().map(CellRef::Mcp))
    }
}
```

### Trust Hierarchy

| Source | Process Boundary | Default Trust | Safety Pipeline |
|---|---|---|---|
| Built-in (roko-std) | In-process | Per-tool declaration | Full (7 hooks for Write) |
| Domain plugin | In-process | Per-tool declaration | Full (7 hooks for Write) |
| MCP tools | Separate process | Write (conservative) | Full (7 hooks) + process isolation |

MCP tools get **default Write trust** regardless of what the server declares, because the server
runs in a separate process where Roko cannot verify its internal behavior. The `readOnly`
annotation can downgrade to Read trust, but upgrading beyond Write requires explicit configuration.

---

## 5. The Four MCP Servers

Roko ships four MCP server crates, each a Connect Cell proxy to an external service:

### `roko-mcp-github` (17 tools)

Proxies GitHub API operations. Used by PR review, plan generation, and triage agents.

| Tool | Capability | What It Does |
|---|---|---|
| `get_pr` | Read | Fetch PR details with optional diff |
| `create_pr` | Write | Create a pull request |
| `list_issues` | Read | Search/filter issues |
| `create_issue` | Write | Create an issue with labels |
| `get_file` | Read | Read file from repository |
| `create_review` | Write | Submit PR review (approve/request changes) |
| ... | ... | (17 total) |

### `roko-mcp-slack` (8 tools)

Proxies Slack API for notifications and event handling.

| Tool | Capability | What It Does |
|---|---|---|
| `post_message` | Write | Post message to channel (supports Block Kit) |
| `get_channel` | Read | Get channel info |
| `reply_thread` | Write | Reply in a thread |
| `list_channels` | Read | List accessible channels |
| ... | ... | (8 total) |

### `roko-mcp-scripts` (N tools, config-driven)

Wraps shell scripts and subprocess commands as MCP tools. The tool set is defined by
a configuration file — not compiled:

```toml
# .roko/scripts.toml
[[tools]]
name = "pm_sync"
description = "Synchronize PM board with GitHub"
command = "node"
args = ["scripts/pm-sync.js", "--direction", "{{direction}}"]
timeout_ms = 30000

[[tools]]
name = "generate_digest"
description = "Generate weekly digest of changes"
command = "node"
args = ["scripts/generate-digest.js"]
timeout_ms = 60000
```

Each entry becomes an MCP tool discovered via `tools/list`. This is a **Tier 3 extension**
(declarative tool) exposed via the MCP protocol.

### `roko-mcp-code` (code-intelligence)

Provides code-intelligence operations: AST parsing, symbol lookup, dependency graph queries.
Used by the `roko index` subsystem for structural understanding.

---

## 6. Tool Change Notifications

MCP supports dynamic tool set changes via `notifications/tools/list_changed`. This maps to
Bus Pulses that trigger registry refresh:

```rust
/// When an MCP server sends tools/list_changed notification:
/// 1. Publish a Pulse on Bus: "mcp.{server}.tools_changed"
/// 2. Trigger Engine fires a re-discovery Graph
/// 3. Graph calls tools/list on the server
/// 4. Registry merges new tool set (remove old, add new)

pub async fn handle_tools_changed(server_name: &str, registry: &mut MergedToolRegistry) {
    // Publish change notification as Pulse
    bus.publish(Pulse {
        topic: format!("mcp.{}.tools_changed", server_name),
        payload: json!({ "server": server_name }),
    });

    // Re-discover tools from the server
    let tools = client.call("tools/list", json!({})).await?;
    let cells: Vec<CellMetadata> = tools.iter()
        .map(|t| mcp_tool_to_cell_metadata(t, server_name))
        .collect();

    // Atomic replace in registry
    registry.replace_mcp_server(server_name, cells);
}
```

This enables **hot-reloading of tool Cells** without restarting the agent. A server can evolve
its tool set (add new tools, deprecate old ones) and the agent adapts in real time.

---

## 7. Transport as Connect Cell Abstraction

MCP supports two transports, each a different Connect Cell implementation:

### stdio Transport (Primary)

```rust
/// stdio: spawn child process, communicate via stdin/stdout JSON-RPC.
pub struct StdioTransport {
    child: tokio::process::Child,
    writer: BufWriter<ChildStdin>,
    reader: BufReader<ChildStdout>,
    pending: DashMap<u64, oneshot::Sender<Response>>,
}
```

Properties:
- ~1ms latency (no network)
- Process isolation (crash isolation)
- Language agnostic (any language can implement)
- Credentials via environment variables (never on the wire)

### Streamable HTTP Transport

```rust
/// HTTP: POST JSON-RPC to endpoint, session-aware.
pub struct HttpTransport {
    client: reqwest::Client,
    endpoint: Url,
    session_id: Option<String>,
}
```

Properties:
- ~10-50ms latency (network)
- Remote server support (cloud-hosted tools)
- Session management via `mcp-session-id` header
- Bearer token or OAuth authentication

### Transport Selection in Config

```toml
# roko.toml
[[agent.mcp_servers]]
name = "github"
transport = "stdio"         # Local: spawn process
command = "roko-mcp-github"
args = ["--repo", "nunchi/roko"]
env = { GITHUB_TOKEN = "${GITHUB_TOKEN}" }

[[agent.mcp_servers]]
name = "remote-analysis"
transport = "http"          # Remote: HTTP endpoint
endpoint = "https://tools.example.com/mcp"
auth = { type = "bearer", token = "${MCP_AUTH_TOKEN}" }
```

---

## 8. Sampling: Reverse Connect Protocol

MCP's sampling capability inverts the Connect direction — the server requests LLM inference from
the client. In unified terms, the server fires a Connect Cell on the client's inference gateway:

```
Normal:  Client ──execute──→ Server (client uses server's tools)
Reverse: Server ──sampling──→ Client (server uses client's LLM)
```

This enables server-side agent loops where the MCP server orchestrates multi-step reasoning
while the client provides the LLM capability. The sampling request flows through the same
inference gateway Pipeline (cache, routing, budgeting) as direct agent inference.

---

## 9. Credential Isolation

MCP credentials follow the Connect protocol's security model:

1. **Environment injection** — credentials passed via process environment, never over stdio
2. **Per-server scoping** — each server gets only its own credentials
3. **Resolution from host** — `${GITHUB_TOKEN}` resolved from host env at spawn time
4. **No cross-contamination** — server A cannot access server B's credentials

```rust
/// Spawn MCP server with scoped credentials.
pub async fn spawn_server(config: &McpServerConfig) -> Result<StdioTransport> {
    let mut cmd = Command::new(&config.command);
    cmd.args(&config.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());

    // Only inject THIS server's declared env vars
    for (key, template) in &config.env {
        let value = resolve_env_template(template)?;
        cmd.env(key, value);
    }

    let child = cmd.spawn()?;
    // ... initialize transport
}
```

---

## What This Enables

1. **Dynamic extensibility** — new tool Cells can be added at runtime without recompiling the agent,
   simply by starting an MCP server.
2. **Language freedom** — MCP servers can be written in TypeScript, Python, Go, or any language.
   The JSON-RPC protocol is the Cell boundary.
3. **Crash isolation** — an MCP server crash does not bring down the agent. The Connect Cell
   detects disconnection and can attempt reconnection or graceful degradation.
4. **Uniform composition** — MCP tools are indistinguishable from built-in tools in Graph
   definitions. The Route Cell selects among all available tools regardless of source.
5. **Hot evolution** — `tools/list_changed` enables runtime tool set evolution without
   agent restart.

---

## Feedback Loops

- **Server health monitoring**: connect/disconnect patterns feed a health Lens Cell. Servers with
  frequent crashes get flagged and optionally auto-disabled after threshold.
- **Tool selection learning**: MCP tool usage patterns feed the CascadeRouter (Route Cell) —
  frequently-used MCP tools get higher selection priority.
- **Latency calibration**: actual execution time vs TickBudget estimate feeds cost model
  adjustment. HTTP transport tools consistently learn their network overhead.
- **Registry refresh rate**: `tools/list_changed` frequency per server feeds adaptive polling
  (for servers that don't support notifications, the client can poll — less frequently for
  stable servers).

---

## Open Questions

1. **MCP Resources and Prompts** — MCP exposes `resources/list` (data injection) and
   `prompts/list` (template discovery). Should these map to Store protocol (resources) and
   Compose protocol (prompts) respectively?
2. **Trust upgrade path** — can an MCP tool earn Read trust over time via behavioral
   observation (100 successful read-only calls, no side effects detected)?
3. **Multi-session MCP** — should a single MCP server process serve multiple agents, or
   should each agent have its own server instance? (Cost vs isolation tradeoff.)
4. **MCP tool versioning** — MCP spec does not include tool versioning. Should Roko's registry
   track schema hashes to detect breaking changes?

---

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| MCP client (stdio transport) | `crates/roko-agent/src/mcp/` | Shipped |
| Tool converter (MCP schema to CellMetadata) | `crates/roko-agent/src/mcp/` | Shipped |
| Config passthrough (roko.toml to --mcp-config) | `crates/roko-cli/src/lib.rs` | Shipped |
| roko-mcp-github (17 tools) | `crates/roko-mcp-github/` | Planned |
| roko-mcp-slack (8 tools) | `crates/roko-mcp-slack/` | Planned |
| roko-mcp-scripts (config-driven) | `crates/roko-mcp-scripts/` | Planned |
| roko-mcp-code (code-intelligence) | `crates/roko-mcp-code/` | Shipped |
| Streamable HTTP transport | `crates/roko-agent/src/mcp/` | Planned |
| Tool change notification handling | `crates/roko-agent/src/mcp/` | Planned |
| Sampling support (reverse Connect) | `crates/roko-agent/src/mcp/` | Planned |
| Merged registry (3-layer with precedence) | `crates/roko-agent/src/mcp/` | Shipped |
