#!/usr/bin/env bash
# 02-multi-agent.sh — Start serve + 3 agents with auto-port, verify all registered.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"

PIDS=()
cleanup() {
  echo "Cleaning up..."
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null
}
trap cleanup EXIT

echo "==> Starting roko serve..."
cargo run -p roko-cli -- serve &
PIDS+=($!)
sleep 3

AGENTS=("agent-alpha" "agent-beta" "agent-gamma")

for agent in "${AGENTS[@]}"; do
  echo "==> Starting $agent (auto-port)..."
  cargo run -p roko-cli -- agent serve --agent-id "$agent" &
  PIDS+=($!)
done
sleep 5

echo "==> Verifying all agents registered..."
ALL_OK=true
for agent in "${AGENTS[@]}"; do
  INFO=$(curl -sf "http://127.0.0.1:6677/api/agents/$agent" || echo "FAIL")
  if echo "$INFO" | grep -q "$agent"; then
    echo "    OK: $agent registered"
  else
    echo "    FAIL: $agent not found"
    ALL_OK=false
  fi
done

if $ALL_OK; then
  echo "==> All agents registered successfully."
else
  echo "==> Some agents failed to register."
  exit 1
fi

echo "==> Listing all agents via API..."
curl -sf http://127.0.0.1:6677/api/agents | python3 -m json.tool 2>/dev/null || \
  curl -sf http://127.0.0.1:6677/api/agents

echo "==> Done."
