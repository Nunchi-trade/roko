#!/bin/bash
# Seed demo agents into a running roko-serve instance.
# Usage: bash seed-agents.sh [base-url]

set -euo pipefail

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
