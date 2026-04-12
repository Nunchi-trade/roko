# Adding Custom Tools via MCP

MCP servers provide tools that roko agents can call during execution.

This is the right path when you want to add new agent tools without changing Roko's built-in tool registry. Roko discovers external MCP servers, asks them for tool definitions, prefixes the tool names with the server name, and exposes them through the same tool-selection flow as built-ins.

Examples in this repo:

- `crates/roko-mcp-scripts`
- `crates/roko-mcp-github`
- `crates/roko-mcp-slack`
- `crates/roko-mcp-stdio`

## Critical rules

1. **Search before writing**: `grep -rn 'StructName\|TraitName' crates/ --include='*.rs' | grep -v target/`
2. **Wire existing code** — don't reimplement what exists.
3. **Only change what's needed** for this ONE task. Don't touch unrelated code.
4. **Run verification**: after your changes, run:
   - `cargo check --workspace` — MUST pass
   - `cargo test --workspace --no-run` — MUST compile
   - If the task spec has a Verification section, run those exact commands
5. **If something fails**, fix it before finishing. Don't leave broken code.

## Context files (read these first)

- `/Users/will/dev/nunchi/roko/roko-mr-stream-beta/tmp/implementation-plans/modelrouting/18-structural-cleanup.md`
- `/Users/will/dev/nunchi/roko/roko-mr-stream-beta/tmp/implementation-plans/modelrouting/01-architecture.md`

## What Roko expects from an MCP server

Roko's current MCP path is simple:

- it spawns the server as a child process over stdio
- it speaks newline-delimited JSON-RPC 2.0
- it calls exactly three methods: `initialize`, `tools/list`, and `tools/call`
- it converts each advertised tool into a `ToolDef` and prefixes the tool name as `server_name__tool_name`

Current implementation details that matter:

- `.mcp.json` discovery walks up from the working directory unless `agent.mcp_config` points at an explicit file
- Roko currently uses the `name`, `command`, and `args` fields from `.mcp.json`
- `.mcp.json` `env` is parsed in config but is not wired into process spawning yet, so custom servers should read credentials from the parent environment for now
- MCP tools are currently converted with read-only permissions by default in `crates/roko-agent/src/mcp/to_tool_def.rs`; if you need first-class write/exec/network semantics, extend that conversion path instead of assuming Roko will infer them

## Step 1: Implement a server

Any executable is fine if it speaks the expected JSON-RPC over stdio. Rust is the easiest path in this repo because `crates/roko-mcp-stdio` already handles the line-delimited transport.

Minimal Rust skeleton:

```rust
use roko_mcp_stdio::{JsonRpcError, JsonRpcRequest, serve_stdio};
use serde_json::{json, Value};
use std::io;

fn main() -> anyhow::Result<()> {
    serve_stdio(io::stdin().lock(), io::stdout().lock(), handle_request)?;
    Ok(())
}

fn handle_request(request: JsonRpcRequest) -> Result<Value, JsonRpcError> {
    match request.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "roko-mcp-acme",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        "tools/list" => Ok(json!({
            "tools": [{
                "name": "echo",
                "description": "Echo a string back to the caller.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" }
                    },
                    "required": ["text"],
                    "additionalProperties": false
                }
            }]
        })),
        "tools/call" => {
            let name = request.params["name"].as_str().unwrap_or_default();
            let args = &request.params["arguments"];
            match name {
                "echo" => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": json!({
                            "echoed": args["text"].as_str().unwrap_or_default()
                        }).to_string()
                    }],
                    "isError": false
                })),
                _ => Err(JsonRpcError::invalid_params(format!("unknown tool: {name}"))),
            }
        }
        _ => Err(JsonRpcError::method_not_found(&request.method)),
    }
}
```

If you want a fuller example, use `crates/roko-mcp-scripts/src/main.rs` as the template.

## Step 2: Advertise tools with `tools/list`

Each tool returned from `tools/list` should include:

- `name`
- `description` (optional but strongly recommended)
- `inputSchema` as JSON Schema

Example response:

```json
{
  "tools": [
    {
      "name": "search_docs",
      "description": "Search internal docs and return the best matches.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "query": { "type": "string" }
        },
        "required": ["query"],
        "additionalProperties": false
      }
    }
  ]
}
```

Roko converts that into a canonical tool definition and exposes it internally as `acme__search_docs` if the server name in config is `acme`.

## Step 3: Handle calls with `tools/call`

Roko sends tool calls in this shape:

```json
{
  "name": "search_docs",
  "arguments": {
    "query": "routing architecture"
  }
}
```

Return a normal MCP tool result:

```json
{
  "content": [
    {
      "type": "text",
      "text": "{\"matches\":[...]}"
    }
  ],
  "isError": false
}
```

Practical guidance:

- return machine-readable JSON inside the text block when the result is structured
- keep failures explicit with `isError: true`
- respond quickly to `initialize`; the current setup path skips servers whose handshake times out after 5 seconds

## Step 4: Register the server in `.mcp.json`

Project-local discovery uses a `.mcp.json` file with this shape:

```json
{
  "servers": [
    {
      "name": "acme",
      "command": "cargo",
      "args": ["run", "-p", "roko-mcp-acme", "--"]
    }
  ]
}
```

Important:

- `name` becomes the tool prefix
- `command` must be executable from the working directory
- `args` are passed through exactly
- if you need secrets, export them in the shell that launches `roko`; do not rely on `.mcp.json` `env` yet

If you do not want walk-up discovery, point Roko at the file explicitly:

```toml
[agent]
mcp_config = "/absolute/path/to/.mcp.json"
```

## Step 5: Use the tools from agents, tasks, or templates

Once the server is configured, Roko will discover its tools during setup and merge them into the runtime registry.

Two naming rules matter:

- the MCP server is selected by server name, for example `acme`
- the individual tool is referred to by prefixed tool name, for example `acme__search_docs`

Per-task example:

```toml
[[tasks]]
id = "research-routing"
prompt = "Inspect the routing docs and summarize MCP integration points."
mcp_servers = ["acme"]
allowed_tools = ["read_file", "grep", "acme__search_docs"]
```

Template example:

```toml
name = "doc-research"
role = "researcher"
mcp_servers = ["acme"]
allowed_tools = ["acme__search_docs", "read_file", "grep"]
```

If you omit `mcp_servers` in a task-level workflow, the broader run may activate every configured MCP server. Use explicit server lists when you want tighter scoping.

## Step 6: Smoke-test the integration

Recommended sequence:

1. Start from the project root that contains `.mcp.json`
2. Run a simple prompt that encourages use of the tool
3. Check stderr/log output for MCP discovery and handshake failures

Example:

```bash
roko run --model glm-5-1 "Use acme__echo to echo the word ok."
```

If the server fails to load, check:

- the binary or command really exists from the current working directory
- `initialize` returns valid JSON-RPC
- `tools/list` returns a `tools` array
- the tool name in `allowed_tools` matches the prefixed form
- required secrets are exported in the parent shell

## When you need code changes in Roko instead

Use plain MCP when you only need to add tools. Change Roko itself when you need one of these:

- non-default permission mapping for MCP tools
- custom server spawn behavior beyond `command` plus `args`
- extra MCP capabilities beyond `initialize`, `tools/list`, and `tools/call`
- a first-class built-in tool rather than an external process

## When done

1. State what files you changed and why (brief)
2. Show the output of `cargo check --workspace`
3. If applicable, show test output
