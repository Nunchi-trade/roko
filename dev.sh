#!/usr/bin/env bash
# dev.sh — Roko dev toolkit
#
# Usage: ./dev.sh <command> [options]
# Run ./dev.sh help for all commands.
#
set -euo pipefail
cd "$(dirname "$0")"

# ── Constants ─────────────────────────────────────────────────
SERVE_PORT=6677
VITE_PORT=5173
ROKO_DIR=".roko"
TARGET_DIR="target"
ROKO_BIN="./target/debug/roko"
WORKSPACE_BASE="${TMPDIR:-/tmp}/roko-dev-pipelines"

# Pipeline globals (set by cmd_pipeline flags, read by run_step)
PIPELINE_MODEL=""
PIPELINE_PROVIDER=""
PIPELINE_DRY_RUN=false
PIPELINE_PROVIDERS_LIST=""
PIPELINE_STEP_PID=""
PIPELINE_STEP_OUTPUT=""
PIPELINE_INTERRUPTED=false
# Per-step timing: parallel arrays filled by run_step, printed by pipeline_finish
PIPELINE_STEP_NAMES=()
PIPELINE_STEP_TIMES=()
PIPELINE_STEP_RESULTS=()

# ── Colors (auto-detect TTY) ─────────────────────────────────
if [ -t 1 ]; then
  BOLD=$'\033[1m'
  DIM=$'\033[2m'
  RED=$'\033[31m'
  GREEN=$'\033[32m'
  YELLOW=$'\033[33m'
  BLUE=$'\033[34m'
  CYAN=$'\033[36m'
  RESET=$'\033[0m'
else
  BOLD="" DIM="" RED="" GREEN="" YELLOW="" BLUE="" CYAN="" RESET=""
fi

# ── Helpers ──────────────────────────────────────────────────
info()  { echo "${BLUE}[dev]${RESET} $*"; }
ok()    { echo "${GREEN}[dev]${RESET} $*"; }
warn()  { echo "${YELLOW}[dev]${RESET} $*"; }
err()   { echo "${RED}[dev]${RESET} $*" >&2; }
die()   { err "$@"; exit 1; }

human_size() {
  local bytes=$1
  if [ "$bytes" -ge 1073741824 ]; then
    echo "$(( bytes / 1073741824 )).$((( bytes % 1073741824 ) * 10 / 1073741824 ))G"
  elif [ "$bytes" -ge 1048576 ]; then
    echo "$(( bytes / 1048576 ))M"
  elif [ "$bytes" -ge 1024 ]; then
    echo "$(( bytes / 1024 ))K"
  else
    echo "${bytes}B"
  fi
}

dir_size_bytes() {
  if [ -d "$1" ]; then
    du -sk "$1" 2>/dev/null | awk '{print $1 * 1024}'
  else
    echo 0
  fi
}

pids_on_port() {
  lsof -ti:"$1" 2>/dev/null || true
}

jq_or_raw() {
  if command -v jq &>/dev/null; then
    jq "$@"
  else
    cat
  fi
}

# ── cmd: up ──────────────────────────────────────────────────
cmd_up() {
  local watch=false no_vite=false release=false chain=true
  while [ $# -gt 0 ]; do
    case "$1" in
      --watch|-w)  watch=true ;;
      --no-vite)   no_vite=true ;;
      --release)   release=true ;;
      --chain|-c)  chain=true ;;
      --no-chain)  chain=false ;;
      *) die "Unknown option: $1" ;;
    esac
    shift
  done

  # Cleanup trap — only installed for 'up'
  CLEANING_UP=false
  cleanup() {
    $CLEANING_UP && return
    CLEANING_UP=true
    trap - EXIT INT TERM
    echo ""
    info "Shutting down..."
    [ -n "${MIRAGE_PID:-}" ]         && kill "$MIRAGE_PID" 2>/dev/null || true
    [ -n "${RELAY_PID:-}" ]          && kill "$RELAY_PID" 2>/dev/null || true
    [ -n "${SERVE_PID:-}" ]          && kill "$SERVE_PID" 2>/dev/null || true
    [ -n "${VITE_PID:-}" ]           && kill "$VITE_PID"  2>/dev/null || true
    [ -n "${LENDING_SCOUT_PID:-}" ]  && kill "$LENDING_SCOUT_PID" 2>/dev/null || true
    [ -n "${STAKING_SCOUT_PID:-}" ]  && kill "$STAKING_SCOUT_PID" 2>/dev/null || true
    lsof -ti:$SERVE_PORT 2>/dev/null | xargs kill 2>/dev/null || true
    lsof -ti:9011 2>/dev/null | xargs kill 2>/dev/null || true
    lsof -ti:8545 2>/dev/null | xargs kill 2>/dev/null || true
    wait 2>/dev/null
    ok "Done."
  }
  trap cleanup EXIT INT TERM

  # Kill stale processes
  local stale
  stale=$(pids_on_port $SERVE_PORT)
  if [ -n "$stale" ]; then
    warn "Killing stale process(es) on :$SERVE_PORT — PIDs: $stale"
    echo "$stale" | xargs kill 2>/dev/null || true
    sleep 1
  fi

  # Start mirage-rs if --chain flag passed
  if $chain; then
    local mirage_bin
    if command -v mirage-rs &>/dev/null; then
      mirage_bin="mirage-rs"
    elif [ -x "./target/release/mirage-rs" ]; then
      mirage_bin="./target/release/mirage-rs"
    else
      die "mirage-rs not found. Run 'cargo build --release -p mirage-rs' or use --no-chain to skip."
    fi

    local mirage_stale
    mirage_stale=$(pids_on_port 8545)
    if [ -n "$mirage_stale" ]; then
      warn "Killing stale process(es) on :8545 — PIDs: $mirage_stale"
      echo "$mirage_stale" | xargs kill 2>/dev/null || true
      sleep 1
    fi

    info "Starting mirage-rs (mainnet fork on :8545)..."
    local rpc_url="${ETH_RPC_URL:-https://ethereum-rpc.publicnode.com}"
    "$mirage_bin" \
      --host 127.0.0.1 \
      --port 8545 \
      --rpc-url "$rpc_url" &
    MIRAGE_PID=$!

    # Wait for mirage BEFORE building contracts (forge needs the fork running)
    echo -n "[dev] Waiting for mirage-rs..."
    for i in $(seq 1 30); do
      if curl -sf http://127.0.0.1:8545 -X POST -H 'Content-Type: application/json' \
           -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' >/dev/null 2>&1; then
        echo " ready!"
        break
      fi
      if [ "$i" -eq 30 ]; then
        echo " timeout (mirage may still be starting)"
      fi
      sleep 1
      echo -n "."
    done

    # Build contracts after mirage is up
    if [ -d "contracts" ] && [ ! -d "contracts/out" ]; then
      if command -v forge &>/dev/null; then
        info "Building ISFR contracts..."
        (cd contracts && forge build) || warn "forge build failed"
      fi
    fi

  fi

  # Start agent-relay (always — provides WS event stream + agent registry)
  local relay_stale
  relay_stale=$(pids_on_port 9011)
  if [ -n "$relay_stale" ]; then
    warn "Killing stale process(es) on :9011 — PIDs: $relay_stale"
    echo "$relay_stale" | xargs kill 2>/dev/null || true
    sleep 1
  fi

  info "Building agent-relay..."
  cargo build -p agent-relay 2>&1

  local relay_args=(--bind 127.0.0.1:9011)
  if $chain; then
    relay_args+=(--rpc-ws-url ws://127.0.0.1:8545 --chain-id 31337)
  fi

  info "Starting agent-relay on :9011..."
  ./target/debug/agent-relay "${relay_args[@]}" &
  RELAY_PID=$!

  # Wait for relay health
  echo -n "[dev] Waiting for agent-relay..."
  for i in $(seq 1 10); do
    if curl -sf http://127.0.0.1:9011/relay/health >/dev/null 2>&1; then
      echo " ready!"
      break
    fi
    if [ "$i" -eq 10 ]; then
      echo " timeout (relay may still be starting)"
    fi
    sleep 1
    echo -n "."
  done

  export ROKO_AGENT_RELAY_URL="http://127.0.0.1:9011"

  # Build
  local profile="debug" bin="$ROKO_BIN"
  if $release; then
    profile="release"
    bin="./target/release/roko"
    info "Building roko-cli (release)..."
    cargo build --release -p roko-cli 2>&1
  else
    info "Building roko-cli..."
    cargo build -p roko-cli 2>&1
  fi

  # Start serve
  if $watch; then
    info "Starting cargo-watch (rebuilds on crate changes)..."
    cargo watch -w crates/ -x "build -p roko-cli" -s "$bin serve" &
    SERVE_PID=$!
  else
    info "Starting roko serve on :$SERVE_PORT..."
    $bin serve &
    SERVE_PID=$!
  fi

  # Wait for serve readiness
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

  # ISFR on-chain submission: the keeper auto-submits rates to ISFROracle on
  # each new epoch via the publish callback in start_isfr_keeper(). No separate
  # agent processes needed — roko serve handles it all.
  if $chain; then
    info "ISFR keeper will auto-submit rates to ISFROracle on-chain each epoch"
  fi

  # Start vite
  if ! $no_vite; then
    info "Starting demo-app on :$VITE_PORT..."
    (cd demo/demo-app && npm run dev) &
    VITE_PID=$!
  fi

  echo ""
  echo "${BOLD}═══════════════════════════════════════════════════════${RESET}"
  $chain && echo "  mirage-rs     → http://localhost:8545 (mainnet fork)"
  echo "  agent-relay   → http://localhost:9011"
  echo "  roko serve    → http://localhost:$SERVE_PORT"
  $no_vite || echo "  demo-app      → http://localhost:$VITE_PORT"
  if $chain; then
    echo "  ISFR data     → LIVE (on-chain via mirage-rs)"
    echo "  ISFR oracle   → keeper auto-submits each epoch"
    echo "  Feed agents   → 15 active (4 keepers + 11 derived)"
  else
    echo "  ISFR data     → MOCK (no chain — use --chain for live data)"
  fi
  echo "  Ctrl+C to stop all"
  echo "${BOLD}═══════════════════════════════════════════════════════${RESET}"
  echo ""

  wait
}

# ── cmd: down ────────────────────────────────────────────────
cmd_down() {
  info "Stopping roko dev processes..."
  local killed=0

  for pattern in "mirage-rs" "agent-relay" "roko serve" "cargo watch" "vite"; do
    local pids
    pids=$(pgrep -f "$pattern" 2>/dev/null || true)
    if [ -n "$pids" ]; then
      echo "$pids" | xargs kill 2>/dev/null || true
      info "Killed: $pattern (PIDs: $(echo "$pids" | tr '\n' ' '))"
      killed=$(( killed + $(echo "$pids" | wc -l | tr -d ' ') ))
    fi
  done

  for port in 8545 9011 $SERVE_PORT $VITE_PORT; do
    local pids
    pids=$(pids_on_port "$port")
    if [ -n "$pids" ]; then
      echo "$pids" | xargs kill 2>/dev/null || true
      info "Killed processes on :$port"
      killed=$(( killed + $(echo "$pids" | wc -l | tr -d ' ') ))
    fi
  done

  if [ "$killed" -eq 0 ]; then
    ok "Nothing running."
  else
    ok "Stopped $killed process(es)."
  fi
}

# ── cmd: status ──────────────────────────────────────────────
cmd_status() {
  echo "${BOLD}Processes${RESET}"
  for pattern in "mirage-rs" "agent-relay" "roko serve" "cargo watch" "vite"; do
    local pids
    pids=$(pgrep -f "$pattern" 2>/dev/null || true)
    if [ -n "$pids" ]; then
      echo "  ${GREEN}●${RESET} $pattern (PIDs: $(echo "$pids" | tr '\n' ' '))"
    else
      echo "  ${DIM}○ $pattern${RESET}"
    fi
  done

  echo ""
  echo "${BOLD}Ports${RESET}"
  for port in 8545 9011 $SERVE_PORT $VITE_PORT; do
    local pids
    pids=$(pids_on_port "$port")
    if [ -n "$pids" ]; then
      echo "  ${GREEN}●${RESET} :$port (PIDs: $(echo "$pids" | tr '\n' ' '))"
    else
      echo "  ${DIM}○ :$port${RESET}"
    fi
  done

  echo ""
  echo "${BOLD}Health${RESET}"
  local health
  if health=$(curl -sf http://localhost:$SERVE_PORT/api/health 2>/dev/null); then
    echo "$health" | jq_or_raw .
  else
    echo "  ${DIM}serve not reachable${RESET}"
  fi

  echo ""
  echo "${BOLD}State files${RESET}"
  if [ -d "$ROKO_DIR" ]; then
    for f in "$ROKO_DIR"/signals.jsonl "$ROKO_DIR"/episodes.jsonl "$ROKO_DIR"/learn/efficiency.jsonl \
             "$ROKO_DIR"/learn/cascade-router.json "$ROKO_DIR"/learn/gate-thresholds.json \
             "$ROKO_DIR"/learn/experiments.json "$ROKO_DIR"/state/executor.json; do
      if [ -f "$f" ]; then
        local sz
        sz=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f" 2>/dev/null || echo 0)
        local lines=""
        if [[ "$f" == *.jsonl ]]; then
          lines=" ($(wc -l < "$f" | tr -d ' ') lines)"
        fi
        echo "  $f  $(human_size "$sz")$lines"
      fi
    done
  else
    echo "  ${DIM}.roko/ not found${RESET}"
  fi
}

# ── cmd: logs ────────────────────────────────────────────────
cmd_logs() {
  local target="${1:-errors}"
  case "$target" in
    serve)
      if [ -f "$ROKO_DIR/roko.log" ]; then
        tail -f "$ROKO_DIR/roko.log"
      else
        die "No log file at $ROKO_DIR/roko.log"
      fi
      ;;
    episodes)
      if [ -f "$ROKO_DIR/episodes.jsonl" ]; then
        tail -f "$ROKO_DIR/episodes.jsonl" | jq_or_raw .
      else
        die "No episodes at $ROKO_DIR/episodes.jsonl"
      fi
      ;;
    efficiency)
      if [ -f "$ROKO_DIR/learn/efficiency.jsonl" ]; then
        tail -f "$ROKO_DIR/learn/efficiency.jsonl" | jq_or_raw .
      else
        die "No efficiency log at $ROKO_DIR/learn/efficiency.jsonl"
      fi
      ;;
    events)
      if [ -f "$ROKO_DIR/events.jsonl" ]; then
        tail -f "$ROKO_DIR/events.jsonl" | jq_or_raw .
      else
        die "No events at $ROKO_DIR/events.jsonl"
      fi
      ;;
    runtime)
      if [ -f "$ROKO_DIR/runtime.log" ]; then
        tail -f "$ROKO_DIR/runtime.log"
      else
        die "No runtime log at $ROKO_DIR/runtime.log"
      fi
      ;;
    errors)
      if [ -f "$ROKO_DIR/roko.log" ]; then
        tail -100 "$ROKO_DIR/roko.log" | grep -i -E "error|panic|fatal|fail" || echo "${DIM}No recent errors${RESET}"
      else
        echo "${DIM}No log file at $ROKO_DIR/roko.log${RESET}"
      fi
      ;;
    all)
      info "Tailing all JSONL logs..."
      tail -f "$ROKO_DIR"/*.jsonl "$ROKO_DIR"/learn/*.jsonl 2>/dev/null || die "No logs found"
      ;;
    *)
      die "Unknown log target: $target (options: serve, episodes, efficiency, events, runtime, errors, all)"
      ;;
  esac
}

# ── cmd: check ───────────────────────────────────────────────
cmd_check() {
  local fix=false
  [ "${1:-}" = "--fix" ] && fix=true

  local failed=0
  local start_time=$SECONDS

  echo "${BOLD}fmt${RESET}"
  if $fix; then
    if cargo +nightly fmt --all 2>&1; then
      ok "Formatted."
    else
      err "fmt failed."
      failed=1
    fi
  else
    if cargo +nightly fmt --all --check 2>&1; then
      ok "Clean."
    else
      err "fmt check failed. Run: ./dev.sh check --fix"
      failed=1
    fi
  fi

  echo ""
  echo "${BOLD}clippy${RESET}"
  if cargo clippy --workspace --no-deps -- -D warnings 2>&1; then
    ok "Clean."
  else
    err "clippy failed."
    failed=1
  fi

  echo ""
  echo "${BOLD}test${RESET}"
  if command -v cargo-nextest &>/dev/null; then
    if cargo nextest run --workspace 2>&1; then
      ok "Passed."
    else
      err "Tests failed."
      failed=1
    fi
  else
    if cargo test --workspace 2>&1; then
      ok "Passed."
    else
      err "Tests failed."
      failed=1
    fi
  fi

  local elapsed=$(( SECONDS - start_time ))
  echo ""
  if [ "$failed" -eq 0 ]; then
    ok "All checks passed in ${elapsed}s."
  else
    die "Checks failed after ${elapsed}s."
  fi
}

# ── cmd: fmt ─────────────────────────────────────────────────
cmd_fmt() {
  cargo +nightly fmt --all
  ok "Formatted."
}

# ── cmd: clippy ──────────────────────────────────────────────
cmd_clippy() {
  cargo clippy --workspace --no-deps -- -D warnings
}

# ── cmd: test ────────────────────────────────────────────────
cmd_test() {
  local crate="${1:-}"
  if command -v cargo-nextest &>/dev/null; then
    if [ -n "$crate" ]; then
      cargo nextest run -p "$crate"
    else
      cargo nextest run --workspace
    fi
  else
    if [ -n "$crate" ]; then
      cargo test -p "$crate"
    else
      cargo test --workspace
    fi
  fi
}

# ── cmd: build ───────────────────────────────────────────────
cmd_build() {
  local release=false crate=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --release|-r) release=true ;;
      *) crate="$1" ;;
    esac
    shift
  done

  local start_time=$SECONDS
  local args=()
  $release && args+=(--release)

  if [ -n "$crate" ]; then
    args+=(-p "$crate")
    info "Building $crate..."
  else
    args+=(-p roko-cli)
    info "Building roko-cli..."
  fi

  cargo build "${args[@]}" 2>&1

  local elapsed=$(( SECONDS - start_time ))
  local profile="debug"
  $release && profile="release"

  # Show binary size for roko-cli builds
  local bin="./target/$profile/roko"
  if [ -f "$bin" ]; then
    local sz
    sz=$(stat -f%z "$bin" 2>/dev/null || stat -c%s "$bin" 2>/dev/null || echo 0)
    ok "Built in ${elapsed}s — roko binary: $(human_size "$sz")"
  else
    ok "Built in ${elapsed}s."
  fi
}

# ── cmd: clean ───────────────────────────────────────────────
cmd_clean() {
  local mode="${1:--i}"
  case "$mode" in
    -i|--incremental)
      local dir="$TARGET_DIR/debug/incremental"
      if [ -d "$dir" ]; then
        local sz
        sz=$(dir_size_bytes "$dir")
        rm -rf "$dir"
        ok "Removed incremental cache — freed $(human_size "$sz")"
      else
        ok "No incremental cache."
      fi
      ;;
    --target)
      if [ -d "$TARGET_DIR" ]; then
        local sz
        sz=$(dir_size_bytes "$TARGET_DIR")
        cargo clean 2>&1
        ok "Removed target/ — freed $(human_size "$sz")"
      else
        ok "No target/ directory."
      fi
      ;;
    --all)
      local freed=0
      if [ -d "$TARGET_DIR" ]; then
        local sz
        sz=$(dir_size_bytes "$TARGET_DIR")
        freed=$sz
        cargo clean 2>&1
        ok "Removed target/ — $(human_size "$sz")"
      fi
      if command -v sccache &>/dev/null; then
        local cache_dir="$HOME/Library/Caches/Mozilla.sccache"
        if [ -d "$cache_dir" ]; then
          local sz
          sz=$(dir_size_bytes "$cache_dir")
          freed=$(( freed + sz ))
          rm -rf "$cache_dir"
          ok "Removed sccache — $(human_size "$sz")"
        fi
      fi
      ok "Total freed: $(human_size "$freed")"
      ;;
    --sccache)
      local cache_dir="$HOME/Library/Caches/Mozilla.sccache"
      if [ -d "$cache_dir" ]; then
        local sz
        sz=$(dir_size_bytes "$cache_dir")
        rm -rf "$cache_dir"
        ok "Removed sccache — freed $(human_size "$sz")"
      else
        ok "No sccache directory."
      fi
      ;;
    *)
      die "Unknown clean mode: $mode (options: -i/--incremental, --target, --all, --sccache)"
      ;;
  esac
}

# ── cmd: doctor ──────────────────────────────────────────────
cmd_doctor() {
  echo "${BOLD}Toolchain${RESET}"
  local ok_count=0 warn_count=0

  # rustc
  if command -v rustc &>/dev/null; then
    local ver
    ver=$(rustc --version 2>/dev/null)
    echo "  ${GREEN}●${RESET} rustc: $ver"
    ok_count=$(( ok_count + 1 ))
  else
    echo "  ${RED}●${RESET} rustc: not found"
    warn_count=$(( warn_count + 1 ))
  fi

  # nightly fmt
  if rustup run nightly rustfmt --version &>/dev/null 2>&1; then
    echo "  ${GREEN}●${RESET} nightly rustfmt: $(rustup run nightly rustfmt --version 2>/dev/null)"
    ok_count=$(( ok_count + 1 ))
  else
    echo "  ${RED}●${RESET} nightly rustfmt: not installed (run: rustup toolchain install nightly)"
    warn_count=$(( warn_count + 1 ))
  fi

  # sccache
  if command -v sccache &>/dev/null; then
    echo "  ${GREEN}●${RESET} sccache: $(sccache --version 2>/dev/null)"
    ok_count=$(( ok_count + 1 ))
  else
    echo "  ${YELLOW}●${RESET} sccache: not found (optional but recommended)"
    warn_count=$(( warn_count + 1 ))
  fi

  # cargo-watch
  if command -v cargo-watch &>/dev/null; then
    echo "  ${GREEN}●${RESET} cargo-watch: installed"
    ok_count=$(( ok_count + 1 ))
  else
    echo "  ${YELLOW}●${RESET} cargo-watch: not found (needed for --watch mode)"
    warn_count=$(( warn_count + 1 ))
  fi

  # nextest
  if command -v cargo-nextest &>/dev/null; then
    echo "  ${GREEN}●${RESET} cargo-nextest: installed"
    ok_count=$(( ok_count + 1 ))
  else
    echo "  ${YELLOW}●${RESET} cargo-nextest: not found (optional, faster tests)"
    warn_count=$(( warn_count + 1 ))
  fi

  # jq
  if command -v jq &>/dev/null; then
    echo "  ${GREEN}●${RESET} jq: $(jq --version 2>/dev/null)"
    ok_count=$(( ok_count + 1 ))
  else
    echo "  ${YELLOW}●${RESET} jq: not found (dump/logs will show raw JSON)"
    warn_count=$(( warn_count + 1 ))
  fi

  # node
  if command -v node &>/dev/null; then
    echo "  ${GREEN}●${RESET} node: $(node --version 2>/dev/null)"
    ok_count=$(( ok_count + 1 ))
  else
    echo "  ${YELLOW}●${RESET} node: not found (needed for demo-app)"
    warn_count=$(( warn_count + 1 ))
  fi

  echo ""
  echo "${BOLD}Disk${RESET}"
  if [ -d "$TARGET_DIR" ]; then
    echo "  target/:           $(human_size "$(dir_size_bytes "$TARGET_DIR")")"
  fi
  if [ -d "$TARGET_DIR/debug/incremental" ]; then
    echo "  target/debug/inc/: $(human_size "$(dir_size_bytes "$TARGET_DIR/debug/incremental")")"
  fi
  local sccache_dir="$HOME/Library/Caches/Mozilla.sccache"
  if [ -d "$sccache_dir" ]; then
    echo "  sccache:           $(human_size "$(dir_size_bytes "$sccache_dir")")"
  fi
  if [ -d "$ROKO_DIR" ]; then
    echo "  .roko/:            $(human_size "$(dir_size_bytes "$ROKO_DIR")")"
  fi
  # Available disk
  local avail
  avail=$(df -h . 2>/dev/null | awk 'NR==2{print $4}')
  echo "  available:         $avail"

  echo ""
  echo "${BOLD}Ports${RESET}"
  for port in $SERVE_PORT $VITE_PORT; do
    local pids
    pids=$(pids_on_port "$port")
    if [ -n "$pids" ]; then
      echo "  ${GREEN}●${RESET} :$port in use (PIDs: $(echo "$pids" | tr '\n' ' '))"
    else
      echo "  ${DIM}○ :$port free${RESET}"
    fi
  done

  echo ""
  echo "${BOLD}Swap${RESET}"
  if [[ "$(uname)" == "Darwin" ]]; then
    sysctl vm.swapusage 2>/dev/null | sed 's/^vm.swapusage: /  /'
  else
    free -h 2>/dev/null | grep -i swap | awk '{print "  total:", $2, " used:", $3, " free:", $4}'
  fi

  echo ""
  ok "$ok_count ok, $warn_count warnings"
}

# ── cmd: dump ────────────────────────────────────────────────
cmd_dump() {
  echo "=== ROKO DEV DUMP ==="
  echo "timestamp: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "branch: $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'unknown')"
  echo "commit: $(git rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
  echo ""

  echo "--- processes ---"
  for pattern in "roko serve" "cargo watch" "vite"; do
    local pids
    pids=$(pgrep -f "$pattern" 2>/dev/null || true)
    if [ -n "$pids" ]; then
      echo "$pattern: running (PIDs: $(echo "$pids" | tr '\n' ' '))"
    else
      echo "$pattern: stopped"
    fi
  done
  echo ""

  echo "--- ports ---"
  for port in $SERVE_PORT $VITE_PORT; do
    local pids
    pids=$(pids_on_port "$port")
    if [ -n "$pids" ]; then
      echo ":$port in use (PIDs: $(echo "$pids" | tr '\n' ' '))"
    else
      echo ":$port free"
    fi
  done
  echo ""

  echo "--- health ---"
  if curl -sf http://localhost:$SERVE_PORT/api/health 2>/dev/null | jq_or_raw .; then
    true
  else
    echo "serve not reachable"
  fi
  echo ""

  echo "--- provider health ---"
  if curl -sf http://localhost:$SERVE_PORT/api/providers/health 2>/dev/null | jq_or_raw .; then
    true
  else
    echo "not available"
  fi
  echo ""

  echo "--- recent errors ---"
  if [ -f "$ROKO_DIR/roko.log" ]; then
    tail -200 "$ROKO_DIR/roko.log" | grep -i -E "error|panic|fatal|fail" | tail -20 || echo "none"
  else
    echo "no log file"
  fi
  echo ""

  echo "--- recent episodes (last 5) ---"
  if [ -f "$ROKO_DIR/episodes.jsonl" ]; then
    tail -5 "$ROKO_DIR/episodes.jsonl" | jq_or_raw .
  else
    echo "no episodes"
  fi
  echo ""

  echo "--- efficiency (last 5) ---"
  if [ -f "$ROKO_DIR/learn/efficiency.jsonl" ]; then
    tail -5 "$ROKO_DIR/learn/efficiency.jsonl" | jq_or_raw .
  else
    echo "no efficiency data"
  fi
  echo ""

  echo "--- cascade router ---"
  if [ -f "$ROKO_DIR/learn/cascade-router.json" ]; then
    jq_or_raw . < "$ROKO_DIR/learn/cascade-router.json"
  else
    echo "no router state"
  fi
  echo ""

  echo "--- gate thresholds ---"
  if [ -f "$ROKO_DIR/learn/gate-thresholds.json" ]; then
    jq_or_raw . < "$ROKO_DIR/learn/gate-thresholds.json"
  else
    echo "no gate thresholds"
  fi
  echo ""

  echo "--- state files ---"
  if [ -d "$ROKO_DIR" ]; then
    find "$ROKO_DIR" -type f \( -name "*.json" -o -name "*.jsonl" -o -name "*.toml" -o -name "*.log" \) -exec ls -lh {} \; 2>/dev/null | awk '{print $5, $NF}'
  else
    echo ".roko/ not found"
  fi
  echo ""

  echo "--- build info ---"
  local bin="$ROKO_BIN"
  if [ -f "$bin" ]; then
    local sz
    sz=$(stat -f%z "$bin" 2>/dev/null || stat -c%s "$bin" 2>/dev/null || echo 0)
    echo "binary: $bin ($(human_size "$sz"))"
    echo "modified: $(stat -f '%Sm' -t '%Y-%m-%d %H:%M' "$bin" 2>/dev/null || stat -c '%y' "$bin" 2>/dev/null || echo unknown)"
  else
    echo "binary: not built"
  fi
  echo "target/ size: $(human_size "$(dir_size_bytes "$TARGET_DIR")")"
  if [ -d "$TARGET_DIR/debug/incremental" ]; then
    echo "incremental: $(human_size "$(dir_size_bytes "$TARGET_DIR/debug/incremental")")"
  fi
  echo ""

  echo "--- git status ---"
  git status --short 2>/dev/null || echo "not a git repo"
  echo ""

  echo "--- system ---"
  echo "os: $(uname -s) $(uname -r) $(uname -m)"
  echo "disk available: $(df -h . 2>/dev/null | awk 'NR==2{print $4}')"
  if [[ "$(uname)" == "Darwin" ]]; then
    echo "swap: $(sysctl vm.swapusage 2>/dev/null | sed 's/^vm.swapusage: //')"
    echo "memory pressure: $(memory_pressure 2>/dev/null | grep 'System-wide' | head -1 || echo 'unavailable')"
  fi
  echo "rustc: $(rustc --version 2>/dev/null || echo 'not found')"
  if command -v sccache &>/dev/null; then
    echo "sccache hit rate: $(sccache --show-stats 2>/dev/null | grep 'Cache hits' | head -1 || echo 'unavailable')"
  fi
  echo ""
  echo "=== END DUMP ==="
}

# ── cmd: nuke-ports ──────────────────────────────────────────
cmd_nuke_ports() {
  for port in $SERVE_PORT $VITE_PORT; do
    local pids
    pids=$(pids_on_port "$port")
    if [ -n "$pids" ]; then
      echo "$pids" | xargs kill -9 2>/dev/null || true
      ok "SIGKILL'd processes on :$port (PIDs: $(echo "$pids" | tr '\n' ' '))"
    else
      info ":$port already free"
    fi
  done
}

# ── cmd: metrics ─────────────────────────────────────────────
cmd_metrics() {
  echo "${BOLD}Build artifacts${RESET}"
  if [ -d "$TARGET_DIR" ]; then
    echo "  target/:           $(human_size "$(dir_size_bytes "$TARGET_DIR")")"
    [ -d "$TARGET_DIR/debug" ] && echo "  target/debug/:     $(human_size "$(dir_size_bytes "$TARGET_DIR/debug")")"
    [ -d "$TARGET_DIR/release" ] && echo "  target/release/:   $(human_size "$(dir_size_bytes "$TARGET_DIR/release")")"
    [ -d "$TARGET_DIR/debug/incremental" ] && echo "  incremental:       $(human_size "$(dir_size_bytes "$TARGET_DIR/debug/incremental")")"
  else
    echo "  ${DIM}No target/ directory${RESET}"
  fi

  echo ""
  echo "${BOLD}Binaries${RESET}"
  for bin in "$TARGET_DIR/debug/roko" "$TARGET_DIR/release/roko"; do
    if [ -f "$bin" ]; then
      local sz
      sz=$(stat -f%z "$bin" 2>/dev/null || stat -c%s "$bin" 2>/dev/null || echo 0)
      echo "  $bin: $(human_size "$sz")"
    fi
  done

  echo ""
  echo "${BOLD}sccache${RESET}"
  if command -v sccache &>/dev/null; then
    sccache --show-stats 2>/dev/null | grep -E "Compile requests|Cache hits|Cache misses|Cache hit rate" | sed 's/^/  /'
    local cache_dir="$HOME/Library/Caches/Mozilla.sccache"
    [ -d "$cache_dir" ] && echo "  Cache size: $(human_size "$(dir_size_bytes "$cache_dir")")"
  else
    echo "  ${DIM}sccache not installed${RESET}"
  fi

  echo ""
  echo "${BOLD}Cost summary${RESET}"
  if [ -f "$ROKO_DIR/learn/efficiency.jsonl" ]; then
    local lines
    lines=$(wc -l < "$ROKO_DIR/learn/efficiency.jsonl" | tr -d ' ')
    echo "  Efficiency events: $lines"
    if command -v jq &>/dev/null; then
      local total_cost
      total_cost=$(jq -s '[.[].cost_usd // 0] | add // 0' < "$ROKO_DIR/learn/efficiency.jsonl" 2>/dev/null || echo "N/A")
      echo "  Total cost: \$$total_cost"
    fi
  else
    echo "  ${DIM}No efficiency data${RESET}"
  fi

  if [ -f "$ROKO_DIR/episodes.jsonl" ]; then
    echo "  Episodes: $(wc -l < "$ROKO_DIR/episodes.jsonl" | tr -d ' ')"
  fi
}

# ── Pipeline helpers ──────────────────────────────────────────

resolve_roko() {
  if command -v roko &>/dev/null; then
    echo "roko"
  elif [ -f "./target/release/roko" ]; then
    echo "./target/release/roko"
  elif [ -f "$ROKO_BIN" ]; then
    echo "$ROKO_BIN"
  else
    die "No roko binary found. Run: ./dev.sh build"
  fi
}

# Create an ephemeral workspace directory with .roko/ layout and config.
# Sets PIPELINE_WORKSPACE to the created path.
create_pipeline_workspace() {
  local name="$1"
  local ts
  ts=$(date +%s)
  PIPELINE_WORKSPACE="${WORKSPACE_BASE}/${name}-${ts}"
  mkdir -p "$PIPELINE_WORKSPACE"

  local roko
  roko=$(resolve_roko)

  # Reset per-step timing arrays for this pipeline run
  PIPELINE_STEP_NAMES=()
  PIPELINE_STEP_TIMES=()
  PIPELINE_STEP_RESULTS=()

  # Init workspace
  info "Creating workspace: $PIPELINE_WORKSPACE"
  if $PIPELINE_DRY_RUN; then
    info "(dry-run) would run: $roko --repo $PIPELINE_WORKSPACE init"
    ok "Workspace ready (dry-run): $PIPELINE_WORKSPACE"
    return 0
  fi

  if ! $roko --repo "$PIPELINE_WORKSPACE" init 2>&1; then
    err "roko init failed in $PIPELINE_WORKSPACE"
    return 1
  fi

  # Overlay project roko.toml so all providers/models are available
  if [ -f "roko.toml" ]; then
    cp roko.toml "$PIPELINE_WORKSPACE/roko.toml"
    info "Copied project roko.toml into workspace"
  fi

  # Init git repo for gate validation
  (
    cd "$PIPELINE_WORKSPACE"
    git init -q
    git config user.email "roko-dev@example.local"
    git config user.name "Roko Dev"
    git add -A
    git commit -q -m "workspace init" --allow-empty
  ) 2>&1 || warn "git init failed (non-fatal)"

  ok "Workspace ready: $PIPELINE_WORKSPACE"
}

# Build the roko command prefix with global flags (--repo, --model).
# Usage: roko_cmd <subcommand>
# Returns the full command string.
roko_cmd() {
  local subcmd="$1"
  local roko
  roko=$(resolve_roko)
  local parts="$roko --repo $PIPELINE_WORKSPACE"

  # --model is a global flag (before subcommand)
  if [ -n "$PIPELINE_MODEL" ]; then
    parts="$parts --model $PIPELINE_MODEL"
  fi

  # --provider only applies to 'run' subcommands
  if [ -n "$PIPELINE_PROVIDER" ] && [[ "$subcmd" == run\ * ]]; then
    parts="$parts $subcmd --provider $PIPELINE_PROVIDER"
  else
    parts="$parts $subcmd"
  fi

  echo "$parts"
}

# Run a single roko command in a workspace, showing the command and timing.
# Usage: run_step <step_num> <total> <description> <roko_subcommand> [timeout_seconds]
run_step() {
  local step_num="$1" total="$2" desc="$3" subcmd="$4" timeout="${5:-300}"
  local full_cmd
  full_cmd=$(roko_cmd "$subcmd")

  echo ""
  echo "${BOLD}[$step_num/$total] $desc${RESET}"
  echo "${DIM}\$ $full_cmd${RESET}"

  if $PIPELINE_DRY_RUN; then
    info "(dry-run) skipped"
    PIPELINE_STEP_NAMES+=("$desc")
    PIPELINE_STEP_TIMES+=("skip")
    PIPELINE_STEP_RESULTS+=("skip")
    return 0
  fi

  if $PIPELINE_INTERRUPTED; then
    info "(interrupted) skipped"
    PIPELINE_STEP_NAMES+=("$desc")
    PIPELINE_STEP_TIMES+=("skip")
    PIPELINE_STEP_RESULTS+=("skip")
    return 130
  fi

  local start_time=$SECONDS
  local exit_code=0
  local output_file
  output_file=$(mktemp)

  # Run in background so Ctrl+C can kill it immediately.
  # We track the PID so the pipeline trap can clean it up.
  # Track output file globally so interrupt handler can clean it too.
  PIPELINE_STEP_OUTPUT="$output_file"
  bash -c "$full_cmd" > "$output_file" 2>&1 &
  PIPELINE_STEP_PID=$!

  # Wait with timeout: poll every second, kill if exceeded.
  # Progress indicator every 30s so the user sees it's alive.
  local waited=0
  while kill -0 "$PIPELINE_STEP_PID" 2>/dev/null; do
    if $PIPELINE_INTERRUPTED; then
      kill "$PIPELINE_STEP_PID" 2>/dev/null || true
      sleep 1
      if kill -0 "$PIPELINE_STEP_PID" 2>/dev/null; then
        kill -9 "$PIPELINE_STEP_PID" 2>/dev/null || true
      fi
      wait "$PIPELINE_STEP_PID" 2>/dev/null || true
      PIPELINE_STEP_PID=""
      rm -f "$output_file"
      PIPELINE_STEP_OUTPUT=""
      PIPELINE_STEP_NAMES+=("$desc")
      PIPELINE_STEP_TIMES+=("${waited}s")
      PIPELINE_STEP_RESULTS+=("interrupted")
      return 130
    fi
    if [ "$waited" -ge "$timeout" ]; then
      warn "Step $step_num timed out after ${timeout}s — sending SIGTERM"
      kill "$PIPELINE_STEP_PID" 2>/dev/null || true
      sleep 2
      # Escalate to SIGKILL if still alive
      if kill -0 "$PIPELINE_STEP_PID" 2>/dev/null; then
        warn "Process still alive — sending SIGKILL"
        kill -9 "$PIPELINE_STEP_PID" 2>/dev/null || true
      fi
      wait "$PIPELINE_STEP_PID" 2>/dev/null || true
      PIPELINE_STEP_PID=""
      cat "$output_file"
      rm -f "$output_file"
      PIPELINE_STEP_OUTPUT=""
      PIPELINE_STEP_NAMES+=("$desc")
      PIPELINE_STEP_TIMES+=("${timeout}s")
      PIPELINE_STEP_RESULTS+=("timeout")
      return 124
    fi
    sleep 1
    waited=$(( waited + 1 ))
    # Progress indicator every 30s
    if [ $(( waited % 30 )) -eq 0 ]; then
      echo "${DIM}  ... ${waited}s / ${timeout}s${RESET}"
    fi
  done

  # Process exited naturally. Clear PID before wait to avoid race with interrupt handler.
  local finished_pid="$PIPELINE_STEP_PID"
  PIPELINE_STEP_PID=""
  wait "$finished_pid" 2>/dev/null
  exit_code=$?

  local elapsed=$(( SECONDS - start_time ))

  # Show output
  cat "$output_file"

  # Record step timing
  PIPELINE_STEP_NAMES+=("$desc")
  PIPELINE_STEP_TIMES+=("${elapsed}s")
  if [ "$exit_code" -eq 0 ]; then
    PIPELINE_STEP_RESULTS+=("pass")
  else
    PIPELINE_STEP_RESULTS+=("FAIL")
  fi

  if [ "$exit_code" -eq 0 ]; then
    ok "Step $step_num passed (${elapsed}s)"
  else
    err "Step $step_num failed (exit=$exit_code, ${elapsed}s)"
    echo ""
    echo "${BOLD}--- step $step_num error details ---${RESET}"

    # Quick context: last 5 lines of step output
    echo "${DIM}Last 5 lines of output:${RESET}"
    tail -5 "$output_file" 2>/dev/null | sed 's/^/  /'

    # Show workspace state for debugging
    echo "${DIM}Workspace: $PIPELINE_WORKSPACE${RESET}"
    if [ -f "$PIPELINE_WORKSPACE/.roko/roko.log" ]; then
      echo "${DIM}roko.log (last 5 lines):${RESET}"
      tail -5 "$PIPELINE_WORKSPACE/.roko/roko.log" 2>/dev/null | sed 's/^/  /'
    fi
    if [ -f "$PIPELINE_WORKSPACE/.roko/episodes.jsonl" ]; then
      echo "${DIM}Last episode:${RESET}"
      tail -1 "$PIPELINE_WORKSPACE/.roko/episodes.jsonl" 2>/dev/null | jq_or_raw . | sed 's/^/  /'
    fi
    echo "${BOLD}--- end error details ---${RESET}"
  fi

  rm -f "$output_file"
  PIPELINE_STEP_OUTPUT=""
  return $exit_code
}

# Dump full workspace diagnostics after a pipeline run.
dump_pipeline_workspace() {
  local ws="$1"
  echo ""
  echo "${BOLD}=== PIPELINE WORKSPACE DUMP ===${RESET}"
  echo "path: $ws"
  echo ""

  echo "--- state files ---"
  if [ -d "$ws/.roko" ]; then
    find "$ws/.roko" -type f \( -name "*.json" -o -name "*.jsonl" -o -name "*.toml" -o -name "*.log" \) 2>/dev/null | while read -r f; do
      local sz
      sz=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f" 2>/dev/null || echo 0)
      local lines=""
      if [[ "$f" == *.jsonl ]]; then
        lines=" ($(wc -l < "$f" | tr -d ' ') lines)"
      fi
      echo "  ${f#$ws/}  $(human_size "$sz")$lines"
    done
  fi
  echo ""

  echo "--- episodes (last 20) ---"
  if [ -f "$ws/.roko/episodes.jsonl" ]; then
    local ep_lines
    ep_lines=$(wc -l < "$ws/.roko/episodes.jsonl" | tr -d ' ')
    if [ "$ep_lines" -gt 20 ]; then
      echo "  ($ep_lines total, showing last 20)"
    fi
    tail -20 "$ws/.roko/episodes.jsonl" | jq_or_raw . 2>/dev/null
  else
    echo "  none"
  fi
  echo ""

  echo "--- efficiency (last 20) ---"
  if [ -f "$ws/.roko/learn/efficiency.jsonl" ]; then
    local eff_lines
    eff_lines=$(wc -l < "$ws/.roko/learn/efficiency.jsonl" | tr -d ' ')
    if [ "$eff_lines" -gt 20 ]; then
      echo "  ($eff_lines total, showing last 20)"
    fi
    tail -20 "$ws/.roko/learn/efficiency.jsonl" | jq_or_raw . 2>/dev/null
  else
    echo "  none"
  fi
  echo ""

  echo "--- errors (last 20) ---"
  if [ -f "$ws/.roko/roko.log" ]; then
    grep -i -E "error|panic|fatal|fail" "$ws/.roko/roko.log" 2>/dev/null | tail -20 || echo "  none"
  else
    echo "  no log file"
  fi
  echo ""

  echo "--- cascade router ---"
  if [ -f "$ws/.roko/learn/cascade-router.json" ]; then
    jq_or_raw . < "$ws/.roko/learn/cascade-router.json" 2>/dev/null
  else
    echo "  none"
  fi
  echo ""

  echo "--- plans ---"
  if [ -d "$ws/.roko/plans" ]; then
    find "$ws/.roko/plans" -type f -name "*.toml" 2>/dev/null | while read -r f; do
      echo "  ${f#$ws/}"
    done
  fi
  echo ""

  echo "--- prd ---"
  if [ -d "$ws/.roko/prd" ]; then
    find "$ws/.roko/prd" -type f 2>/dev/null | while read -r f; do
      echo "  ${f#$ws/}"
    done
  fi
  echo ""

  echo "--- git log ---"
  (cd "$ws" && git log --oneline -10 2>/dev/null) || echo "  not a git repo"

  echo "${BOLD}=== END WORKSPACE DUMP ===${RESET}"
}

# Pre-flight check: verify workspace and provider are usable before running a pipeline.
preflight_check() {
  local ws="$1"
  local roko
  roko=$(resolve_roko)

  if $PIPELINE_DRY_RUN; then
    info "(dry-run) would run preflight checks for $ws"
    return 0
  fi

  # Validate workspace structure
  if [ ! -d "$ws" ]; then
    err "Preflight: workspace directory does not exist: $ws"
    return 1
  fi
  if [ ! -d "$ws/.roko" ]; then
    err "Preflight: workspace missing .roko/ directory: $ws"
    return 1
  fi
  if [ ! -f "$ws/roko.toml" ]; then
    err "Preflight: workspace missing roko.toml: $ws"
    return 1
  fi

  # Verify roko binary actually runs
  if ! "$roko" --version >/dev/null 2>&1; then
    err "Preflight: roko binary at $roko is not executable or crashes"
    return 1
  fi

  # Check provider health
  local output
  if ! output=$($roko --repo "$ws" config providers health 2>&1); then
    warn "Provider health check failed:"
    echo "$output" | sed 's/^/  /'
    return 1
  fi
  ok "Preflight passed (workspace + provider)"
  return 0
}

# ── Pipeline definitions ────────────────────────────────────

pipeline_prd() {
  local keep="$1"
  create_pipeline_workspace "prd-pipeline" || return 1
  preflight_check "$PIPELINE_WORKSPACE" || return 1

  local total=7
  local failed=0
  local start_time=$SECONDS

  echo ""
  echo "${BOLD}PRD Pipeline${RESET}: idea → draft → promote → plan → validate → execute → status"
  echo "${DIM}Workspace: $PIPELINE_WORKSPACE${RESET}"
  echo ""

  run_step 1 $total "Capture work item" \
    'prd idea "Build a CLI that fetches BTC funding rates from Binance, calculates average funding over 7 days, and alerts when funding exceeds 0.1%"' \
    30 || failed=1

  if [ "$failed" -eq 0 ]; then
    run_step 2 $total "Generate PRD via LLM" \
      'prd draft new "BTC Funding Alert CLI"' \
      180 || failed=1
  fi

  if [ "$failed" -eq 0 ]; then
    run_step 3 $total "Promote to published" \
      'prd draft promote btc-funding-alert-cli' \
      30 || failed=1
  fi

  if [ "$failed" -eq 0 ]; then
    run_step 4 $total "Generate implementation plan" \
      'prd plan btc-funding-alert-cli' \
      300 || failed=1
  fi

  if [ "$failed" -eq 0 ]; then
    run_step 5 $total "Lint the generated plan" \
      'plan validate .roko/plans' \
      30 || failed=1
  fi

  if [ "$failed" -eq 0 ]; then
    run_step 6 $total "Execute: agents + gates" \
      'plan run .roko/plans --max-retries 1' \
      600 || failed=1
  fi

  # Always run status even if earlier steps failed
  run_step 7 $total "View results and costs" \
    'status' \
    30 || true

  pipeline_finish "$failed" "$start_time" "$keep"
}

pipeline_research() {
  local keep="$1"
  create_pipeline_workspace "research-loop" || return 1
  preflight_check "$PIPELINE_WORKSPACE" || return 1

  local total=8
  local failed=0
  local start_time=$SECONDS

  echo ""
  echo "${BOLD}Research Loop${RESET}: idea → draft → research → plan → execute → gates → learn → summary"
  echo "${DIM}Workspace: $PIPELINE_WORKSPACE${RESET}"
  echo ""

  run_step 1 $total "Capture idea" \
    'prd idea "Add config validation with schema checking and helpful error messages"' \
    30 || failed=1

  if [ "$failed" -eq 0 ]; then
    run_step 2 $total "Draft PRD" \
      'prd draft new cli-config-validation' \
      120 || failed=1
  fi

  if [ "$failed" -eq 0 ]; then
    run_step 3 $total "Research enhance PRD" \
      'research enhance-prd cli-config-validation' \
      180 || failed=1
  fi

  if [ "$failed" -eq 0 ]; then
    run_step 4 $total "Generate plan (research-informed)" \
      'prd plan cli-config-validation' \
      180 || failed=1
  fi

  if [ "$failed" -eq 0 ]; then
    run_step 5 $total "Execute plan" \
      'plan run .roko/plans --max-retries 1' \
      300 || failed=1
  fi

  run_step 6 $total "Learning state" \
    'learn all' \
    30 || true

  run_step 7 $total "Tune routing" \
    'learn tune routing' \
    30 || true

  run_step 8 $total "Status + efficiency" \
    'status' \
    30 || true

  pipeline_finish "$failed" "$start_time" "$keep"
}

pipeline_cost_race() {
  local keep="$1"
  create_pipeline_workspace "cost-race" || return 1
  preflight_check "$PIPELINE_WORKSPACE" || return 1

  local total=3
  local failed=0
  local start_time=$SECONDS

  echo ""
  echo "${BOLD}Cost Race${RESET}: naive (no replan) vs cascade (full pipeline)"
  [ -n "$PIPELINE_MODEL" ] && echo "${DIM}Model: $PIPELINE_MODEL${RESET}"
  [ -n "$PIPELINE_PROVIDER" ] && echo "${DIM}Provider: $PIPELINE_PROVIDER${RESET}"
  echo "${DIM}Workspace: $PIPELINE_WORKSPACE${RESET}"
  echo ""

  local prompt='Build a CLI calculator in Rust'

  run_step 1 $total "Naive run (--no-replan)" \
    "run \"$prompt\" --no-replan" \
    180 || failed=1

  run_step 2 $total "Cascade run (full pipeline)" \
    "run \"$prompt\"" \
    180 || failed=1

  run_step 3 $total "Status" 'status' 30 || true

  pipeline_finish "$failed" "$start_time" "$keep"
}

pipeline_gate_retry() {
  local keep="$1"
  create_pipeline_workspace "gate-retry" || return 1
  preflight_check "$PIPELINE_WORKSPACE" || return 1

  local total=5
  local failed=0
  local start_time=$SECONDS

  echo ""
  echo "${BOLD}Gate Retry${RESET}: run → fail gates → classify → replan → retry"
  [ -n "$PIPELINE_MODEL" ] && echo "${DIM}Model: $PIPELINE_MODEL${RESET}"
  [ -n "$PIPELINE_PROVIDER" ] && echo "${DIM}Provider: $PIPELINE_PROVIDER${RESET}"
  echo "${DIM}Workspace: $PIPELINE_WORKSPACE${RESET}"
  echo ""

  # replan_on_gate_failure is already true in the project roko.toml (copied at workspace creation)

  run_step 1 $total "Run with retries" \
    'run "Build a small Rust async HTTP client with exponential backoff, JSON config loading, and focused tests. Keep compile, test, and clippy green." --max-retries 2' \
    360 || failed=1

  run_step 2 $total "Gate tuning state" \
    'learn tune gates' \
    30 || true

  run_step 3 $total "Full learning state" \
    'learn all' \
    30 || true

  run_step 4 $total "Efficiency metrics" \
    'learn efficiency' \
    30 || true

  run_step 5 $total "Final status" \
    'status' \
    30 || true

  pipeline_finish "$failed" "$start_time" "$keep"
}

pipeline_providers() {
  local keep="$1"
  create_pipeline_workspace "providers" || return 1
  preflight_check "$PIPELINE_WORKSPACE" || return 1

  local failed=0
  local start_time=$SECONDS

  # Use custom list or defaults
  local providers_str="${PIPELINE_PROVIDERS_LIST:-anthropic,openai,gemini,moonshot}"
  IFS=',' read -ra providers <<< "$providers_str"
  local total=${#providers[@]}

  echo ""
  echo "${BOLD}Provider Test${RESET}: same prompt to $total providers sequentially"
  [ -n "$PIPELINE_MODEL" ] && echo "${DIM}Model: $PIPELINE_MODEL${RESET}"
  echo "${DIM}Providers: ${providers[*]}${RESET}"
  echo "${DIM}Workspace: $PIPELINE_WORKSPACE${RESET}"
  echo ""

  local prompt='Build a hello-world web server'

  # Save and clear PIPELINE_PROVIDER — we set it per-step here
  local saved_provider="$PIPELINE_PROVIDER"
  PIPELINE_PROVIDER=""

  local provider_failures=0
  for i in "${!providers[@]}"; do
    local p="${providers[$i]}"
    local step=$(( i + 1 ))
    PIPELINE_PROVIDER="$p"
    run_step $step $total "Provider: $p" \
      "run \"$prompt\"" \
      180 || { warn "Provider $p failed"; provider_failures=$(( provider_failures + 1 )); }
  done

  # Fail pipeline only if ALL providers failed
  if [ "$provider_failures" -eq "$total" ]; then
    failed=1
    err "All $total providers failed"
  elif [ "$provider_failures" -gt 0 ]; then
    warn "$provider_failures/$total providers failed (pipeline passes)"
  fi

  PIPELINE_PROVIDER="$saved_provider"
  pipeline_finish "$failed" "$start_time" "$keep"
}

# Print the per-step timing summary table.
# Usage: print_step_summary <total_elapsed_seconds>
print_step_summary() {
  local total_elapsed="$1"
  if [ "${#PIPELINE_STEP_NAMES[@]}" -gt 0 ]; then
    echo ""
    echo "${BOLD}Step Timing Summary${RESET}"
    echo "${DIM}──────────────────────────────────────────────────────────${RESET}"
    printf "  ${DIM}%-4s %-40s %8s %s${RESET}\n" "#" "Step" "Time" "Result"
    echo "${DIM}  ──── ──────────────────────────────────────── ──────── ──────${RESET}"
    for i in "${!PIPELINE_STEP_NAMES[@]}"; do
      local step_n=$(( i + 1 ))
      local name="${PIPELINE_STEP_NAMES[$i]}"
      local time="${PIPELINE_STEP_TIMES[$i]}"
      local result="${PIPELINE_STEP_RESULTS[$i]}"
      local color="$GREEN"
      case "$result" in
        FAIL)        color="$RED" ;;
        timeout)     color="$RED" ;;
        interrupted) color="$YELLOW" ;;
        skip)        color="$DIM" ;;
      esac
      printf "  %-4s %-40s %8s ${color}%s${RESET}\n" "$step_n" "$name" "$time" "$result"
    done
    echo "${DIM}  ──── ──────────────────────────────────────── ──────── ──────${RESET}"
    printf "  %-4s %-40s ${BOLD}%8s${RESET}\n" "" "Total" "${total_elapsed}s"
  fi
}

# Common finish logic for all pipelines
pipeline_finish() {
  local failed="$1" start_time="$2" keep="$3"
  local total_elapsed=$(( SECONDS - start_time ))

  # Kill any orphaned step process before clearing the trap
  if [ -n "${PIPELINE_STEP_PID:-}" ] && kill -0 "$PIPELINE_STEP_PID" 2>/dev/null; then
    warn "Orphaned step process $PIPELINE_STEP_PID still alive — killing"
    kill "$PIPELINE_STEP_PID" 2>/dev/null || true
    sleep 1
    if kill -0 "$PIPELINE_STEP_PID" 2>/dev/null; then
      kill -9 "$PIPELINE_STEP_PID" 2>/dev/null || true
    fi
    wait "$PIPELINE_STEP_PID" 2>/dev/null || true
    PIPELINE_STEP_PID=""
  fi

  # Restore default signal handling
  trap - INT TERM

  if $PIPELINE_INTERRUPTED; then
    echo ""
    if [ -d "${PIPELINE_WORKSPACE:-}" ] && ! $PIPELINE_DRY_RUN; then
      dump_pipeline_workspace "$PIPELINE_WORKSPACE"
      echo ""
      if $keep; then
        ok "Workspace preserved (interrupted): $PIPELINE_WORKSPACE"
      else
        info "Cleaning up workspace..."
        rm -rf "$PIPELINE_WORKSPACE"
        ok "Workspace removed."
      fi
    fi
    print_step_summary "$total_elapsed"

    err "Pipeline interrupted after ${total_elapsed}s."
    exit 130
  fi

  if ! $PIPELINE_DRY_RUN; then
    echo ""
    dump_pipeline_workspace "$PIPELINE_WORKSPACE"
  fi

  print_step_summary "$total_elapsed"

  echo ""
  if [ "$failed" -eq 0 ]; then
    ok "Pipeline completed in ${total_elapsed}s."
  else
    err "Pipeline had failures after ${total_elapsed}s."
  fi

  if $keep; then
    ok "Workspace preserved: $PIPELINE_WORKSPACE"
  elif $PIPELINE_DRY_RUN; then
    rm -rf "$PIPELINE_WORKSPACE"
    info "(dry-run) workspace removed."
  else
    info "Cleaning up workspace..."
    rm -rf "$PIPELINE_WORKSPACE"
    ok "Workspace removed."
  fi

  return "$failed"
}

# ── cmd: pipeline ────────────────────────────────────────────
cmd_pipeline() {
  local scenario="${1:-}"
  shift 2>/dev/null || true
  local keep=false
  local bail=false

  # Parse flags
  while [ $# -gt 0 ]; do
    case "$1" in
      --keep|-k)             keep=true ;;
      --bail|-b)             bail=true ;;
      --model|-m)            PIPELINE_MODEL="${2:?--model requires a value}"; shift ;;
      --model=*)             PIPELINE_MODEL="${1#--model=}" ;;
      --provider|-P)         PIPELINE_PROVIDER="${2:?--provider requires a value}"; shift ;;
      --provider=*)          PIPELINE_PROVIDER="${1#--provider=}" ;;
      --providers)           PIPELINE_PROVIDERS_LIST="${2:?--providers requires a comma-separated list}"; shift ;;
      --providers=*)         PIPELINE_PROVIDERS_LIST="${1#--providers=}" ;;
      --dry-run|-n)          PIPELINE_DRY_RUN=true ;;
      *) die "Unknown option: $1" ;;
    esac
    shift
  done

  # Ensure roko binary exists
  resolve_roko >/dev/null

  # Ensure workspace base exists
  mkdir -p "$WORKSPACE_BASE"

  # Install Ctrl+C handler for pipelines
  PIPELINE_INTERRUPTED=false
  pipeline_interrupt() {
    echo ""
    warn "Interrupted! Cleaning up..."
    PIPELINE_INTERRUPTED=true
    # Kill the current step if running — SIGTERM first, then SIGKILL escalation
    if [ -n "$PIPELINE_STEP_PID" ] && kill -0 "$PIPELINE_STEP_PID" 2>/dev/null; then
      kill "$PIPELINE_STEP_PID" 2>/dev/null || true
      # Give process 2s to exit gracefully
      local grace=0
      while [ "$grace" -lt 20 ] && kill -0 "$PIPELINE_STEP_PID" 2>/dev/null; do
        sleep 0.1
        grace=$(( grace + 1 ))
      done
      # Escalate to SIGKILL if still alive
      if kill -0 "$PIPELINE_STEP_PID" 2>/dev/null; then
        warn "Process $PIPELINE_STEP_PID did not exit — sending SIGKILL"
        kill -9 "$PIPELINE_STEP_PID" 2>/dev/null || true
      fi
      wait "$PIPELINE_STEP_PID" 2>/dev/null || true
      PIPELINE_STEP_PID=""
    fi
    # Clean up any temp output file from run_step
    if [ -n "${PIPELINE_STEP_OUTPUT:-}" ]; then
      rm -f "$PIPELINE_STEP_OUTPUT"
      PIPELINE_STEP_OUTPUT=""
    fi
  }
  trap pipeline_interrupt INT TERM

  # Show active overrides
  if [ -n "$PIPELINE_MODEL" ] || [ -n "$PIPELINE_PROVIDER" ]; then
    local overrides=""
    [ -n "$PIPELINE_MODEL" ] && overrides="model=$PIPELINE_MODEL"
    [ -n "$PIPELINE_PROVIDER" ] && overrides="${overrides:+$overrides, }provider=$PIPELINE_PROVIDER"
    info "Overrides: $overrides"
  fi
  $PIPELINE_DRY_RUN && info "Dry-run mode: showing commands without executing"

  case "$scenario" in
    prd|prd-pipeline)
      pipeline_prd $keep ;;
    research|research-loop)
      pipeline_research $keep ;;
    race|cost-race)
      pipeline_cost_race $keep ;;
    gate|gate-retry)
      pipeline_gate_retry $keep ;;
    providers)
      pipeline_providers $keep ;;
    all)
      local any_failed=0
      for s in prd research race gate providers; do
        echo ""
        echo "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
        echo "${BOLD}  Pipeline: $s${RESET}"
        echo "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
        cmd_pipeline "$s" $(${keep} && echo "--keep") || any_failed=1
        echo ""
        if $bail && [ "$any_failed" -eq 1 ]; then
          err "Bailing after first pipeline failure (--bail)."
          return 1
        fi
      done
      if [ "$any_failed" -eq 1 ]; then
        err "Some pipelines had failures."
        return 1
      else
        ok "All pipelines passed."
      fi
      ;;
    list|"")
      echo "${BOLD}Available pipelines${RESET}"
      echo ""
      echo "  ${CYAN}prd${RESET}         PRD Pipeline: idea → draft → promote → plan → validate → execute → status"
      echo "  ${CYAN}research${RESET}    Research Loop: idea → draft → research-enhance → plan → execute → learn"
      echo "  ${CYAN}race${RESET}        Cost Race: naive (no replan) vs cascade (full pipeline)"
      echo "  ${CYAN}gate${RESET}        Gate Retry: run → fail gates → classify → replan → retry"
      echo "  ${CYAN}providers${RESET}   Provider Test: same prompt to N providers"
      echo "  ${CYAN}all${RESET}         Run all pipelines sequentially"
      echo ""
      echo "${BOLD}Options${RESET}"
      echo "  --keep, -k                 Preserve workspace after run (default: clean up)"
      echo "  --bail, -b                 Stop 'all' after first pipeline failure"
      echo "  --model, -m <model>        Use specific model for all LLM calls (global roko flag)"
      echo "  --provider, -P <provider>  Use specific provider for 'run' commands"
      echo "  --providers <p1,p2,...>     Override provider list for 'providers' pipeline"
      echo "  --dry-run, -n              Show commands without executing"
      echo ""
      echo "${BOLD}Examples${RESET}"
      echo "  ./dev.sh pipeline prd                              Run with default model"
      echo "  ./dev.sh pipeline prd --model glm-4-plus           Run with specific model"
      echo "  ./dev.sh pipeline prd --provider anthropic         Force anthropic for run steps"
      echo "  ./dev.sh pipeline gate --model gpt-4o --keep       Gate retry with GPT-4o, keep workspace"
      echo "  ./dev.sh pipeline race --model claude-sonnet-4-20250514   Compare naive vs cascade with Sonnet"
      echo "  ./dev.sh pipeline providers --providers anthropic,openai  Test only 2 providers"
      echo "  ./dev.sh pipeline all --model glm-4-plus           Full suite with one model"
      echo "  ./dev.sh pipeline prd --dry-run                    Preview commands without running"
      echo "  ./dev.sh pipeline prd --model gpt-4o 2>&1 | tee run.log   Capture output"
      ;;
    *)
      die "Unknown pipeline: $scenario (run ./dev.sh pipeline list)"
      ;;
  esac
}

# ── cmd: clean-workspaces ────────────────────────────────────
cmd_clean_workspaces() {
  if [ ! -d "$WORKSPACE_BASE" ]; then
    ok "No pipeline workspaces found."
    return
  fi

  local count=0
  local total_size=0

  echo "${BOLD}Pipeline workspaces${RESET}"
  for d in "$WORKSPACE_BASE"/*/; do
    [ -d "$d" ] || continue
    local sz
    sz=$(dir_size_bytes "$d")
    total_size=$(( total_size + sz ))
    count=$(( count + 1 ))
    local name
    name=$(basename "$d")
    echo "  $name  $(human_size "$sz")"
  done

  if [ "$count" -eq 0 ]; then
    ok "No workspaces to clean."
    return
  fi

  echo ""
  echo "  Total: $count workspace(s), $(human_size "$total_size")"

  if [ "${1:-}" = "--confirm" ]; then
    rm -rf "$WORKSPACE_BASE"
    ok "Removed $count workspace(s), freed $(human_size "$total_size")."
  else
    echo ""
    warn "Pass --confirm to delete all, or remove individually:"
    echo "  rm -rf $WORKSPACE_BASE/<name>"
  fi
}

# ── cmd: reset-state ─────────────────────────────────────────
cmd_reset_state() {
  if [ "${1:-}" != "--confirm" ]; then
    warn "This will delete all data in .roko/ (signals, episodes, learning state)."
    warn "Directory structure will be preserved."
    die "Pass --confirm to proceed."
  fi

  if [ ! -d "$ROKO_DIR" ]; then
    die ".roko/ does not exist."
  fi

  info "Clearing .roko/ data..."
  find "$ROKO_DIR" -type f \( -name "*.jsonl" -o -name "*.json" -o -name "*.log" \) -delete 2>/dev/null
  ok "Cleared. Directory structure preserved."
}

# ── cmd: help ────────────────────────────────────────────────
cmd_mirage() {
  info "Starting mirage-rs in mainnet fork mode..."

  # Kill any existing mirage on port 8545
  local pids
  pids=$(pids_on_port 8545)
  if [ -n "$pids" ]; then
    echo "$pids" | xargs kill 2>/dev/null || true
    info "Killed existing mirage on :8545"
    sleep 1
  fi

  # Start mirage-rs in fork mode
  local mirage_bin
  if command -v mirage-rs &>/dev/null; then
    mirage_bin="mirage-rs"
  elif [ -x "./target/release/mirage-rs" ]; then
    mirage_bin="./target/release/mirage-rs"
  else
    die "mirage-rs not found. Run 'cargo build --release -p mirage-rs' first."
  fi
  "$mirage_bin" \
    --host 127.0.0.1 \
    --port 8545 \
    --rpc-url https://ethereum-rpc.publicnode.com &
  local mirage_pid=$!
  info "mirage-rs started pid=${mirage_pid}"

  # Build contracts if needed
  if [ -d "demo-ide/demo/contracts" ] && [ ! -d "demo-ide/demo/contracts/out" ]; then
    if command -v forge &>/dev/null; then
      info "Building ISFR contracts (first time)..."
      (cd demo-ide/demo/contracts && forge build) || warn "forge build failed"
    else
      warn "forge not found, skipping contract build"
    fi
  fi

  # Wait for health check
  local attempts=30
  for _ in $(seq 1 "$attempts"); do
    if curl -fsS http://127.0.0.1:8545/health >/dev/null 2>&1; then
      ok "mirage-rs ready at http://127.0.0.1:8545 (mainnet fork)"
      return 0
    fi
    sleep 1
  done

  err "mirage-rs did not become ready after ${attempts}s"
  return 1
}

cmd_help() {
  cat <<'HELP'
dev.sh — Roko dev toolkit

USAGE
  ./dev.sh <command> [options]

COMMANDS
  up [--watch] [--no-vite] [--release] [--chain]
                                         Start dev environment
                                         --chain/-c: start mirage-rs for live ISFR data
  down                                     Kill all roko dev processes
  status (st)                              Processes, ports, health, state
  logs <target> (l)                        Tail logs (serve|episodes|efficiency|events|runtime|errors|all)
  check [--fix] (ci)                       Pre-commit checks (fmt+clippy+test)
  fmt                                      Format (cargo +nightly fmt --all)
  clippy (lint)                            Lint (cargo clippy)
  test [crate] (t)                         Run tests (nextest if available)
  build [--release] [crate] (b)            Build with timing + binary size
  clean [mode]                             Clean artifacts (-i|--incremental|--target|--all|--sccache)
  doctor (doc)                             Toolchain + disk + ports + swap diagnostics
  dump                                     Full diagnostic snapshot (pipe to Claude)
  nuke-ports                               SIGKILL everything on :6677 and :5173
  metrics (m)                              Build sizes, sccache stats, cost summary
  reset-state --confirm                    Clear .roko/ data (preserves structure)
  pipeline <name> [options] (p)             Run demo pipeline end-to-end in ephemeral workspace
  clean-workspaces [--confirm]             List/remove pipeline workspaces
  mirage                                   Start mirage-rs in mainnet fork mode (:8545)
  help                                     This message

PIPELINES (./dev.sh pipeline <name>)
  prd           PRD Pipeline: idea → draft → promote → plan → validate → execute → status
  research      Research Loop: idea → draft → research-enhance → plan → execute → learn
  race          Cost Race: naive vs cascade routing
  gate          Gate Retry: fail → classify → replan → retry
  providers     Provider Test: same prompt to N providers
  all           Run all pipelines sequentially

PIPELINE OPTIONS
  --keep, -k                 Preserve workspace after run
  --model, -m <model>        Model for all LLM calls (e.g. glm-4-plus, gpt-4o, claude-sonnet-4-20250514)
  --provider, -P <provider>  Provider for 'run' steps (e.g. anthropic, openai, zhipu)
  --providers <p1,p2,...>     Override provider list for 'providers' pipeline
  --dry-run, -n              Show commands without executing

EXAMPLES
  ./dev.sh up                                          Start serve + vite
  ./dev.sh dump | pbcopy                               Diagnostics for Claude
  ./dev.sh check --fix                                 Format, then lint + test
  ./dev.sh pipeline prd                                Run PRD pipeline (default model)
  ./dev.sh pipeline prd --model gpt-4o                 Run with GPT-4o
  ./dev.sh pipeline gate --provider anthropic --keep   Gate retry via Anthropic
  ./dev.sh pipeline providers --providers anthropic,openai  Test 2 providers
  ./dev.sh pipeline all --model glm-4-plus             Full suite with one model
  ./dev.sh pipeline prd --dry-run                      Preview commands
HELP
}

# ── Main dispatch ────────────────────────────────────────────
cmd="${1:-help}"
shift 2>/dev/null || true

case "$cmd" in
  up)                      cmd_up "$@" ;;
  down)                    cmd_down ;;
  status|st)               cmd_status ;;
  logs|l)                  cmd_logs "$@" ;;
  check|ci)                cmd_check "$@" ;;
  fmt)                     cmd_fmt ;;
  clippy|lint)             cmd_clippy ;;
  test|t)                  cmd_test "$@" ;;
  build|b)                 cmd_build "$@" ;;
  clean)                   cmd_clean "$@" ;;
  doctor|doc)              cmd_doctor ;;
  dump)                    cmd_dump ;;
  nuke-ports)              cmd_nuke_ports ;;
  metrics|m)               cmd_metrics ;;
  reset-state)             cmd_reset_state "$@" ;;
  pipeline|p)              cmd_pipeline "$@" ;;
  clean-workspaces)        cmd_clean_workspaces "$@" ;;
  mirage)                  cmd_mirage ;;
  help|-h|--help)          cmd_help ;;
  *)                       die "Unknown command: $cmd (run ./dev.sh help)" ;;
esac
