# 12 — roko-mcp-scripts

> Config-driven tool wrappers: scripts.toml format, executor, discovery,
> collaboration scripts, knowledge-base scripts.


> **Implementation**: Scaffold

---

## Overview

`roko-mcp-scripts` is an MCP server that wraps arbitrary shell scripts and executables as
MCP tools. It provides **zero-code tool extension** — add a TOML configuration entry and a
script, and it becomes an agent-callable tool with JSON Schema validation, structured output
parsing, and timeout handling.

**Status:** Planned (spec complete, implementation pending)

**Crate:** `crates/roko-mcp-scripts/`

**Protocol:** MCP (JSON-RPC 2.0 over stdio)

**Agent templates using this:** pm-board-agent, enrich-agent, triage-agent, pm-health-agent,
action-tracker-agent, sync-agent, freshness-agent, digest-agent, conflict-detector-agent

---

## Architecture

```
roko-mcp-scripts
    ├── config.rs        # Parse scripts.toml
    ├── executor.rs      # Run scripts, capture output, enforce timeouts
    ├── discovery.rs     # Enumerate available scripts from config
    └── main.rs          # MCP server (stdio transport)
```

### How It Works

1. On startup, `roko-mcp-scripts` reads a `scripts.toml` configuration file
2. Each entry in the config becomes an MCP tool with:
   - A name (derived from the TOML key)
   - A description (from the TOML `description` field)
   - An input schema (from the TOML `params` field, mapped to JSON Schema)
   - A handler that invokes the configured script/command
3. Tool discovery (`tools/list`) returns all configured scripts
4. Tool execution (`tools/call`) runs the script with parameters as environment variables
   or command-line arguments

### Executor

```rust
pub struct ScriptExecutor {
    /// Working directory for script execution.
    working_dir: PathBuf,
    /// Default timeout for script execution.
    default_timeout: Duration,
    /// Environment variables passed to all scripts.
    base_env: HashMap<String, String>,
}

impl ScriptExecutor {
    pub async fn execute(
        &self,
        script: &ScriptConfig,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let mut cmd = tokio::process::Command::new(&script.command);
        cmd.current_dir(&self.working_dir);

        // Pass parameters as environment variables
        for (key, value) in params.as_object().unwrap_or(&serde_json::Map::new()) {
            cmd.env(format!("PARAM_{}", key.to_uppercase()), value.to_string());
        }

        // Or as command-line arguments (based on script config)
        if script.pass_as == "args" {
            for arg in &script.args {
                let resolved = resolve_param(arg, params)?;
                cmd.arg(resolved);
            }
        }

        // Set timeout
        let timeout = script.timeout.unwrap_or(self.default_timeout);
        let output = tokio::time::timeout(timeout, cmd.output()).await??;

        // Parse output
        if script.output_format == "json" {
            Ok(serde_json::from_slice(&output.stdout)?)
        } else {
            Ok(json!({ "stdout": String::from_utf8_lossy(&output.stdout) }))
        }
    }
}
```

---

## scripts.toml Format

```toml
# scripts.toml — Tool definitions for roko-mcp-scripts

[meta]
working_dir = "/path/to/repo"
default_timeout = "30s"

# Each [[tool]] entry becomes an MCP tool
[[tool]]
name = "pm_sync"
description = "Synchronize PM state between GitHub and TOML task files"
command = "node"
args = ["scripts/pm/sync.js", "--direction", "${direction}"]
timeout = "60s"
output_format = "json"

[tool.params]
direction = { type = "string", enum = ["push", "pull"], description = "Sync direction" }

[[tool]]
name = "pm_validate"
description = "Validate referential integrity of PM TOML task files"
command = "node"
args = ["scripts/pm/validate.js"]
output_format = "json"

[[tool]]
name = "pm_views"
description = "Regenerate PM dashboard views (board, health, people, timeline)"
command = "node"
args = ["scripts/pm/views.js", "--view", "${view}"]

[tool.params]
view = { type = "string", enum = ["all", "board", "health", "people", "timeline"], default = "all" }
```

### Config Schema

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | Yes | Tool name (becomes `scripts.<name>` in MCP) |
| `description` | string | Yes | LLM-facing description |
| `command` | string | Yes | Executable to run |
| `args` | array of string | No | Command-line arguments. `${param}` syntax resolves from input. |
| `timeout` | duration string | No | Max execution time (default from `[meta]`) |
| `output_format` | string | No | `"json"` or `"text"` (default: `"text"`) |
| `env` | table | No | Additional environment variables |
| `params` | table | No | Input parameters (mapped to JSON Schema) |
| `pass_as` | string | No | `"env"` (default) or `"args"` — how params are passed to script |

---

## Collaboration Repo Scripts

Scripts for the collaboration repository (`/Users/will/dev/nunchi/collaboration/`):

```toml
# collaboration/.roko/scripts.toml

[[tool]]
name = "doc_lifecycle_check"
description = "Check document lifecycle status (draft/review/canonical/archived)"
command = "node"
args = ["scripts/doc-lifecycle/check.js", "--path", "${path}"]
output_format = "json"

[tool.params]
path = { type = "string", description = "Path to document" }

[[tool]]
name = "generate_digest"
description = "Generate weekly digest of document changes across repos"
command = "node"
args = ["scripts/digest/generate.js", "--repos", "${repos}", "--since", "${since}"]
output_format = "json"

[tool.params]
repos = { type = "string", default = "collaboration,knowledge-base,roko" }
since = { type = "string", default = "7d", description = "Time period (e.g., '7d', '2w')" }

[[tool]]
name = "sync_repos"
description = "Synchronize documents between collaboration and knowledge-base repos"
command = "node"
args = ["scripts/sync/run.js"]
output_format = "json"

[[tool]]
name = "detect_conflicts"
description = "Detect conflicting claims across documents"
command = "node"
args = ["scripts/conflicts/detect.js", "--path", "${path}"]
output_format = "json"

[tool.params]
path = { type = "string", default = "docs/", description = "Directory to scan" }

[[tool]]
name = "check_freshness"
description = "Check document freshness and flag stale content"
command = "node"
args = ["scripts/freshness/check.js", "--max-age", "${max_age}"]
output_format = "json"

[tool.params]
max_age = { type = "string", default = "30d", description = "Maximum age before flagging" }

[[tool]]
name = "process_meeting"
description = "Extract action items and decisions from meeting notes"
command = "node"
args = ["scripts/meetings/process.js", "--path", "${path}"]
output_format = "json"

[tool.params]
path = { type = "string", description = "Path to meeting notes file" }
```

---

## Knowledge-Base Repo Scripts

Scripts for the knowledge-base repository:

```toml
# knowledge-base/.roko/scripts.toml

[[tool]]
name = "pm_sync"
description = "Synchronize PM state between GitHub and TOML task files"
command = "node"
args = ["scripts/pm/sync.js", "--direction", "${direction}"]
output_format = "json"

[tool.params]
direction = { type = "string", enum = ["push", "pull"] }

[[tool]]
name = "pm_validate"
description = "Validate referential integrity of PM TOML task files"
command = "node"
args = ["scripts/pm/validate.js"]
output_format = "json"

[[tool]]
name = "pm_views"
description = "Regenerate PM dashboard views"
command = "node"
args = ["scripts/pm/views.js", "--view", "${view}"]
output_format = "json"

[tool.params]
view = { type = "string", enum = ["all", "board", "health", "people", "timeline"], default = "all" }

[[tool]]
name = "extract_actions"
description = "Extract action items from recent documents"
command = "node"
args = ["scripts/actions/extract.js", "--since", "${since}"]
output_format = "json"

[tool.params]
since = { type = "string", default = "24h" }

[[tool]]
name = "deep_enrich"
description = "Enrich a document with cross-references, citations, and related links"
command = "node"
args = ["scripts/enrich/deep.js", "--path", "${path}", "--depth", "${depth}"]
output_format = "json"

[tool.params]
path = { type = "string", description = "Path to document" }
depth = { type = "string", enum = ["light", "deep"], default = "deep" }
```

---

## Security Considerations

### Script Execution Isolation

Scripts run as child processes with:
- **Working directory** constrained to the configured repo root
- **Timeout enforcement** via `tokio::time::timeout`
- **Environment isolation** — only explicitly declared env vars plus PARAM_* vars
- **No stdin** — scripts receive input via env vars or args, not stdin (preventing injection)

### Input Validation

Parameters are validated against the JSON Schema derived from `[tool.params]` before the
script is invoked. Invalid parameters are rejected with a structured error — the script
never sees them.

### Output Sanitization

Script output is parsed and sanitized:
- JSON output: validated as valid JSON
- Text output: wrapped in `{ "stdout": "..." }`
- stderr: captured but not returned to the LLM (logged for debugging)
- Exit codes: non-zero exit → error result with stderr content

---

## Discovery Mechanism

On `tools/list`, `roko-mcp-scripts` reads the configured `scripts.toml` and returns all
`[[tool]]` entries as MCP tools:

```rust
pub fn discover_tools(config: &ScriptsConfig) -> Vec<McpTool> {
    config.tools.iter().map(|tool| McpTool {
        name: tool.name.clone(),
        description: tool.description.clone(),
        input_schema: generate_schema(&tool.params),
    }).collect()
}
```

This means agents automatically see new tools when entries are added to `scripts.toml` — no
recompilation needed. The agent just needs to be restarted (or the MCP server reconnected) to
pick up new tools.
