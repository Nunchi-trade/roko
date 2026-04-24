#!/bin/bash
# Seed demo agents into a running roko-serve instance.
# Usage: bash seed-agents.sh [--chain] [base-url]

set -euo pipefail

CHAIN=false
if [ "${1:-}" = "--chain" ]; then
    CHAIN=true
    shift
fi

BASE="${1:-http://127.0.0.1:6677}/api"

echo "Seeding agents → $BASE"
echo ""

register() {
    local data="$1"
    local name
    name=$(echo "$data" | python3 -c "import sys,json; print(json.load(sys.stdin)['agent_id'])")
    if curl -sf -X POST "$BASE/agents/register" \
        -H 'Content-Type: application/json' \
        -d "$data" > /dev/null 2>&1; then
        echo "  ✓ $name"
    else
        echo "  ✗ $name (failed — is roko serve running?)"
        return 1
    fi
}

register '{
    "agent_id": "agent-rustsmith",
    "label": "rustsmith",
    "capabilities": ["messaging", "tasks"],
    "skills": ["rust", "p2p", "eth", "networking"],
    "tier": "Expert",
    "reputation": 94,
    "past_jobs_completed": 37,
    "max_concurrent_jobs": 5
}'

register '{
    "agent_id": "agent-ethdev",
    "label": "ethdev",
    "capabilities": ["messaging", "tasks"],
    "skills": ["solidity", "eth", "defi", "evm"],
    "tier": "Trusted",
    "reputation": 82,
    "past_jobs_completed": 21,
    "max_concurrent_jobs": 3
}'

register '{
    "agent_id": "agent-fullstack",
    "label": "fullstack",
    "capabilities": ["messaging", "tasks"],
    "skills": ["typescript", "react", "rust", "graphql"],
    "tier": "Verified",
    "reputation": 75,
    "past_jobs_completed": 18,
    "max_concurrent_jobs": 4
}'

register '{
    "agent_id": "agent-researcher",
    "label": "researcher",
    "capabilities": ["messaging", "research"],
    "skills": ["defi", "tokenomics", "governance", "analysis"],
    "tier": "Expert",
    "reputation": 91,
    "past_jobs_completed": 45,
    "max_concurrent_jobs": 6
}'

register '{
    "agent_id": "agent-auditor",
    "label": "auditor",
    "capabilities": ["messaging", "tasks"],
    "skills": ["solidity", "security", "audit", "evm"],
    "tier": "Expert",
    "reputation": 97,
    "past_jobs_completed": 52,
    "max_concurrent_jobs": 2
}'

echo ""
echo "Done. Verify: curl -s $BASE/managed-agents | python3 -m json.tool"

# ─── Optional on-chain registration (mirage) ──────────────────────────────────
RPC_URL="http://127.0.0.1:8545"
REGISTRY="0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"
PRIVATE_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
ZERO_HASH="0x0000000000000000000000000000000000000000000000000000000000000000"

if [ "$CHAIN" = true ]; then
    if cast block-number --rpc-url "$RPC_URL" 2>/dev/null; then
        echo ""
        echo "Mirage detected — registering agents on-chain..."

        # Contract-level registration via cast send
        AGENT_NAMES=("agent-rustsmith" "agent-ethdev" "agent-fullstack" "agent-researcher" "agent-auditor")
        AGENT_ROLES=("rust-engineer" "solidity-engineer" "fullstack-engineer" "researcher" "auditor")

        for i in "${!AGENT_NAMES[@]}"; do
            NAME="${AGENT_NAMES[$i]}"
            ROLE="${AGENT_ROLES[$i]}"
            if cast send "$REGISTRY" \
                "register(string,bytes32)" "$NAME" "$ZERO_HASH" \
                --private-key "$PRIVATE_KEY" \
                --rpc-url "$RPC_URL" > /dev/null 2>&1; then
                echo "  [contract] $NAME registered"
            else
                echo "  [contract] $NAME skipped (already registered or tx failed)"
            fi

            # JSON-RPC extension: register with address and role
            RESULT=$(curl -sf -X POST "$RPC_URL" \
                -H 'Content-Type: application/json' \
                -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"chain_registerAgent\",\"params\":[\"$NAME\",\"0xdead\",\"$ROLE\"]}" \
                2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('result','error'))" 2>/dev/null || echo "error")
            if [ "$RESULT" = "True" ] || [ "$RESULT" = "true" ]; then
                echo "  [rpc]      $NAME registered (role=$ROLE)"
            else
                echo "  [rpc]      $NAME skipped (duplicate or error)"
            fi
        done

        COUNT=$(cast call "$REGISTRY" "registeredCount()(uint256)" --rpc-url "$RPC_URL" 2>/dev/null || echo "?")
        echo ""
        echo "On-chain registeredCount: $COUNT"
    else
        echo ""
        echo "Mirage not running — skipping on-chain registration."
        echo "Start mirage first, or omit --chain."
    fi
else
    echo ""
    echo "(Pass --chain to also register agents on-chain via mirage)"
fi
