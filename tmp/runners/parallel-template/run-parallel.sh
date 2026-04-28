#!/usr/bin/env bash
# run-parallel.sh — dependency-aware parallel batch runner
#
# Usage:
#   RUNNER_NAME=myrunner RUNNER_ROOT=/path/to/runner ./run-parallel.sh [OPTIONS]
#
# Required env:
#   RUNNER_NAME   — unique name for this runner instance (used for namespacing)
#   RUNNER_ROOT   — path to the runner directory (must contain batches.toml, prompts/, context-pack/)
#
# Options:
#   --parallel N    Max concurrent batches (default: 3)
#   --only A01,B02  Run only these batches
#   --group A       Run only batches in this group/wave
#   --continue      Resume from last run (skip completed batches)
#   --dry-run       Show what would run without executing
#   --list          List batches and exit
#   --cleanup       Clean old runs and exit
#   --cleanup-keep N  Keep N most recent runs (default: 2)
#   --disk          Show disk usage and exit
#   --no-gate       Skip wave gates (cargo check/clippy)
#   --no-test       Skip end-of-run test gate
#   --no-merge-back Skip auto merge-back to source branch
#   --watch         Live dashboard with progress bar, active batches, merge status
#   --watch-interval N  Refresh interval for --watch (default: 5)
#   --tail          Tail all batch logs (uses multitail if available)
#   --pause         Pause after each wave for manual inspection
#   --status        Show live status JSON from the latest run and exit

set -euo pipefail

# Always set ERR trap so we know WHERE things fail
trap 'echo "[DEBUG] ERR trap: line $LINENO, exit $?, BASH_COMMAND=$BASH_COMMAND" >&2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/common.sh"
source "$SCRIPT_DIR/lib/spawn.sh"
source "$SCRIPT_DIR/lib/dag.sh"
source "$SCRIPT_DIR/lib/merge-back.sh"

# ── Defaults ──

: "${PARALLEL:=3}"
OPT_ONLY=""
OPT_GROUP=""
OPT_CONTINUE=0
OPT_DRY_RUN=0
OPT_LIST=0
OPT_CLEANUP=0
OPT_CLEANUP_KEEP=2
OPT_DISK=0
OPT_NO_GATE=0
OPT_NO_TEST=0
OPT_NO_MERGE_BACK=0
OPT_PAUSE=0
OPT_STATUS=0
OPT_WATCH=0
OPT_WATCH_INTERVAL=5
OPT_TAIL=0
: "${MAX_RETRIES:=2}"

# ── CLI parsing ──

while [[ $# -gt 0 ]]; do
  case "$1" in
    --parallel)   PARALLEL="$2"; shift 2 ;;
    --only)       OPT_ONLY="$2"; shift 2 ;;
    --group)      OPT_GROUP="$2"; shift 2 ;;
    --continue)   OPT_CONTINUE=1; shift ;;
    --dry-run)    OPT_DRY_RUN=1; shift ;;
    --list)       OPT_LIST=1; shift ;;
    --cleanup)    OPT_CLEANUP=1; shift ;;
    --cleanup-keep) OPT_CLEANUP_KEEP="$2"; shift 2 ;;
    --disk)       OPT_DISK=1; shift ;;
    --no-gate)    OPT_NO_GATE=1; shift ;;
    --no-test)    OPT_NO_TEST=1; shift ;;
    --no-merge-back) OPT_NO_MERGE_BACK=1; shift ;;
    --watch)      OPT_WATCH=1; shift ;;
    --watch-interval) OPT_WATCH_INTERVAL="$2"; shift 2 ;;
    --tail)       OPT_TAIL=1; shift ;;
    --pause)      OPT_PAUSE=1; shift ;;
    --status)     OPT_STATUS=1; shift ;;
    -h|--help)
      head -27 "${BASH_SOURCE[0]}" | tail -22
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

# ── Load batch registry ──

load_batch_registry

# ── Filter batches ──

filter_batches() {
  local -a result=()
  for batch in "${ALL_BATCHES[@]}"; do
    if [[ -n "$OPT_ONLY" ]]; then
      [[ ",$OPT_ONLY," == *",$batch,"* ]] || continue
    fi
    if [[ -n "$OPT_GROUP" ]]; then
      local grp
      grp="$(batch_group "$batch")"
      [[ "$grp" == "$OPT_GROUP" ]] || continue
    fi
    result+=("$batch")
  done
  echo "${result[@]}"
}

ACTIVE_BATCHES=()
IFS=' ' read -r -a ACTIVE_BATCHES <<< "$(filter_batches)"

if (( ${#ACTIVE_BATCHES[@]} == 0 )); then
  log_err "main" "No batches match filters"
  exit 1
fi

# ── --list ──

if (( OPT_LIST )); then
  printf '%-6s %-50s %-6s %-20s\n' "ID" "TITLE" "GROUP" "DEPS"
  printf '%-6s %-50s %-6s %-20s\n' "------" "--------------------------------------------------" "------" "--------------------"
  for batch in "${ACTIVE_BATCHES[@]}"; do
    printf '%-6s %-50s %-6s %-20s\n' \
      "$batch" \
      "$(batch_title "$batch" | head -c 50)" \
      "$(batch_group "$batch")" \
      "$(batch_deps "$batch")"
  done
  echo
  echo "Total: ${#ACTIVE_BATCHES[@]} batches"
  exit 0
fi

# ── --status ──

if (( OPT_STATUS )); then
  RUN_ID="$(latest_run_id 2>/dev/null)" || { log_err "status" "No runs found"; exit 1; }
  local_status="$(run_status_json "$RUN_ID")"
  if [[ -f "$local_status" ]]; then
    cat "$local_status"
  else
    log_err "status" "No status.json for $RUN_ID"
    exit 1
  fi
  exit 0
fi

# ── --watch (live dashboard) ──

if (( OPT_WATCH )); then
  RUN_ID="$(latest_run_id 2>/dev/null)" || { log_err "watch" "No runs found"; exit 1; }
  local_status="$(run_status_json "$RUN_ID")"
  local_events="$(run_json_log "$RUN_ID")"

  log_header "LIVE WATCH: $RUN_ID (Ctrl-C to exit)"
  echo "  Status:  $local_status"
  echo "  Events:  $local_events"
  echo "  Logs:    $(run_dir "$RUN_ID")/<BATCH_ID>.log"
  echo
  echo "  Tip: In another terminal, tail a specific batch:"
  echo "    tail -f $(run_dir "$RUN_ID")/R2_A01.log"
  echo

  while true; do
    clear 2>/dev/null || printf '\033[2J\033[H'
    printf '%s━━━ LIVE MONITOR: %s ━━━%s\n\n' "$C_BOLD$C_CYAN" "$RUNNER_NAME / $RUN_ID" "$C_RESET"

    if [[ -f "$local_status" ]]; then
      local json
      json="$(cat "$local_status")"

      # Parse key fields (portable — no jq required)
      local total success failed in_prog pending eta elapsed
      total="$(echo "$json" | grep -o '"total": *[0-9]*' | grep -o '[0-9]*')"
      success="$(echo "$json" | grep -o '"success": *[0-9]*' | grep -o '[0-9]*')"
      failed="$(echo "$json" | grep -o '"failed": *[0-9]*' | grep -o '[0-9]*')"
      in_prog="$(echo "$json" | grep -o '"in_progress": *[0-9]*' | grep -o '[0-9]*')"
      pending="$(echo "$json" | grep -o '"pending": *[0-9]*' | grep -o '[0-9]*')"
      eta="$(echo "$json" | grep -o '"eta": *"[^"]*"' | cut -d'"' -f4)"
      elapsed="$(echo "$json" | grep -o '"elapsed": *"[^"]*"' | cut -d'"' -f4)"

      # Progress bar
      local pct=0
      (( total > 0 )) && pct=$(( (success + failed) * 100 / total ))
      local bar_width=40
      local filled=$(( pct * bar_width / 100 ))
      local empty=$(( bar_width - filled ))
      printf '  [%s%s] %d%%\n' \
        "$(printf '%0.s█' $(seq 1 $filled) 2>/dev/null)" \
        "$(printf '%0.s░' $(seq 1 $empty) 2>/dev/null)" \
        "$pct"
      echo

      # Stats
      printf '  %sElapsed:%s %-12s  %sETA:%s %s\n' "$C_BOLD" "$C_RESET" "${elapsed:-?}" "$C_BOLD" "$C_RESET" "${eta:-?}"
      printf '  %s✓ Success:%s %-4s  %s✗ Failed:%s %-4s  %s⟳ Active:%s %-4s  %s◦ Pending:%s %s\n' \
        "$C_GREEN" "$C_RESET" "${success:-0}" \
        "$C_RED" "$C_RESET" "${failed:-0}" \
        "$C_YELLOW" "$C_RESET" "${in_prog:-0}" \
        "$C_DIM" "$C_RESET" "${pending:-0}"
      echo

      # Active batches with last log line
      local active_list
      active_list="$(echo "$json" | grep -o '"active": *\[[^]]*\]' | grep -o '"[A-Z0-9_]*"' | tr -d '"')"
      if [[ -n "$active_list" ]]; then
        printf '  %s── Active Batches ──%s\n' "$C_BOLD" "$C_RESET"
        for ab in $active_list; do
          local ab_log="$(run_dir "$RUN_ID")/${ab}.log"
          local last_line=""
          if [[ -f "$ab_log" ]]; then
            last_line="$(tail -1 "$ab_log" 2>/dev/null | head -c 80)"
          fi
          printf '  %s%-8s%s %s\n' "$C_YELLOW" "$ab" "$C_RESET" "$last_line"
        done
        echo
      fi

      # Recent failures
      local fail_list
      fail_list="$(echo "$json" | grep -o '"failed_list": *\[[^]]*\]' | grep -o '"[A-Z0-9_]*"' | tr -d '"')"
      if [[ -n "$fail_list" ]]; then
        printf '  %s── Failed Batches ──%s\n' "$C_BOLD$C_RED" "$C_RESET"
        for fb in $fail_list; do
          local fb_result
          fb_result="$(cat "$(run_result_file "$RUN_ID" "$fb")" 2>/dev/null || echo "?")"
          printf '  %s%-8s%s %s\n' "$C_RED" "$fb" "$C_RESET" "$fb_result"
        done
        echo
      fi

      # Recent events (last 5)
      if [[ -f "$local_events" ]]; then
        printf '  %s── Recent Events ──%s\n' "$C_DIM" "$C_RESET"
        tail -5 "$local_events" 2>/dev/null | while IFS= read -r line; do
          local ev_batch ev_event
          ev_batch="$(echo "$line" | grep -o '"batch":"[^"]*"' | cut -d'"' -f4)"
          ev_event="$(echo "$line" | grep -o '"event":"[^"]*"' | cut -d'"' -f4)"
          printf '  %s%-8s%s %s\n' "$C_DIM" "$ev_batch" "$C_RESET" "$ev_event"
        done
        echo
      fi

      # Merge-back status
      if [[ -f "$(run_json_log "$RUN_ID")" ]]; then
        local merge_events
        merge_events="$(grep 'MERGE_BACK' "$(run_json_log "$RUN_ID")" 2>/dev/null | tail -3)"
        if [[ -n "$merge_events" ]]; then
          printf '  %s── Merge-Back ──%s\n' "$C_MAGENTA" "$C_RESET"
          echo "$merge_events" | while IFS= read -r line; do
            local mb_event
            mb_event="$(echo "$line" | grep -o '"event":"[^"]*"' | cut -d'"' -f4)"
            local mb_detail
            mb_detail="$(echo "$line" | grep -o '"details":"[^"]*"' | cut -d'"' -f4)"
            printf '  %s%s%s %s\n' "$C_MAGENTA" "$mb_event" "$C_RESET" "$mb_detail"
          done
          echo
        fi
      fi
    else
      echo "  Waiting for first status update..."
    fi

    printf '%s  Updated: %s | Refresh: %ss%s\n' "$C_DIM" "$(date +%H:%M:%S)" "$OPT_WATCH_INTERVAL" "$C_RESET"
    sleep "$OPT_WATCH_INTERVAL"
  done
  exit 0
fi

# ── --tail (multiplex active batch logs) ──

if (( OPT_TAIL )); then
  RUN_ID="$(latest_run_id 2>/dev/null)" || { log_err "tail" "No runs found"; exit 1; }
  RUN_LOG_DIR="$(run_dir "$RUN_ID")"

  if command -v multitail >/dev/null 2>&1; then
    # Use multitail if available (best experience)
    log_args=()
    for f in "$RUN_LOG_DIR"/*.log; do
      [[ -f "$f" ]] || continue
      log_args+=("-l" "$f")
    done
    exec multitail "${log_args[@]}"
  else
    # Fallback: tail -f with prefixed output
    log_info "tail" "Tailing all batch logs (run: $RUN_ID)"
    log_info "tail" "Install 'multitail' for split-pane view"
    echo
    tail -f "$RUN_LOG_DIR"/*.log 2>/dev/null
  fi
  exit 0
fi

# ── --disk ──

show_disk_usage() {
  log_header "DISK USAGE"
  local target_parent="${TMPDIR:-/tmp}/roko-par-${RUNNER_NAME}"
  if [[ -d "$target_parent" ]]; then
    log_info "disk" "Target dirs:"
    du -sh "$target_parent"/*/ 2>/dev/null | while read -r size dir; do
      printf '  %8s  %s\n' "$size" "$(basename "$dir")"
    done
  else
    log_info "disk" "No target dirs"
  fi
  echo
  if [[ -d "$WORKTREE_ROOT" ]]; then
    local wt_count=0
    for wt in "$WORKTREE_ROOT"/${RUNNER_NAME}-*/; do
      [[ -d "$wt" ]] || continue
      wt_count=$((wt_count + 1))
    done
    log_info "disk" "Worktrees for $RUNNER_NAME: $wt_count"
  fi
  echo
  local free_mb
  free_mb="$(get_free_mb /)"
  log_info "disk" "Free: ${free_mb}MB"
}

if (( OPT_DISK )); then
  show_disk_usage
  exit 0
fi

# ── --cleanup ──

if (( OPT_CLEANUP )); then
  log_header "CLEANUP"
  cleanup_old_runs "$OPT_CLEANUP_KEEP"
  show_disk_usage
  exit 0
fi

# ── Preflight ──

preflight_check || exit 1

# ── Run ID + continue ──

if (( OPT_CONTINUE )); then
  RUN_ID="$(latest_run_id)" || { log_err "main" "No previous run to continue"; exit 1; }
  log_info "main" "Continuing run: $RUN_ID"

  # Restore main worktree/branch/source from manifest
  MANIFEST_FILE="$(run_manifest_file "$RUN_ID")"
  if [[ -f "$MANIFEST_FILE" ]]; then
    MAIN_WORKTREE="$(grep "^MAIN_WORKTREE=" "$MANIFEST_FILE" | cut -d"'" -f2)"
    MAIN_BRANCH="$(grep "^MAIN_BRANCH=" "$MANIFEST_FILE" | cut -d"'" -f2)"
    SOURCE_BRANCH="$(grep "^SOURCE_BRANCH=" "$MANIFEST_FILE" | cut -d"'" -f2)"
  fi
  if [[ -z "${MAIN_WORKTREE:-}" || ! -d "${MAIN_WORKTREE:-}" ]]; then
    log_err "main" "Main worktree from previous run not found — cannot continue"
    log_err "main" "Expected: $MAIN_WORKTREE"
    exit 1
  fi
  log_ok "main" "Restored worktree: ${MAIN_WORKTREE##*/}"
else
  RUN_ID="run-$(date +%Y%m%d-%H%M%S)"
  log_info "main" "New run: $RUN_ID"

  MAIN_BRANCH="codex/${RUNNER_NAME}-${RUN_ID}"
  MAIN_WORKTREE="$WORKTREE_ROOT/${RUNNER_NAME}-${RUN_ID}-main"

  # Record source branch for merge-back
  SOURCE_BRANCH="$(git -C "$ROKO_ROOT" rev-parse --abbrev-ref HEAD)"
fi

RUN_DIR="$(run_dir "$RUN_ID")"
ensure_dir "$RUN_DIR"
ensure_dir "$(run_prompts_dir "$RUN_ID")"

# ── Merge-back config ──
if (( OPT_NO_MERGE_BACK )); then
  MERGE_BACK_ENABLED=0
fi
export MERGE_BACK_ENABLED

# ── Main worktree ──

create_main_worktree() {
  if [[ -d "$MAIN_WORKTREE" ]]; then
    log_info "main" "Reusing main worktree"
    return 0
  fi
  local base_ref
  base_ref="$(git -C "$ROKO_ROOT" rev-parse HEAD)"
  log_info "main" "Creating main worktree from $(git -C "$ROKO_ROOT" rev-parse --short HEAD)"
  git -C "$ROKO_ROOT" worktree add -b "$MAIN_BRANCH" "$MAIN_WORKTREE" "$base_ref" >/dev/null 2>&1 || {
    # Branch might exist from a previous aborted run
    git -C "$ROKO_ROOT" worktree add "$MAIN_WORKTREE" "$MAIN_BRANCH" >/dev/null 2>&1 || {
      log_err "main" "Failed to create main worktree"
      exit 1
    }
  }
  log_ok "main" "Main worktree: ${MAIN_WORKTREE##*/}"
}

create_main_worktree

# ── Manifest ──

write_manifest() {
  cat > "$(run_manifest_file "$RUN_ID")" <<EOF
RUNNER_NAME='$RUNNER_NAME'
RUN_ID='$RUN_ID'
MAIN_WORKTREE='$MAIN_WORKTREE'
MAIN_BRANCH='$MAIN_BRANCH'
SOURCE_BRANCH='${SOURCE_BRANCH:-}'
PARALLEL='$PARALLEL'
BATCHES='${ACTIVE_BATCHES[*]}'
STARTED_AT='$(date -Iseconds)'
MODEL='$CONV_MODEL'
BASE_REF='$(git -C "$ROKO_ROOT" rev-parse HEAD)'
MERGE_BACK_ENABLED='$MERGE_BACK_ENABLED'
EOF
}

write_manifest

# ── Dry run ──

if (( OPT_DRY_RUN )); then
  log_header "DRY RUN"
  echo "Runner:     $RUNNER_NAME"
  echo "Run ID:     $RUN_ID"
  echo "Parallel:   $PARALLEL"
  echo "Model:      $CONV_MODEL"
  echo "Batches:    ${#ACTIVE_BATCHES[@]}"
  echo "Wave gates: $(( OPT_NO_GATE ? 0 : 1 ))"
  echo "Test gate:  $(( OPT_NO_TEST ? 0 : 1 ))"
  echo "Worktree:   $MAIN_WORKTREE"
  echo

  wave=0
  while true; do
    ready=()
    mapfile -t ready < <(dag_ready_batches "$RUN_ID" "${ACTIVE_BATCHES[@]}")
    (( ${#ready[@]} == 0 )) && break
    wave=$((wave + 1))
    # Show with parallel chunking
    if (( ${#ready[@]} <= PARALLEL )); then
      echo "Wave $wave: ${ready[*]}"
    else
      echo "Wave $wave: ${ready[*]}  (${#ready[@]} batches, $PARALLEL at a time)"
    fi
    for b in "${ready[@]}"; do
      echo "success" > "$(run_result_file "$RUN_ID" "$b")"
    done
  done
  echo
  echo "Total waves: $wave"
  # Clean up simulated results
  for b in "${ACTIVE_BATCHES[@]}"; do
    rm -f "$(run_result_file "$RUN_ID" "$b")"
  done
  # Clean up worktree + dir + branch
  git -C "$ROKO_ROOT" worktree remove --force "$MAIN_WORKTREE" 2>/dev/null || true
  git -C "$ROKO_ROOT" branch -D "$MAIN_BRANCH" 2>/dev/null || true
  rm -rf "$RUN_DIR"
  exit 0
fi

# ── Trap: protect worktrees on exit ──

cleanup_on_exit() {
  local exit_code=$?
  echo
  if (( exit_code != 0 )); then
    log_warn "main" "Run interrupted (exit $exit_code)"
    log_warn "main" "Main worktree preserved: $MAIN_WORKTREE"
    log_warn "main" "Resume with: --continue"
  fi
  # Preserve all sub-worktrees for inspection — list any that remain
  for batch in "${ACTIVE_BATCHES[@]}"; do
    local wt
    wt="$(batch_worktree_path "$RUN_ID" "$batch")"
    if [[ -d "$wt" ]]; then
      log_info "main" "Batch worktree preserved: $wt"
    fi
  done
  # Remove merge lock and source merge worktree
  rm -f "$(run_merge_lock "$RUN_ID")"
  cleanup_source_worktree "$RUN_ID" 2>/dev/null || true
  # Write final timestamp to manifest
  if [[ -f "$(run_manifest_file "$RUN_ID")" ]]; then
    echo "FINISHED_AT='$(date -Iseconds)'" >> "$(run_manifest_file "$RUN_ID")"
    echo "EXIT_CODE='$exit_code'" >> "$(run_manifest_file "$RUN_ID")"
  fi
  # Final status update
  update_status_json "$RUN_ID" "${ACTIVE_BATCHES[@]}" 2>/dev/null || true
}

trap cleanup_on_exit EXIT

# ── Dispatch a single batch ──

dispatch_one() {
  local batch="$1" attempt="$2"
  local wt
  wt="$(create_batch_worktree "$RUN_ID" "$batch" "$MAIN_BRANCH")"
  set_current_batch "$RUN_ID" "$batch" "$attempt"
  record_event "$RUN_ID" "$batch" "$attempt" "dispatch_start"

  # Spawn codex
  local spawn_rc=0
  spawn_batch "$batch" "$RUN_ID" "$wt" "$attempt" || spawn_rc=$?

  if (( spawn_rc != 0 )); then
    local fail_status="spawn_failed"
    (( spawn_rc == 124 )) && fail_status="timeout"
    echo "$fail_status" > "$(run_result_file "$RUN_ID" "$batch")"
    record_event "$RUN_ID" "$batch" "$attempt" "$fail_status"

    # Save failure context for retry prompt
    {
      echo "## Failure in attempt $attempt"
      echo "Exit code: $spawn_rc ($fail_status)"
      echo
      local log_file
      log_file="$(run_log_file "$RUN_ID" "$batch")"
      if [[ -f "$log_file" ]]; then
        echo "### Last 50 lines of log:"
        echo '```'
        tail -50 "$log_file"
        echo '```'
      fi
    } > "$(run_failure_file "$RUN_ID" "$batch")"

    # Preserve worktree for inspection — don't delete on failure
    log_warn "$batch" "Worktree preserved: $wt"
    return "$spawn_rc"
  fi

  # AP checks
  local ap_rc=0
  run_ap_checks "$batch" "$wt" "$RUN_ID" || ap_rc=$?
  if (( ap_rc != 0 )); then
    echo "antipattern_failed" > "$(run_result_file "$RUN_ID" "$batch")"
    record_event "$RUN_ID" "$batch" "$attempt" "antipattern_failed"
    {
      echo "## Anti-pattern check failed (attempt $attempt)"
      echo
      echo "Review the AP violations above and fix them."
    } > "$(run_failure_file "$RUN_ID" "$batch")"
    # Preserve worktree for inspection
    log_warn "$batch" "Worktree preserved: $wt"
    return 1
  fi

  # Merge to main (serialized via flock inside merge_batch_to_main)
  local merge_rc=0
  merge_batch_to_main "$batch" "$RUN_ID" "$MAIN_WORKTREE" || merge_rc=$?
  if (( merge_rc == 10 )); then
    echo "success_noop" > "$(run_result_file "$RUN_ID" "$batch")"
    record_event "$RUN_ID" "$batch" "$attempt" "success_noop"
    log_warn "$batch" "No changes produced"
    remove_batch_worktree "$RUN_ID" "$batch"
  elif (( merge_rc != 0 )); then
    echo "merge_failed" > "$(run_result_file "$RUN_ID" "$batch")"
    record_event "$RUN_ID" "$batch" "$attempt" "merge_failed"
    {
      echo "## Merge conflict (attempt $attempt)"
      echo
      echo "Your changes conflicted with another batch that was merged first."
      echo "Rebase your changes on the current main branch state."
    } > "$(run_failure_file "$RUN_ID" "$batch")"
    # Preserve worktree for inspection — don't delete on merge failure
    log_warn "$batch" "Worktree preserved (merge failed): $wt"
    return 1
  else
    echo "success" > "$(run_result_file "$RUN_ID" "$batch")"
    record_event "$RUN_ID" "$batch" "$attempt" "success"
    # cumulative context is snapshotted inside merge_batch_to_main (under lock)
    # Only clean up worktree after successful merge
    remove_batch_worktree "$RUN_ID" "$batch"
  fi

  return 0
}

# ── Process a batch with retries (runs in subshell for parallel) ──
# Note: set +e inside to prevent subshell exit-on-error killing retry logic

process_batch() {
  local batch="$1"
  local attempt=1

  # Disable exit-on-error so retries work in subshells
  set +e

  while (( attempt <= MAX_RETRIES )); do
    log_info "$batch" "Attempt $attempt/$MAX_RETRIES — $(batch_title "$batch")"

    local rc=0
    dispatch_one "$batch" "$attempt" || rc=$?

    if (( rc == 0 )); then
      set -e
      return 0
    fi

    if (( attempt < MAX_RETRIES )); then
      log_warn "$batch" "Retrying ($((attempt+1))/$MAX_RETRIES)..."
      rm -f "$(run_result_file "$RUN_ID" "$batch")"
    fi
    attempt=$((attempt + 1))
  done

  log_err "$batch" "All $MAX_RETRIES attempts failed"
  set -e
  return 1
}

# ── Track which groups have been gated ──

declare -A GATED_GROUPS=()

maybe_run_group_gate() {
  local group="$1"
  # Skip if already gated or gates disabled
  (( OPT_NO_GATE )) && return 0
  [[ -n "${GATED_GROUPS[$group]:-}" ]] && return 0

  # Check if all batches in this group are done
  if dag_group_complete "$RUN_ID" "$group" "${ACTIVE_BATCHES[@]}"; then
    GATED_GROUPS["$group"]=1
    run_wave_gate "$group" "$RUN_ID" "$MAIN_WORKTREE" || {
      log_err "gate" "Wave gate failed for group $group — continuing"
      record_status "$RUN_ID" "gate:$group" "0" "gate_failed" "check+clippy"
    }

    # Checkpoint merge-back after each runner group completes
    # Groups are named like "2A", "2B", etc. — merge at the last group of each runner
    maybe_merge_back_checkpoint "$group"
  fi
}

# ── Merge-back checkpoint logic ──

declare -A MERGED_RUNNERS=()

maybe_merge_back_checkpoint() {
  local group="$1"
  (( MERGE_BACK_ENABLED )) || return 0

  # Extract runner number from group (e.g. "2A" → "2", "5C" → "5")
  local runner_num="${group%%[A-Z]*}"
  [[ -n "$runner_num" ]] || return 0

  # Skip if already merged this runner
  [[ -n "${MERGED_RUNNERS[$runner_num]:-}" ]] && return 0

  # Check if ALL groups for this runner are complete
  local all_groups_done=1
  for batch in "${ACTIVE_BATCHES[@]}"; do
    local grp
    grp="$(batch_group "$batch")"
    local batch_runner="${grp%%[A-Z]*}"
    [[ "$batch_runner" == "$runner_num" ]] || continue
    local rf
    rf="$(run_result_file "$RUN_ID" "$batch")"
    if [[ ! -f "$rf" ]]; then
      all_groups_done=0; break
    fi
    local s; s="$(cat "$rf")"
    if [[ "$s" == "in_progress" ]]; then
      all_groups_done=0; break
    fi
  done

  if (( all_groups_done )); then
    MERGED_RUNNERS["$runner_num"]=1
    log_info "merge-back" "Runner $runner_num complete — triggering checkpoint merge"
    merge_back_checkpoint "$RUN_ID" "runner-${runner_num}" "$MAIN_WORKTREE" || {
      log_warn "merge-back" "Checkpoint merge for runner $runner_num failed — will retry at final"
    }
  fi
}

# ── Main parallel dispatch loop ──

run_main_loop() {
  local total=${#ACTIVE_BATCHES[@]}
  local wave=0

  # Init status tracking
  _STATUS_RUN_ID="$RUN_ID"
  _STATUS_START_TS=$(date +%s)

  log_header "DISPATCH (${total} batches, parallel=$PARALLEL)"
  log_info "main" "Monitor commands:"
  log_info "main" "  Live dashboard:   $0 --watch"
  log_info "main" "  Status JSON:      cat $(run_status_json "$RUN_ID")"
  log_info "main" "  All events:       tail -f $(run_json_log "$RUN_ID")"
  log_info "main" "  Specific batch:   tail -f $(run_dir "$RUN_ID")/<BATCH>.log"
  log_info "main" "  Active batches:   cat $(run_dir "$RUN_ID")/*.result | sort | uniq -c"
  if (( MERGE_BACK_ENABLED )); then
    log_info "main" "  Merge-back:       grep MERGE_BACK $(run_json_log "$RUN_ID")"
  fi
  echo

  while true; do
    # Check disk space before dispatching
    check_disk_space "dispatch" 2000 || {
      log_err "main" "Low disk space — attempting cleanup"
      cleanup_old_runs 1
      check_disk_space "dispatch" 2000 || {
        log_err "main" "Still low on disk after cleanup — aborting"
        return 1
      }
    }

    # Find ready batches
    local -a ready=()
    mapfile -t ready < <(dag_ready_batches "$RUN_ID" "${ACTIVE_BATCHES[@]}")

    # Mark blocked batches
    local -a blocked=()
    mapfile -t blocked < <(dag_blocked_batches "$RUN_ID" "${ACTIVE_BATCHES[@]}")
    for b in "${blocked[@]}"; do
      [[ -z "$b" ]] && continue
      if [[ ! -f "$(run_result_file "$RUN_ID" "$b")" ]]; then
        echo "blocked" > "$(run_result_file "$RUN_ID" "$b")"
        log_warn "$b" "Blocked (dependency failed)"
        record_event "$RUN_ID" "$b" "0" "blocked"
      fi
    done

    # Update live status
    update_status_json "$RUN_ID" "${ACTIVE_BATCHES[@]}"

    # Nothing ready? We're done
    if (( ${#ready[@]} == 0 )); then
      break
    fi

    wave=$((wave + 1))

    # Pause between waves if requested
    if (( OPT_PAUSE && wave > 1 )); then
      log_info "main" "Paused. Press Enter to continue or Ctrl-C to abort..."
      read -r
    fi

    # Status line
    local done_count fail_count
    done_count="$(dag_count "$RUN_ID" success "${ACTIVE_BATCHES[@]}")"
    fail_count="$(dag_count "$RUN_ID" failed "${ACTIVE_BATCHES[@]}")"

    # Cap dispatch at PARALLEL
    local -a dispatch_batch=("${ready[@]:0:$PARALLEL}")
    log_header "WAVE $wave — ${#dispatch_batch[@]} batch(es): ${dispatch_batch[*]}  [$done_count/$total done, $fail_count failed]"

    if (( ${#dispatch_batch[@]} == 1 )); then
      # Single batch — run inline
      local b="${dispatch_batch[0]}"
      echo "in_progress" > "$(run_result_file "$RUN_ID" "$b")"
      update_status_json "$RUN_ID" "${ACTIVE_BATCHES[@]}"
      process_batch "$b" || true

      # Check if this batch's group is now complete
      maybe_run_group_gate "$(batch_group "$b")"
    else
      # Multiple batches — run in parallel
      local -a pids=()
      local -A pid_batch=()

      for b in "${dispatch_batch[@]}"; do
        echo "in_progress" > "$(run_result_file "$RUN_ID" "$b")"
      done
      update_status_json "$RUN_ID" "${ACTIVE_BATCHES[@]}"

      for b in "${dispatch_batch[@]}"; do
        process_batch "$b" &
        local pid=$!
        pids+=("$pid")
        pid_batch[$pid]="$b"
        log_info "$b" "Spawned (PID $pid)"
      done

      # Wait for all, update status as each completes
      local -a failed_pids=()
      for pid in "${pids[@]}"; do
        local wait_rc=0
        wait "$pid" || wait_rc=$?
        local b="${pid_batch[$pid]}"
        if (( wait_rc != 0 )); then
          failed_pids+=("$pid")
          log_err "$b" "Process exited $wait_rc"
        fi
        # Update status after each completion
        update_status_json "$RUN_ID" "${ACTIVE_BATCHES[@]}"
      done

      if (( ${#failed_pids[@]} > 0 )); then
        log_warn "wave$wave" "${#failed_pids[@]}/${#dispatch_batch[@]} batches had errors"
      fi

      # Check if any group completed in this wave
      local -A wave_groups=()
      for b in "${dispatch_batch[@]}"; do
        wave_groups["$(batch_group "$b")"]=1
      done
      for grp in "${!wave_groups[@]}"; do
        maybe_run_group_gate "$grp"
      done
    fi
  done
}

# ── Summary ──

print_summary() {
  log_header "SUMMARY"

  local total=${#ACTIVE_BATCHES[@]}
  local success_count fail_count
  success_count="$(dag_count "$RUN_ID" success "${ACTIVE_BATCHES[@]}")"
  fail_count="$(dag_count "$RUN_ID" failed "${ACTIVE_BATCHES[@]}")"
  local skip_count=$((total - success_count - fail_count))

  printf '  Total:     %d\n' "$total"
  printf '  %sSuccess:   %d%s\n' "$C_GREEN" "$success_count" "$C_RESET"
  if (( fail_count > 0 )); then
    printf '  %sFailed:    %d%s\n' "$C_RED" "$fail_count" "$C_RESET"
  fi
  if (( skip_count > 0 )); then
    printf '  %sSkipped:   %d%s\n' "$C_YELLOW" "$skip_count" "$C_RESET"
  fi
  echo

  # Per-batch results
  printf '  %-6s %-10s %s\n' "ID" "STATUS" "TITLE"
  for batch in "${ACTIVE_BATCHES[@]}"; do
    local rf
    rf="$(run_result_file "$RUN_ID" "$batch")"
    local status="pending"
    [[ -f "$rf" ]] && status="$(cat "$rf")"
    local color="$C_DIM"
    case "$status" in
      success|success_noop|verified) color="$C_GREEN" ;;
      spawn_failed|verify_failed|timeout|blocked|antipattern_failed|merge_failed) color="$C_RED" ;;
      in_progress) color="$C_YELLOW" ;;
    esac
    printf '  %-6s %s%-10s%s %s\n' "$batch" "$color" "$status" "$C_RESET" "$(batch_title "$batch" | head -c 60)"
  done
  echo

  # Disk report
  local target_dir
  target_dir="$(run_target_dir "$RUN_ID")"
  if [[ -d "$target_dir" ]]; then
    local target_size
    target_size="$(du -sh "$target_dir" 2>/dev/null | awk '{print $1}')"
    log_info "disk" "Target dir: $target_size ($target_dir)"
  fi
  local free_mb
  free_mb="$(get_free_mb /)"
  log_info "disk" "Free: ${free_mb}MB"

  # Main worktree location
  echo
  log_info "main" "Main worktree: $MAIN_WORKTREE"
  log_info "main" "Branch: $MAIN_BRANCH"
  log_info "main" "Logs: $RUN_DIR"
  log_info "main" "Status: $(run_status_json "$RUN_ID")"

  if (( fail_count > 0 )); then
    echo
    log_warn "main" "To retry failed: --continue"
    log_warn "main" "To clean up: --cleanup"
  fi

  if (( success_count == total )); then
    echo
    log_ok "main" "All batches succeeded!"
    log_info "main" "Inspect: cd $MAIN_WORKTREE && git log --oneline"
    if (( MERGE_BACK_ENABLED )); then
      local source
      source="$(get_source_branch "$RUN_ID")"
      if [[ -n "$source" ]]; then
        log_ok "main" "Merged back to: $source"
      fi
    else
      log_info "main" "Merge:   git -C $ROKO_ROOT merge $MAIN_BRANCH"
    fi
  fi
}

# ── Main ──

main() {
  local start_ts
  start_ts=$(date +%s)
  _STATUS_START_TS=$start_ts

  log_header "PARALLEL RUNNER: $RUNNER_NAME"
  log_info "main" "Run:      $RUN_ID"
  log_info "main" "Batches:  ${#ACTIVE_BATCHES[@]}"
  log_info "main" "Parallel: $PARALLEL"
  log_info "main" "Model:    $CONV_MODEL"
  log_info "main" "Worktree: $MAIN_WORKTREE"
  if (( MERGE_BACK_ENABLED )); then
    log_info "main" "Source:   ${SOURCE_BRANCH:-?} (auto merge-back ON)"
  else
    log_info "main" "Source:   ${SOURCE_BRANCH:-?} (merge-back OFF)"
  fi
  echo

  # Cleanup old runs first (run in subshell to isolate set -e failures)
  ( set +e; cleanup_old_runs "$OPT_CLEANUP_KEEP" 2>/dev/null; true ) || true

  # Run the dispatch loop
  run_main_loop

  # End-of-run test gate
  local success_count
  success_count="$(dag_count "$RUN_ID" success "${ACTIVE_BATCHES[@]}")"
  if (( success_count > 0 && ! OPT_NO_TEST )); then
    run_test_gate "$RUN_ID" "$MAIN_WORKTREE" || {
      log_warn "test" "Test gate failed — results preserved for inspection"
      record_status "$RUN_ID" "test" "0" "test_failed" ""
    }
  fi

  # Final merge-back to source branch
  if (( success_count > 0 && MERGE_BACK_ENABLED )); then
    merge_back_final "$RUN_ID" "$MAIN_WORKTREE" || {
      log_warn "merge-back" "Final merge requires manual intervention"
    }
  fi

  local elapsed=$(( $(date +%s) - start_ts ))

  print_summary

  echo
  log_ok "main" "Run completed in $(fmt_duration "$elapsed")"
}

main
