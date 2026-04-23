#!/bin/bash
# Demo the full job lifecycle: match → create → assign → start → submit → evaluate.
# Usage: bash demo-lifecycle.sh [base-url]

set -euo pipefail

BASE="${1:-http://127.0.0.1:6677}/api"

pause() {
    echo ""
    read -rp "  [press enter to continue] " < /dev/tty
    echo ""
}

show_state() {
    local job="$1" label="$2"
    echo "  $label"
    echo "$job" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(f'    state:     {d[\"state\"]}')
print(f'    assigned:  {d.get(\"assigned_to\", \"—\") or \"—\"}')
if d.get('reward'):
    print(f'    reward:    {d[\"reward\"]}')
if d.get('committed_candidates'):
    print(f'    candidates: {d[\"committed_candidates\"]}')
if d.get('submission'):
    print(f'    summary:   {d[\"submission\"].get(\"result_summary\", \"\")}')
if d.get('evaluation'):
    e = d['evaluation']
    print(f'    accepted:  {e.get(\"accepted\", \"\")}')
    print(f'    feedback:  {e.get(\"feedback\", \"\")}')
"
}

echo "═══════════════════════════════════════════"
echo "  JOB LIFECYCLE DEMO"
echo "═══════════════════════════════════════════"

echo ""
echo "STEP 1: MATCH AGENTS"
echo "   Finding Rust developers for a relay implementation..."
MATCH=$(curl -sf -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' \
    -d '{"title":"implement walrus gateway relay","skills":["rust","p2p"],"reward":"2500 KORAI","minTier":"Verified"}')
echo "$MATCH" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(f'   Found {len(d[\"candidates\"])} candidate(s), total fee: {d[\"totalFee\"]}, ETA: ~{d[\"etaHours\"]}h')
for c in d['candidates']:
    print(f'   • {c[\"label\"]} ({c[\"tier\"]}, rep {c[\"reputation\"]})')
"
pause

echo "STEP 2: CREATE JOB"
echo "   Posting bounty with committed candidates..."
CANDIDATES=$(echo "$MATCH" | python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps([c['agentId'] for c in d['candidates']]))")
JOB=$(curl -sf -X POST "$BASE/jobs" -H 'Content-Type: application/json' \
    -d "{\"title\":\"implement walrus gateway relay\",\"description\":\"Build a libp2p relay for the Walrus data availability layer\",\"reward\":\"2500 KORAI\",\"committed_candidates\":$CANDIDATES}")
JID=$(echo "$JOB" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
show_state "$JOB" "Job created:"
pause

echo "STEP 3: ASSIGN TO TOP CANDIDATE"
AGENT=$(echo "$MATCH" | python3 -c "import sys,json; print(json.load(sys.stdin)['candidates'][0]['agentId'])")
R=$(curl -sf -X POST "$BASE/jobs/$JID/assign" -H 'Content-Type: application/json' -d "{\"agent_id\":\"$AGENT\"}")
show_state "$R" "Assigned to $AGENT:"
pause

echo "STEP 4: AGENT STARTS WORK"
R=$(curl -sf -X POST "$BASE/jobs/$JID/start")
show_state "$R" "Work started:"
pause

echo "STEP 5: AGENT SUBMITS RESULT"
R=$(curl -sf -X POST "$BASE/jobs/$JID/submit" -H 'Content-Type: application/json' \
    -d '{"result_summary":"Implemented libp2p relay with noise encryption, multiplexing, and Walrus blob routing. Tests pass.","artifacts":[{"type":"file","path":"src/relay/mod.rs"},{"type":"file","path":"src/relay/transport.rs"}],"gate_results":[{"gate":"compile","passed":true},{"gate":"test","passed":true},{"gate":"clippy","passed":true}]}')
show_state "$R" "Result submitted:"
pause

echo "STEP 6: EVALUATE & ACCEPT"
R=$(curl -sf -X POST "$BASE/jobs/$JID/evaluate" -H 'Content-Type: application/json' \
    -d '{"accepted":true,"feedback":"Clean implementation with good test coverage. Approved for merge."}')
show_state "$R" "Evaluation complete:"

echo ""
echo "═══════════════════════════════════════════"
echo "  ✓ Job completed successfully"
echo "═══════════════════════════════════════════"
