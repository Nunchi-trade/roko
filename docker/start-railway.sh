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
MIRAGE_BLOCK_INTERVAL_MS="${MIRAGE_BLOCK_INTERVAL_MS:-50}"
MIRAGE_SNAPSHOT_INTERVAL_SECS="${MIRAGE_SNAPSHOT_INTERVAL_SECS:-15}"

RELAY_BIND="${ROKO_AGENT_RELAY_BIND:-127.0.0.1:9011}"
export ROKO_AGENT_RELAY_URL="${ROKO_AGENT_RELAY_URL:-http://${RELAY_BIND}}"
export ROKO_MIRAGE_URL="${ROKO_MIRAGE_URL:-http://${MIRAGE_HEALTH_HOST}:${MIRAGE_PORT}}"
export MIRAGE_RPC_URL="${MIRAGE_RPC_URL:-${ROKO_MIRAGE_URL}}"

CHILD_PIDS=()
CORE_PIDS=()
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
    "${WORKDIR}/.roko/state" \
    "${WORKDIR}/.roko/plans"

  # Prune stale plan directories with names longer than 200 chars
  # (generated before the slugify length cap was deployed).
  if [ -d "${WORKDIR}/.roko/plans" ]; then
    find "${WORKDIR}/.roko/plans" -maxdepth 1 -mindepth 1 -type d \
      | while IFS= read -r d; do
          name="$(basename "$d")"
          if [ "${#name}" -gt 200 ]; then
            log "pruning stale long-slug plan: ${name:0:80}..."
            rm -rf "$d"
          fi
        done
  fi

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

MIRAGE_OK=0
start_child mirage mirage-rs "${mirage_args[@]}"
mirage_pid="${CHILD_PIDS[-1]}"
if wait_http mirage "http://${MIRAGE_HEALTH_HOST}:${MIRAGE_PORT}/health" "${mirage_pid}" 60; then
  MIRAGE_OK=1
  if command -v forge &>/dev/null && [ -d "$WORKDIR/contracts" ]; then
      log "Building ISFR contracts..."
      (cd "$WORKDIR/contracts" && forge build) || log "WARN: forge build failed, contract deployment may fail"
  fi
else
  log "WARN: mirage failed to start — continuing without chain backend"
  # Remove mirage from CORE_PIDS so its exit doesn't bring down the service
  NEW_CORE_PIDS=()
  for cpid in "${CORE_PIDS[@]}"; do
    [ "${cpid}" != "${mirage_pid}" ] && NEW_CORE_PIDS+=("${cpid}")
  done
  CORE_PIDS=("${NEW_CORE_PIDS[@]}")
fi

start_child roko roko serve \
  --bind "${PUBLIC_BIND}" \
  --port "${PUBLIC_PORT}" \
  --workdir "${WORKDIR}" \
  "$@"
roko_pid="${CHILD_PIDS[-1]}"
wait_http roko "http://127.0.0.1:${PUBLIC_PORT}/health" "${roko_pid}" 60

# ---------- ISFR agent fleet gate ----------
# The ISFR agent fleet uses `roko do` which generates plans, runs agents that
# write Rust code, and validates with cargo gates. This requires the full source
# tree + Rust toolchain in the container. The Railway image is a deployment
# target with only compiled binaries — the fleet always fails.
#
# The ISFR *keeper* (live rate polling from RPC) is separate and works fine.
# Set ISFR_AGENTS_ENABLED=1 explicitly to force-enable the fleet.
if [ "${ISFR_AGENTS_ENABLED:-0}" != "1" ]; then
  if [ "${ISFR_AGENTS_ENABLED:-0}" = "0" ]; then
    log "ISFR agent fleet disabled (default for deployed containers)"
  elif [ -z "${OPENAI_API_KEY:-}" ] && [ -z "${ANTHROPIC_API_KEY:-}" ] && [ -z "${OPENROUTER_API_KEY:-}" ]; then
    log "WARN: no LLM API keys set; disabling ISFR agent fleet"
  fi
  ISFR_AGENTS_ENABLED=0
fi

# ---------- ISFR agent fleet (fire-and-forget, serialized) ----------
# Agents run one-at-a-time to avoid OOM on small Railway containers.
# Each `roko do` loads the full config + creates an LLM client; 15 at once
# easily exceeds the container memory limit and OOM-kills core processes.
if [ "${ISFR_AGENTS_ENABLED:-1}" != "0" ]; then
  log "spawning ISFR agent fleet (15 agents, 5 roles, serialized)"

  ISFR_PROMPTS=(
    # Lending analysts (3) — covers Aave, Compound, Spark, Morpho
    "lending-aave|Analyze Aave V3 current lending/borrowing rates on Ethereum mainnet. Report supply APY, borrow APY, and utilization for USDC, USDT, ETH, and WBTC. Compare with Spark Protocol DAI rates. Write findings to the knowledge store."
    "lending-compound|Analyze Compound V3 current lending/borrowing rates on Ethereum mainnet. Report supply APY, borrow APY, and utilization for USDC, USDT, ETH, and WBTC. Include Morpho USDC vault rates for comparison. Write findings to the knowledge store."
    "lending-comparative|Compare lending rates across Aave V3, Compound V3, Spark Protocol, and Morpho on Ethereum mainnet. Identify the best supply and borrow rates for major assets (USDC, USDT, ETH, WBTC, DAI). Highlight rate spreads, yield curve shape, and arbitrage opportunities across all four protocols. Write findings to the knowledge store."
    # Staking analysts (3) — covers Lido, Rocket Pool, Swell
    "staking-lido|Analyze Lido stETH staking yield on Ethereum mainnet. Report current APR, 7d/30d averages, validator count, and total ETH staked. Compare with Swell swETH emerging yield. Write findings to the knowledge store."
    "staking-rocketpool|Analyze Rocket Pool rETH staking yield on Ethereum mainnet. Report current APR, 7d/30d averages, minipool count, and total ETH staked. Note commission structure vs Lido. Write findings to the knowledge store."
    "staking-comparative|Compare ETH liquid staking yields across Lido (stETH), Rocket Pool (rETH), Swell (swETH), and Coinbase (cbETH). Rank by net APR after fees. Assess liquidity depth, redemption times, and decentralization metrics. Write findings to the knowledge store."
    # Funding rate analysts (3) — covers ETH, BTC, dYdX perps
    "funding-eth-perps|Analyze ETH perpetual funding rates across major venues (Binance, Bybit, dYdX, Hyperliquid). Report current rate, 7d average, and open interest. Identify funding rate arbitrage vs spot lending rates on Aave. Write findings to the knowledge store."
    "funding-btc-perps|Analyze BTC perpetual funding rates across major venues (Binance, Bybit, dYdX, Hyperliquid). Report current rate, 7d average, and open interest. Compare BTC funding basis with ETH. Write findings to the knowledge store."
    "funding-cross-asset|Compare funding rates across ETH, BTC, SOL, and ARB perpetuals including dYdX ETH-specific funding. Identify cross-asset funding rate dislocations and basis trade opportunities. Compute the funding-lending spread for each asset. Write findings to the knowledge store."
    # Structured yield analysts (3) — covers Ethena, Pendle, Yearn
    "structured-ethena|Analyze Ethena USDe yield: current sUSDe APY, backing composition, delta-neutral strategy health, and insurance fund status. Compare with Yearn USDC vault performance. Write findings to the knowledge store."
    "structured-pendle|Analyze Pendle yield markets: top 5 pools by TVL, current fixed vs variable yields, and implied yield curves. Focus on USDe June 2025 market dynamics. Write findings to the knowledge store."
    "structured-survey|Survey structured yield products on Ethereum: Ethena sUSDe, Pendle USDe-Jun25, Yearn USDC vault. For each: report current APY, strategy type, risk tier, TVL, and correlation to base lending rates. Write findings to the knowledge store."
    # Oracle / synthesis agents (3)
    "oracle-composite|Compute the ISFR composite rate from 13 sources: weighted average of lending rates (Aave, Compound, Spark, Morpho), staking yields (Lido, Rocket Pool, Swell), funding rates (ETH/BTC/dYdX perps), and structured yields (Ethena, Pendle, Yearn). Apply the ISFR weighting formula. Write the composite rate to the knowledge store."
    "oracle-confidence|Compute confidence intervals for the ISFR composite rate across all 13 sources. Analyze variance within each rate class, flag stale or outlier readings, weight by source reliability, and produce a quality score (0-100). Write findings to the knowledge store."
    "oracle-summary|Produce an executive summary of the current ISFR state across all 13 sources and 4 rate classes. Pull all agent findings from the knowledge store. Include: composite rate, per-class breakdown, confidence band, top opportunities, risk flags, data freshness, and market regime assessment. Write the summary to the knowledge store."
  )

  # Run agents sequentially in a background subshell so the main loop can
  # start watching core PIDs immediately.
  (
    agent_idx=0
    total=${#ISFR_PROMPTS[@]}
    for entry in "${ISFR_PROMPTS[@]}"; do
      name="${entry%%|*}"
      prompt="${entry#*|}"
      agent_idx=$((agent_idx + 1))
      log "isfr-agent [${agent_idx}/${total}] starting: ${name}"
      app_cmd roko do "${prompt}" --workdir "${WORKDIR}" || \
        log "isfr-agent/${name} failed (non-fatal)"
      log "isfr-agent [${agent_idx}/${total}] finished: ${name}"
    done
    log "ISFR fleet: all ${total} agents completed"
  ) &
  ISFR_RUNNER_PID=$!
  CHILD_PIDS+=("${ISFR_RUNNER_PID}")
  CHILD_NAMES["${ISFR_RUNNER_PID}"]="isfr-runner"
  log "ISFR fleet runner started pid=${ISFR_RUNNER_PID} (15 agents, serialized)"
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
