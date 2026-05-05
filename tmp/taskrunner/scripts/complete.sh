#!/usr/bin/env bash
# Mark a task's status. Called by agent or manually.
#
# Usage:
#   ./scripts/complete.sh 003 implemented "Code compiles, tests written"
#   ./scripts/complete.sh 003 tested "All tests pass"
#   ./scripts/complete.sh 003 wired "Called from roko status command"
#   ./scripts/complete.sh 003 verified "roko status shows correct output"
#   ./scripts/complete.sh 003 done "Audit confirmed"

set -euo pipefail
cd "$(dirname "$0")/.."

TASK_ID="${1:?Usage: complete.sh <task-id> <status> [notes]}"
NEW_STATUS="${2:?Usage: complete.sh <task-id> <status> [notes]}"
NOTES="${3:-}"
TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

VALID_STATUSES="pending claimed implemented tested wired verified done"
if ! echo "$VALID_STATUSES" | grep -qw "$NEW_STATUS"; then
    echo "ERROR: Invalid status '${NEW_STATUS}'. Must be one of: ${VALID_STATUSES}"
    exit 1
fi

# Update status in STATUS.toml
# Use sed to update the status line after [tasks.TASK_ID]
if grep -q "^\[tasks\.${TASK_ID}\]" STATUS.toml; then
    # Task exists, update status
    sed -i '' "/^\[tasks\.${TASK_ID}\]/,/^\[/ {
        s/^status = .*/status = \"${NEW_STATUS}\"/
        s/^completed_at = .*/completed_at = \"${TIMESTAMP}\"/
        s/^notes = .*/notes = \"${NOTES}\"/
    }" STATUS.toml
else
    echo "ERROR: Task ${TASK_ID} not found in STATUS.toml. Was it claimed?"
    exit 1
fi

echo "Task ${TASK_ID} → ${NEW_STATUS}"
[[ -n "$NOTES" ]] && echo "  Notes: ${NOTES}"

# If done/verified, remind about merge
if [[ "$NEW_STATUS" == "verified" || "$NEW_STATUS" == "done" ]]; then
    echo ""
    echo "Ready to merge. Run: ./scripts/merge.sh ${TASK_ID}"
fi
