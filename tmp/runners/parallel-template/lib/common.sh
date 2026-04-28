#!/usr/bin/env bash
# common.sh — shared constants, logging, batch registry, disk monitoring
#
# Designed for multi-instance safety: all paths are namespaced by RUNNER_NAME + RUN_ID.
# Two runners with different RUNNER_NAME can execute simultaneously on the same repo.

set -uo pipefail

: "${ROKO_ROOT:=$(git -C "$(dirname "${BASH_SOURCE[0]}")/../../../.." rev-parse --show-toplevel)}"
: "${RUNNER_NAME:=default}"
: "${RUNNER_ROOT:=$ROKO_ROOT/tmp/runners/parallel-template}"
: "${LOG_ROOT:=$RUNNER_ROOT/logs}"
: "${PROMPTS_DIR:=$RUNNER_ROOT/prompts}"
: "${CONTEXT_DIR:=$RUNNER_ROOT/context-pack}"

# Namespaced paths — unique per runner instance, safe for concurrent use
: "${WORKTREE_ROOT:=$ROKO_ROOT/.roko/worktrees}"
: "${CONV_MIN_FREE_MB:=5000}"

# ── Colors ──

if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
  C_RESET=$'\e[0m' C_BOLD=$'\e[1m' C_DIM=$'\e[2m'
  C_RED=$'\e[31m' C_GREEN=$'\e[32m' C_YELLOW=$'\e[33m'
  C_BLUE=$'\e[34m' C_MAGENTA=$'\e[35m' C_CYAN=$'\e[36m'
else
  C_RESET='' C_BOLD='' C_DIM='' C_RED='' C_GREEN='' C_YELLOW='' C_BLUE='' C_MAGENTA='' C_CYAN=''
fi

# ── Logging (atomic writes to prevent interleaving from parallel subshells) ──

_log_atomic() {
  local line="$1"
  # printf with \n is atomic on macOS/Linux for lines < PIPE_BUF (4096)
  printf '%s\n' "$line"
}

log_info()   { _log_atomic "$(printf '%s[INFO]%s  %s%-12s%s %s' "$C_BLUE"    "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2")"; }
log_ok()     { _log_atomic "$(printf '%s[ OK ]%s  %s%-12s%s %s' "$C_GREEN"   "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2")"; }
log_warn()   { _log_atomic "$(printf '%s[WARN]%s  %s%-12s%s %s' "$C_YELLOW"  "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2")" >&2; }
log_err()    { _log_atomic "$(printf '%s[FAIL]%s  %s%-12s%s %s' "$C_RED"     "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2")" >&2; }
log_header() { printf '\n%s━━━ %s ━━━%s\n\n' "$C_BOLD$C_MAGENTA" "$1" "$C_RESET"; }

fmt_duration() {
  local s="${1:-0}"
  if (( s >= 3600 )); then printf '%dh %dm %ds' $((s/3600)) $(((s%3600)/60)) $((s%60))
  elif (( s >= 60 )); then printf '%dm %ds' $((s/60)) $((s%60))
  else printf '%ds' "$s"
  fi
}

ensure_dir() { mkdir -p "$@"; }

# ── Path helpers — all namespaced by RUNNER_NAME ──

run_dir()               { echo "$LOG_ROOT/$1"; }
run_manifest_file()     { echo "$LOG_ROOT/$1/manifest.env"; }
run_result_file()       { echo "$LOG_ROOT/$1/$2.result"; }
run_log_file()          { echo "$LOG_ROOT/$1/$2.log"; }
run_json_log()          { echo "$LOG_ROOT/$1/events.jsonl"; }
run_prompts_dir()       { echo "$LOG_ROOT/$1/prompts"; }
run_prompt_snapshot()   { echo "$LOG_ROOT/$1/prompts/$2.prompt.md"; }
run_failure_file()      { echo "$LOG_ROOT/$1/$2.failure.md"; }
run_status_file()       { echo "$LOG_ROOT/$1/status.tsv"; }
run_status_json()       { echo "$LOG_ROOT/$1/status.json"; }
run_current_file()      { echo "$LOG_ROOT/$1/current-batch.env"; }
run_backups_dir()       { echo "$LOG_ROOT/$1/backups"; }
run_cumulative_dir()    { echo "$LOG_ROOT/$1/cumulative"; }
run_merge_lock()        { echo "$LOG_ROOT/$1/.merge.lock"; }
batch_prompt_file()     { echo "$PROMPTS_DIR/$1.prompt.md"; }

# Target dir: namespaced by RUNNER_NAME so multiple runners don't collide
run_target_dir() { echo "${TMPDIR:-/tmp}/roko-par-${RUNNER_NAME}/$1"; }

# ── Atomic file operations (safe for concurrent subshells) ──

# Atomic append: write to temp then append via cat (prevents partial lines)
atomic_append() {
  local target="$1" content="$2"
  local tmp="${target}.$$"
  printf '%s\n' "$content" > "$tmp"
  cat "$tmp" >> "$target"
  rm -f "$tmp"
}

# Atomic write: write to temp then mv (atomic on same filesystem)
atomic_write() {
  local target="$1" content="$2"
  local tmp="${target}.tmp.$$"
  printf '%s' "$content" > "$tmp"
  mv -f "$tmp" "$target"
}

# ── Recording (atomic for parallel safety) ──

record_status() {
  local line
  line="$(printf '%s\t%s\t%s\t%s\t%s' "$(date -Iseconds)" "$2" "$3" "$4" "${5:-}")"
  atomic_append "$(run_status_file "$1")" "$line"
}

record_event() {
  local run_id="$1" batch="$2" attempt="$3" event="$4"; shift 4
  local line
  line="$(printf '{"ts":"%s","batch":"%s","attempt":%s,"event":"%s","details":"%s"}' \
    "$(date -Iseconds)" "$batch" "$attempt" "$event" "${*//\"/\\\"}")"
  atomic_append "$(run_json_log "$run_id")" "$line"
}

# ── Live status JSON (for external monitoring) ──

# Global tracking arrays — set by run-parallel.sh, read by update_status_json
declare -g _STATUS_RUN_ID=""
declare -g _STATUS_START_TS=0

update_status_json() {
  local run_id="$1"
  shift
  local -a batches=("$@")
  local total=${#batches[@]}
  local now
  now=$(date +%s)
  local elapsed=$(( now - _STATUS_START_TS ))

  local success=0 failed=0 in_progress=0 pending=0 blocked=0
  local -a active_names=() done_names=() failed_names=()

  for batch in "${batches[@]}"; do
    local rf
    rf="$(run_result_file "$run_id" "$batch")"
    if [[ -f "$rf" ]]; then
      local s
      s="$(cat "$rf")"
      if success_status "$s"; then
        success=$((success + 1))
        done_names+=("$batch")
      elif terminal_failure "$s"; then
        failed=$((failed + 1))
        failed_names+=("$batch")
      elif [[ "$s" == "in_progress" ]]; then
        in_progress=$((in_progress + 1))
        active_names+=("$batch")
      fi
    else
      pending=$((pending + 1))
    fi
  done

  # ETA: extrapolate from average batch time
  local eta="unknown"
  local remaining=$((total - success - failed))
  if (( success > 0 && remaining > 0 )); then
    local avg_per_batch=$(( elapsed / success ))
    local eta_seconds=$(( avg_per_batch * remaining / ${PARALLEL:-3} ))
    eta="$(fmt_duration "$eta_seconds")"
  fi

  local json
  json="$(cat <<ENDJSON
{
  "run_id": "$run_id",
  "runner": "$RUNNER_NAME",
  "elapsed": "$(fmt_duration "$elapsed")",
  "elapsed_s": $elapsed,
  "total": $total,
  "success": $success,
  "failed": $failed,
  "in_progress": $in_progress,
  "pending": $pending,
  "blocked": $blocked,
  "eta": "$eta",
  "active": [$(printf '"%s",' "${active_names[@]}" | sed 's/,$//')],
  "completed": [$(printf '"%s",' "${done_names[@]}" | sed 's/,$//')],
  "failed_list": [$(printf '"%s",' "${failed_names[@]}" | sed 's/,$//')],
  "updated_at": "$(date -Iseconds)"
}
ENDJSON
)"
  atomic_write "$(run_status_json "$run_id")" "$json"
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
  case "${1:-}" in spawn_failed|verify_failed|timeout|blocked|antipattern_failed|merge_failed) return 0 ;; *) return 1 ;; esac
}

latest_run_id() {
  [[ -d "$LOG_ROOT" ]] || return 1
  ls -1t "$LOG_ROOT" | grep '^run-' | head -1
}

worktree_dirty() { git -C "$1" status --porcelain -uno 2>/dev/null | grep -q .; }

# ── Disk space ──

get_free_mb() {
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
    log_err "$label" "Low disk: ${free_mb}MB free (need ${min_mb}MB)"
    return 1
  fi
  return 0
}

# ── Cleanup ──

cleanup_run() {
  local run_id="$1" freed=0
  local target_dir
  target_dir="$(run_target_dir "$run_id")"
  if [[ -d "$target_dir" ]]; then
    local size; size="$(du -sm "$target_dir" 2>/dev/null | awk '{print $1}')"
    rm -rf "$target_dir"; freed=$((freed + ${size:-0}))
    log_ok "cleanup" "Target dir: ${size:-?}MB"
  fi
  local manifest; manifest="$(run_manifest_file "$run_id")"
  if [[ -f "$manifest" ]]; then
    local wt_path; wt_path="$(grep "^MAIN_WORKTREE=" "$manifest" | cut -d"'" -f2)"
    if [[ -n "$wt_path" && -d "$wt_path" ]]; then
      git -C "$ROKO_ROOT" worktree remove --force "$wt_path" 2>/dev/null || rm -rf "$wt_path"
      log_ok "cleanup" "Worktree: ${wt_path##*/}"
    fi
  fi
  local cum_dir; cum_dir="$(run_cumulative_dir "$run_id")"
  [[ -d "$cum_dir" ]] && rm -rf "$cum_dir"
  local backup_dir; backup_dir="$(run_backups_dir "$run_id")"
  [[ -d "$backup_dir" ]] && rm -rf "$backup_dir"
  echo "$freed"
}

cleanup_old_runs() {
  local keep="${1:-2}" total_freed=0
  [[ -d "$LOG_ROOT" ]] || return 0
  local -a all_runs=()
  mapfile -t all_runs < <(ls -1t "$LOG_ROOT" | grep -v latest | grep '^run-')
  (( ${#all_runs[@]} <= keep )) && return 0
  local -a to_clean=("${all_runs[@]:$keep}")
  for run_id in "${to_clean[@]}"; do
    local freed; freed="$(cleanup_run "$run_id")"
    total_freed=$((total_freed + freed))
  done
  # Orphaned target dirs
  local target_parent="${TMPDIR:-/tmp}/roko-par-${RUNNER_NAME}"
  if [[ -d "$target_parent" ]]; then
    for dir in "$target_parent"/*/; do
      [[ -d "$dir" ]] || continue
      local dn; dn="$(basename "$dir")"
      [[ -d "$LOG_ROOT/$dn" ]] || { rm -rf "$dir"; log_ok "cleanup" "Orphan target: $dn"; }
    done
  fi
  log_ok "cleanup" "Freed ~${total_freed}MB"
}

# ── Preflight ──

preflight_check() {
  local errors=0
  log_header "PREFLIGHT"
  command -v codex >/dev/null 2>&1 && log_ok "preflight" "codex $(codex --version 2>/dev/null || echo '?')" || { log_err "preflight" "codex not found"; errors=$((errors+1)); }
  command -v cargo >/dev/null 2>&1 && log_ok "preflight" "$(rustc --version 2>/dev/null)" || { log_err "preflight" "cargo not found"; errors=$((errors+1)); }
  git -C "$ROKO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1 || { log_err "preflight" "not a git repo"; errors=$((errors+1)); }
  check_disk_space "preflight" || errors=$((errors+1))
  ensure_dir "$LOG_ROOT" "$WORKTREE_ROOT"
  log_ok "preflight" "Runner: $RUNNER_NAME"
  return "$errors"
}
