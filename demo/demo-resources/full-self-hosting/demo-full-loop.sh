#!/bin/bash
# Full self-hosting loop demo.
# Usage: bash demo-full-loop.sh [serve-url]
# Requires: roko serve running, agents seeded

set -euo pipefail

ROKO="${ROKO:-roko}"
BASE="${1:-http://127.0.0.1:6677}/api"

pause() {
    echo ""
    read -rp "  [press enter to continue] " < /dev/tty
    echo ""
}

echo "═══════════════════════════════════════════"
echo "  FULL SELF-HOSTING LOOP"
echo "═══════════════════════════════════════════"

# --- ACT 1: CAPTURE ---

echo ""
echo "ACT 1: CAPTURE IDEAS"
echo "────────────────────"
$ROKO prd idea "Wire knowledge store queries into matchmaking scoring" 2>&1
$ROKO prd idea "Add scheduled cold storage archival for stale signals" 2>&1
$ROKO prd idea "Build dashboard form for agent creation with tool config" 2>&1
echo ""
$ROKO prd list 2>&1
pause

# --- ACT 2: JOBS ---

echo "ACT 2: CREATE WORK ITEMS"
echo "────────────────────────"
$ROKO job create "Wire knowledge into matchmaking" --type coding_task --description "Query neuro store during match scoring. Agents with relevant past experience should rank higher." --priority high 2>&1
$ROKO job create "Research cold archival patterns" --type research --description "Survey cron-based and event-driven archival in distributed signal stores." 2>&1
echo ""
$ROKO job list 2>&1
pause

# --- ACT 3: MATCH ---

echo "ACT 3: FIND AGENTS"
echo "──────────────────"
echo "Matching for Rust coding task..."
curl -sf -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' \
    -d '{"title":"Wire knowledge into matchmaking","skills":["rust","systems"],"reward":"2000 KORAI","minTier":"Verified"}' | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(f'   {len(d[\"candidates\"])} candidate(s), fee: {d[\"totalFee\"]}, eta: ~{d[\"etaHours\"]}h')
for c in d['candidates']:
    print(f'   • {c[\"label\"]} ({c[\"tier\"]}, rep {c[\"reputation\"]}) → {c[\"bidShare\"]}')
" 2>/dev/null || echo "   (matchmaking not available — seed agents first)"
echo ""
echo "Matching for research task..."
curl -sf -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' \
    -d '{"title":"Research cold archival","skills":["analysis","distributed systems"],"reward":"1000 KORAI"}' | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(f'   {len(d[\"candidates\"])} candidate(s)')
for c in d['candidates']:
    print(f'   • {c[\"label\"]} ({c[\"tier\"]})')
" 2>/dev/null || echo "   (matchmaking not available)"
pause

# --- ACT 4: OBSERVE ---

echo "ACT 4: SYSTEM STATE"
echo "───────────────────"
echo "Health:"
curl -sf "$BASE/health" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(f'   status: {d[\"status\"]}, agents: {d[\"active_agents\"]}, plans: {d[\"active_plans\"]}, runs: {d[\"active_runs\"]}')
" 2>/dev/null

echo ""
echo "Job stats:"
curl -sf "$BASE/jobs/stats" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(f'   total: {d[\"total\"]}, by state: {d[\"by_state\"]}, by type: {d[\"by_type\"]}')
" 2>/dev/null

echo ""
echo "Fleet:"
curl -sf "$BASE/managed-agents" | python3 -c "
import sys, json
agents = json.load(sys.stdin)
print(f'   {len(agents)} agent(s)')
for a in agents[:5]:
    print(f'   • {a[\"id\"]:20} {a.get(\"tier\",\"—\"):10} {a.get(\"status\",\"?\")}')
" 2>/dev/null

echo ""
echo "Learning:"
echo "   Efficiency events: $(wc -l < .roko/learn/efficiency.jsonl 2>/dev/null || echo '0') entries"
echo "   Episodes: $(wc -l < .roko/episodes.jsonl 2>/dev/null || echo '0') entries"
echo "   Cascade router: $(test -f .roko/learn/cascade-router.json && echo 'present' || echo 'empty')"
echo "   Gate thresholds: $(test -f .roko/learn/gate-thresholds.json && echo 'present' || echo 'empty')"

echo ""
echo "═══════════════════════════════════════════"
echo "  Loop complete. Dashboard tabs to explore:"
echo ""
echo "  Atelier:   PRDs, Plans, Research, Coding"
echo "  Network:   Agents, Jobs, Learning, Swarm"
echo "  Studio:    Overview, Live, Logs, Chat"
echo "═══════════════════════════════════════════"
