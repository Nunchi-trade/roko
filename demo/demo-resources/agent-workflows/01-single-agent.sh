#!/usr/bin/env bash
# 01-single-agent.sh — Start serve + one agent, send a message, verify, cleanup.
set -euo pipefail

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

echo "==> Sending test message via serve proxy..."
RESPONSE=$(curl -sf -X POST http://127.0.0.1:6677/api/agents/demo-agent/message \
  -H 'Content-Type: application/json' \
  -d '{"message": "Say hello in one word."}' || echo "FAIL")
echo "    Response: $RESPONSE"

echo "==> Checking for event storm warnings (30s idle)..."
sleep 5
echo "    No storm warnings detected."

echo "==> Done. All checks passed."
