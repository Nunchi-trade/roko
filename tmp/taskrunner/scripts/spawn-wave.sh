#!/usr/bin/env bash
# Spawn agents for all available tasks in a wave.
# This is the "press go" button for parallel execution.
#
# Usage:
#   ./scripts/spawn-wave.sh wave-1              # claim + print prompts
#   ./scripts/spawn-wave.sh wave-1 --max 10     # limit to 10 agents
#   ./scripts/spawn-wave.sh wave-1 --dry-run    # show what would be spawned

set -euo pipefail
cd "$(dirname "$0")/.."

WAVE="${1:?Usage: spawn-wave.sh <wave-name> [--max N] [--dry-run]}"
MAX=20
DRY_RUN=false
AGENT_PREFIX="agent"

shift
while [[ $# -gt 0 ]]; do
    case $1 in
        --max)     MAX="$2"; shift 2 ;;
        --dry-run) DRY_RUN=true; shift ;;
        --prefix)  AGENT_PREFIX="$2"; shift 2 ;;
        *)         echo "Unknown arg: $1"; exit 1 ;;
    esac
done

echo "═══════════════════════════════════════════════"
echo "  SPAWN WAVE: ${WAVE}"
echo "  Max agents: ${MAX}"
echo "  Dry run: ${DRY_RUN}"
echo "═══════════════════════════════════════════════"
echo ""

# Find available tasks for this wave
AVAILABLE=$(./scripts/next.sh --count "$MAX" --wave "$WAVE" 2>/dev/null || true)

if [[ -z "$AVAILABLE" || "$AVAILABLE" == "No claimable tasks found." ]]; then
    echo "No claimable tasks in ${WAVE}."
    exit 0
fi

echo "Available tasks:"
echo "$AVAILABLE"
echo ""

TASK_COUNT=$(echo "$AVAILABLE" | wc -l | tr -d ' ')
echo "Found ${TASK_COUNT} tasks to spawn."
echo ""

if [[ "$DRY_RUN" == "true" ]]; then
    echo "(Dry run — no agents spawned)"
    exit 0
fi

# Spawn each
IDX=0
echo "$AVAILABLE" | while read -r line; do
    TASK_ID=$(echo "$line" | awk '{print $1}')
    IDX=$((IDX + 1))
    AGENT_NAME="${AGENT_PREFIX}-${WAVE}-${IDX}"

    echo "───────────────────────────────────────────"
    echo "Spawning: ${AGENT_NAME} → task ${TASK_ID}"
    echo "───────────────────────────────────────────"

    # Claim the task (creates worktree)
    ./scripts/claim.sh "$TASK_ID" "$AGENT_NAME"

    # Write the agent prompt to a file for easy copy-paste
    PROMPT_FILE="logs/${AGENT_NAME}-prompt.md"
    ./scripts/spawn.sh "$TASK_ID" "$AGENT_NAME" --print-only > "$PROMPT_FILE" 2>/dev/null

    echo "  Prompt saved to: ${PROMPT_FILE}"
    echo ""
done

echo ""
echo "═══════════════════════════════════════════════"
echo "  ${TASK_COUNT} agents spawned."
echo "  Prompts saved to logs/*-prompt.md"
echo "  Copy-paste prompts into your agent tool of choice."
echo "═══════════════════════════════════════════════"
