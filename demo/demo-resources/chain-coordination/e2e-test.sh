#!/usr/bin/env bash
# e2e-test.sh — CI-grade automated assertions for the chain-coordination demo.
#
# Prerequisites: mirage-rs running on RPC_URL (default http://127.0.0.1:8545)
#                with contracts deployed at expected addresses.
#
# Usage: bash e2e-test.sh [rpc-url]

set -euo pipefail

source "$(dirname "$0")/common.sh"

require_cast
require_curl
require_python

RPC_URL="${1:-$RPC_URL}"
PASS=0
FAIL=0

check() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        echo -e "  ${GREEN}PASS${NC} $label"
        PASS=$((PASS + 1))
    else
        echo -e "  ${RED}FAIL${NC} $label -- expected '$expected', got '$actual'"
        FAIL=$((FAIL + 1))
    fi
}

check_nonzero() {
    local label="$1" actual="$2"
    if [ -n "$actual" ] && [ "$actual" != "0" ] && [ "$actual" != "null" ] && [ "$actual" != "" ]; then
        echo -e "  ${GREEN}PASS${NC} $label (=$actual)"
        PASS=$((PASS + 1))
    else
        echo -e "  ${RED}FAIL${NC} $label -- expected nonzero, got '$actual'"
        FAIL=$((FAIL + 1))
    fi
}

echo "======================================================="
echo "  CHAIN COORDINATION E2E TEST SUITE"
echo "  RPC: $RPC_URL"
echo "======================================================="

# ═══════════════════════════════════════════════════════════
echo ""
echo "1. AGENT REGISTRY (ON-CHAIN)"
echo "----------------------------"

# Register 3 agents
PASSPORT1=$(cast keccak "e2e-test-researcher")
cast_send "$AGENT_REGISTRY" "register(string,bytes32)" \
    "researcher;analysis" "$PASSPORT1" \
    --private-key "$DEPLOYER_PK" > /dev/null 2>&1

PASSPORT2=$(cast keccak "e2e-test-coder")
cast_send "$AGENT_REGISTRY" "register(string,bytes32)" \
    "coder;rust,solidity" "$PASSPORT2" \
    --private-key "$ACCOUNT1_PK" > /dev/null 2>&1

PASSPORT3=$(cast keccak "e2e-test-sentinel")
cast_send "$AGENT_REGISTRY" "register(string,bytes32)" \
    "sentinel;monitoring" "$PASSPORT3" \
    --private-key "$ACCOUNT2_PK" > /dev/null 2>&1

COUNT=$(cast_call "$AGENT_REGISTRY" "registeredCount()(uint256)")
check "registeredCount >= 3" "True" "$(python3 -c "print(int('$COUNT') >= 3)")"

# Check isActive after heartbeat
cast_send "$AGENT_REGISTRY" "heartbeat()" --private-key "$DEPLOYER_PK" > /dev/null 2>&1
ACTIVE=$(cast_call "$AGENT_REGISTRY" "isActive(address)(bool)" "$DEPLOYER_ADDR")
check "deployer isActive after heartbeat" "true" "$ACTIVE"

cast_send "$AGENT_REGISTRY" "heartbeat()" --private-key "$ACCOUNT1_PK" > /dev/null 2>&1
ACTIVE=$(cast_call "$AGENT_REGISTRY" "isActive(address)(bool)" "$ACCOUNT1_ADDR")
check "account1 isActive after heartbeat" "true" "$ACTIVE"

# ═══════════════════════════════════════════════════════════
echo ""
echo "2. CHAIN EXTENSION: AGENT REGISTRATION"
echo "---------------------------------------"

R=$(chain_rpc "chain_registerAgent" "[\"e2e-researcher\", \"$DEPLOYER_ADDR\", \"researcher\"]")
check "chain_registerAgent(e2e-researcher)" "true" "$R"

# Duplicate should return false
R=$(chain_rpc "chain_registerAgent" "[\"e2e-researcher\", \"$DEPLOYER_ADDR\", \"researcher\"]")
check "duplicate registration returns false" "false" "$R"

R=$(chain_rpc "chain_registerAgent" "[\"e2e-coder\", \"$ACCOUNT1_ADDR\", \"coder\"]")
check "chain_registerAgent(e2e-coder)" "true" "$R"

# ═══════════════════════════════════════════════════════════
echo ""
echo "3. CHAIN EXTENSION: HEARTBEATS"
echo "------------------------------"

R=$(chain_rpc "chain_agentHeartbeat" "[\"e2e-researcher\"]")
check "heartbeat(e2e-researcher)" "true" "$R"

R=$(chain_rpc "chain_agentHeartbeat" "[\"e2e-coder\"]")
check "heartbeat(e2e-coder)" "true" "$R"

# Heartbeat for nonexistent agent
R=$(chain_rpc "chain_agentHeartbeat" "[\"nonexistent-agent\"]")
check "heartbeat(nonexistent) returns false" "false" "$R"

# ═══════════════════════════════════════════════════════════
echo ""
echo "4. WORKER REGISTRY + BOUNTY POSTING"
echo "------------------------------------"

BOND=$(ether_to_wei 1000)
MINT=$(ether_to_wei 100000)

# Mint + register workers
for PK in "$DEPLOYER_PK" "$ACCOUNT1_PK" "$ACCOUNT2_PK"; do
    ADDR=$(cast wallet address "$PK")
    cast_send "$DAEJI" "mint(address,uint256)" "$ADDR" "$MINT" \
        --private-key "$DEPLOYER_PK" > /dev/null 2>&1
    cast_send "$DAEJI" "approve(address,uint256)" "$WORKER_REGISTRY" "$BOND" \
        --private-key "$PK" > /dev/null 2>&1
    cast_send "$WORKER_REGISTRY" "register(uint256)" "$BOND" \
        --private-key "$PK" > /dev/null 2>&1 || true
done

WORKER_COUNT=$(cast_call "$WORKER_REGISTRY" "registeredCount()(uint256)")
check "workerRegistry count >= 3" "True" "$(python3 -c "print(int('$WORKER_COUNT') >= 3)")"

# Post a bounty
BOUNTY_AMT=$(ether_to_wei 5)
DEADLINE=$(future_deadline 7200)
cast_send "$DAEJI" "approve(address,uint256)" "$BOUNTY_MARKET" "$BOUNTY_AMT" \
    --private-key "$DEPLOYER_PK" > /dev/null 2>&1
SPEC=$(cast keccak "e2e-test-bounty")
cast_send "$BOUNTY_MARKET" \
    "postJob(bytes32,uint256,uint64,uint8)(uint256)" \
    "$SPEC" "$BOUNTY_AMT" "$DEADLINE" 2 \
    --private-key "$DEPLOYER_PK" > /dev/null 2>&1

NEXT_ID=$(cast_call "$BOUNTY_MARKET" "nextJobId()(uint256)")
JOB_ID=$(( NEXT_ID - 1 ))
STATE=$(cast_call "$BOUNTY_MARKET" "stateOf(uint256)(uint8)" "$JOB_ID")
check "bounty posted (state=Funded)" "2" "$STATE"

# ═══════════════════════════════════════════════════════════
echo ""
echo "5. BOUNTY LIFECYCLE STATE MACHINE"
echo "----------------------------------"

# Assign
cast_send "$BOUNTY_MARKET" "assign(uint256,address)" "$JOB_ID" "$ACCOUNT1_ADDR" \
    --private-key "$DEPLOYER_PK" > /dev/null 2>&1
STATE=$(cast_call "$BOUNTY_MARKET" "stateOf(uint256)(uint8)" "$JOB_ID")
check "assign -> Assigned" "3" "$STATE"

# Submit
RESULT_HASH=$(cast keccak "e2e-result-hash")
cast_send "$BOUNTY_MARKET" "submit(uint256,bytes32)" "$JOB_ID" "$RESULT_HASH" \
    --private-key "$ACCOUNT1_PK" > /dev/null 2>&1
STATE=$(cast_call "$BOUNTY_MARKET" "stateOf(uint256)(uint8)" "$JOB_ID")
check "submit -> Submitted" "4" "$STATE"

# Resolve (deployer is default resolver)
cast_send "$BOUNTY_MARKET" "resolve(uint256,bool)" "$JOB_ID" true \
    --private-key "$DEPLOYER_PK" > /dev/null 2>&1
STATE=$(cast_call "$BOUNTY_MARKET" "stateOf(uint256)(uint8)" "$JOB_ID")
check "resolve(accept) -> Terminal" "5" "$STATE"

# Worker reputation should have changed
REP=$(cast_call "$WORKER_REGISTRY" "reputationOf(address)(uint256)" "$ACCOUNT1_ADDR")
check_nonzero "worker reputation after success" "$REP"

# ═══════════════════════════════════════════════════════════
echo ""
echo "6. INSIGHTS (CHAIN EXTENSION)"
echo "-----------------------------"

R=$(chain_rpc "chain_postInsight" '[{"author":"e2e-agent","kind":"insight","content":"Test insight for e2e validation of chain coordination"}]')
OUTCOME=$(echo "$R" | python3 -c "import sys,json; print(json.loads(sys.stdin.read()).get('outcome',''))")
check "postInsight outcome=accepted" "accepted" "$OUTCOME"

INSIGHT_ID=$(echo "$R" | python3 -c "import sys,json; print(json.loads(sys.stdin.read()).get('id',''))")
check_nonzero "postInsight returns id" "$INSIGHT_ID"

# Search should find it
SEARCH=$(chain_rpc "chain_searchInsights" '[{"query":"e2e validation chain coordination","k":5}]')
RESULT_COUNT=$(echo "$SEARCH" | python3 -c "
import sys,json
data = json.loads(sys.stdin.read())
results = data.get('results', data) if isinstance(data, dict) else data
print(len(results) if isinstance(results, list) else 0)
")
check "searchInsights finds >= 1 result" "True" "$(python3 -c "print(int('$RESULT_COUNT') >= 1)")"

# ═══════════════════════════════════════════════════════════
echo ""
echo "7. PHEROMONES (CHAIN EXTENSION)"
echo "-------------------------------"

R=$(chain_rpc "chain_depositPheromone" '[{"kind":"THREAT","content":"E2E test threat pheromone","intensity":0.8,"halfLifeSeconds":3600}]')
PHER_ID=$(echo "$R" | python3 -c "import sys,json; print(json.loads(sys.stdin.read()).get('id',''))")
check_nonzero "depositPheromone returns id" "$PHER_ID"

R=$(chain_rpc "chain_depositPheromone" '[{"kind":"OPPORTUNITY","content":"E2E test opportunity pheromone","intensity":0.6}]')
PHER_ID2=$(echo "$R" | python3 -c "import sys,json; print(json.loads(sys.stdin.read()).get('id',''))")
check_nonzero "second pheromone deposited" "$PHER_ID2"

# Query pheromones
QUERY=$(chain_rpc "chain_queryPheromones" '[{"query":"e2e test threat","k":5}]')
PHER_COUNT=$(echo "$QUERY" | python3 -c "
import sys,json
data = json.loads(sys.stdin.read())
results = data if isinstance(data, list) else data.get('results', [])
print(len(results))
")
check "queryPheromones finds >= 1" "True" "$(python3 -c "print(int('$PHER_COUNT') >= 1)")"

# ═══════════════════════════════════════════════════════════
echo ""
echo "8. CHAIN STATS"
echo "--------------"

STATS=$(chain_rpc "chain_stats" '[{}]')
HAS_INSIGHTS=$(echo "$STATS" | python3 -c "import sys,json; d=json.loads(sys.stdin.read()); print('insights' in d)")
check "chain_stats has insights key" "True" "$HAS_INSIGHTS"
HAS_PHEROMONES=$(echo "$STATS" | python3 -c "import sys,json; d=json.loads(sys.stdin.read()); print('pheromones' in d)")
check "chain_stats has pheromones key" "True" "$HAS_PHEROMONES"

# ═══════════════════════════════════════════════════════════
echo ""
echo "9. AGENT STATS (CHAIN EXTENSION)"
echo "---------------------------------"

R=$(chain_rpc "chain_agentStats" "[\"e2e-researcher\", {\"tasks_completed\":2,\"delta_cycles\":5,\"total_cost_usd\":0.05,\"total_tokens\":2000,\"confirmations_given\":1,\"challenges_given\":0,\"warnings_posted\":0,\"insights_posted\":1,\"tasks_failed\":0}]")
check "agentStats update returns true" "true" "$R"

# Second delta accumulates
R=$(chain_rpc "chain_agentStats" "[\"e2e-researcher\", {\"tasks_completed\":1,\"delta_cycles\":2,\"total_cost_usd\":0.01,\"total_tokens\":500,\"confirmations_given\":0,\"challenges_given\":0,\"warnings_posted\":0,\"insights_posted\":0,\"tasks_failed\":0}]")
check "agentStats second delta returns true" "true" "$R"

# Stats for nonexistent agent
R=$(chain_rpc "chain_agentStats" "[\"nonexistent\", {\"tasks_completed\":1,\"delta_cycles\":1,\"total_cost_usd\":0.0,\"total_tokens\":0,\"confirmations_given\":0,\"challenges_given\":0,\"warnings_posted\":0,\"insights_posted\":0,\"tasks_failed\":0}]")
check "agentStats(nonexistent) returns false" "false" "$R"

# ═══════════════════════════════════════════════════════════
echo ""
echo "======================================================="
echo "  RESULTS: $PASS passed, $FAIL failed"
echo "======================================================="

[ "$FAIL" -eq 0 ] && exit 0 || exit 1
