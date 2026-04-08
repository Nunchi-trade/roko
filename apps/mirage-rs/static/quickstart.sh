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
# Stop with Ctrl+C (server killed cleanly).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MIRAGE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$MIRAGE_DIR/../.." && pwd)"

PORT="${MIRAGE_PORT:-8545}"
PIDS=()

cleanup() {
  echo ""
  echo "shutting down…"
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
  echo "done."
}
trap cleanup EXIT INT TERM

echo "=========================================================="
echo "  mirage-rs dashboard · quickstart"
echo "=========================================================="
echo ""

# 1. Build
echo "[1/3] building mirage-rs…"
cd "$REPO_ROOT"
cargo build -p mirage-rs --features chain,roko --bin mirage-rs 2>&1 | tail -3

# 2. Start server (fork ETH mainnet via public node)
ETH_RPC="${ETH_RPC_URL:-https://ethereum-rpc.publicnode.com}"
echo "[2/4] starting mirage-rs on :${PORT} (forking mainnet via $ETH_RPC)…"
MIRAGE_DASHBOARD_DIR="$MIRAGE_DIR/static" \
  cargo run -p mirage-rs --features chain,roko --bin mirage-rs -- \
    --rpc-url "$ETH_RPC" \
    --block-time 50 \
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

# 3. Seed demo data
echo "[3/4] seeding demo data (50 insights, 20 pheromones, 3 agents)…"
cargo run -p mirage-rs --features chain,roko --example seed_chain_fixtures -- \
  --rpc-url "http://127.0.0.1:${PORT}" 2>&1 | tail -5

# 4. Start 20-agent simulation
echo "[4/4] starting 20-agent continuous simulation…"
cargo run -p mirage-rs --features chain,roko --example agent_simulation -- \
  --rpc-url "http://127.0.0.1:${PORT}" 2>&1 &
PIDS+=($!)

echo ""
echo "=========================================================="
echo "  READY"
echo ""
echo "  Dashboard:  http://127.0.0.1:${PORT}/dashboard/"
echo "  Health:     http://127.0.0.1:${PORT}/api/health"
echo "  Stats:      http://127.0.0.1:${PORT}/api/stats"
echo "  API:        http://127.0.0.1:${PORT}/api/"
echo ""
echo "  Ctrl+C to stop"
echo "=========================================================="
echo ""

# Open browser (macOS)
if command -v open &>/dev/null; then
  open "http://127.0.0.1:${PORT}/dashboard/"
fi

wait "${PIDS[0]}"
