#!/usr/bin/env bash
# Spawn an agent on a task. Claims the task, creates worktree,
# prints the full agent prompt with all context.
#
# Usage:
#   ./scripts/spawn.sh 003 "claude-1"
#   ./scripts/spawn.sh 003 "codex-1" --print-only   # don't claim, just show prompt

set -euo pipefail
cd "$(dirname "$0")/.."

TASK_ID="${1:?Usage: spawn.sh <task-id> <agent-name> [--print-only]}"
AGENT="${2:?Usage: spawn.sh <task-id> <agent-name> [--print-only]}"
PRINT_ONLY="${3:-}"

TASK_FILE=$(ls tasks/${TASK_ID}-*.md 2>/dev/null | head -1)
if [[ -z "$TASK_FILE" ]]; then
    echo "ERROR: No task file for ID ${TASK_ID}"
    exit 1
fi

# Claim (unless print-only)
if [[ "$PRINT_ONLY" != "--print-only" ]]; then
    ./scripts/claim.sh "$TASK_ID" "$AGENT"
fi

# Read task file
TASK_CONTENT=$(cat "$TASK_FILE")

# Read agent preamble
PREAMBLE=""
if [[ -f templates/agent-prompt.md ]]; then
    PREAMBLE=$(cat templates/agent-prompt.md)
fi

# Get worktree path from STATUS.toml
WORKTREE_PATH=$(grep -A5 "^\[tasks\.${TASK_ID}\]" STATUS.toml | grep "worktree_path" | cut -d'"' -f2)

# Build full prompt
cat << PROMPT
════════════════════════════════════════════════════════════════
AGENT TASK: ${TASK_ID}
WORKTREE: ${WORKTREE_PATH:-"(not created)"}
════════════════════════════════════════════════════════════════

${PREAMBLE}

---

${TASK_CONTENT}

---

IMPORTANT REMINDERS:
1. Work ONLY in your worktree: ${WORKTREE_PATH:-"(see claim output)"}
2. Do NOT touch files outside the "touches" list in the task.
3. When done, run the verification commands in the task.
4. Report your status using:
   cd /Users/will/dev/nunchi/roko/roko/tmp/taskrunner
   ./scripts/complete.sh ${TASK_ID} <status> "<notes>"

Status progression: implemented → tested → wired → verified
PROMPT
