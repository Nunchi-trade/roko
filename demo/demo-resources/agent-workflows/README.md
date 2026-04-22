# Agent Workflow Demo Scripts

Three scripts that exercise the agent infrastructure end-to-end.

## Prerequisites

1. **Build roko**: `cargo build -p roko-cli`
2. **Set `ANTHROPIC_API_KEY`** (for LLM-backed agents):
   ```bash
   export ANTHROPIC_API_KEY=sk-ant-...
   ```
3. **`roko.toml`** in the project root. Minimal config:
   ```toml
   [agent]
   default_model = "claude-sonnet-4-6"
   default_backend = "anthropic_api"
   ```
   If `ANTHROPIC_API_KEY` is set, `default_backend` auto-detects to `anthropic_api`.

## Scripts

### `01-single-agent.sh`
Starts `roko serve` + one agent sidecar, sends a message through the serve proxy,
verifies the agent registered, and checks for event storm warnings.

```bash
bash tmp/demo-resources/agent-workflows/01-single-agent.sh
```

### `02-multi-agent.sh`
Starts `roko serve` + 3 agents, each auto-picking a free port. Verifies all three
register with serve and are discoverable via the API.

```bash
bash tmp/demo-resources/agent-workflows/02-multi-agent.sh
```

### `03-chat-repl.sh`
Starts a single agent sidecar (no `roko serve` needed) and launches the interactive
chat REPL pointing directly at it. Good for quick local development.

```bash
bash tmp/demo-resources/agent-workflows/03-chat-repl.sh [agent-id]
```

## Troubleshooting

| Problem | Fix |
|---|---|
| Agent not found after startup | Registration retries 3x with 2s gaps. Check serve is reachable. |
| Port conflict | `--bind` defaults to `127.0.0.1:0` (auto-pick). Pass `--bind 0.0.0.0:8081` for a fixed port. |
| Chat can't find agent | Chat probes: agents.json → serve registry → roko-serve proxy. Ensure at least one is running. |
| `claude_cli` backend fails | Set `ANTHROPIC_API_KEY` to auto-use `anthropic_api` backend instead. |
| Event storm / WS lag | Fixed in this branch. If still occurring, check `roko serve` logs. |
