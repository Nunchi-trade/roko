# Demo Resources — Overview

Scripts, workflows, and setup guides for demoing roko + nunchi-dashboard.

## Directories

| Dir | What | For |
|-----|------|-----|
| `agent-matchmaking/` | Agent registration, matchmaking, job lifecycle | Dashboard `/coding` flow |
| `prd-workflow/` | Idea → Draft → Publish → Plan → Execute | Self-hosting loop |
| `research-workflow/` | Research dispatch, enhancement, analysis | Dashboard research panel |
| `agent-setup/` | Agent creation, configuration, tool profiles | Studio mode fleet |
| `full-self-hosting/` | End-to-end: PRD → research → plan → agents → gates → learn | The full loop |
| `dashboard-quickstart/` | Get dashboard + roko-serve running from scratch | First-time setup |

## Quick start (any demo)

```bash
# 1. Build roko
cd /path/to/roko && cargo build -p roko-cli

# 2. Init workspace + start serve
roko init && roko serve &

# 3. Seed agents (for matchmaking/studio demos)
bash demo-resources/agent-matchmaking/seed-agents.sh

# 4. Start dashboard
cd /path/to/nunchi-dashboard && npm run dev
```

## Key gaps to be aware of

1. **Agent creation is CLI-only** — dashboard can list/message agents but not create them
2. **Config editing is CLI-only** — dashboard shows roko.toml read-only (PUT /api/config exists but no UI)
3. **MCP config lives in roko.toml `[agent]` section** as `mcp_config` path, not a dedicated section
4. **Tool profiles** are configured in `[tools.profiles.<domain>]` in roko.toml
5. **`job list` shows `unknown` state** for jobs written by serve (field name mismatch: serve writes `state`, CLI reads `status`)
