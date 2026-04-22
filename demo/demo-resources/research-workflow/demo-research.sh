#!/bin/bash
# Demo research workflows via CLI and API.
# Usage: bash demo-research.sh [serve-url]

set -euo pipefail

ROKO="${ROKO:-roko}"
BASE="${1:-http://127.0.0.1:6677}/api"

pause() {
    echo ""
    read -rp "  [press enter to continue] " < /dev/tty
    echo ""
}

echo "═══════════════════════════════════════════"
echo "  RESEARCH WORKFLOW DEMO"
echo "═══════════════════════════════════════════"

echo ""
echo "1. DISPATCH RESEARCH VIA API"
echo "   POST /api/research/topic"
R=$(curl -sf -X POST "$BASE/research/topic" -H 'Content-Type: application/json' \
    -d '{"topic":"Agent matchmaking algorithms in decentralized networks","depth":"shallow"}' 2>&1) || R="error: $?"
echo "   Response: $(echo "$R" | head -c 200)"
pause

echo "2. LIST RESEARCH ARTIFACTS"
echo "   GET /api/research"
curl -sf "$BASE/research" | python3 -c "
import sys, json
data = json.load(sys.stdin)
if isinstance(data, list):
    print(f'   {len(data)} artifact(s)')
    for a in data[:5]:
        if isinstance(a, dict):
            print(f'   • {a.get(\"title\", a.get(\"slug\", str(a)[:60]))}')
        else:
            print(f'   • {str(a)[:60]}')
elif isinstance(data, dict):
    print(f'   {data}')
" 2>/dev/null || echo "   (no artifacts or endpoint not available)"
pause

echo "3. CAPTURE RELATED IDEAS"
$ROKO prd idea "Implement reputation-weighted agent matching" 2>&1 || true
$ROKO prd idea "Add skill-overlap scoring to matchmaking" 2>&1 || true
echo ""

echo "4. CREATE RESEARCH JOBS"
$ROKO job create "Research decentralized matchmaking" --type research --description "Survey agent-to-task matching algorithms: capability-based, reputation-weighted, auction-based" 2>&1

echo ""
echo "5. LIST JOBS (research + coding)"
$ROKO job list 2>&1

echo ""
echo "═══════════════════════════════════════════"
echo "  Done. Dashboard tabs to check:"
echo "  • Atelier → Research bounty"
echo "  • Atelier → PRDs (new ideas)"
echo "  • Network → Jobs (research job)"
echo "═══════════════════════════════════════════"
