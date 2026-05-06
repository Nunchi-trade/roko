#!/usr/bin/env bash
# Merge a completed task's worktree back to the working branch.
#
# Usage:
#   ./scripts/merge.sh 003
#   ./scripts/merge.sh 003 --dry-run   # show what would be merged

set -euo pipefail
cd "$(dirname "$0")/.."

TASK_ID="${1:?Usage: merge.sh <task-id> [--dry-run]}"
DRY_RUN="${2:-}"
PROJECT_ROOT="$(cd ../.. && pwd)"

# Get worktree info from STATUS
BRANCH=$(grep -A5 "^\[tasks\.${TASK_ID}\]" STATUS.toml | grep "^worktree " | cut -d'"' -f2)
WORKTREE_PATH=$(grep -A5 "^\[tasks\.${TASK_ID}\]" STATUS.toml | grep "worktree_path" | cut -d'"' -f2)

if [[ -z "$BRANCH" ]]; then
    echo "ERROR: No worktree info for task ${TASK_ID}. Was it claimed?"
    exit 1
fi

cd "$PROJECT_ROOT"

# Check status
STATUS=$(grep -A2 "^\[tasks\.${TASK_ID}\]" STATUS.toml | grep "^status" | cut -d'"' -f2)
if [[ "$STATUS" != "verified" && "$STATUS" != "done" && "$STATUS" != "wired" && "$STATUS" != "tested" ]]; then
    echo "WARNING: Task ${TASK_ID} status is '${STATUS}' (expected: verified/done)"
    echo "Merging anyway, but you should verify first."
fi

echo "Merging branch: ${BRANCH}"
echo ""

if [[ "$DRY_RUN" == "--dry-run" ]]; then
    echo "--- Changes that would be merged ---"
    git diff HEAD..."${BRANCH}" --stat
    exit 0
fi

# Merge
git merge "${BRANCH}" --no-ff -m "task(${TASK_ID}): merge from ${BRANCH}" || {
    echo ""
    echo "ERROR: Merge conflict. Resolve manually, then run:"
    echo "  git merge --continue"
    exit 1
}

echo ""
echo "Task ${TASK_ID} merged successfully."
echo "Worktree can be cleaned up with: git worktree remove ${WORKTREE_PATH}"
