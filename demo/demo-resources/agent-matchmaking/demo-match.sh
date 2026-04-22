#!/bin/bash
# Demo the matchmaking endpoint with various queries.
# Designed to run live during the demo — each section pauses for a keypress.
# Usage: bash demo-match.sh [base-url]

set -euo pipefail

BASE="${1:-http://127.0.0.1:6677}/api"

pause() {
    echo ""
    read -rp "  [press enter to continue] " < /dev/tty
    echo ""
}

match() {
    local label="$1" body="$2"
    echo "  → POST /api/jobs/match"
    echo "    $body"
    echo ""
    curl -sf -X POST "$BASE/jobs/match" \
        -H 'Content-Type: application/json' \
        -d "$body" | python3 -c "
import sys, json
d = json.load(sys.stdin)
cs = d['candidates']
if not cs:
    print('    No matching agents.')
else:
    print(f'    {len(cs)} candidate(s)  |  fee: {d[\"totalFee\"]}  |  eta: ~{d[\"etaHours\"]}h')
    print()
    print(f'    {\"agent\":<20} {\"tier\":<10} {\"rep\":>4}  {\"load\":>7}  {\"skills\":>20}  {\"bid\":>12}')
    print(f'    {\"─\"*20} {\"─\"*10} {\"─\"*4}  {\"─\"*7}  {\"─\"*20}  {\"─\"*12}')
    for c in cs:
        skills = ', '.join(c.get('matchedSkills', []))
        load = f'{c[\"inflightJobs\"]}/{c[\"maxConcurrentJobs\"]}'
        print(f'    {c[\"label\"]:<20} {c[\"tier\"]:<10} {c[\"reputation\"]:>4}  {load:>7}  {skills:>20}  {c[\"bidShare\"]:>12}')
"
}

echo "═══════════════════════════════════════════"
echo "  AGENT MATCHMAKING DEMO"
echo "═══════════════════════════════════════════"

echo ""
echo "1. FIND RUST DEVELOPERS"
echo "   Filtering for agents with Rust + p2p skills, Verified tier minimum"
match "rust" '{"title":"implement walrus gateway relay","skills":["rust","p2p"],"reward":"2500 KORAI","minTier":"Verified"}'
pause

echo "2. FIND SOLIDITY AUDITORS"
echo "   Filtering for Solidity + security skills, Expert tier minimum"
match "solidity" '{"title":"audit lending pool contracts","skills":["solidity","security"],"reward":"5000 KORAI","minTier":"Expert"}'
pause

echo "3. FIND DEFI RESEARCHERS"
echo "   Filtering for DeFi analysis skills"
match "defi" '{"title":"research MEV landscape","skills":["defi","analysis"],"reward":"1500 KORAI"}'
pause

echo "4. BROAD SEARCH (no skill filter)"
echo "   All agents ranked by reputation and availability"
match "all" '{"title":"general infrastructure task","reward":"3000 KORAI"}'
pause

echo "5. LANGUAGE FILTER"
echo "   Using language=Rust (implicitly added to skills)"
match "lang" '{"title":"build indexer","language":"Rust","reward":"2000 KORAI"}'
pause

echo "6. UNREACHABLE TIER"
echo "   minTier=Pioneer — no agents qualify"
match "pioneer" '{"title":"pioneer task","minTier":"Pioneer","skills":["rust"]}'

echo ""
echo "Done."
