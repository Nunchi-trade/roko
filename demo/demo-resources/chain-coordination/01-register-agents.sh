#!/usr/bin/env bash
# 01-register-agents.sh — Register 3 agents on-chain via AgentRegistry + chain_registerAgent.
source "$(dirname "$0")/common.sh"

require_cast
require_curl
require_python

header "Agent Registration (On-Chain + Chain Extension)"

# ── 1. On-chain registration via AgentRegistry.register(string,bytes32) ──────
info "Registering agents via AgentRegistry contract at $AGENT_REGISTRY"
echo

# Agent 1: researcher (Account #0)
PASSPORT1=$(cast keccak "researcher-passport-v1")
info "Registering agent-researcher (Account #0)..."
cast_send "$AGENT_REGISTRY" "register(string,bytes32)" \
    "researcher;rust,analysis,defi" "$PASSPORT1" \
    --private-key "$DEPLOYER_PK" > /dev/null
ok "agent-researcher registered (passport: ${PASSPORT1:0:18}...)"

# Agent 2: coder (Account #1)
PASSPORT2=$(cast keccak "coder-passport-v1")
info "Registering agent-coder (Account #1)..."
cast_send "$AGENT_REGISTRY" "register(string,bytes32)" \
    "coder;rust,solidity,typescript" "$PASSPORT2" \
    --private-key "$ACCOUNT1_PK" > /dev/null
ok "agent-coder registered (passport: ${PASSPORT2:0:18}...)"

# Agent 3: sentinel (Account #2)
PASSPORT3=$(cast keccak "sentinel-passport-v1")
info "Registering agent-sentinel (Account #2)..."
cast_send "$AGENT_REGISTRY" "register(string,bytes32)" \
    "sentinel;monitoring,alerts,risk" "$PASSPORT3" \
    --private-key "$ACCOUNT2_PK" > /dev/null
ok "agent-sentinel registered (passport: ${PASSPORT3:0:18}...)"

# ── 2. Verify on-chain count ─────────────────────────────────────────────────
echo
info "Verifying registeredCount()..."
COUNT=$(cast_call "$AGENT_REGISTRY" "registeredCount()(uint256)")
ok "AgentRegistry.registeredCount() = $COUNT"

# ── 3. Verify individual agent data ──────────────────────────────────────────
echo
info "Querying agent data..."
for ADDR in "$DEPLOYER_ADDR" "$ACCOUNT1_ADDR" "$ACCOUNT2_ADDR"; do
    AGENT_DATA=$(cast_call "$AGENT_REGISTRY" "getAgent(address)((string,bytes32,uint64,uint64,bool))" "$ADDR")
    dim "  $ADDR => $AGENT_DATA"
done

# ── 4. Heartbeats ────────────────────────────────────────────────────────────
echo
info "Sending initial heartbeats..."
cast_send "$AGENT_REGISTRY" "heartbeat()" --private-key "$DEPLOYER_PK" > /dev/null
cast_send "$AGENT_REGISTRY" "heartbeat()" --private-key "$ACCOUNT1_PK" > /dev/null
cast_send "$AGENT_REGISTRY" "heartbeat()" --private-key "$ACCOUNT2_PK" > /dev/null
ok "All 3 agents heartbeated"

# ── 5. Liveness check ────────────────────────────────────────────────────────
echo
info "Checking liveness..."
for ADDR in "$DEPLOYER_ADDR" "$ACCOUNT1_ADDR" "$ACCOUNT2_ADDR"; do
    ACTIVE=$(cast_call "$AGENT_REGISTRY" "isActive(address)(bool)" "$ADDR")
    dim "  $ADDR isActive = $ACTIVE"
done

# ── 6. Register via chain_registerAgent JSON-RPC extension ───────────────────
header "Chain Extension Registration"
info "Registering agents via chain_registerAgent RPC..."
echo

RESULT=$(chain_rpc "chain_registerAgent" "[\"agent-researcher\", \"$DEPLOYER_ADDR\", \"researcher\"]")
ok "chain_registerAgent(agent-researcher) = $RESULT"

RESULT=$(chain_rpc "chain_registerAgent" "[\"agent-coder\", \"$ACCOUNT1_ADDR\", \"coder\"]")
ok "chain_registerAgent(agent-coder) = $RESULT"

RESULT=$(chain_rpc "chain_registerAgent" "[\"agent-sentinel\", \"$ACCOUNT2_ADDR\", \"sentinel\"]")
ok "chain_registerAgent(agent-sentinel) = $RESULT"

header "Done"
ok "3 agents registered on-chain + 3 registered via chain extension"
