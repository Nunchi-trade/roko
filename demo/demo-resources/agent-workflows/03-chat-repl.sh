#!/usr/bin/env bash
# 03-chat-repl.sh — Start agent sidecar only (no serve), launch chat pointing at it.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"

AGENT_ID="${1:-demo-chat-agent}"
BIND="127.0.0.1:8081"

cleanup() {
  echo "Cleaning up..."
  [ -n "${AGENT_PID:-}" ] && kill "$AGENT_PID" 2>/dev/null || true
  wait 2>/dev/null
}
trap cleanup EXIT

echo "==> Starting agent sidecar on $BIND (no roko-serve needed)..."
cargo run -p roko-cli -- agent serve --agent-id "$AGENT_ID" --bind "$BIND" &
AGENT_PID=$!
sleep 3

echo "==> Verifying sidecar health..."
HEALTH=$(curl -sf "http://$BIND/health" || echo "FAIL")
if echo "$HEALTH" | grep -q "ok"; then
  echo "    OK: sidecar is healthy"
else
  echo "    FAIL: sidecar not responding"
  echo "    Response: $HEALTH"
  exit 1
fi

echo "==> Launching chat REPL (Ctrl-D to exit)..."
echo "    The chat will connect directly to the sidecar at http://$BIND"
echo ""
cargo run -p roko-cli -- agent chat --agent "$AGENT_ID"
