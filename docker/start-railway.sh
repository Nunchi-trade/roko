#!/usr/bin/env bash
set -Eeuo pipefail

APP_USER="${APP_USER:-roko}"
WORKDIR="${ROKO_WORKDIR:-/workspace}"
PUBLIC_BIND="${ROKO_BIND:-0.0.0.0}"
PUBLIC_PORT="${PORT:-${ROKO_PORT:-6677}}"

MIRAGE_HOST="${MIRAGE_HOST:-127.0.0.1}"
MIRAGE_HEALTH_HOST="${MIRAGE_HEALTH_HOST:-127.0.0.1}"
MIRAGE_PORT="${MIRAGE_PORT:-8545}"
MIRAGE_CHAIN_ID="${MIRAGE_CHAIN_ID:-31337}"
MIRAGE_BLOCK_INTERVAL_MS="${MIRAGE_BLOCK_INTERVAL_MS:-1000}"
MIRAGE_SNAPSHOT_INTERVAL_SECS="${MIRAGE_SNAPSHOT_INTERVAL_SECS:-15}"

RELAY_BIND="${ROKO_AGENT_RELAY_BIND:-127.0.0.1:9011}"
export ROKO_AGENT_RELAY_URL="${ROKO_AGENT_RELAY_URL:-http://${RELAY_BIND}}"
export ROKO_MIRAGE_URL="${ROKO_MIRAGE_URL:-http://${MIRAGE_HEALTH_HOST}:${MIRAGE_PORT}}"
export MIRAGE_RPC_URL="${MIRAGE_RPC_URL:-${ROKO_MIRAGE_URL}}"

CHILD_PIDS=()
declare -A CHILD_NAMES=()
SHUTTING_DOWN=0

log() {
  printf '[railway] %s\n' "$*"
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    log "missing required command: $1"
    exit 127
  fi
}

run_as_app() {
  if [ "$(id -u)" = "0" ]; then
    exec gosu "${APP_USER}" "$@"
  fi
  exec "$@"
}

setup_state() {
  local state_root="${ROKO_STATE_ROOT:-${WORKDIR}/.roko}"

  if [ -n "${RAILWAY_VOLUME_MOUNT_PATH:-}" ]; then
    state_root="${RAILWAY_VOLUME_MOUNT_PATH%/}"
  fi

  mkdir -p "${WORKDIR}" "${state_root}"

  if [ "${state_root}" != "${WORKDIR}/.roko" ]; then
    rm -rf "${WORKDIR}/.roko"
    ln -s "${state_root}" "${WORKDIR}/.roko"
  fi

  mkdir -p \
    "${WORKDIR}/.roko/dreams" \
    "${WORKDIR}/.roko/learn" \
    "${WORKDIR}/.roko/neuro" \
    "${WORKDIR}/.roko/state"

  export MIRAGE_STATE_DIR="${MIRAGE_STATE_DIR:-${WORKDIR}/.roko/state}"

  if [ "$(id -u)" = "0" ]; then
    chown -R "${APP_USER}:${APP_USER}" "${state_root}" "${WORKDIR}" 2>/dev/null || true
  fi
}

start_child() {
  local name="$1"
  shift

  (run_as_app "$@") &
  local pid=$!
  CHILD_PIDS+=("${pid}")
  CHILD_NAMES["${pid}"]="${name}"
  log "${name} started pid=${pid}"
}

child_exited() {
  local pid="$1"
  if [ ! -d "/proc/${pid}" ]; then
    return 0
  fi

  local stat state
  IFS= read -r stat <"/proc/${pid}/stat" 2>/dev/null || return 0
  stat="${stat#*) }"
  state="${stat%% *}"
  [ "${state}" = "Z" ]
}

wait_http() {
  local name="$1"
  local url="$2"
  local pid="$3"
  local attempts="${4:-60}"

  for _ in $(seq 1 "${attempts}"); do
    if curl -fsS "${url}" >/dev/null 2>&1; then
      log "${name} ready at ${url}"
      return 0
    fi

    if child_exited "${pid}"; then
      log "${name} exited before becoming ready"
      return 1
    fi

    sleep 1
  done

  log "${name} did not become ready at ${url}"
  return 1
}

shutdown() {
  local status="${1:-0}"

  if [ "${SHUTTING_DOWN}" = "1" ]; then
    exit "${status}"
  fi
  SHUTTING_DOWN=1

  log "shutting down"
  for pid in "${CHILD_PIDS[@]}"; do
    kill "${pid}" 2>/dev/null || true
  done
  for pid in "${CHILD_PIDS[@]}"; do
    wait "${pid}" 2>/dev/null || true
  done

  exit "${status}"
}

trap 'shutdown 143' INT TERM
trap 'shutdown 1' ERR

require_cmd curl
require_cmd roko
require_cmd mirage-rs
require_cmd agent-relay
if [ "$(id -u)" = "0" ]; then
  require_cmd gosu
fi

setup_state

log "public roko endpoint: ${PUBLIC_BIND}:${PUBLIC_PORT}"
log "internal mirage endpoint: ${MIRAGE_HOST}:${MIRAGE_PORT}"
log "internal relay endpoint: ${RELAY_BIND}"
log "state root: ${WORKDIR}/.roko"

start_child relay agent-relay --bind "${RELAY_BIND}"
relay_pid="${CHILD_PIDS[-1]}"
wait_http relay "http://${RELAY_BIND}/relay/health" "${relay_pid}" 30

mirage_args=(
  --host "${MIRAGE_HOST}"
  --port "${MIRAGE_PORT}"
  --chain-id "${MIRAGE_CHAIN_ID}"
  --enable-hdc
  --enable-knowledge
  --enable-stigmergy
  --state-dir "${MIRAGE_STATE_DIR}"
  --snapshot-interval-secs "${MIRAGE_SNAPSHOT_INTERVAL_SECS}"
)

if [ -n "${MIRAGE_BLOCK_INTERVAL_MS}" ]; then
  mirage_args+=(--block-interval-ms "${MIRAGE_BLOCK_INTERVAL_MS}")
fi

if [ -n "${ETH_RPC_URL:-}" ]; then
  mirage_args+=(--rpc-url "${ETH_RPC_URL}")
fi

if [ "${MIRAGE_NO_PERSIST:-}" = "1" ]; then
  mirage_args+=(--no-persist)
fi

start_child mirage mirage-rs "${mirage_args[@]}"
mirage_pid="${CHILD_PIDS[-1]}"
wait_http mirage "http://${MIRAGE_HEALTH_HOST}:${MIRAGE_PORT}/health" "${mirage_pid}" 60

start_child roko roko serve \
  --bind "${PUBLIC_BIND}" \
  --port "${PUBLIC_PORT}" \
  --workdir "${WORKDIR}" \
  "$@"
roko_pid="${CHILD_PIDS[-1]}"
wait_http roko "http://127.0.0.1:${PUBLIC_PORT}/health" "${roko_pid}" 60

while :; do
  exited_pid=""
  if wait -n -p exited_pid "${CHILD_PIDS[@]}"; then
    status=0
  else
    status=$?
  fi

  if [ "${status}" = "127" ]; then
    shutdown 0
  fi

  name="${CHILD_NAMES[${exited_pid}]:-child}"
  log "${name} exited status=${status}; stopping service"
  shutdown "${status}"
done
