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

# Relay restart settings
RELAY_MAX_RESTARTS=10
RELAY_BACKOFF_BASE=1
RELAY_BACKOFF_MAX=30

relay_pid=""
mirage_pid=""
relay_restarts=0
relay_backoff="${RELAY_BACKOFF_BASE}"

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

start_relay() {
  /usr/local/bin/agent-relay --bind "${RELAY_BIND}" &
  relay_pid=$!
  echo "relay started (pid=${relay_pid})"
}

start_relay

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
  # Check mirage (primary process) — if it exits, container exits.
  if ! kill -0 "${mirage_pid}" 2>/dev/null; then
    if wait "${mirage_pid}"; then
      status=0
    else
      status=$?
    fi
    echo "mirage exited (status=${status}) — shutting down container"
    cleanup
    exit "${status}"
  fi

  # Check relay (sidecar) — restart with backoff on failure.
  if [ -n "${relay_pid}" ] && ! kill -0 "${relay_pid}" 2>/dev/null; then
    wait "${relay_pid}" 2>/dev/null || true
    relay_restarts=$((relay_restarts + 1))
    if [ "${relay_restarts}" -gt "${RELAY_MAX_RESTARTS}" ]; then
      echo "relay exceeded ${RELAY_MAX_RESTARTS} restarts — giving up on relay"
      relay_pid=""
    else
      echo "relay died (restart ${relay_restarts}/${RELAY_MAX_RESTARTS}), restarting in ${relay_backoff}s..."
      sleep "${relay_backoff}"
      # Exponential backoff: 1, 2, 4, 8, 16, 30, 30, ...
      relay_backoff=$((relay_backoff * 2))
      if [ "${relay_backoff}" -gt "${RELAY_BACKOFF_MAX}" ]; then
        relay_backoff="${RELAY_BACKOFF_MAX}"
      fi
      start_relay
      # Reset backoff on successful start (stays alive for >10s)
      (
        sleep 10
        if kill -0 "${relay_pid}" 2>/dev/null; then
          # Signal parent to reset backoff — we just write a marker file
          touch /tmp/.relay-stable
        fi
      ) &
    fi
  fi

  # Reset backoff if relay has been stable
  if [ -f /tmp/.relay-stable ]; then
    relay_backoff="${RELAY_BACKOFF_BASE}"
    relay_restarts=0
    rm -f /tmp/.relay-stable
  fi

  sleep 1
done
