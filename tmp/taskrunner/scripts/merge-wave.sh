#!/usr/bin/env bash
# Merge all completed tasks in a wave, then run the gate.
#
# Usage:
#   ./scripts/merge-wave.sh wave-1
#   ./scripts/merge-wave.sh wave-1 --dry-run

set -euo pipefail
cd "$(dirname "$0")/.."

WAVE="${1:?Usage: merge-wave.sh <wave-name> [--dry-run]}"
DRY_RUN="${2:-}"
PROJECT_ROOT="$(cd ../.. && pwd)"

echo "═══════════════════════════════════════════════"
echo "  MERGE WAVE: ${WAVE}"
echo "═══════════════════════════════════════════════"
echo ""

# Find all tasks in this wave that are verified/done and have worktrees
MERGEABLE=()
while IFS= read -r line; do
    if [[ "$line" =~ ^\[tasks\.([0-9]+)\]$ ]]; then
        task_id="${BASH_REMATCH[1]}"
    elif [[ -n "${task_id:-}" && "$line" =~ ^wave\ =\ \"(.+)\"$ ]]; then
        task_wave="${BASH_REMATCH[1]}"
        if [[ "$task_wave" == "$WAVE" ]]; then
            status=$(grep -A2 "^\[tasks\.${task_id}\]" STATUS.toml 2>/dev/null | grep "^status" | cut -d'"' -f2)
            if [[ "$status" == "verified" || "$status" == "done" || "$status" == "wired" || "$status" == "tested" || "$status" == "implemented" ]]; then
                MERGEABLE+=("$task_id")
            fi
        fi
        task_id=""
    fi
done < dag.toml

if [[ ${#MERGEABLE[@]} -eq 0 ]]; then
    echo "No mergeable tasks in ${WAVE}."
    exit 0
fi

echo "Tasks to merge: ${MERGEABLE[*]}"
echo ""

FAILED=()
for task_id in "${MERGEABLE[@]}"; do
    echo "--- Merging task ${task_id} ---"
    if [[ "$DRY_RUN" == "--dry-run" ]]; then
        ./scripts/merge.sh "$task_id" --dry-run 2>/dev/null || true
    else
        if ! ./scripts/merge.sh "$task_id" 2>/dev/null; then
            FAILED+=("$task_id")
            echo "  FAILED — resolve conflicts manually"
        fi
    fi
    echo ""
done

if [[ ${#FAILED[@]} -gt 0 ]]; then
    echo "═══════════════════════════════════════════════"
    echo "  MERGE INCOMPLETE — ${#FAILED[@]} conflicts:"
    echo "  ${FAILED[*]}"
    echo "  Resolve manually, then run: ./scripts/gate.sh ${WAVE}"
    echo "═══════════════════════════════════════════════"
    exit 1
fi

if [[ "$DRY_RUN" != "--dry-run" ]]; then
    echo "All tasks merged. Running gate..."
    echo ""
    ./scripts/gate.sh "$WAVE"
fi
