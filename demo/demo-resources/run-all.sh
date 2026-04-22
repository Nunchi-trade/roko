#!/bin/bash
# run-all.sh — Run every demo workflow non-interactively and report results.
# Usage: bash run-all.sh [base-url]
# Exit: 0 if all pass, 1 if any fail.
# Requires: roko serve running, roko binary built.

set -euo pipefail

BASE="${1:-http://127.0.0.1:6677}"
API="$BASE/api"
ROKO="${ROKO:-./target/debug/roko}"
DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DIR/../.." && pwd)"
cd "$ROOT"

SUITE_PASS=0
SUITE_FAIL=0

run_suite() {
    local name="$1"
    shift
    echo ""
    echo "━━━ $name ━━━"
    if "$@" 2>&1; then
        echo "  ✓ $name"
        SUITE_PASS=$((SUITE_PASS + 1))
    else
        echo "  ✗ $name FAILED"
        SUITE_FAIL=$((SUITE_FAIL + 1))
    fi
}

# ═══════════════════════════════════════════
# 1. SMOKE TEST
# ═══════════════════════════════════════════
run_suite "Smoke Test" bash "$DIR/smoke-test.sh" "$BASE"

# ═══════════════════════════════════════════
# 2. SEED AGENTS
# ═══════════════════════════════════════════
run_suite "Seed Agents" bash "$DIR/agent-matchmaking/seed-agents.sh" "$BASE"

# ═══════════════════════════════════════════
# 3. E2E TEST SUITE (40 tests)
# ═══════════════════════════════════════════
run_suite "E2E Test Suite" bash "$DIR/agent-matchmaking/e2e-test.sh" "$BASE"

# ═══════════════════════════════════════════
# 4. PRD WORKFLOW (CLI)
# ═══════════════════════════════════════════
run_suite "PRD CLI" bash -c '
    ROKO="'"$ROKO"'"
    $ROKO prd idea "run-all: test idea capture" 2>&1 || exit 1
    $ROKO prd list 2>&1 || exit 1
    $ROKO prd status 2>&1 || exit 1
'

# ═══════════════════════════════════════════
# 5. PRD WORKFLOW (API)
# ═══════════════════════════════════════════
run_suite "PRD API" bash -c '
    API="'"$API"'"
    R=$(curl -sf -X POST "$API/prds/ideas" -H "Content-Type: application/json" \
        -d "{\"text\":\"run-all: API test idea\"}" 2>&1)
    echo "$R" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get(\"slug\"), \"no slug\"" || exit 1
    curl -sf "$API/prds" | python3 -c "import sys,json; d=json.load(sys.stdin); assert isinstance(d,list)" || exit 1
    curl -sf "$API/prds/status" | python3 -c "import sys,json; json.load(sys.stdin)" || exit 1
    curl -sf "$API/plans" | python3 -c "import sys,json; json.load(sys.stdin)" || exit 1
'

# ═══════════════════════════════════════════
# 6. RESEARCH WORKFLOW
# ═══════════════════════════════════════════
run_suite "Research" bash -c '
    API="'"$API"'"
    R=$(curl -sf -X POST "$API/research/topic" -H "Content-Type: application/json" \
        -d "{\"topic\":\"run-all validation\",\"depth\":\"shallow\"}" 2>&1)
    echo "$R" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get(\"id\"), \"no id\"" || exit 1
    curl -sf "$API/research" | python3 -c "import sys,json; json.load(sys.stdin)" || exit 1
'

# ═══════════════════════════════════════════
# 7. FLEET SETUP
# ═══════════════════════════════════════════
run_suite "Fleet Setup" bash -c '
    ROKO="'"$ROKO"'"
    API="'"$API"'"
    $ROKO agent create --name run-all-dev --domain coding --prompt "Automation test agent" 2>&1 | head -1 || true
    curl -sf -X POST "$API/agents/register" -H "Content-Type: application/json" \
        -d "{\"agent_id\":\"run-all-dev\",\"label\":\"Automation Dev\",\"capabilities\":[\"messaging\",\"tasks\"],\"skills\":[\"rust\"],\"tier\":\"Expert\",\"reputation\":90}" > /dev/null 2>&1 || exit 1
    $ROKO agent list 2>&1 || exit 1
    curl -sf "$API/managed-agents" | python3 -c "import sys,json; a=json.load(sys.stdin); assert len(a)>0" || exit 1
'

# ═══════════════════════════════════════════
# 8. JOB LIFECYCLE (full state machine)
# ═══════════════════════════════════════════
run_suite "Job Lifecycle" bash -c '
    API="'"$API"'"
    # Match
    M=$(curl -sf -X POST "$API/jobs/match" -H "Content-Type: application/json" \
        -d "{\"title\":\"run-all test\",\"skills\":[\"rust\"],\"reward\":\"1000 KORAI\"}")
    echo "$M" | python3 -c "import sys,json; d=json.load(sys.stdin); assert len(d[\"candidates\"])>0" || exit 1

    # Create
    JOB=$(curl -sf -X POST "$API/jobs" -H "Content-Type: application/json" \
        -d "{\"title\":\"run-all lifecycle\",\"reward\":\"1000 KORAI\",\"committed_candidates\":[\"agent-rustsmith\"]}")
    JID=$(echo "$JOB" | python3 -c "import sys,json; print(json.load(sys.stdin)[\"id\"])")

    # Assign → Start → Submit → Evaluate
    curl -sf -X POST "$API/jobs/$JID/assign" -H "Content-Type: application/json" -d "{\"agent_id\":\"agent-rustsmith\"}" > /dev/null
    curl -sf -X POST "$API/jobs/$JID/start" > /dev/null
    curl -sf -X POST "$API/jobs/$JID/submit" -H "Content-Type: application/json" \
        -d "{\"result_summary\":\"run-all done\",\"artifacts\":[]}" > /dev/null
    R=$(curl -sf -X POST "$API/jobs/$JID/evaluate" -H "Content-Type: application/json" \
        -d "{\"accepted\":true,\"feedback\":\"pass\"}")
    echo "$R" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d[\"state\"]==\"completed\", f\"got {d[\"state\"]}\"" || exit 1

    # Rejection cycle
    J2=$(curl -sf -X POST "$API/jobs" -H "Content-Type: application/json" -d "{\"title\":\"rework test\"}" | python3 -c "import sys,json; print(json.load(sys.stdin)[\"id\"])")
    curl -sf -X POST "$API/jobs/$J2/assign" -H "Content-Type: application/json" -d "{\"agent_id\":\"agent-rustsmith\"}" > /dev/null
    curl -sf -X POST "$API/jobs/$J2/start" > /dev/null
    curl -sf -X POST "$API/jobs/$J2/submit" -H "Content-Type: application/json" -d "{\"result_summary\":\"attempt 1\"}" > /dev/null
    curl -sf -X POST "$API/jobs/$J2/evaluate" -H "Content-Type: application/json" -d "{\"accepted\":false,\"feedback\":\"redo\"}" > /dev/null
    curl -sf -X POST "$API/jobs/$J2/submit" -H "Content-Type: application/json" -d "{\"result_summary\":\"attempt 2\"}" > /dev/null
    R=$(curl -sf -X POST "$API/jobs/$J2/evaluate" -H "Content-Type: application/json" -d "{\"accepted\":true,\"feedback\":\"ok\"}")
    echo "$R" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d[\"state\"]==\"completed\"" || exit 1
'

# ═══════════════════════════════════════════
# 9. OLLAMA PROVIDER (if configured)
# ═══════════════════════════════════════════
run_suite "Ollama Provider" bash -c '
    API="'"$API"'"
    R=$(curl -sf "$API/providers" 2>/dev/null || echo "{}")
    HAS_OLLAMA=$(echo "$R" | python3 -c "
import sys, json
d = json.load(sys.stdin)
providers = d.get(\"providers\", []) if isinstance(d, dict) else d
print(any(p.get(\"id\") == \"ollama\" for p in providers))
" 2>/dev/null || echo "False")
    if [ "$HAS_OLLAMA" = "True" ]; then
        H=$(curl -sf "$API/providers/ollama/health" 2>/dev/null || echo "{}")
        echo "$H" | python3 -c "import sys,json; d=json.load(sys.stdin); assert d.get(\"state\")==\"healthy\", f\"got {d}\"" || exit 1
    else
        echo "  (ollama not configured, skipping)"
    fi
'

# ═══════════════════════════════════════════
# SUMMARY
# ═══════════════════════════════════════════
echo ""
echo "═══════════════════════════════════════════════════"
TOTAL=$((SUITE_PASS + SUITE_FAIL))
if [ "$SUITE_FAIL" -eq 0 ]; then
    echo "  ✓ ALL $TOTAL SUITES PASSED"
    echo "═══════════════════════════════════════════════════"
    exit 0
else
    echo "  ✗ $SUITE_PASS/$TOTAL passed, $SUITE_FAIL failed"
    echo "═══════════════════════════════════════════════════"
    exit 1
fi
