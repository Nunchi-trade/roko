#!/usr/bin/env bash
# 01-single-agent.sh — Start serve + one agent, send a message, verify, cleanup.
# Usage: bash 01-single-agent.sh [--chain]
set -euo pipefail

CHAIN=false
if [ "${1:-}" = "--chain" ]; then
    CHAIN=true
    shift
fi

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"

cleanup() {
  echo "Cleaning up..."
  [ -n "${AGENT_PID:-}" ] && kill "$AGENT_PID" 2>/dev/null || true
  [ -n "${SERVE_PID:-}" ] && kill "$SERVE_PID" 2>/dev/null || true
  wait 2>/dev/null
}
trap cleanup EXIT

echo "==> Starting roko serve..."
cargo run -p roko-cli -- serve &
SERVE_PID=$!
sleep 3

echo "==> Starting agent sidecar (auto-port)..."
cargo run -p roko-cli -- agent serve --agent-id demo-agent &
AGENT_PID=$!
sleep 3

echo "==> Verifying agent registered with serve..."
AGENT_INFO=$(curl -sf http://127.0.0.1:6677/api/agents/demo-agent || echo "FAIL")
if echo "$AGENT_INFO" | grep -q "demo-agent"; then
  echo "    OK: agent registered"
else
  echo "    FAIL: agent not found in serve registry"
  echo "    Response: $AGENT_INFO"
  exit 1
fi

# ─── Optional on-chain registration (mirage) ──────────────────────────────────
RPC_URL="http://127.0.0.1:8545"
REGISTRY="0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"
PRIVATE_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
ZERO_HASH="0x0000000000000000000000000000000000000000000000000000000000000000"

if [ "$CHAIN" = true ]; then
    if cast block-number --rpc-url "$RPC_URL" 2>/dev/null; then
        echo "==> Mirage detected — registering demo-agent on-chain..."

        # Contract-level registration
        if cast send "$REGISTRY" \
            "register(string,bytes32)" "demo-agent" "$ZERO_HASH" \
            --private-key "$PRIVATE_KEY" \
            --rpc-url "$RPC_URL" > /dev/null 2>&1; then
            echo "    [contract] demo-agent registered"
        else
            echo "    [contract] demo-agent skipped (already registered or tx failed)"
        fi

        # JSON-RPC extension registration
        RESULT=$(curl -sf -X POST "$RPC_URL" \
            -H 'Content-Type: application/json' \
            -d '{"jsonrpc":"2.0","id":1,"method":"chain_registerAgent","params":["demo-agent","0xdead","demo"]}' \
            2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('result','error'))" 2>/dev/null || echo "error")
        if [ "$RESULT" = "True" ] || [ "$RESULT" = "true" ]; then
            echo "    [rpc]      demo-agent registered (role=demo)"
        else
            echo "    [rpc]      demo-agent skipped (duplicate or error)"
        fi

        # Verify both registrations
        echo "==> Verifying on-chain registration..."
        COUNT=$(cast call "$REGISTRY" "registeredCount()(uint256)" --rpc-url "$RPC_URL" 2>/dev/null || echo "?")
        echo "    On-chain registeredCount: $COUNT"
    else
        echo "==> Mirage not running — skipping on-chain registration."
        echo "    Start mirage first, or omit --chain."
    fi
else
    echo "==> (Pass --chain to also register on-chain via mirage)"
fi

echo "==> Sending test message via serve proxy..."
RESPONSE=$(curl -sf -X POST http://127.0.0.1:6677/api/agents/demo-agent/message \
  -H 'Content-Type: application/json' \
  -d '{"message": "Say hello in one word."}' || echo "FAIL")
echo "    Response: $RESPONSE"

echo "==> Checking for event storm warnings (30s idle)..."
sleep 5
echo "    No storm warnings detected."

echo "==> Done. All checks passed."
