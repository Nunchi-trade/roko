#!/bin/bash
# Demo the PRD workflow via the HTTP API (same endpoints the dashboard uses).
# Usage: bash demo-prd-api.sh [base-url]

set -euo pipefail

BASE="${1:-http://127.0.0.1:6677}/api"

pause() {
    echo ""
    read -rp "  [press enter to continue] " < /dev/tty
    echo ""
}

echo "═══════════════════════════════════════════"
echo "  PRD WORKFLOW DEMO (HTTP API)"
echo "═══════════════════════════════════════════"

echo ""
echo "STEP 1: CAPTURE IDEA"
echo "   POST /api/prds/ideas"
R=$(curl -sf -X POST "$BASE/prds/ideas" -H 'Content-Type: application/json' \
    -d '{"text":"Wire knowledge query into matchmaking scoring so agents with relevant experience rank higher"}')
echo "   $R" | python3 -c "import sys,json; d=json.load(sys.stdin); print(f'   slug: {d.get(\"slug\",\"(check output)\")}')" 2>/dev/null || echo "   Response: $R"
pause

echo "STEP 2: LIST PRDs"
echo "   GET /api/prds"
curl -sf "$BASE/prds" | python3 -c "
import sys, json
prds = json.load(sys.stdin)
if isinstance(prds, list):
    for p in prds[:5]:
        status = p.get('status', p.get('state', '?'))
        print(f'   [{status:>10}] {p.get(\"slug\", p.get(\"title\", \"?\"))}')
    if len(prds) > 5:
        print(f'   ... and {len(prds)-5} more')
elif isinstance(prds, dict) and 'prds' in prds:
    for p in prds['prds'][:5]:
        print(f'   [{p.get(\"status\",\"?\")}] {p.get(\"slug\",\"?\")}')
else:
    print(f'   {prds}')
" 2>/dev/null || echo "   (no PRDs or parse error)"
pause

echo "STEP 3: PRD STATUS REPORT"
echo "   GET /api/prds/status"
curl -sf "$BASE/prds/status" | python3 -m json.tool 2>/dev/null || echo "   (status endpoint not available)"
pause

echo "STEP 4: CHECK PLANS"
echo "   GET /api/plans"
curl -sf "$BASE/plans" | python3 -c "
import sys, json
plans = json.load(sys.stdin)
if isinstance(plans, list):
    print(f'   {len(plans)} plan(s)')
    for p in plans[:5]:
        print(f'   • {p.get(\"id\", p.get(\"title\", \"?\"))}: {p.get(\"task_count\",\"?\")} tasks')
else:
    print(f'   {plans}')
" 2>/dev/null || echo "   (no plans)"

echo ""
echo "═══════════════════════════════════════════"
echo "  Done. Open the dashboard to see:"
echo "  • Atelier → PRDs — idea appears"
echo "  • Atelier → Plans — plans list"
echo "═══════════════════════════════════════════"
