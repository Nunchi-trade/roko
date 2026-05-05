#!/usr/bin/env bash
# Spawn audit agents to verify wiring for completed tasks in a wave.
#
# An audit agent is a FRESH agent (no implementation context) that:
# 1. Reads the task's wire target
# 2. Runs the verification commands
# 3. Confirms the code is actually called from runtime
# 4. Marks the task as verified (or flags issues)
#
# Usage:
#   ./scripts/audit.sh wave-1           # audit all implemented tasks in wave-1
#   ./scripts/audit.sh 003              # audit specific task

set -euo pipefail
cd "$(dirname "$0")/.."

TARGET="${1:?Usage: audit.sh <wave-name|task-id>}"
TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

echo "═══════════════════════════════════════════════"
echo "  AUDIT: ${TARGET}"
echo "  Time: ${TIMESTAMP}"
echo "═══════════════════════════════════════════════"
echo ""

# If it's a task ID (numeric), audit just that task
if [[ "$TARGET" =~ ^[0-9]+$ ]]; then
    TASK_FILE=$(ls tasks/${TARGET}-*.md 2>/dev/null | head -1)
    if [[ -z "$TASK_FILE" ]]; then
        echo "ERROR: No task file for ID ${TARGET}"
        exit 1
    fi

    echo "Audit task ${TARGET}:"
    echo "  Task file: ${TASK_FILE}"
    echo ""
    echo "To audit, a fresh agent should:"
    echo "  1. Read ${TASK_FILE} (only the Wire Target and Verification sections)"
    echo "  2. Run the verification commands"
    echo "  3. Confirm output matches expected"
    echo "  4. Run: grep -rn '<key function>' crates/ --include='*.rs' | grep -v test"
    echo "     to confirm code is called from non-test paths"
    echo "  5. Mark: ./scripts/complete.sh ${TARGET} verified '<notes>'"
    exit 0
fi

# Otherwise, audit all tasks in the wave that are implemented/tested/wired
echo "Tasks to audit in ${TARGET}:"
echo ""

# Find tasks in this wave that need auditing
while IFS= read -r line; do
    if [[ "$line" =~ ^\[tasks\.([0-9]+)\]$ ]]; then
        task_id="${BASH_REMATCH[1]}"
    elif [[ -n "${task_id:-}" && "$line" =~ ^wave\ =\ \"(.+)\"$ ]]; then
        task_wave="${BASH_REMATCH[1]}"
        if [[ "$task_wave" == "$TARGET" ]]; then
            # Check status
            status=$(grep -A2 "^\[tasks\.${task_id}\]" STATUS.toml 2>/dev/null | grep "^status" | cut -d'"' -f2)
            if [[ "$status" == "implemented" || "$status" == "tested" || "$status" == "wired" ]]; then
                task_file=$(ls tasks/${task_id}-*.md 2>/dev/null | head -1)
                echo "  ${task_id}: ${status} → needs audit (${task_file})"
            fi
        fi
        task_id=""
    fi
done < dag.toml

echo ""
echo "Run individual audits with: ./scripts/audit.sh <task-id>"
