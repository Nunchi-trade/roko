#!/usr/bin/env bash
# Show overall task status: per-wave progress, blocked tasks, etc.
#
# Usage:
#   ./scripts/status.sh
#   ./scripts/status.sh --wave wave-1
#   ./scripts/status.sh --track ide-acp

set -euo pipefail
cd "$(dirname "$0")/.."

FILTER_WAVE="${2:-}"
FILTER_TRACK=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --wave)  FILTER_WAVE="$2"; shift 2 ;;
        --track) FILTER_TRACK="$2"; shift 2 ;;
        *)       shift ;;
    esac
done

echo "═══════════════════════════════════════════════"
echo "  TASK RUNNER STATUS"
echo "  $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "═══════════════════════════════════════════════"
echo ""

# Count statuses
total=0
pending=0
claimed=0
implemented=0
tested=0
wired=0
verified=0
done_count=0

# Read all task statuses
while IFS= read -r line; do
    if [[ "$line" =~ ^status\ =\ \"(.+)\"$ ]]; then
        status="${BASH_REMATCH[1]}"
        total=$((total + 1))
        case $status in
            pending)     pending=$((pending + 1)) ;;
            claimed)     claimed=$((claimed + 1)) ;;
            implemented) implemented=$((implemented + 1)) ;;
            tested)      tested=$((tested + 1)) ;;
            wired)       wired=$((wired + 1)) ;;
            verified)    verified=$((verified + 1)) ;;
            done)        done_count=$((done_count + 1)) ;;
        esac
    fi
done < STATUS.toml

echo "  Total:       ${total}"
echo "  Pending:     ${pending}"
echo "  Claimed:     ${claimed}"
echo "  Implemented: ${implemented}"
echo "  Tested:      ${tested}"
echo "  Wired:       ${wired}"
echo "  Verified:    ${verified}"
echo "  Done:        ${done_count}"
echo ""

# Show task files count
task_files=$(ls tasks/*.md 2>/dev/null | wc -l | tr -d ' ')
echo "  Task files:  ${task_files}"
echo ""

# Show claimed tasks (who's working on what)
if [[ $claimed -gt 0 ]]; then
    echo "  Active agents:"
    grep -B0 -A3 'status = "claimed"' STATUS.toml | grep -E "(^\[tasks|agent)" | paste - - | while read -r line; do
        task=$(echo "$line" | grep -oP 'tasks\.\K[0-9]+')
        agent=$(echo "$line" | grep -oP 'agent = "\K[^"]+')
        echo "    ${task}: ${agent}"
    done 2>/dev/null || echo "    (parse error — check STATUS.toml)"
    echo ""
fi

# Show recent gate results
if ls audits/gate-*.md 1>/dev/null 2>&1; then
    echo "  Recent gates:"
    ls -1t audits/gate-*.md | head -3 | while read -r f; do
        result=$(grep "Result" "$f" | head -1 | cut -d: -f2 | tr -d ' *')
        echo "    $(basename "$f" .md): ${result}"
    done
    echo ""
fi

echo "═══════════════════════════════════════════════"
