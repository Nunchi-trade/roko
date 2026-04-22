#!/bin/bash
# Demo the PRD workflow via CLI commands.
# Usage: bash demo-prd-cli.sh
# Requires: roko workspace initialized (roko init)

set -euo pipefail

ROKO="${ROKO:-roko}"

pause() {
    echo ""
    read -rp "  [press enter to continue] " < /dev/tty
    echo ""
}

echo "═══════════════════════════════════════════"
echo "  PRD WORKFLOW DEMO (CLI)"
echo "═══════════════════════════════════════════"

echo ""
echo "STEP 1: CAPTURE IDEAS"
$ROKO prd idea "Wire knowledge query into matchmaking scoring"
$ROKO prd idea "Add cold storage archival trigger on schedule"
$ROKO prd idea "Dashboard UI for agent creation and tool configuration"
pause

echo "STEP 2: LIST ALL PRDs"
$ROKO prd list
pause

echo "STEP 3: STATUS REPORT"
$ROKO prd status
pause

echo "STEP 4: CREATE JOBS FOR TASKS"
$ROKO job create "Wire knowledge query" --type coding_task --description "Query neuro store during matchmaking to boost agents with relevant experience" --priority high
$ROKO job create "Research cold archival patterns" --type research --description "Survey cron-based archival patterns in distributed systems"
pause

echo "STEP 5: LIST JOBS"
$ROKO job list

echo ""
echo "═══════════════════════════════════════════"
echo "  Done. Next steps:"
echo "  • roko prd draft new <slug>   — draft a PRD"
echo "  • roko prd plan <slug>        — generate tasks"
echo "  • roko plan run plans/        — execute"
echo "═══════════════════════════════════════════"
