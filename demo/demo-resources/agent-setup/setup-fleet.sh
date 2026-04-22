#!/bin/bash
# Create a fleet of agents via CLI and register them with roko-serve.
# Usage: bash setup-fleet.sh [serve-url]
# Requires: roko init already done, roko serve running

set -euo pipefail

ROKO="${ROKO:-roko}"
BASE="${1:-http://127.0.0.1:6677}/api"

echo "═══════════════════════════════════════════"
echo "  AGENT FLEET SETUP"
echo "═══════════════════════════════════════════"

echo ""
echo "1. Creating agent manifests..."

$ROKO agent create --name rust-dev --domain coding --prompt "You are a senior Rust developer specializing in systems programming, networking, and performance optimization." 2>&1 | head -1 || true
$ROKO agent create --name researcher --domain research --prompt "You are a thorough researcher who synthesizes information from multiple sources with citations." 2>&1 | head -1 || true
$ROKO agent create --name auditor --domain coding --prompt "You are a security auditor. Review code for vulnerabilities, unsafe patterns, and correctness issues." 2>&1 | head -1 || true

echo ""
echo "2. Registering with roko-serve for matchmaking..."

register() {
    local data="$1"
    local name
    name=$(echo "$data" | python3 -c "import sys,json; print(json.load(sys.stdin)['agent_id'])")
    curl -sf -X POST "$BASE/agents/register" -H 'Content-Type: application/json' -d "$data" > /dev/null 2>&1 \
        && echo "  ✓ $name" || echo "  ✗ $name"
}

register '{"agent_id":"rust-dev","label":"Rust Developer","capabilities":["messaging","tasks"],"skills":["rust","systems","networking","performance"],"tier":"Expert","reputation":92,"past_jobs_completed":34,"max_concurrent_jobs":4}'
register '{"agent_id":"researcher","label":"Research Analyst","capabilities":["messaging","research"],"skills":["defi","tokenomics","analysis","citations"],"tier":"Expert","reputation":88,"past_jobs_completed":41,"max_concurrent_jobs":6}'
register '{"agent_id":"auditor","label":"Security Auditor","capabilities":["messaging","tasks"],"skills":["security","audit","rust","solidity"],"tier":"Expert","reputation":96,"past_jobs_completed":28,"max_concurrent_jobs":2}'

echo ""
echo "3. Fleet status:"
$ROKO agent list 2>&1

echo ""
echo "4. Serve fleet:"
curl -sf "$BASE/managed-agents" | python3 -c "
import sys, json
agents = json.load(sys.stdin)
print(f'   {len(agents)} agent(s) visible to dashboard')
for a in agents:
    print(f'   • {a[\"id\"]:20} {a.get(\"tier\",\"—\"):10} {a.get(\"status\",\"?\")}')
"

echo ""
echo "═══════════════════════════════════════════"
echo "  Fleet ready. Open dashboard → Network → Agents"
echo "═══════════════════════════════════════════"
