#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <path-to-MORI-PARITY-CHECKLIST.md>" >&2
  exit 1
fi

python3 - "$1" <<'PY'
import sys
from pathlib import Path

source = Path(sys.argv[1])
text = source.read_text()

replacements = [
    ("apps/mori/src/agent/connection/claude.rs:120-125", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:135-160", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:165", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:170-180", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:185-210", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:215-225", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:230-245", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:250", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:255", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:300-340", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:350-370", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:50-90", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs:96-112", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/orchestrator/platform/github.rs:148", "(no current equivalent)"),
    ("apps/mori/src/support_enrich/mod.rs:160-176", "crates/roko-compose/src/enrichment/step.rs"),
    ("apps/mori/src/agent/connection.rs", "crates/roko-agent/src/dispatcher/mod.rs + crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/claude.rs", "crates/roko-agent/src/claude_cli_agent.rs"),
    ("apps/mori/src/agent/connection/codex.rs", "crates/roko-agent/src/codex_agent.rs"),
    ("apps/mori/src/agent/connection/common.rs", "crates/roko-agent/src/process/mod.rs + crates/roko-agent/src/process/group.rs"),
    ("apps/mori/src/agent/connection/cursor.rs", "crates/roko-agent/src/cursor_agent.rs"),
    ("apps/mori/src/app/headless_driver.rs", "crates/roko-cli/src/daemon.rs"),
    ("apps/mori/src/app/parallel.rs", "crates/roko-orchestrator/src/executor/mod.rs"),
    ("apps/mori/src/conductor/diagnosis.rs", "crates/roko-conductor/src/diagnosis.rs"),
    ("apps/mori/src/conductor/heuristics.rs", "crates/roko-conductor/src/watchers/ghost_turn.rs"),
    ("apps/mori/src/conductor/stuck_detection.rs", "crates/roko-conductor/src/watchers/stuck_pattern.rs"),
    ("apps/mori/src/enrichment/steps.rs", "crates/roko-compose/src/enrichment/step.rs"),
    ("apps/mori/src/orchestrator/plan.rs", "crates/roko-orchestrator/src/plan_discovery.rs + crates/roko-cli/src/orchestrate.rs"),
    ("apps/mori/src/orchestrator/prompts/assembly.rs", "crates/roko-compose/src/prompt.rs"),
    ("apps/mori/src/orchestrator/prompts/common.rs", "crates/roko-compose/src/templates/common.rs"),
    ("apps/mori/src/orchestrator/prompts/implementer.rs", "crates/roko-compose/src/templates/implementer.rs"),
    ("apps/mori/src/orchestrator/prompts/integration.rs", "crates/roko-compose/src/templates/integration.rs"),
    ("apps/mori/src/orchestrator/prompts/quick.rs", "crates/roko-compose/src/templates/quick.rs"),
    ("apps/mori/src/orchestrator/prompts/reviewer.rs", "crates/roko-compose/src/templates/reviewer.rs"),
    ("apps/mori/src/orchestrator/prompts/scribe.rs", "crates/roko-compose/src/templates/scribe.rs"),
    ("apps/mori/src/orchestrator/prompts/strategist.rs", "crates/roko-compose/src/templates/strategist.rs"),
    ("apps/mori/src/orchestrator/prompts/task_impl.rs", "crates/roko-compose/src/templates/task_impl.rs"),
    ("apps/mori/src/orchestrator/tasks/", "crates/roko-orchestrator/src/dag.rs"),
    ("apps/mori/src/server/", "crates/roko-serve/src/routes/ + crates/roko-agent-server/src/"),
    ("apps/mori/src/support_enrich/mod.rs", "crates/roko-compose/src/enrichment/step.rs"),
]

for old, new in replacements:
    text = text.replace(old, new)

sys.stdout.write(text)
PY
