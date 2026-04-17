#!/usr/bin/env bash
# mirage-rs dashboard quick start
#
# Starts mirage-rs with chain extensions enabled, seeds demo data, and
# opens the dashboard in your browser.
#
# Usage:
#   cd apps/mirage-rs
#   ./static/quickstart.sh
#
# Or from the repo root:
#   ./apps/mirage-rs/static/quickstart.sh
#
# Environment:
#   ETH_RPC_URL / MIRAGE_UPSTREAM_URL      upstream JSON-RPC for local proof mode
#   MIRAGE_DASHBOARD_RPC_URL / MIRAGE_RPC_URL
#                                          remote mirage base URL for static demo mode
#   MIRAGE_STATIC_PORT                    local static-server port for remote demo mode
#   MIRAGE_RELAY_PORT                     local relay port for local proof mode
#
# Stop with Ctrl+C (server killed cleanly).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MIRAGE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$MIRAGE_DIR/../.." && pwd)"

PORT="${MIRAGE_PORT:-8545}"
STATIC_PORT="${MIRAGE_STATIC_PORT:-8080}"
RELAY_PORT="${MIRAGE_RELAY_PORT:-9011}"
RELAY_URL="http://127.0.0.1:${RELAY_PORT}"
UPSTREAM_URL="${MIRAGE_UPSTREAM_URL:-${ETH_RPC_URL:-https://ethereum-rpc.publicnode.com}}"
REMOTE_RPC_URL="${MIRAGE_DASHBOARD_RPC_URL:-${MIRAGE_RPC_URL:-${MIRAGE_REMOTE_URL:-}}}"
PIDS=()
TMP_DIRS=()

cleanup() {
  echo ""
  echo "shutting down…"
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
  for dir in "${TMP_DIRS[@]}"; do
    rm -rf "$dir" 2>/dev/null || true
  done
  echo "done."
}
trap cleanup EXIT INT TERM

probe_rpc() {
  local url="${1%/}"
  local body
  body=$(curl -sf --max-time 8 -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"eth_blockNumber","id":1}' \
    "$url" 2>/dev/null || true)
  echo "$body" | grep -q '"result"'
}

prepare_dashboard_dir() {
  local dir
  dir="$(mktemp -d "${TMPDIR:-/tmp}/mirage-dashboard.XXXXXX")"
  TMP_DIRS+=("$dir")
  cp -R "$MIRAGE_DIR/static/." "$dir/"
  printf '%s\n' "$dir"
}

start_static_server() {
  local dir="$1"
  local log_file="${TMPDIR:-/tmp}/mirage-static-${STATIC_PORT}.log"
  (
    cd "$dir"
    python3 -m http.server "$STATIC_PORT" --bind 127.0.0.1
  ) >"$log_file" 2>&1 &
  PIDS+=($!)
}

echo "=========================================================="
echo "  mirage-rs dashboard · quickstart"
echo "=========================================================="
echo ""

if [[ -n "$REMOTE_RPC_URL" ]]; then
  REMOTE_RPC_URL="${REMOTE_RPC_URL%/}"
  if ! probe_rpc "$REMOTE_RPC_URL"; then
    echo "ERROR: remote mirage URL is not reachable: $REMOTE_RPC_URL"
    exit 1
  fi

  echo "[1/2] preparing static dashboard for remote mirage: $REMOTE_RPC_URL"
  DASHBOARD_DIR="$(prepare_dashboard_dir)"

  echo "[2/2] starting static file server on :${STATIC_PORT}…"
  start_static_server "$DASHBOARD_DIR"
  sleep 1

  DASHBOARD_URL="http://127.0.0.1:${STATIC_PORT}/?base=${REMOTE_RPC_URL}"
  echo ""
  echo "=========================================================="
  echo "  READY"
  echo ""
  echo "  Dashboard:  $DASHBOARD_URL"
  echo "  Target RPC:  $REMOTE_RPC_URL"
  echo ""
  echo "  Ctrl+C to stop"
  echo "=========================================================="
  echo ""

  if command -v open &>/dev/null; then
    open "$DASHBOARD_URL"
  fi

  wait "${PIDS[0]}"
else
  # 1. Build
  echo "[1/5] building mirage-rs + agent-relay…"
  cd "$REPO_ROOT"
  cargo build -p mirage-rs --features chain,roko --bin mirage-rs 2>&1 | tail -3
  cargo build -p agent-relay --bin agent-relay 2>&1 | tail -3

  echo "[2/5] starting agent-relay on :${RELAY_PORT}…"
  cargo run -p agent-relay --bin agent-relay -- --bind "127.0.0.1:${RELAY_PORT}" 2>&1 &
  PIDS+=($!)
  sleep 2

  # 3. Start server (fork upstream via public node by default)
  echo "[3/5] starting mirage-rs on :${PORT} (forking via $UPSTREAM_URL)…"
  MIRAGE_DASHBOARD_DIR="$MIRAGE_DIR/static" \
  ROKO_AGENT_RELAY_URL="$RELAY_URL" \
  cargo run -p mirage-rs --features chain,roko --bin mirage-rs -- \
    --rpc-url "$UPSTREAM_URL" \
    --block-interval-ms 50 \
    --enable-hdc --enable-knowledge --enable-stigmergy 2>&1 &
  PIDS+=($!)
  sleep 5

  # Wait for health
  for i in $(seq 1 10); do
    if curl -sf "http://127.0.0.1:${PORT}/api/health" > /dev/null 2>&1; then
      echo "  server ready"
      break
    fi
    sleep 1
  done

  # 4. Seed demo data
  echo "[4/5] seeding demo data (50 insights, 20 pheromones, 3 agents)…"
  cargo run -p mirage-rs --features chain,roko --example seed_chain_fixtures -- \
    --rpc-url "http://127.0.0.1:${PORT}" 2>&1 | tail -5

  # 5. Start 20-agent simulation
  echo "[5/5] starting 20-agent continuous simulation…"
  cargo run -p mirage-rs --features chain,roko --example agent_simulation -- \
    --rpc-url "http://127.0.0.1:${PORT}" 2>&1 &
  PIDS+=($!)

  echo ""
  echo "=========================================================="
  echo "  READY"
  echo ""
  echo "  Dashboard:  http://127.0.0.1:${PORT}/dashboard/"
  echo "  Health:     http://127.0.0.1:${PORT}/api/health"
  echo "  Relay:      http://127.0.0.1:${PORT}/relay/health"
  echo "  Stats:      http://127.0.0.1:${PORT}/api/stats"
  echo "  API:        http://127.0.0.1:${PORT}/api/"
  echo ""
  echo "  Local relay-backed agent path:"
  echo "    start an agent against ${RELAY_URL} and the dashboard will merge relay presence automatically"
  echo ""
  echo "  Ctrl+C to stop"
  echo "=========================================================="
  echo ""

  # Open browser (macOS)
  if command -v open &>/dev/null; then
    open "http://127.0.0.1:${PORT}/dashboard/"
  fi

  wait "${PIDS[0]}"
fi
