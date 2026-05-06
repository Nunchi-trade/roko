#!/usr/bin/env bash
# Claim a task for an agent. Creates a worktree for isolation.
#
# Usage:
#   ./scripts/claim.sh 003 "claude-session-1"
#   ./scripts/claim.sh 003 "codex-run-5"

set -euo pipefail
cd "$(dirname "$0")/.."

TASK_ID="${1:?Usage: claim.sh <task-id> <agent-name>}"
AGENT="${2:?Usage: claim.sh <task-id> <agent-name>}"
PROJECT_ROOT="$(cd ../.. && pwd)"
TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
BRANCH="task/${TASK_ID}"

# Check task file exists
TASK_FILE="tasks/${TASK_ID}-"*.md
if ! ls $TASK_FILE 1>/dev/null 2>&1; then
    echo "ERROR: No task file found for ID ${TASK_ID}"
    exit 1
fi
TASK_FILE=$(ls $TASK_FILE | head -1)

# Check not already claimed
if grep -q "^\[tasks\.${TASK_ID}\]" STATUS.toml 2>/dev/null; then
    existing_status=$(grep -A1 "^\[tasks\.${TASK_ID}\]" STATUS.toml | grep "status" | cut -d'"' -f2)
    if [[ "$existing_status" != "pending" ]]; then
        echo "ERROR: Task ${TASK_ID} already has status: ${existing_status}"
        exit 1
    fi
fi

# Create worktree
echo "Creating worktree for task ${TASK_ID}..."
cd "$PROJECT_ROOT"
git worktree add -b "$BRANCH" ".claude/worktrees/${BRANCH}" HEAD 2>/dev/null || {
    echo "Worktree or branch already exists, reusing..."
    git worktree add ".claude/worktrees/${BRANCH}" "$BRANCH" 2>/dev/null || true
}
WORKTREE_PATH="${PROJECT_ROOT}/.claude/worktrees/${BRANCH}"

# Update STATUS.toml
cd "$(dirname "$0")/.."
cat >> STATUS.toml << EOF

[tasks.${TASK_ID}]
status = "claimed"
agent = "${AGENT}"
worktree = "${BRANCH}"
worktree_path = "${WORKTREE_PATH}"
claimed_at = "${TIMESTAMP}"
completed_at = ""
notes = ""
EOF

echo ""
echo "═══════════════════════════════════════════════"
echo "  Task ${TASK_ID} claimed by ${AGENT}"
echo "  Worktree: ${WORKTREE_PATH}"
echo "  Task file: ${TASK_FILE}"
echo "═══════════════════════════════════════════════"
echo ""
echo "Agent should work in: ${WORKTREE_PATH}"
echo "Task context is in:   $(pwd)/${TASK_FILE}"
