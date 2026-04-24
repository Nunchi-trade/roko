#!/usr/bin/env bash
set -euo pipefail

PUBLIC_PORT="${PORT:-8545}"
MIRAGE_HOST="${MIRAGE_HOST:-0.0.0.0}"
RELAY_BIND="${ROKO_AGENT_RELAY_BIND:-127.0.0.1:9011}"
export ROKO_AGENT_RELAY_URL="${ROKO_AGENT_RELAY_URL:-http://127.0.0.1:9011}"
STATE_DIR="${MIRAGE_STATE_DIR:-/workspace/.roko/state}"
VOLUME_ROOT="${RAILWAY_VOLUME_MOUNT_PATH:-}"
SNAPSHOT_INTERVAL="${MIRAGE_SNAPSHOT_INTERVAL_SECS:-15}"
BLOCK_INTERVAL_MS="${MIRAGE_BLOCK_INTERVAL_MS:-}"

ETH_RPC_URL="${ETH_RPC_URL:-}"

if [ -n "${VOLUME_ROOT}" ]; then
  VOLUME_ROOT="${VOLUME_ROOT%/}"
  case "${STATE_DIR}" in
    "${VOLUME_ROOT}"|"${VOLUME_ROOT}"/*) ;;
    *)
      STATE_DIR="${VOLUME_ROOT}/state"
      export MIRAGE_STATE_DIR="${STATE_DIR}"
      ;;
  esac
fi

mkdir -p "${STATE_DIR}"

has_arg() {
  local needle="$1"
  shift
  for arg in "$@"; do
    if [ "${arg}" = "${needle}" ] || [[ "${arg}" == "${needle}="* ]]; then
      return 0
    fi
  done
  return 1
}

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

if [ -n "${BLOCK_INTERVAL_MS}" ] && ! has_arg "--block-interval-ms" "$@"; then
  MIRAGE_ARGS+=(--block-interval-ms "${BLOCK_INTERVAL_MS}")
fi

if [ "${MIRAGE_NO_PERSIST:-}" = "1" ] && ! has_arg "--no-persist" "$@"; then
  MIRAGE_ARGS+=(--no-persist)
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
