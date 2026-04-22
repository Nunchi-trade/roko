# Agent Setup & Configuration

How to create, configure, and manage agents. Currently CLI-only (no dashboard UI for creation).

## Create an agent

```bash
# Minimal (general domain)
roko agent create --name my-agent --domain general

# Coding agent with custom prompt
roko agent create --name rust-dev --domain coding --prompt "You are a Rust systems programmer"

# Research agent
roko agent create --name researcher --domain research
```

This creates `.roko/agents/<name>/manifest.toml`.

## Agent domains

| Domain | What it adds | Use case |
|--------|-------------|----------|
| `general` | Default tool set | General tasks |
| `coding` | Workspace awareness, language hints | Implementation |
| `research` | Citation retrieval, deep-research | Analysis |
| `chain` | Network config, custody mode | Blockchain ops |

## Configure tools in roko.toml

```toml
[tools]
# Global allow/deny (affects all agents)
prefer_mcp = false
global_denied = ["bash"]  # e.g. block shell access

# Domain-specific tool profiles
[tools.profiles.coding]
extra_tools = ["run_tests", "apply_patch"]
excluded_tools = ["web_search"]

[tools.profiles.research]
extra_tools = ["web_search", "web_fetch"]
excluded_tools = ["write_file", "bash"]
```

## Configure MCP servers

MCP config is passed via the agent section:

```toml
[agent]
command = "claude"
# Path to MCP server config JSON (same format as Claude CLI)
mcp_config = ".roko/mcp-servers.json"
```

Example `.roko/mcp-servers.json`:
```json
{
  "mcpServers": {
    "code-intelligence": {
      "command": "roko-mcp-code",
      "args": ["--project-root", "."]
    },
    "github": {
      "command": "roko-mcp-github",
      "env": { "GITHUB_TOKEN": "" }
    }
  }
}
```

## Per-role model routing

```toml
[agent.roles.implementer]
model = "claude-sonnet-4-6"
effort = "high"
tools = ["read_file", "write_file", "edit_file", "bash", "glob", "grep"]

[agent.roles.reviewer]
model = "claude-opus-4-6"
effort = "max"
tools = ["read_file", "glob", "grep"]

[agent.roles.researcher]
model = "claude-sonnet-4-6"
backend = "perplexity"
tools = ["web_search", "web_fetch"]
```

## Register agents with roko-serve (for matchmaking)

After creating an agent, register it with serve so the dashboard and matchmaking can see it:

```bash
curl -X POST http://localhost:6677/api/agents/register \
  -H 'Content-Type: application/json' \
  -d '{
    "agent_id": "rust-dev",
    "label": "Rust Developer",
    "capabilities": ["messaging", "tasks"],
    "skills": ["rust", "systems", "networking"],
    "tier": "Expert",
    "reputation": 90,
    "past_jobs_completed": 25,
    "max_concurrent_jobs": 3
  }'
```

Or use `seed-agents.sh` from the `agent-matchmaking/` demo.

## Start an agent sidecar

```bash
# Start the per-agent HTTP sidecar (enables dashboard Studio features)
roko agent serve --agent-id rust-dev --bind 127.0.0.1:9001
```

This provides `/health`, `/stats`, `/logs`, `/tasks`, `/message` endpoints that the
dashboard's Studio mode polls.

## Agent lifecycle

```bash
roko agent list              # List all agents with status
roko agent status --name X   # Detailed health
roko agent start --name X    # Start agent process
roko agent stop --name X     # Stop agent
roko agent delete --name X   # Remove agent and state
roko agent chat --agent X    # Interactive REPL
```

## What the dashboard shows

- **Network → Agents** — Fleet roster (all registered agents)
- **Studio → Overview** — Selected agent identity, metrics, reputation
- **Studio → Live** — Real-time health from sidecar `/health`
- **Studio → Logs** — Streaming logs from sidecar `/logs`
- **Studio → Chat** — Send messages via `/agents/{id}/message`
