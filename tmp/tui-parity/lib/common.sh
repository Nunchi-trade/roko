#!/usr/bin/env bash

set -uo pipefail

: "${ROKO_ROOT:=/Users/will/dev/nunchi/roko/roko}"
: "${TUI_ROOT:=$ROKO_ROOT/tmp/tui-parity}"
: "${LOG_ROOT:=$TUI_ROOT/logs}"
: "${PROMPTS_DIR:=$TUI_ROOT/prompts}"
: "${CONTEXT_DIR:=$TUI_ROOT/context-pack}"
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

# Execution order respects dependency DAG:
# T1 first (no deps), then T7 (independent), T2 (independent), T5 (deps: T1),
# T3 (deps: T1), T6 (deps: T1+T2), T4 (deps: T1+T3), T8 (independent)
ALL_BATCHES=(
  "T1"
  "T7"
  "T2"
  "T5"
  "T3"
  "T6"
  "T4"
  "T8"
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
tmp_target_root() { echo "${TMPDIR:-/tmp}/roko-tui-targets"; }
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
    T1) echo "StateHub subscription (replace polling with streaming)" ;;
    T2) echo "Agent output segment parsing" ;;
    T3) echo "Approval flow IPC" ;;
    T4) echo "Process supervision display" ;;
    T5) echo "Parallel pool + wave ribbon" ;;
    T6) echo "Context metrics + route display" ;;
    T7) echo "Dead field cleanup" ;;
    T8) echo "Visual effects (NervViz + particles)" ;;
    *) return 1 ;;
  esac
}

batch_deps() {
  case "$1" in
    T3) echo "T1" ;;
    T4) echo "T1 T3" ;;
    T5) echo "T1" ;;
    T6) echo "T1 T2" ;;
    *) echo "" ;;
  esac
}

batch_group() {
  case "$1" in
    T1|T3|T4) echo "streaming" ;;
    T2|T6) echo "display" ;;
    T5) echo "pool" ;;
    T7) echo "cleanup" ;;
    T8) echo "effects" ;;
    *) echo "misc" ;;
  esac
}

batch_verify_commands() {
  case "$1" in
    T1)
      cat <<'EOF'
cargo check -p roko-cli
cargo test -p roko-cli --lib --no-run
EOF
      ;;
    T2)
      cat <<'EOF'
cargo check -p roko-cli
cargo test -p roko-cli --lib --no-run -- tui::segment
EOF
      ;;
    T3)
      cat <<'EOF'
cargo check -p roko-cli
cargo test -p roko-cli --lib --no-run -- tui::approval_ipc
EOF
      ;;
    T4)
      cat <<'EOF'
cargo check -p roko-cli -p roko-runtime
EOF
      ;;
    T5)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    T6)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    T7)
      cat <<'EOF'
cargo check -p roko-cli
cargo clippy -p roko-cli --no-deps -- -D warnings
EOF
      ;;
    T8)
      cat <<'EOF'
cargo check -p roko-cli
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

  require_file "$TUI_ROOT/README.md"
  require_file "$TUI_ROOT/BATCHES.md"

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
