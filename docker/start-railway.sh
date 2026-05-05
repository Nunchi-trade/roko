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
CORE_PIDS=()
ISFR_AGENT_PIDS=()
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

app_cmd() {
  if [ "$(id -u)" = "0" ]; then
    gosu "${APP_USER}" "$@"
  else
    "$@"
  fi
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

setup_git_workspace() {
  local ignore_file="${WORKDIR}/.gitignore"

  if [ ! -f "${ignore_file}" ]; then
    cat >"${ignore_file}" <<'EOF'
.roko/
demo-dist/
EOF
    if [ "$(id -u)" = "0" ]; then
      chown "${APP_USER}:${APP_USER}" "${ignore_file}" 2>/dev/null || true
    fi
  fi

  app_cmd git config --global user.email "${GIT_AUTHOR_EMAIL:-roko-demo@example.local}" || true
  app_cmd git config --global user.name "${GIT_AUTHOR_NAME:-Roko Demo}" || true

  if ! app_cmd git -C "${WORKDIR}" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    app_cmd git -C "${WORKDIR}" init >/dev/null
  fi

  app_cmd git -C "${WORKDIR}" config user.email "${GIT_AUTHOR_EMAIL:-roko-demo@example.local}"
  app_cmd git -C "${WORKDIR}" config user.name "${GIT_AUTHOR_NAME:-Roko Demo}"
  app_cmd git -C "${WORKDIR}" add .gitignore roko.toml >/dev/null 2>&1 || true
  app_cmd git -C "${WORKDIR}" commit -m "workspace init" --allow-empty >/dev/null 2>&1 || true
}

start_child() {
  local name="$1"
  shift

  (run_as_app "$@") &
  local pid=$!
  CHILD_PIDS+=("${pid}")
  CORE_PIDS+=("${pid}")
  CHILD_NAMES["${pid}"]="${name}"
  log "${name} started pid=${pid}"
}

spawn_isfr_agent() {
  local name="$1"
  local prompt="$2"

  app_cmd roko do "${prompt}" --workdir "${WORKDIR}" &
  local pid=$!
  CHILD_PIDS+=("${pid}")
  ISFR_AGENT_PIDS+=("${pid}")
  CHILD_NAMES["${pid}"]="${name}"
  log "isfr-agent/${name} spawned pid=${pid}"
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
require_cmd git
if [ "$(id -u)" = "0" ]; then
  require_cmd gosu
fi

setup_state
setup_git_workspace

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

if command -v forge &>/dev/null && [ -d "$WORKDIR/contracts" ]; then
    log "Building ISFR contracts..."
    (cd "$WORKDIR/contracts" && forge build) || log "WARN: forge build failed, contract deployment may fail"
fi

start_child roko roko serve \
  --bind "${PUBLIC_BIND}" \
  --port "${PUBLIC_PORT}" \
  --workdir "${WORKDIR}" \
  "$@"
roko_pid="${CHILD_PIDS[-1]}"
wait_http roko "http://127.0.0.1:${PUBLIC_PORT}/health" "${roko_pid}" 60

# ---------- ISFR agent fleet (fire-and-forget) ----------
if [ "${ISFR_AGENTS_ENABLED:-1}" != "0" ]; then
  log "spawning ISFR agent fleet (15 agents, 5 roles)"

  # Lending analysts (3)
  spawn_isfr_agent "lending-aave" \
    "Analyze Aave V3 current lending/borrowing rates on Ethereum mainnet. Report supply APY, borrow APY, and utilization for USDC, USDT, ETH, and WBTC. Write findings to the knowledge store."
  spawn_isfr_agent "lending-compound" \
    "Analyze Compound V3 current lending/borrowing rates on Ethereum mainnet. Report supply APY, borrow APY, and utilization for USDC, USDT, ETH, and WBTC. Write findings to the knowledge store."
  spawn_isfr_agent "lending-comparative" \
    "Compare lending rates across Aave V3 and Compound V3 on Ethereum mainnet. Identify the best supply and borrow rates for major assets (USDC, USDT, ETH, WBTC). Highlight rate spreads and arbitrage opportunities. Write findings to the knowledge store."

  # Staking analysts (3)
  spawn_isfr_agent "staking-lido" \
    "Analyze Lido stETH staking yield on Ethereum mainnet. Report current APR, 7d/30d averages, validator count, and total ETH staked. Write findings to the knowledge store."
  spawn_isfr_agent "staking-rocketpool" \
    "Analyze Rocket Pool rETH staking yield on Ethereum mainnet. Report current APR, 7d/30d averages, minipool count, and total ETH staked. Write findings to the knowledge store."
  spawn_isfr_agent "staking-comparative" \
    "Compare ETH liquid staking yields across Lido, Rocket Pool, and Coinbase cbETH. Rank by net APR after fees. Assess liquidity depth and redemption times. Write findings to the knowledge store."

  # Funding rate analysts (3)
  spawn_isfr_agent "funding-eth-perps" \
    "Analyze ETH perpetual funding rates across major venues (Binance, Bybit, dYdX, Hyperliquid). Report current rate, 7d average, and open interest. Identify funding rate arbitrage. Write findings to the knowledge store."
  spawn_isfr_agent "funding-btc-perps" \
    "Analyze BTC perpetual funding rates across major venues (Binance, Bybit, dYdX, Hyperliquid). Report current rate, 7d average, and open interest. Identify funding rate arbitrage. Write findings to the knowledge store."
  spawn_isfr_agent "funding-cross-asset" \
    "Compare funding rates across ETH, BTC, SOL, and ARB perpetuals. Identify cross-asset funding rate dislocations and basis trade opportunities. Write findings to the knowledge store."

  # Structured yield analysts (3)
  spawn_isfr_agent "structured-ethena" \
    "Analyze Ethena USDe yield: current sUSDe APY, backing composition, delta-neutral strategy health, and insurance fund status. Write findings to the knowledge store."
  spawn_isfr_agent "structured-pendle" \
    "Analyze Pendle yield markets: top 5 pools by TVL, current fixed vs variable yields, and implied yield curves. Write findings to the knowledge store."
  spawn_isfr_agent "structured-survey" \
    "Survey the top 5 structured yield products by TVL on Ethereum. For each: report current APY, strategy type, risk tier, and TVL. Write findings to the knowledge store."

  # Oracle / synthesis agents (3)
  spawn_isfr_agent "oracle-composite" \
    "Compute the ISFR composite rate: weighted average of lending rates, staking yields, funding rates, and structured yields. Use knowledge store entries from peer agents. Apply the ISFR weighting formula. Write the composite rate to the knowledge store."
  spawn_isfr_agent "oracle-confidence" \
    "Compute confidence intervals for the ISFR composite rate. Analyze variance across data sources, flag stale or outlier readings, and produce a quality score (0-100). Write findings to the knowledge store."
  spawn_isfr_agent "oracle-summary" \
    "Produce an executive summary of the current ISFR state. Pull all agent findings from the knowledge store. Include: composite rate, confidence band, top opportunities, risk flags, and data freshness. Write the summary to the knowledge store."

  log "ISFR fleet: ${#ISFR_AGENT_PIDS[@]} agents spawned"
fi

# ---------- Watch core processes only ----------
# Agent exits are fire-and-forget; only core service exits bring down the container.
while :; do
  exited_pid=""
  if wait -n -p exited_pid "${CORE_PIDS[@]}"; then
    status=0
  else
    status=$?
  fi

  if [ "${status}" = "127" ]; then
    shutdown 0
  fi

  # Check if this was a core process or an agent
  is_core=0
  for cpid in "${CORE_PIDS[@]}"; do
    if [ "${cpid}" = "${exited_pid}" ]; then
      is_core=1
      break
    fi
  done

  name="${CHILD_NAMES[${exited_pid}]:-child}"

  if [ "${is_core}" = "1" ]; then
    log "CORE ${name} exited status=${status}; stopping service"
    shutdown "${status}"
  else
    log "agent ${name} exited status=${status} (non-fatal)"
  fi
done
