#!/usr/bin/env bash
# 05-multi-agent-coordination.sh — Multi-agent concurrent coordination:
#   heartbeats, bounty claims, concurrent submissions, stats reporting.
source "$(dirname "$0")/common.sh"

require_cast
require_curl
require_python

header "Multi-Agent Coordination"

# ── 1. Register agents via chain extension ────────────────────────────────────
info "Registering 3 agents via chain_registerAgent..."
chain_rpc "chain_registerAgent" "[\"coordinator\", \"$DEPLOYER_ADDR\", \"coordinator\"]" > /dev/null
chain_rpc "chain_registerAgent" "[\"worker-alpha\", \"$ACCOUNT1_ADDR\", \"coder\"]" > /dev/null
chain_rpc "chain_registerAgent" "[\"worker-beta\", \"$ACCOUNT2_ADDR\", \"analyst\"]" > /dev/null
ok "3 agents registered"

# ── 2. Heartbeat burst — all agents heartbeating concurrently ─────────────────
echo
header "Concurrent Heartbeats"
info "Sending heartbeat bursts (5 rounds)..."

for ROUND in $(seq 1 5); do
    chain_rpc "chain_agentHeartbeat" "[\"coordinator\"]" > /dev/null &
    chain_rpc "chain_agentHeartbeat" "[\"worker-alpha\"]" > /dev/null &
    chain_rpc "chain_agentHeartbeat" "[\"worker-beta\"]" > /dev/null &
    wait
    dim "  Round $ROUND: all 3 agents heartbeated"

    # Mine a block to advance the chain
    cast_send "$DEPLOYER_ADDR" --value 0 --private-key "$DEPLOYER_PK" > /dev/null 2>&1 || true
done
ok "15 heartbeats sent across 5 rounds"

# ── 3. Setup: ensure workers are registered and funded for bounties ───────────
echo
header "Bounty Setup"
BOND=$(ether_to_wei 1000)
MINT=$(ether_to_wei 100000)
DEADLINE=$(future_deadline 7200)

for PK in "$DEPLOYER_PK" "$ACCOUNT1_PK" "$ACCOUNT2_PK"; do
    ADDR=$(cast wallet address "$PK")
    cast_send "$DAEJI" "mint(address,uint256)" "$ADDR" "$MINT" \
        --private-key "$DEPLOYER_PK" > /dev/null
    cast_send "$DAEJI" "approve(address,uint256)" "$WORKER_REGISTRY" "$BOND" \
        --private-key "$PK" > /dev/null
    cast_send "$WORKER_REGISTRY" "register(uint256)" "$BOND" \
        --private-key "$PK" > /dev/null 2>&1 || true
done
ok "Workers registered and funded"

# Ensure deployer is resolver
cast_send "$BOUNTY_MARKET" "setResolver(address)" "$DEPLOYER_ADDR" \
    --private-key "$DEPLOYER_PK" > /dev/null 2>&1 || true

# ── 4. Post 3 bounties for different agents to claim ─────────────────────────
echo
header "Concurrent Bounty Claims"
info "Posting 3 bounties..."

APPROVAL=$(ether_to_wei 50000)
cast_send "$DAEJI" "approve(address,uint256)" "$BOUNTY_MARKET" "$APPROVAL" \
    --private-key "$DEPLOYER_PK" > /dev/null

BOUNTY_AMT=$(ether_to_wei 5)
for I in $(seq 1 3); do
    SPEC=$(cast keccak "multi-agent-task-$I")
    cast_send "$BOUNTY_MARKET" \
        "postJob(bytes32,uint256,uint64,uint8)(uint256)" \
        "$SPEC" "$BOUNTY_AMT" "$DEADLINE" 2 \
        --private-key "$DEPLOYER_PK" > /dev/null
done

NEXT_ID=$(cast_call "$BOUNTY_MARKET" "nextJobId()(uint256)")
BASE_ID=$(( NEXT_ID - 3 ))
ok "3 bounties posted (IDs: $BASE_ID, $((BASE_ID+1)), $((BASE_ID+2)))"

# ── 5. Different agents claim different bounties ──────────────────────────────
echo
info "Agent-coder claims bounty #$BASE_ID..."
cast_send "$BOUNTY_MARKET" "assign(uint256,address)" "$BASE_ID" "$ACCOUNT1_ADDR" \
    --private-key "$DEPLOYER_PK" > /dev/null
ok "Bounty #$BASE_ID assigned to agent-coder"

info "Agent-sentinel claims bounty #$((BASE_ID+1))..."
cast_send "$BOUNTY_MARKET" "assign(uint256,address)" "$((BASE_ID+1))" "$ACCOUNT2_ADDR" \
    --private-key "$DEPLOYER_PK" > /dev/null
ok "Bounty #$((BASE_ID+1)) assigned to agent-sentinel"

info "Agent-coordinator claims bounty #$((BASE_ID+2))..."
cast_send "$BOUNTY_MARKET" "assign(uint256,address)" "$((BASE_ID+2))" "$DEPLOYER_ADDR" \
    --private-key "$DEPLOYER_PK" > /dev/null
ok "Bounty #$((BASE_ID+2)) assigned to agent-coordinator"

# ── 6. Concurrent result submissions ─────────────────────────────────────────
echo
header "Concurrent Submissions"
info "All 3 workers submitting results concurrently..."

HASH1=$(cast keccak "result-coder-task-1")
HASH2=$(cast keccak "result-sentinel-task-2")
HASH3=$(cast keccak "result-coordinator-task-3")

# Submit in parallel
cast_send "$BOUNTY_MARKET" "submit(uint256,bytes32)" "$BASE_ID" "$HASH1" \
    --private-key "$ACCOUNT1_PK" > /dev/null &
PID1=$!

cast_send "$BOUNTY_MARKET" "submit(uint256,bytes32)" "$((BASE_ID+1))" "$HASH2" \
    --private-key "$ACCOUNT2_PK" > /dev/null &
PID2=$!

cast_send "$BOUNTY_MARKET" "submit(uint256,bytes32)" "$((BASE_ID+2))" "$HASH3" \
    --private-key "$DEPLOYER_PK" > /dev/null &
PID3=$!

wait $PID1 $PID2 $PID3
ok "All 3 results submitted concurrently"

# Verify all in Submitted state
for ID in "$BASE_ID" "$((BASE_ID+1))" "$((BASE_ID+2))"; do
    STATE=$(cast_call "$BOUNTY_MARKET" "stateOf(uint256)(uint8)" "$ID")
    dim "  Job #$ID state = $STATE (4 = Submitted)"
done

# ── 7. Resolve all bounties ──────────────────────────────────────────────────
echo
header "Resolve All"
info "Resolver accepting all 3 submissions..."

for ID in "$BASE_ID" "$((BASE_ID+1))" "$((BASE_ID+2))"; do
    cast_send "$BOUNTY_MARKET" "resolve(uint256,bool)" "$ID" true \
        --private-key "$DEPLOYER_PK" > /dev/null
    STATE=$(cast_call "$BOUNTY_MARKET" "stateOf(uint256)(uint8)" "$ID")
    ok "Job #$ID resolved (state=$STATE, 5=Terminal)"
done

# ── 8. Report agent stats via chain extension ────────────────────────────────
echo
header "Agent Stats"
info "Reporting stats deltas via chain_agentStats..."

STATS1=$(chain_rpc "chain_agentStats" "[\"coordinator\", {\"tasks_completed\":1,\"delta_cycles\":5,\"total_cost_usd\":0.02,\"total_tokens\":1500,\"confirmations_given\":0,\"challenges_given\":0,\"warnings_posted\":0,\"insights_posted\":0,\"tasks_failed\":0}]")
ok "coordinator stats updated: $STATS1"

STATS2=$(chain_rpc "chain_agentStats" "[\"worker-alpha\", {\"tasks_completed\":1,\"delta_cycles\":3,\"total_cost_usd\":0.01,\"total_tokens\":800,\"confirmations_given\":0,\"challenges_given\":0,\"warnings_posted\":0,\"insights_posted\":0,\"tasks_failed\":0}]")
ok "worker-alpha stats updated: $STATS2"

STATS3=$(chain_rpc "chain_agentStats" "[\"worker-beta\", {\"tasks_completed\":1,\"delta_cycles\":4,\"total_cost_usd\":0.015,\"total_tokens\":1200,\"confirmations_given\":0,\"challenges_given\":0,\"warnings_posted\":0,\"insights_posted\":0,\"tasks_failed\":0}]")
ok "worker-beta stats updated: $STATS3"

# ── 9. Final heartbeats + liveness check ──────────────────────────────────────
echo
header "Final Liveness Check"
for AGENT in "coordinator" "worker-alpha" "worker-beta"; do
    chain_rpc "chain_agentHeartbeat" "[\"$AGENT\"]" > /dev/null
done
ok "Final heartbeats sent"

# On-chain liveness via contract
for ADDR in "$DEPLOYER_ADDR" "$ACCOUNT1_ADDR" "$ACCOUNT2_ADDR"; do
    ACTIVE=$(cast_call "$AGENT_REGISTRY" "isActive(address)(bool)" "$ADDR")
    dim "  $ADDR isActive = $ACTIVE"
done

header "Done"
ok "Multi-agent coordination complete: 3 agents, 3 bounties, concurrent execution"
