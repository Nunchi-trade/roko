#!/usr/bin/env bash

set -uo pipefail

: "${ROKO_ROOT:=/Users/will/dev/nunchi/roko/roko}"
: "${UX_ROOT:=$ROKO_ROOT/tmp/ux-refactoring}"
: "${LOG_ROOT:=$UX_ROOT/logs}"
: "${PROMPTS_DIR:=$UX_ROOT/prompts}"
: "${CONTEXT_DIR:=$UX_ROOT/context-pack}"
: "${WORKTREE_ROOT:=$ROKO_ROOT/.roko/worktrees}"

if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
  C_RESET=$'\e[0m'
  C_BOLD=$'\e[1m'
  C_DIM=$'\e[2m'
  C_RED=$'\e[31m'
  C_GREEN=$'\e[32m'
  C_YELLOW=$'\e[33m'
  C_BLUE=$'\e[34m'
  C_MAGENTA=$'\e[35m'
  C_CYAN=$'\e[36m'
else
  C_RESET='' C_BOLD='' C_DIM='' C_RED='' C_GREEN='' C_YELLOW='' C_BLUE='' C_MAGENTA='' C_CYAN=''
fi

log_info()   { printf '%s[INFO]%s  %s%-10s%s %s\n' "$C_BLUE"   "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2"; }
log_ok()     { printf '%s[OK]%s    %s%-10s%s %s\n' "$C_GREEN"  "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2"; }
log_warn()   { printf '%s[WARN]%s  %s%-10s%s %s\n' "$C_YELLOW" "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2" >&2; }
log_err()    { printf '%s[ERR]%s   %s%-10s%s %s\n' "$C_RED"    "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2" >&2; }
log_header() { printf '\n%s=== %s ===%s\n\n' "$C_BOLD$C_MAGENTA" "$1" "$C_RESET"; }

ALL_BATCHES=(
  "A1"
  "A2"
  "B1"
  "B2"
  "C1"
  "C2"
  "D1"
  "E1"
  "D2"
  "D3"
  "F1"
  "F2"
)

success_status() {
  case "${1:-}" in
    success|success_noop|verified|skipped) return 0 ;;
    *) return 1 ;;
  esac
}

terminal_failure_status() {
  case "${1:-}" in
    spawn_failed|verify_failed|timeout|blocked) return 0 ;;
    *) return 1 ;;
  esac
}

fmt_duration() {
  local s="${1:-0}"
  local h=$((s / 3600))
  local m=$(((s % 3600) / 60))
  local sec=$((s % 60))
  if (( h > 0 )); then
    printf '%dh %dm %ds' "$h" "$m" "$sec"
  elif (( m > 0 )); then
    printf '%dm %ds' "$m" "$sec"
  else
    printf '%ds' "$sec"
  fi
}

ensure_dir() {
  mkdir -p "$1"
}

require_file() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    log_err "bootstrap" "Missing file: $path"
    exit 1
  fi
}

latest_run_id() {
  [[ -d "$LOG_ROOT" ]] || return 1
  find "$LOG_ROOT" -maxdepth 1 -mindepth 1 -type f -name manifest.env -exec dirname {} \; \
    | xargs -n 1 basename \
    | sort \
    | tail -1
}

run_manifest_file() { echo "$LOG_ROOT/$1/manifest.env"; }
run_result_file() { echo "$LOG_ROOT/$1/$2.result"; }
run_log_file() { echo "$LOG_ROOT/$1/$2.log"; }
run_prompts_dir() { echo "$LOG_ROOT/$1/prompts"; }
run_prompt_snapshot() { echo "$(run_prompts_dir "$1")/$2.prompt.md"; }
run_last_message_file() { echo "$LOG_ROOT/$1/$2.last.txt"; }
run_failure_file() { echo "$LOG_ROOT/$1/$2.failure.txt"; }
run_status_file() { echo "$LOG_ROOT/$1/status.tsv"; }
run_current_batch_file() { echo "$LOG_ROOT/$1/current-batch.env"; }
run_backups_dir() { echo "$LOG_ROOT/$1/backups"; }
batch_prompt_file() { echo "$PROMPTS_DIR/$1.prompt.md"; }
tmp_target_root() { echo "${TMPDIR:-/tmp}/roko-ux-targets"; }
batch_target_dir() { echo "$(tmp_target_root)/$1/$2/$3-attempt-$4"; }

current_batch_value() {
  local run_id="$1"
  local key="$2"
  local file
  file="$(run_current_batch_file "$run_id")"
  [[ -f "$file" ]] || return 1
  awk -F= -v key="$key" '$1 == key { gsub(/\047/, "", $2); print $2 }' "$file"
}

current_batch_name() {
  current_batch_value "$1" "BATCH"
}

current_batch_attempt() {
  current_batch_value "$1" "ATTEMPT"
}

worktree_dirty() {
  local worktree="$1"
  git -C "$worktree" status --porcelain=v1 -uall \
    | grep -Ev '^[ MADRCU?!]{2} (\.cargo-target/|target/)' \
    | grep -q .
}

record_status() {
  local run_id="$1"
  local batch="$2"
  local attempt="$3"
  local status="$4"
  local note="${5:-}"
  printf '%s\t%s\t%s\t%s\t%s\n' \
    "$(date -Iseconds)" \
    "$batch" \
    "$attempt" \
    "$status" \
    "$note" >> "$(run_status_file "$run_id")"
}

set_current_batch() {
  local run_id="$1"
  local batch="$2"
  local attempt="$3"
  cat > "$(run_current_batch_file "$run_id")" <<EOF
BATCH='$batch'
ATTEMPT='$attempt'
UPDATED_AT='$(date -Iseconds)'
EOF
}

clear_current_batch() {
  local run_id="$1"
  rm -f "$(run_current_batch_file "$run_id")"
}

batch_title() {
  case "$1" in
    A1) echo "Dashboard backend foundations (A.01-A.05)" ;;
    A2) echo "Dashboard backend completion (A.06-A.10)" ;;
    B1) echo "Demo foundations (B.01-B.06)" ;;
    B2) echo "Demo integration and polish (B.07-B.18)" ;;
    C1) echo "Agent-server architecture core (C.01-C.05)" ;;
    C2) echo "Migration cleanup and muxing (C.06-C.08)" ;;
    D1) echo "Core runtime gaps (D.02-D.17)" ;;
    E1) echo "Feedback-loop wiring (E.01-E.08)" ;;
    D2) echo "Orchestrator and dreams middle layer (D.18-D.33)" ;;
    D3) echo "Long-horizon learning and deployment (D.34-D.54)" ;;
    F1) echo "TUI and API core (F.01-F.06)" ;;
    F2) echo "Interface extras (F.07-F.12)" ;;
    *) return 1 ;;
  esac
}

batch_deps() {
  case "$1" in
    A2) echo "A1" ;;
    B2) echo "B1" ;;
    C2) echo "C1" ;;
    E1) echo "D1" ;;
    D2) echo "D1 E1" ;;
    D3) echo "D2" ;;
    F2) echo "F1" ;;
    *) echo "" ;;
  esac
}

batch_group() {
  case "$1" in
    A*|C*) echo "backend" ;;
    B*) echo "demo" ;;
    D*|E*|F*) echo "cli" ;;
    *) echo "misc" ;;
  esac
}

batch_verify_commands() {
  case "$1" in
    A1)
      cat <<'EOF'
cargo check -p mirage-rs -p roko-serve -p roko-cli
cargo test -p mirage-rs --lib --no-run
cargo test -p roko-serve --lib --no-run
EOF
      ;;
    A2)
      cat <<'EOF'
cargo check -p mirage-rs -p roko-serve -p roko-cli
cargo test -p mirage-rs --lib --no-run
cargo test -p roko-cli --lib --no-run
EOF
      ;;
    B1)
      cat <<'EOF'
cargo build -p roko-demo
cargo test -p roko-demo --lib --no-run
bash -lc 'cd contracts && forge test --match-contract FeeDistributor -q'
EOF
      ;;
    B2)
      cat <<'EOF'
cargo build -p roko-demo
cargo test -p roko-demo --lib --no-run
EOF
      ;;
    C1)
      cat <<'EOF'
cargo check -p roko-agent-server -p roko-serve -p mirage-rs
cargo test -p roko-agent-server --lib --no-run
EOF
      ;;
    C2)
      cat <<'EOF'
cargo check -p roko-serve -p mirage-rs
cargo build -p mirage-rs --no-default-features --features binary
EOF
      ;;
    D1)
      cat <<'EOF'
cargo check -p roko-core -p roko-chain -p roko-neuro -p roko-daimon -p roko-agent -p roko-cli
cargo test -p roko-core --lib --no-run
cargo test -p roko-neuro --lib --no-run
cargo test -p roko-agent --lib --no-run
EOF
      ;;
    E1)
      cat <<'EOF'
cargo check -p roko-learn -p roko-conductor -p roko-compose -p roko-orchestrator -p roko-cli
cargo test -p roko-learn --lib --no-run
cargo test -p roko-orchestrator --lib --no-run
EOF
      ;;
    D2)
      cat <<'EOF'
cargo check -p roko-orchestrator -p roko-agent -p roko-runtime -p roko-dreams
cargo test -p roko-orchestrator --lib --no-run
cargo test -p roko-dreams --lib --no-run
EOF
      ;;
    D3)
      cat <<'EOF'
cargo check -p roko-neuro -p roko-learn -p roko-daimon -p roko-dreams -p roko-compose -p roko-cli
cargo test -p roko-learn --lib --no-run
cargo test -p roko-dreams --lib --no-run
EOF
      ;;
    F1)
      cat <<'EOF'
cargo check -p roko-cli -p roko-serve
cargo test -p roko-cli --lib --no-run
cargo test -p roko-serve --lib --no-run
EOF
      ;;
    F2)
      cat <<'EOF'
cargo check -p roko-cli -p roko-learn -p roko-index -p roko-mcp-stdio -p roko-mcp-github -p roko-mcp-slack -p roko-mcp-scripts
bash -lc 'if cargo metadata --format-version 1 --no-deps | rg -q "\"name\":\"roko-mcp-code\""; then cargo build -p roko-mcp-code; else echo "roko-mcp-code not present; skipping optional crate build"; fi'
cargo test -p roko-learn --lib --no-run
EOF
      ;;
    *)
      return 1
      ;;
  esac
}

preflight_check() {
  local errors=0
  log_header "PREFLIGHT"

  if command -v codex >/dev/null 2>&1; then
    log_ok "preflight" "codex CLI: $(command -v codex)"
  else
    log_err "preflight" "codex CLI not found"
    errors=$((errors + 1))
  fi

  if git -C "$ROKO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    log_ok "preflight" "git repo detected"
  else
    log_err "preflight" "ROKO_ROOT is not a git repo: $ROKO_ROOT"
    errors=$((errors + 1))
  fi

  ensure_dir "$LOG_ROOT"
  ensure_dir "$WORKTREE_ROOT"

  require_file "$UX_ROOT/README.md"
  require_file "$UX_ROOT/BATCHES.md"
  require_file "$UX_ROOT/SOURCE-INDEX.md"

  local batch
  for batch in "${ALL_BATCHES[@]}"; do
    require_file "$(batch_prompt_file "$batch")"
  done

  local dirty_count
  dirty_count=$(git -C "$ROKO_ROOT" status --porcelain | wc -l | tr -d ' ')
  if (( dirty_count > 0 )); then
    log_warn "preflight" "main repo has $dirty_count uncommitted change(s); the overnight worktree starts from committed HEAD only"
  else
    log_ok "preflight" "main repo is clean"
  fi

  return "$errors"
}
