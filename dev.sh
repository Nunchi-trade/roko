#!/usr/bin/env bash
# dev.sh — Start roko dev environment (serve + demo-app)
#
# Usage:
#   ./dev.sh          # Build once + start serve + vite
#   ./dev.sh --watch  # cargo-watch rebuild + serve + vite
#
set -euo pipefail
cd "$(dirname "$0")"

SERVE_PORT=6677
VITE_PORT=5173

# ── Cleanup ──────────────────────────────────────────────────
cleanup() {
  echo ""
  echo "[dev] Shutting down..."
  # Kill all background jobs in this process group
  kill -- -$$ 2>/dev/null || true
  # Belt-and-suspenders: kill anything still on our ports
  lsof -ti:$SERVE_PORT 2>/dev/null | xargs kill 2>/dev/null || true
  wait 2>/dev/null
  echo "[dev] Done."
}
trap cleanup EXIT INT TERM

# ── Pre-check: kill stale processes on our ports ─────────────
stale=$(lsof -ti:$SERVE_PORT 2>/dev/null || true)
if [ -n "$stale" ]; then
  echo "[dev] Killing stale process(es) on :$SERVE_PORT — PIDs: $stale"
  echo "$stale" | xargs kill 2>/dev/null || true
  sleep 1
fi

# ── Build roko-cli ───────────────────────────────────────────
echo "[dev] Building roko-cli..."
cargo build -p roko-cli 2>&1

# ── Start roko serve ─────────────────────────────────────────
if [ "${1:-}" = "--watch" ]; then
  echo "[dev] Starting cargo-watch (rebuilds on crate changes)..."
  cargo watch -w crates/ -x "build -p roko-cli" -s "./target/debug/roko serve" &
  SERVE_PID=$!
else
  echo "[dev] Starting roko serve on :$SERVE_PORT..."
  ./target/debug/roko serve &
  SERVE_PID=$!
fi

# Wait for serve to be ready
echo -n "[dev] Waiting for serve..."
for i in $(seq 1 30); do
  if curl -sf http://localhost:$SERVE_PORT/api/health >/dev/null 2>&1; then
    echo " ready!"
    break
  fi
  if [ "$i" -eq 30 ]; then
    echo " timeout (serve may still be starting)"
  fi
  sleep 1
  echo -n "."
done

# ── Start demo-app vite ──────────────────────────────────────
echo "[dev] Starting demo-app on :$VITE_PORT..."
cd demo/demo-app
npm run dev &
VITE_PID=$!
cd ../..

echo ""
echo "═══════════════════════════════════════════════════════"
echo "  roko serve  → http://localhost:$SERVE_PORT"
echo "  demo-app    → http://localhost:$VITE_PORT"
echo "  Ctrl+C to stop all"
echo "═══════════════════════════════════════════════════════"
echo ""

# ── Wait for any child to exit ───────────────────────────────
wait
