#!/usr/bin/env bash
# Find next claimable task(s) in the current wave.
#
# Usage:
#   ./scripts/next.sh              # show next available task
#   ./scripts/next.sh --count 5    # show up to 5 available tasks
#   ./scripts/next.sh --wave wave-1 # show tasks in specific wave

set -euo pipefail
cd "$(dirname "$0")/.."

COUNT=1
WAVE=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --count) COUNT="$2"; shift 2 ;;
        --wave)  WAVE="$2"; shift 2 ;;
        *)       echo "Unknown arg: $1"; exit 1 ;;
    esac
done

# Parse STATUS.toml for claimed/done tasks
declare -A TASK_STATUS
while IFS= read -r line; do
    if [[ "$line" =~ ^\[tasks\.([0-9]+)\]$ ]]; then
        current_id="${BASH_REMATCH[1]}"
    elif [[ -n "${current_id:-}" && "$line" =~ ^status\ =\ \"(.+)\"$ ]]; then
        TASK_STATUS[$current_id]="${BASH_REMATCH[1]}"
        current_id=""
    fi
done < STATUS.toml

# Parse dag.toml for tasks and dependencies
found=0
while IFS= read -r line; do
    if [[ "$line" =~ ^\[tasks\.([0-9]+)\]$ ]]; then
        task_id="${BASH_REMATCH[1]}"
        task_title=""
        task_wave=""
        task_blocked_by=""
    elif [[ -n "${task_id:-}" ]]; then
        if [[ "$line" =~ ^title\ =\ \"(.+)\"$ ]]; then
            task_title="${BASH_REMATCH[1]}"
        elif [[ "$line" =~ ^wave\ =\ \"(.+)\"$ ]]; then
            task_wave="${BASH_REMATCH[1]}"
        elif [[ "$line" =~ ^blocked_by\ =\ \[(.*)$ ]]; then
            task_blocked_by="${BASH_REMATCH[1]}"
        elif [[ "$line" =~ ^\[tasks\. || "$line" =~ ^$ ]]; then
            # End of this task block — check if claimable
            status="${TASK_STATUS[$task_id]:-pending}"

            if [[ "$status" == "pending" ]]; then
                # Check wave filter
                if [[ -n "$WAVE" && "$task_wave" != "$WAVE" ]]; then
                    task_id=""
                    continue
                fi

                # Check dependencies resolved
                blocked=false
                if [[ -n "$task_blocked_by" ]]; then
                    # Extract dependency IDs (crude but works)
                    for dep in $(echo "$task_blocked_by" | tr -d '[]",' | tr ' ' '\n'); do
                        dep_status="${TASK_STATUS[$dep]:-pending}"
                        if [[ "$dep_status" != "done" && "$dep_status" != "verified" ]]; then
                            blocked=true
                            break
                        fi
                    done
                fi

                if [[ "$blocked" == "false" ]]; then
                    echo "$task_id  $task_wave  $task_title"
                    found=$((found + 1))
                    if [[ $found -ge $COUNT ]]; then
                        exit 0
                    fi
                fi
            fi
            task_id=""
        fi
    fi
done < dag.toml

if [[ $found -eq 0 ]]; then
    echo "No claimable tasks found."
    exit 1
fi
