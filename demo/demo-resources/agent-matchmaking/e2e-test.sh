#!/bin/bash
set -euo pipefail

BASE="${1:-http://127.0.0.1:6677}/api"
ROKO="${ROKO:-/Users/will/dev/nunchi/roko/roko/target/debug/roko}"
PASS=0
FAIL=0

check() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        echo "  ✓ $label"
        PASS=$((PASS + 1))
    else
        echo "  ✗ $label — expected '$expected', got '$actual'"
        FAIL=$((FAIL + 1))
    fi
}

echo "═══════════════════════════════════════════"
echo "  AGENT MATCHMAKING E2E TEST SUITE"
echo "═══════════════════════════════════════════"

echo ""
echo "1. AGENT REGISTRATION"
echo "─────────────────────"

R=$(curl -sf -X POST "$BASE/agents/register" -H 'Content-Type: application/json' \
  -d '{"agent_id":"agent-rustsmith","label":"rustsmith","capabilities":["messaging","tasks"],"skills":["rust","p2p","eth"],"tier":"Expert","reputation":94,"past_jobs_completed":37,"max_concurrent_jobs":5}')
check "register rustsmith" "agent-rustsmith" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['agent']['agent_id'])")"

R=$(curl -sf -X POST "$BASE/agents/register" -H 'Content-Type: application/json' \
  -d '{"agent_id":"agent-ethdev","label":"ethdev","capabilities":["messaging","tasks"],"skills":["solidity","eth","defi"],"tier":"Trusted","reputation":82,"past_jobs_completed":21,"max_concurrent_jobs":3}')
check "register ethdev" "agent-ethdev" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['agent']['agent_id'])")"

R=$(curl -sf -X POST "$BASE/agents/register" -H 'Content-Type: application/json' \
  -d '{"agent_id":"agent-jsdev","label":"jsdev","capabilities":["messaging"],"skills":["javascript","react"],"tier":"Verified","reputation":65,"past_jobs_completed":12,"max_concurrent_jobs":2}')
check "register jsdev" "agent-jsdev" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['agent']['agent_id'])")"

echo ""
echo "2. AGENT LISTING"
echo "────────────────"

AGENTS=$(curl -sf "$BASE/managed-agents")
AGENT_COUNT=$(echo "$AGENTS" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))")
check "managed-agents has agents" "True" "$(python3 -c "print($AGENT_COUNT >= 3)")"

R=$(curl -sf "$BASE/agents/agent-rustsmith")
check "get agent tier" "Expert" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['tier'])")"
check "get agent reputation" "94" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['reputation'])")"

echo ""
echo "3. MATCHMAKING"
echo "──────────────"

# Match with skills filter — rustsmith matches both, fullstack matches rust only
M=$(curl -sf -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' \
  -d '{"title":"build relay","skills":["rust","p2p"],"reward":"2500 KORAI","minTier":"Verified"}')
MATCH_COUNT=$(echo "$M" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['candidates']))")
check "match rust skills: ≥1 candidate" "True" "$(python3 -c "print($MATCH_COUNT >= 1)")"
check "match returns rustsmith first" "agent-rustsmith" "$(echo "$M" | python3 -c "import sys,json; print(json.load(sys.stdin)['candidates'][0]['agentId'])")"
check "totalFee preserved" "2500 KORAI" "$(echo "$M" | python3 -c "import sys,json; print(json.load(sys.stdin)['totalFee'])")"
check "etaHours is integer" "int" "$(echo "$M" | python3 -c "import sys,json; print(type(json.load(sys.stdin)['etaHours']).__name__)")"

# Match all agents (no skills filter) — at least the 3 we registered
M=$(curl -sf -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' -d '{"title":"general task"}')
NO_FILTER_COUNT=$(echo "$M" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['candidates']))")
check "match no filter: ≥3 candidates" "True" "$(python3 -c "print($NO_FILTER_COUNT >= 3)")"

# Match with language — at least rustsmith should match
M=$(curl -sf -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' -d '{"title":"build thing","language":"Rust"}')
LANG_COUNT=$(echo "$M" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['candidates']))")
check "match language=Rust: ≥1" "True" "$(python3 -c "print($LANG_COUNT >= 1)")"

# Match with high tier (only Expert qualifies) — at least rustsmith
M=$(curl -sf -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' -d '{"title":"elite","minTier":"Expert","skills":["rust"]}')
EXPERT_COUNT=$(echo "$M" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['candidates']))")
check "match Expert tier: ≥1" "True" "$(python3 -c "print($EXPERT_COUNT >= 1)")"

# Match Pioneer (none)
M=$(curl -sf -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' -d '{"title":"pioneer","minTier":"Pioneer"}')
check "match Pioneer: 0" "0" "$(echo "$M" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['candidates']))")"

# Validation: blank title
R=$(curl -s -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' -d '{"title":""}')
check "blank title → 400" "bad_request" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin).get('code',''))")"

# Validation: invalid tier
R=$(curl -s -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' -d '{"title":"x","minTier":"Principal"}')
check "invalid tier → 400" "bad_request" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin).get('code',''))")"

echo ""
echo "4. JOB LIFECYCLE (HTTP)"
echo "───────────────────────"

# Create
JOB=$(curl -sf -X POST "$BASE/jobs" -H 'Content-Type: application/json' \
  -d '{"title":"build relay","reward":"2500 KORAI","committed_candidates":["agent-rustsmith","agent-ethdev"]}')
JID=$(echo "$JOB" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
check "create → open" "open" "$(echo "$JOB" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])")"
check "committed_candidates" "2" "$(echo "$JOB" | python3 -c "import sys,json; print(len(json.load(sys.stdin)['committed_candidates']))")"
check "reward roundtrip" "2500 KORAI" "$(echo "$JOB" | python3 -c "import sys,json; print(json.load(sys.stdin)['reward'])")"

# Assign
R=$(curl -sf -X POST "$BASE/jobs/$JID/assign" -H 'Content-Type: application/json' -d '{"agent_id":"agent-rustsmith"}')
check "assign → assigned" "assigned" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])")"

# Start
R=$(curl -sf -X POST "$BASE/jobs/$JID/start")
check "start → in_progress" "in_progress" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])")"

# Inflight count
M=$(curl -sf -X POST "$BASE/jobs/match" -H 'Content-Type: application/json' -d '{"title":"x","skills":["rust"]}')
check "rustsmith inflight=1" "1" "$(echo "$M" | python3 -c "import sys,json; print(json.load(sys.stdin)['candidates'][0]['inflightJobs'])")"

# Submit
R=$(curl -sf -X POST "$BASE/jobs/$JID/submit" -H 'Content-Type: application/json' \
  -d '{"result_summary":"Relay module done","artifacts":[{"type":"file","path":"src/relay.rs"}]}')
check "submit → submitted" "submitted" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])")"

# Evaluate (accept)
R=$(curl -sf -X POST "$BASE/jobs/$JID/evaluate" -H 'Content-Type: application/json' \
  -d '{"accepted":true,"feedback":"Clean implementation"}')
check "evaluate(accept) → completed" "completed" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])")"

# Final GET
F=$(curl -sf "$BASE/jobs/$JID")
check "final state=completed" "completed" "$(echo "$F" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])")"
check "has submission" "True" "$(echo "$F" | python3 -c "import sys,json; print(json.load(sys.stdin)['submission'] is not None)")"
check "has evaluation" "True" "$(echo "$F" | python3 -c "import sys,json; print(json.load(sys.stdin)['evaluation'] is not None)")"

echo ""
echo "5. REJECTION + REWORK CYCLE"
echo "───────────────────────────"

JOB2=$(curl -sf -X POST "$BASE/jobs" -H 'Content-Type: application/json' -d '{"title":"rework test"}')
J2=$(echo "$JOB2" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
curl -sf -X POST "$BASE/jobs/$J2/assign" -H 'Content-Type: application/json' -d '{"agent_id":"agent-ethdev"}' > /dev/null
curl -sf -X POST "$BASE/jobs/$J2/start" > /dev/null
curl -sf -X POST "$BASE/jobs/$J2/submit" -H 'Content-Type: application/json' -d '{"result_summary":"first attempt"}' > /dev/null

# Reject
R=$(curl -sf -X POST "$BASE/jobs/$J2/evaluate" -H 'Content-Type: application/json' -d '{"accepted":false,"feedback":"needs work"}')
check "reject → in_progress" "in_progress" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])")"

# Resubmit
R=$(curl -sf -X POST "$BASE/jobs/$J2/submit" -H 'Content-Type: application/json' -d '{"result_summary":"second attempt"}')
check "resubmit → submitted" "submitted" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])")"

# Accept
R=$(curl -sf -X POST "$BASE/jobs/$J2/evaluate" -H 'Content-Type: application/json' -d '{"accepted":true,"feedback":"LGTM"}')
check "accept → completed" "completed" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])")"

echo ""
echo "6. JOB CLI COMMANDS"
echo "───────────────────"

ROKO_DIR="$(dirname "$0")/../../.."
cd "$ROKO_DIR"

$ROKO job list --quiet 2>/dev/null
check "job list exits 0" "0" "$?"

$ROKO job create "CLI test job" --type coding_task --description "Test from CLI" 2>/dev/null
check "job create exits 0" "0" "$?"

SERVE_HOST="${BASE%/api}"
M=$($ROKO job match "Build relay" --skills rust --reward "1000 KORAI" --serve-url "$SERVE_HOST" 2>&1)
check "job match exits 0" "0" "$?"
echo "$M" | grep -q "agent-rustsmith" && check "match finds rustsmith" "yes" "yes" || check "match finds rustsmith" "yes" "no"

echo ""
echo "7. EDGE CASES"
echo "─────────────"

# Cancel
JOB3=$(curl -sf -X POST "$BASE/jobs" -H 'Content-Type: application/json' -d '{"title":"cancel test"}')
J3=$(echo "$JOB3" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
R=$(curl -sf -X POST "$BASE/jobs/$J3/cancel")
check "cancel → cancelled" "cancelled" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['state'])")"

# Double cancel
R=$(curl -s -X POST "$BASE/jobs/$J3/cancel")
check "double cancel → 422" "unprocessable_entity" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin).get('code',''))")"

# Nonexistent job
R=$(curl -s "$BASE/jobs/nonexistent")
check "get missing job → 404" "not_found" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin).get('code',''))")"

# Numeric reward accepted
R=$(curl -sf -X POST "$BASE/jobs" -H 'Content-Type: application/json' -d '{"title":"numeric","reward":1500}')
check "numeric reward → string" "str" "$(echo "$R" | python3 -c "import sys,json; print(type(json.load(sys.stdin)['reward']).__name__)")"
check "numeric reward value" "1500" "$(echo "$R" | python3 -c "import sys,json; print(json.load(sys.stdin)['reward'])")"

# Stats
R=$(curl -sf "$BASE/jobs/stats")
check "stats has total" "True" "$(echo "$R" | python3 -c "import sys,json; print('total' in json.load(sys.stdin))")"

echo ""
echo "═══════════════════════════════════════════"
echo "  RESULTS: $PASS passed, $FAIL failed"
echo "═══════════════════════════════════════════"

[ "$FAIL" -eq 0 ] && exit 0 || exit 1
