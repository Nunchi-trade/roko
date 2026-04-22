#!/bin/bash
# smoke-test.sh — Fast validation of roko-serve + demo resources (~15s).
# Usage: bash smoke-test.sh [base-url]
# Exit: 0 on success, 1 on failure.
# Requires: roko serve running.

set -euo pipefail

BASE="${1:-http://127.0.0.1:6677}/api"
PASS=0
FAIL=0

check() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1))
    else
        echo "  ✗ $label — expected '$expected', got '$actual'"
        FAIL=$((FAIL + 1))
    fi
}

check_ok() {
    local label="$1" actual="$2"
    if [ -n "$actual" ] && [ "$actual" != "FAIL" ]; then
        PASS=$((PASS + 1))
    else
        echo "  ✗ $label — empty or failed response"
        FAIL=$((FAIL + 1))
    fi
}

echo "═══ ROKO SMOKE TEST ═══"
echo "target: $BASE"
echo ""

# ─── 1. Server health ───
echo "1. Server"
R=$(curl -sf "$BASE/health" 2>/dev/null || echo "FAIL")
check "health endpoint" "ok" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin).get('status',''))" 2>/dev/null || echo "")"

# ─── 2. Register agents ───
echo "2. Agents"
for agent in '{"agent_id":"smoke-rust","label":"rust","capabilities":["messaging","tasks"],"skills":["rust","p2p"],"tier":"Expert","reputation":90,"max_concurrent_jobs":3}' \
             '{"agent_id":"smoke-sol","label":"solidity","capabilities":["messaging","tasks"],"skills":["solidity","security"],"tier":"Trusted","reputation":80,"max_concurrent_jobs":2}'; do
    name=$(echo "$agent" | python3 -c "import sys,json; print(json.load(sys.stdin)['agent_id'])")
    R=$(curl -sf -X POST "$BASE/agents/register" -H 'Content-Type: application/json' -d "$agent" 2>/dev/null || echo "FAIL")
    check_ok "register $name" "$R"
done

# ─── 3. Matchmaking ───
echo "3. Matchmaking"
M=$(curl -sf -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' \
    -d '{"title":"test task","skills":["rust"],"reward":"1000 KORAI","minTier":"Verified"}' 2>/dev/null || echo "")
COUNT=$(echo "$M" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['candidates']))" 2>/dev/null || echo "0")
check "match returns candidates" "True" "$(python3 -c "print($COUNT >= 1)")"
check "has totalFee" "True" "$(echo "$M" | python3 -c "import sys,json; print('totalFee' in json.load(sys.stdin))" 2>/dev/null || echo "False")"

M=$(curl -sf -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' \
    -d '{"title":"pioneer","minTier":"Pioneer"}' 2>/dev/null || echo "")
check "pioneer tier → 0" "0" "$(echo "$M" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['candidates']))" 2>/dev/null || echo "-1")"

# ─── 4. Job lifecycle ───
echo "4. Jobs"
JOB=$(curl -sf -X POST "$BASE/jobs" -H 'Content-Type: application/json' \
    -d '{"title":"smoke test job","reward":"500 KORAI","committed_candidates":["smoke-rust"]}' 2>/dev/null || echo "")
JID=$(echo "$JOB" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])" 2>/dev/null || echo "")
check "create → open" "open" "$(echo "$JOB" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])" 2>/dev/null || echo "")"

if [ -n "$JID" ]; then
    R=$(curl -sf -X POST "$BASE/jobs/$JID/assign" -H 'Content-Type: application/json' -d '{"agent_id":"smoke-rust"}' 2>/dev/null || echo "")
    check "assign → assigned" "assigned" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])" 2>/dev/null || echo "")"

    R=$(curl -sf -X POST "$BASE/jobs/$JID/start" 2>/dev/null || echo "")
    check "start → in_progress" "in_progress" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])" 2>/dev/null || echo "")"

    R=$(curl -sf -X POST "$BASE/jobs/$JID/submit" -H 'Content-Type: application/json' \
        -d '{"result_summary":"Smoke test done","artifacts":[]}' 2>/dev/null || echo "")
    check "submit → submitted" "submitted" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])" 2>/dev/null || echo "")"

    R=$(curl -sf -X POST "$BASE/jobs/$JID/evaluate" -H 'Content-Type: application/json' \
        -d '{"accepted":true,"feedback":"LGTM"}' 2>/dev/null || echo "")
    check "evaluate → completed" "completed" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])" 2>/dev/null || echo "")"
fi

# ─── 5. PRD + Research ───
echo "5. PRDs & Research"
R=$(curl -sf -X POST "$BASE/prds/ideas" -H 'Content-Type: application/json' \
    -d '{"text":"Smoke test idea"}' 2>/dev/null || echo "FAIL")
check_ok "capture idea" "$R"

R=$(curl -sf "$BASE/prds" 2>/dev/null || echo "FAIL")
check_ok "list prds" "$R"

R=$(curl -sf -X POST "$BASE/research/topic" -H 'Content-Type: application/json' \
    -d '{"topic":"smoke test","depth":"shallow"}' 2>/dev/null || echo "FAIL")
check_ok "dispatch research" "$R"

# ─── 6. Key endpoints ───
echo "6. Endpoints"
ENDPOINT_PASS=0
ENDPOINT_TOTAL=0
for path in /api/health /api/status /api/dashboard /api/managed-agents /api/agents \
    /api/jobs /api/jobs/stats /api/plans /api/prds /api/prds/status /api/research \
    /api/providers /api/models /api/config /api/episodes /api/signals \
    /api/metrics /api/gates/summary /api/learning/efficiency /api/learn/experiments \
    /api/knowledge/entries /api/tasks /api/subscriptions /api/integrations \
    /api/deployments /api/templates /api/heartbeats /api/truth_map /api/parity; do
    ENDPOINT_TOTAL=$((ENDPOINT_TOTAL + 1))
    code=$(curl -s -o /dev/null -w "%{http_code}" "${BASE%/api}$path" 2>/dev/null || echo "000")
    if [ "${code:-000}" -ge 200 ] && [ "${code:-000}" -lt 300 ]; then
        ENDPOINT_PASS=$((ENDPOINT_PASS + 1))
    else
        echo "  ✗ [$code] $path"
    fi
done
check "endpoints $ENDPOINT_PASS/$ENDPOINT_TOTAL ok" "$ENDPOINT_TOTAL" "$ENDPOINT_PASS"

# ─── Summary ───
echo ""
TOTAL=$((PASS + FAIL))
if [ "$FAIL" -eq 0 ]; then
    echo "✓ SMOKE TEST PASSED ($PASS/$TOTAL checks)"
    exit 0
else
    echo "✗ SMOKE TEST FAILED ($PASS passed, $FAIL failed out of $TOTAL)"
    exit 1
fi
