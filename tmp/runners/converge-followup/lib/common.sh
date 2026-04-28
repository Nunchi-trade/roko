#!/usr/bin/env bash
# common.sh — constants, logging, batch registry, shared state

set -uo pipefail

: "${ROKO_ROOT:=$(git -C "$(dirname "${BASH_SOURCE[0]}")/../../../.." rev-parse --show-toplevel)}"
: "${RUNNER_ROOT:=$ROKO_ROOT/tmp/runners/converge-followup}"
: "${LOG_ROOT:=$RUNNER_ROOT/logs}"
: "${PROMPTS_DIR:=$RUNNER_ROOT/prompts}"
: "${CONTEXT_DIR:=$RUNNER_ROOT/context-pack}"
: "${WORKTREE_ROOT:=$ROKO_ROOT/.roko/worktrees}"

# ── Colors ──

if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
  C_RESET=$'\e[0m' C_BOLD=$'\e[1m' C_DIM=$'\e[2m'
  C_RED=$'\e[31m' C_GREEN=$'\e[32m' C_YELLOW=$'\e[33m'
  C_BLUE=$'\e[34m' C_MAGENTA=$'\e[35m' C_CYAN=$'\e[36m'
else
  C_RESET='' C_BOLD='' C_DIM='' C_RED='' C_GREEN='' C_YELLOW='' C_BLUE='' C_MAGENTA='' C_CYAN=''
fi

log_info()   { printf '%s[INFO]%s  %s%-10s%s %s\n' "$C_BLUE"    "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2"; }
log_ok()     { printf '%s[ OK ]%s  %s%-10s%s %s\n' "$C_GREEN"   "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2"; }
log_warn()   { printf '%s[WARN]%s  %s%-10s%s %s\n' "$C_YELLOW"  "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2" >&2; }
log_err()    { printf '%s[FAIL]%s  %s%-10s%s %s\n' "$C_RED"     "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2" >&2; }
log_header() { printf '\n%s━━━ %s ━━━%s\n\n' "$C_BOLD$C_MAGENTA" "$1" "$C_RESET"; }

fmt_duration() {
  local s="${1:-0}"
  if (( s >= 3600 )); then printf '%dh %dm %ds' $((s/3600)) $(((s%3600)/60)) $((s%60))
  elif (( s >= 60 )); then printf '%dm %ds' $((s/60)) $((s%60))
  else printf '%ds' "$s"
  fi
}

ensure_dir() { mkdir -p "$1"; }

# ── Path helpers ──

run_dir()               { echo "$LOG_ROOT/$1"; }
run_manifest_file()     { echo "$LOG_ROOT/$1/manifest.env"; }
run_result_file()       { echo "$LOG_ROOT/$1/$2.result"; }
run_log_file()          { echo "$LOG_ROOT/$1/$2.log"; }
run_json_log()          { echo "$LOG_ROOT/$1/events.jsonl"; }
run_prompts_dir()       { echo "$LOG_ROOT/$1/prompts"; }
run_prompt_snapshot()   { echo "$LOG_ROOT/$1/prompts/$2.prompt.md"; }
run_failure_file()      { echo "$LOG_ROOT/$1/$2.failure.md"; }
run_status_file()       { echo "$LOG_ROOT/$1/status.tsv"; }
run_current_file()      { echo "$LOG_ROOT/$1/current-batch.env"; }
run_backups_dir()       { echo "$LOG_ROOT/$1/backups"; }
run_cumulative_dir()    { echo "$LOG_ROOT/$1/cumulative"; }
batch_prompt_file()     { echo "$PROMPTS_DIR/$1.prompt.md"; }
# Single shared target dir for the entire run — incremental compilation
run_target_dir()        { echo "${TMPDIR:-/tmp}/roko-followup-targets/$1"; }

# ── Recording ──

record_status() {
  printf '%s\t%s\t%s\t%s\t%s\n' "$(date -Iseconds)" "$2" "$3" "$4" "${5:-}" \
    >> "$(run_status_file "$1")"
}

record_event() {
  local run_id="$1" batch="$2" attempt="$3" event="$4"; shift 4
  printf '{"ts":"%s","batch":"%s","attempt":%s,"event":"%s","details":"%s"}\n' \
    "$(date -Iseconds)" "$batch" "$attempt" "$event" "${*//\"/\\\"}" \
    >> "$(run_json_log "$run_id")"
}

# ── Current batch tracking ──

set_current_batch() {
  printf "BATCH='%s'\nATTEMPT='%s'\nUPDATED_AT='%s'\n" "$2" "$3" "$(date -Iseconds)" \
    > "$(run_current_file "$1")"
}
clear_current_batch() { rm -f "$(run_current_file "$1")"; }

# ── Batch registry from batches.toml ──

ALL_BATCHES=()

load_batch_registry() {
  local toml_file="$RUNNER_ROOT/batches.toml"
  if [[ ! -f "$toml_file" ]]; then
    log_warn "registry" "No batches.toml — batch list empty"
    return 0
  fi
  mapfile -t ALL_BATCHES < <(awk -F'"' '/^id *= *"/ { print $2 }' "$toml_file")
  log_info "registry" "Loaded ${#ALL_BATCHES[@]} batches"
}

# Field extractors — all read from batches.toml
_batch_field() {
  local id="$1" field="$2"
  awk -F'"' -v id="$id" -v field="$field" '
    /^id *= */ { current = $2 }
    $0 ~ "^"field" *= *" && current == id { print $2; exit }
  ' "$RUNNER_ROOT/batches.toml" 2>/dev/null
}

_batch_array_field() {
  local id="$1" field="$2"
  awk -F'"' -v id="$id" -v field="$field" '
    /^id *= */ { current = $2 }
    $0 ~ "^"field" *= *" && current == id {
      gsub(/[\[\] ]/, "", $0); sub("^"field"=", "", $0)
      gsub(/,/, " ", $0); gsub(/"/, "", $0)
      print; exit
    }
  ' "$RUNNER_ROOT/batches.toml" 2>/dev/null
}

batch_title()       { _batch_field "$1" "title" || echo "(unknown)"; }
batch_group()       { _batch_field "$1" "group" || echo "misc"; }
batch_verify_mode() { _batch_field "$1" "verify" || echo "quick"; }
batch_deps()        { _batch_array_field "$1" "deps"; }
batch_scope()       { _batch_array_field "$1" "scope"; }
batch_also_read()   { _batch_array_field "$1" "also_read"; }

# ── Status helpers ──

success_status() {
  case "${1:-}" in success|success_noop|verified|skipped) return 0 ;; *) return 1 ;; esac
}
terminal_failure() {
  case "${1:-}" in spawn_failed|verify_failed|timeout|blocked|antipattern_failed) return 0 ;; *) return 1 ;; esac
}

latest_run_id() {
  [[ -d "$LOG_ROOT" ]] || return 1
  ls -1t "$LOG_ROOT" | grep -v latest | head -1
}

worktree_dirty() { git -C "$1" status --porcelain -uno 2>/dev/null | grep -q .; }

# ── Disk space ──

# Minimum free space required to start/continue (in MB).
: "${CONV_MIN_FREE_MB:=5000}"

get_free_mb() {
  # Works on macOS (df -m) and Linux (df -BM)
  local mount_point="${1:-/}"
  if df -m "$mount_point" >/dev/null 2>&1; then
    df -m "$mount_point" | awk 'NR==2 { print $4 }'
  else
    df -BM "$mount_point" | awk 'NR==2 { gsub(/M/, "", $4); print $4 }'
  fi
}

check_disk_space() {
  local label="${1:-check}" min_mb="${2:-$CONV_MIN_FREE_MB}"
  local free_mb
  free_mb="$(get_free_mb /)"
  if (( free_mb < min_mb )); then
    log_err "$label" "Low disk space: ${free_mb}MB free (need ${min_mb}MB)"
    log_err "$label" "Run: bash tmp/runners/converge-followup/run.sh --cleanup"
    return 1
  fi
  log_ok "$label" "Disk: ${free_mb}MB free"
  return 0
}

# ── Cleanup ──

# Clean up artifacts from a specific run
cleanup_run() {
  local run_id="$1"
  local freed=0

  # 1. Target dir (biggest: 5-10GB of Rust build artifacts)
  local target_dir
  target_dir="$(run_target_dir "$run_id")"
  if [[ -d "$target_dir" ]]; then
    local size
    size="$(du -sm "$target_dir" 2>/dev/null | awk '{print $1}')"
    rm -rf "$target_dir"
    freed=$((freed + ${size:-0}))
    log_ok "cleanup" "Removed target dir: ${size:-?}MB ($target_dir)"
  fi

  # 2. Worktree (full repo clone, ~250MB)
  local manifest
  manifest="$(run_manifest_file "$run_id")"
  if [[ -f "$manifest" ]]; then
    local wt_path=""
    wt_path="$(grep "^WORKTREE=" "$manifest" | cut -d"'" -f2)"
    if [[ -n "$wt_path" && -d "$wt_path" ]]; then
      local branch=""
      branch="$(grep "^BRANCH=" "$manifest" | cut -d"'" -f2)"
      git -C "$ROKO_ROOT" worktree remove --force "$wt_path" 2>/dev/null || rm -rf "$wt_path"
      local size
      size="$(du -sm "$wt_path" 2>/dev/null | awk '{print $1}')"
      freed=$((freed + ${size:-250}))
      log_ok "cleanup" "Removed worktree: ${wt_path##*/}"
    fi
  fi

  # 3. Cumulative snapshots (file content copies, ~50-200MB)
  local cum_dir
  cum_dir="$(run_cumulative_dir "$run_id")"
  if [[ -d "$cum_dir" ]]; then
    local size
    size="$(du -sm "$cum_dir" 2>/dev/null | awk '{print $1}')"
    rm -rf "$cum_dir"
    freed=$((freed + ${size:-0}))
    log_ok "cleanup" "Removed cumulative snapshots: ${size:-?}MB"
  fi

  # 4. Backup patches (usually small but can pile up)
  local backup_dir
  backup_dir="$(run_backups_dir "$run_id")"
  if [[ -d "$backup_dir" ]]; then
    rm -rf "$backup_dir"
    log_ok "cleanup" "Removed backup patches"
  fi

  echo "$freed"
}

# Clean up ALL old runs except the most recent N
cleanup_old_runs() {
  local keep="${1:-2}"
  local total_freed=0

  [[ -d "$LOG_ROOT" ]] || return 0

  local -a all_runs=()
  mapfile -t all_runs < <(ls -1t "$LOG_ROOT" | grep -v latest | grep '^run-')

  if (( ${#all_runs[@]} <= keep )); then
    log_info "cleanup" "Only ${#all_runs[@]} runs, keeping all (threshold: $keep)"
    return 0
  fi

  local -a to_clean=("${all_runs[@]:$keep}")
  log_info "cleanup" "Cleaning ${#to_clean[@]} old runs (keeping $keep most recent)"

  for run_id in "${to_clean[@]}"; do
    local freed
    freed="$(cleanup_run "$run_id")"
    total_freed=$((total_freed + freed))
  done

  # Also clean stale target dirs that don't correspond to any run
  local target_parent="${TMPDIR:-/tmp}/roko-followup-targets"
  if [[ -d "$target_parent" ]]; then
    for dir in "$target_parent"/*/; do
      [[ -d "$dir" ]] || continue
      local dir_name
      dir_name="$(basename "$dir")"
      if [[ ! -d "$LOG_ROOT/$dir_name" ]]; then
        local size
        size="$(du -sm "$dir" 2>/dev/null | awk '{print $1}')"
        rm -rf "$dir"
        total_freed=$((total_freed + ${size:-0}))
        log_ok "cleanup" "Removed orphaned target dir: $dir_name (${size:-?}MB)"
      fi
    done
  fi

  # Clean stale worktrees
  if [[ -d "$WORKTREE_ROOT" ]]; then
    for wt in "$WORKTREE_ROOT"/followup-*/; do
      [[ -d "$wt" ]] || continue
      local wt_run
      wt_run="$(basename "$wt" | sed 's/^followup-//')"
      if [[ ! -d "$LOG_ROOT/$wt_run" ]]; then
        git -C "$ROKO_ROOT" worktree remove --force "$wt" 2>/dev/null || rm -rf "$wt"
        log_ok "cleanup" "Removed orphaned worktree: $(basename "$wt")"
      fi
    done
  fi

  log_ok "cleanup" "Freed ~${total_freed}MB total"
}

# Emergency cleanup: remove ALL target dirs immediately
cleanup_emergency() {
  log_header "EMERGENCY CLEANUP"
  local target_parent="${TMPDIR:-/tmp}/roko-followup-targets"
  if [[ -d "$target_parent" ]]; then
    local size
    size="$(du -sm "$target_parent" 2>/dev/null | awk '{print $1}')"
    rm -rf "$target_parent"
    log_ok "cleanup" "Removed all target dirs: ${size:-?}MB"
  fi
  cleanup_old_runs 1
}

# ── Preflight ──

preflight_check() {
  local errors=0
  log_header "PREFLIGHT"
  command -v codex >/dev/null 2>&1 && log_ok "preflight" "codex: $(codex --version 2>/dev/null || echo found)" || { log_err "preflight" "codex not found"; errors=$((errors+1)); }
  command -v cargo >/dev/null 2>&1 && log_ok "preflight" "$(rustc --version 2>/dev/null)" || { log_err "preflight" "cargo/rustc not found"; errors=$((errors+1)); }
  git -C "$ROKO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1 && log_ok "preflight" "git: $ROKO_ROOT" || { log_err "preflight" "not a git repo"; errors=$((errors+1)); }
  check_disk_space "preflight" || errors=$((errors+1))
  ensure_dir "$LOG_ROOT"
  ensure_dir "$WORKTREE_ROOT"
  return "$errors"
}
