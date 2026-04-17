#!/usr/bin/env bash
set -euo pipefail

PUBLIC_PORT="${PORT:-8545}"
MIRAGE_HOST="${MIRAGE_HOST:-0.0.0.0}"
RELAY_BIND="${ROKO_AGENT_RELAY_BIND:-127.0.0.1:9011}"
export ROKO_AGENT_RELAY_URL="${ROKO_AGENT_RELAY_URL:-http://127.0.0.1:9011}"
STATE_DIR="${MIRAGE_STATE_DIR:-/workspace/.roko/state}"
SNAPSHOT_INTERVAL="${MIRAGE_SNAPSHOT_INTERVAL_SECS:-15}"

ETH_RPC_URL="${ETH_RPC_URL:-}"

mkdir -p "${STATE_DIR}"

cleanup() {
  for pid in "${relay_pid:-}" "${mirage_pid:-}"; do
    if [ -n "${pid}" ]; then
      kill "${pid}" 2>/dev/null || true
    fi
  done

  for pid in "${relay_pid:-}" "${mirage_pid:-}"; do
    if [ -n "${pid}" ]; then
      wait "${pid}" 2>/dev/null || true
    fi
  done
}

trap 'cleanup; exit 143' INT TERM

/usr/local/bin/agent-relay --bind "${RELAY_BIND}" &
relay_pid=$!

MIRAGE_ARGS=(
  --host "${MIRAGE_HOST}"
  --port "${PUBLIC_PORT}"
  --enable-hdc
  --enable-knowledge
  --enable-stigmergy
  --state-dir "${STATE_DIR}"
  --snapshot-interval-secs "${SNAPSHOT_INTERVAL}"
)

if [ -n "${ETH_RPC_URL}" ]; then
  MIRAGE_ARGS+=(--rpc-url "${ETH_RPC_URL}")
fi

/usr/local/bin/mirage-rs "${MIRAGE_ARGS[@]}" "$@" &
mirage_pid=$!

while :; do
  for pid in "${relay_pid}" "${mirage_pid}"; do
    if ! kill -0 "${pid}" 2>/dev/null; then
      if wait "${pid}"; then
        status=0
      else
        status=$?
      fi
      cleanup
      exit "${status}"
    fi
  done
  sleep 1
done
