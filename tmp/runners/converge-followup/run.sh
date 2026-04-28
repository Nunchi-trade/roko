#!/usr/bin/env bash
# run.sh — batch runner with cumulative context, tiered verification, track gates
#
# Architecture (what's different from the converge runner):
#
#   1. CUMULATIVE CONTEXT: After each batch, we snapshot what it actually changed.
#      The next batch's prompt includes all prior batch outputs, so Codex sees
#      real code — not a description of what should have been produced.
#
#   2. TIERED VERIFICATION for efficiency:
#      - Per-batch:  cargo check -p <crate>  + anti-pattern grep     (~5-10s)
#      - Per-track:  cargo check --workspace + clippy + semantic AP   (~1-2min)
#      - End-of-run: cargo test --workspace                          (~3-5min)
#      All share ONE CARGO_TARGET_DIR for incremental compilation.
#
#   3. ANTI-PATTERN DETECTION: Specific checks for problems the converge runner
#      introduced: stubs-that-pass, block_on-in-async, duplicate traits,
#      inline prompts, shell-out-to-CLI, std::Mutex-across-await.
#
#   4. TRACK GATES with optional human review pause between tracks.
#
#   5. LARGER BATCHES: batches.toml supports an also_read field so a batch
#      can see related files it doesn't modify, enabling coherent decisions.
#
# Usage:
#   bash tmp/runners/converge-followup/run.sh
#   bash tmp/runners/converge-followup/run.sh --only FX01,FX02
#   bash tmp/runners/converge-followup/run.sh --group fix
#   bash tmp/runners/converge-followup/run.sh --continue last
#   bash tmp/runners/converge-followup/run.sh --dry-run
#   bash tmp/runners/converge-followup/run.sh --list
#   bash tmp/runners/converge-followup/run.sh --no-gate   # skip track gates
#   bash tmp/runners/converge-followup/run.sh --pause      # pause between tracks

set -euo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

source "$SCRIPT_DIR/lib/common.sh"
source "$SCRIPT_DIR/lib/spawn.sh"
source "$SCRIPT_DIR/lib/verify.sh"

# ── Defaults ──

: "${CONV_MODEL:=gpt-5.5}"
: "${CONV_REASONING:=high}"
: "${CONV_TIMEOUT:=5400}"
: "${CONV_MAX_RETRIES:=2}"
: "${CONV_BASE_REF:=HEAD}"

DRY_RUN=0
FORCE=0
VERIFY_ONLY=0
LIST_ONLY=0
CONTINUE_RUN=""
SELECTED_BATCHES=()
GROUP_FILTER=""
SKIP_TRACK_GATES=0
PAUSE_BETWEEN_TRACKS=0
SKIP_TEST_GATE=0
DO_CLEANUP=0
CLEANUP_KEEP=2

# ── Disk usage report ──

show_disk_usage() {
  log_header "DISK USAGE"
  local target_parent="${TMPDIR:-/tmp}/roko-followup-targets"

  printf '  %-50s %s\n' "LOCATION" "SIZE"
  printf '  %-50s %s\n' "────────" "────"

  if [[ -d "$target_parent" ]]; then
    for dir in "$target_parent"/*/; do
      [[ -d "$dir" ]] || continue
      printf '  %-50s %s\n' "target: $(basename "$dir")" "$(du -sh "$dir" 2>/dev/null | awk '{print $1}')"
    done
    printf '  %-50s %s\n' "TOTAL target dirs" "$(du -sh "$target_parent" 2>/dev/null | awk '{print $1}')"
  else
    printf '  %-50s %s\n' "target dirs" "(none)"
  fi

  if [[ -d "$WORKTREE_ROOT" ]]; then
    for wt in "$WORKTREE_ROOT"/followup-*/; do
      [[ -d "$wt" ]] || continue
      printf '  %-50s %s\n' "worktree: $(basename "$wt")" "$(du -sh "$wt" 2>/dev/null | awk '{print $1}')"
    done
  fi

  if [[ -d "$LOG_ROOT" ]]; then
    printf '  %-50s %s\n' "logs" "$(du -sh "$LOG_ROOT" 2>/dev/null | awk '{print $1}')"
  fi

  echo
  printf '  Free space: %sMB\n' "$(get_free_mb /)"
}

# ── CLI parsing ──

print_usage() {
  cat <<'EOF'
run.sh — converge followup runner

Usage:
  bash tmp/runners/converge-followup/run.sh [OPTIONS]

Options:
  --only LIST         Comma-separated batch ids
  --group NAME        Run all batches in a group
  --continue RUN      Continue a prior run (or 'last')
  --dry-run           Preview prompts without executing
  --force             Re-run successful batches
  --verify-only       Skip Codex, verify current worktree state
  --list              Show batch manifest
  --no-gate           Skip track gates (per-batch checks only)
  --no-test           Skip end-of-run test gate
  --pause             Pause for review between tracks
  --cleanup           Clean up old run artifacts (target dirs, worktrees, snapshots)
  --cleanup-keep N    Keep N most recent runs when cleaning (default: 2)
  --disk              Show disk usage of all runner artifacts
  --model MODEL       Override model (default: gpt-5.5)
  --reasoning LEVEL   Reasoning effort (default: high)
  --timeout SECONDS   Per-batch timeout (default: 5400)
  --retries N         Max retries per batch (default: 2)
  --base-ref REF      Git ref for worktree (default: HEAD)
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --only)       IFS=',' read -r -a SELECTED_BATCHES <<< "$2"; shift 2 ;;
    --only=*)     IFS=',' read -r -a SELECTED_BATCHES <<< "${1#*=}"; shift ;;
    --group)      GROUP_FILTER="$2"; shift 2 ;;
    --group=*)    GROUP_FILTER="${1#*=}"; shift ;;
    --continue)   CONTINUE_RUN="$2"; shift 2 ;;
    --continue=*) CONTINUE_RUN="${1#*=}"; shift ;;
    --dry-run)    DRY_RUN=1; shift ;;
    --force)      FORCE=1; shift ;;
    --verify-only) VERIFY_ONLY=1; shift ;;
    --list)       LIST_ONLY=1; shift ;;
    --no-gate)    SKIP_TRACK_GATES=1; shift ;;
    --no-test)    SKIP_TEST_GATE=1; shift ;;
    --pause)      PAUSE_BETWEEN_TRACKS=1; shift ;;
    --cleanup)    DO_CLEANUP=1; shift ;;
    --cleanup-keep) CLEANUP_KEEP="$2"; shift 2 ;;
    --cleanup-keep=*) CLEANUP_KEEP="${1#*=}"; shift ;;
    --disk)       show_disk_usage; exit 0 ;;
    --model)      CONV_MODEL="$2"; shift 2 ;;
    --model=*)    CONV_MODEL="${1#*=}"; shift ;;
    --reasoning)  CONV_REASONING="$2"; shift 2 ;;
    --reasoning=*) CONV_REASONING="${1#*=}"; shift ;;
    --timeout)    CONV_TIMEOUT="$2"; shift 2 ;;
    --timeout=*)  CONV_TIMEOUT="${1#*=}"; shift ;;
    --retries)    CONV_MAX_RETRIES="$2"; shift 2 ;;
    --retries=*)  CONV_MAX_RETRIES="${1#*=}"; shift ;;
    --base-ref)   CONV_BASE_REF="$2"; shift 2 ;;
    --base-ref=*) CONV_BASE_REF="${1#*=}"; shift ;;
    -h|--help)    print_usage; exit 0 ;;
    *) log_err "cli" "Unknown: $1"; print_usage; exit 1 ;;
  esac
done

# ── Load batch registry ──

load_batch_registry

# ── Batch selection ──

select_batches() {
  if [[ -n "$GROUP_FILTER" ]]; then
    for batch in "${ALL_BATCHES[@]}"; do
      [[ "$(batch_group "$batch")" == "$GROUP_FILTER" ]] && echo "$batch"
    done; return
  fi
  if [[ ${#SELECTED_BATCHES[@]} -eq 0 ]]; then
    printf '%s\n' "${ALL_BATCHES[@]}"; return
  fi
  for batch in "${ALL_BATCHES[@]}"; do
    for sel in "${SELECTED_BATCHES[@]}"; do
      [[ "$batch" == "$sel" ]] && echo "$batch"
    done
  done
}

list_batches() {
  printf '%s%-8s %-50s %-10s %-8s %s%s\n' "$C_BOLD" "ID" "TITLE" "GROUP" "VERIFY" "DEPS" "$C_RESET"
  for batch in "${ALL_BATCHES[@]}"; do
    printf '%-8s %-50s %-10s %-8s %s\n' \
      "$batch" "$(batch_title "$batch")" "$(batch_group "$batch")" \
      "$(batch_verify_mode "$batch")" "$(batch_deps "$batch")"
  done
}

# ── Run management ──

create_run() {
  RUN_ID="run-$(date +%Y%m%d-%H%M%S)"
  WORKTREE="$WORKTREE_ROOT/followup-$RUN_ID"
  BRANCH="codex/followup-$RUN_ID"

  ensure_dir "$LOG_ROOT/$RUN_ID"
  ensure_dir "$(run_prompts_dir "$RUN_ID")"
  ensure_dir "$(run_cumulative_dir "$RUN_ID")"
  : > "$(run_status_file "$RUN_ID")"
  : > "$(run_json_log "$RUN_ID")"
  ln -sfn "$LOG_ROOT/$RUN_ID" "$LOG_ROOT/latest"

  git -C "$ROKO_ROOT" worktree add -b "$BRANCH" "$WORKTREE" "$CONV_BASE_REF" >/dev/null

  # Pre-warm the shared target directory with an initial build
  local target_dir
  target_dir="$(run_target_dir "$RUN_ID")"
  mkdir -p "$target_dir"

  cat > "$(run_manifest_file "$RUN_ID")" <<EOF
RUN_ID='$RUN_ID'
WORKTREE='$WORKTREE'
BRANCH='$BRANCH'
CONV_MODEL='$CONV_MODEL'
CONV_REASONING='$CONV_REASONING'
CONV_TIMEOUT='$CONV_TIMEOUT'
CONV_MAX_RETRIES='$CONV_MAX_RETRIES'
CONV_BASE_REF='$CONV_BASE_REF'
CREATED_AT='$(date -Iseconds)'
EOF

  log_info "runner" "Created run $RUN_ID"
  log_info "runner" "Worktree: $WORKTREE"
  log_info "runner" "Branch: $BRANCH"
  log_info "runner" "Target dir: $target_dir"

  # Pre-warm: initial cargo check so all subsequent checks are incremental
  log_info "runner" "Pre-warming build cache..."
  local warmup_start
  warmup_start=$(date +%s)
  if (cd "$WORKTREE" && env CARGO_TARGET_DIR="$target_dir" cargo check --workspace >/dev/null 2>&1); then
    log_ok "runner" "Build cache warm ($(fmt_duration $(( $(date +%s) - warmup_start ))))"
  else
    log_warn "runner" "Pre-warm failed — workspace may not compile clean at base ref"
  fi
}

load_run() {
  if [[ "$CONTINUE_RUN" == "last" ]]; then
    CONTINUE_RUN="$(latest_run_id || true)"
  fi
  [[ -n "$CONTINUE_RUN" ]] || { log_err "cli" "No prior run found"; exit 1; }
  local manifest
  manifest="$(run_manifest_file "$CONTINUE_RUN")"
  [[ -f "$manifest" ]] || { log_err "cli" "Missing manifest: $manifest"; exit 1; }
  # shellcheck disable=SC1090
  source "$manifest"
  RUN_ID="$CONTINUE_RUN"
  [[ -d "$WORKTREE" ]] || { log_err "cli" "Worktree missing: $WORKTREE"; exit 1; }
  ln -sfn "$LOG_ROOT/$RUN_ID" "$LOG_ROOT/latest"
  log_info "runner" "Continuing run $RUN_ID"
}

# ── Dependency resolution ──

batch_status() {
  local f; f="$(run_result_file "$RUN_ID" "$1")"
  [[ -f "$f" ]] && cat "$f" || true
}

deps_satisfied() {
  local -a deps=()
  IFS=' ' read -r -a deps <<< "$(batch_deps "$1")"
  for dep in "${deps[@]}"; do
    [[ -z "$dep" ]] && continue
    success_status "$(batch_status "$dep")" || return 1
  done; return 0
}

deps_failed() {
  local -a deps=()
  IFS=' ' read -r -a deps <<< "$(batch_deps "$1")"
  for dep in "${deps[@]}"; do
    [[ -z "$dep" ]] && continue
    terminal_failure "$(batch_status "$dep")" && return 0
  done; return 1
}

# ── Single batch execution ──

run_one_batch() {
  local batch="$1"
  local result_file
  result_file="$(run_result_file "$RUN_ID" "$batch")"

  local existing
  existing="$(batch_status "$batch")"
  if [[ -n "$existing" ]] && success_status "$existing" && (( ! FORCE )); then
    log_info "$batch" "Already succeeded; skipping"
    return 0
  fi

  if (( DRY_RUN )); then
    compose_prompt "$batch" "$RUN_ID" "dry" "${WORKTREE:-$ROKO_ROOT}" >/dev/null
    log_info "$batch" "[DRY] $(batch_title "$batch") | verify=$(batch_verify_mode "$batch")"
    echo "dry_run" > "$result_file"
    return 0
  fi

  if (( VERIFY_ONLY )); then
    if verify_batch "$batch" "$RUN_ID" "$WORKTREE"; then
      echo "verified" > "$result_file"; return 0
    fi
    echo "verify_failed" > "$result_file"; return 1
  fi

  # Check disk space before starting (catch problems early, not mid-batch)
  if ! check_disk_space "$batch" 2000; then
    echo "spawn_failed" > "$result_file"
    return 1
  fi

  local attempt
  for attempt in $(seq 1 "$CONV_MAX_RETRIES"); do
    set_current_batch "$RUN_ID" "$batch" "$attempt"
    log_header "$batch — attempt $attempt/$CONV_MAX_RETRIES"
    record_status "$RUN_ID" "$batch" "$attempt" "attempt_start" "$(batch_title "$batch")"

    # Reset worktree
    if worktree_dirty "$WORKTREE"; then
      backup_worktree "$RUN_ID" "$batch" "$attempt" "$WORKTREE" "pre-reset"
      reset_worktree "$WORKTREE"
    fi

    # Spawn
    if ! spawn_batch "$batch" "$RUN_ID" "$WORKTREE" "$attempt"; then
      echo "spawn_failed" > "$result_file"
      if (( attempt < CONV_MAX_RETRIES )); then
        log_warn "$batch" "Retrying after spawn failure"; continue
      fi
      clear_current_batch "$RUN_ID"; return 1
    fi

    # Per-batch verify (fast: per-crate check + anti-pattern grep)
    if ! verify_batch "$batch" "$RUN_ID" "$WORKTREE" "$attempt"; then
      echo "verify_failed" > "$result_file"
      if (( attempt < CONV_MAX_RETRIES )); then
        backup_worktree "$RUN_ID" "$batch" "$attempt" "$WORKTREE" "verify-failed"
        log_warn "$batch" "Retrying after verify failure"; continue
      fi
      clear_current_batch "$RUN_ID"; return 1
    fi

    # Commit
    local commit_rc=0
    commit_batch "$batch" "$WORKTREE" "$RUN_ID" "$attempt" || commit_rc=$?
    if [[ "$commit_rc" -eq 10 ]]; then
      echo "success_noop" > "$result_file"
    elif [[ "$commit_rc" -eq 0 ]]; then
      echo "success" > "$result_file"
      # Snapshot cumulative context for next batches
      snapshot_cumulative_context "$RUN_ID" "$batch" "$WORKTREE"
    else
      echo "verify_failed" > "$result_file"
      clear_current_batch "$RUN_ID"; return 1
    fi

    record_status "$RUN_ID" "$batch" "$attempt" "batch_done" "success"
    clear_current_batch "$RUN_ID"
    return 0
  done

  clear_current_batch "$RUN_ID"
  return 1
}

# ── Track-aware execution ──
# Groups batches by track, runs track gate after each track completes.

get_ordered_tracks() {
  local batch
  local -a tracks=()
  for batch in "${SELECTED[@]}"; do
    local group
    group="$(batch_group "$batch")"
    # Only add if not already present
    local found=0
    for t in "${tracks[@]:-}"; do
      [[ "$t" == "$group" ]] && { found=1; break; }
    done
    (( found )) || tracks+=("$group")
  done
  printf '%s\n' "${tracks[@]}"
}

batches_in_track() {
  local track="$1"
  for batch in "${SELECTED[@]}"; do
    [[ "$(batch_group "$batch")" == "$track" ]] && echo "$batch"
  done
}

run_track() {
  local track="$1"
  local track_batches track_failed=0

  log_header "TRACK: $track"
  mapfile -t track_batches < <(batches_in_track "$track")
  log_info "track" "${#track_batches[@]} batches in track $track"

  for batch in "${track_batches[@]}"; do
    if deps_satisfied "$batch"; then
      run_one_batch "$batch" || track_failed=1
    elif deps_failed "$batch"; then
      log_warn "$batch" "Blocked (dep failed)"
      echo "blocked" > "$(run_result_file "$RUN_ID" "$batch")"
    else
      log_warn "$batch" "Blocked (deps pending)"
      echo "blocked" > "$(run_result_file "$RUN_ID" "$batch")"
    fi
  done

  # Track gate
  if (( ! SKIP_TRACK_GATES && ! DRY_RUN && ! VERIFY_ONLY )); then
    if (( track_failed )); then
      log_warn "gate" "Skipping track gate — batch failures in $track"
    else
      if ! run_track_gate "$track" "$RUN_ID" "$WORKTREE"; then
        log_err "gate" "Track gate FAILED for $track"
        log_err "gate" "Fix issues before continuing to next track"
        track_failed=1
      fi
    fi
  fi

  # Optional human review pause
  if (( PAUSE_BETWEEN_TRACKS && ! DRY_RUN )); then
    echo
    printf '%s[PAUSE]%s Track %s complete. Review changes, then press Enter to continue (or Ctrl-C to abort)... ' \
      "$C_YELLOW" "$C_RESET" "$track"
    read -r
    echo
  fi

  return "$track_failed"
}

# ── Summary ──

print_summary() {
  local success=0 fail=0 skip=0 other=0
  log_header "SUMMARY ($RUN_ID)"

  for batch in "${SELECTED[@]}"; do
    local result
    result="$(batch_status "$batch")"
    result="${result:-pending}"
    if success_status "$result"; then
      printf '  %s✓%s %-8s %s\n' "$C_GREEN" "$C_RESET" "$batch" "$(batch_title "$batch")"
      success=$((success + 1))
    elif terminal_failure "$result"; then
      printf '  %s✗%s %-8s %s (%s)\n' "$C_RED" "$C_RESET" "$batch" "$(batch_title "$batch")" "$result"
      fail=$((fail + 1))
    elif [[ "$result" == "dry_run" ]]; then
      printf '  %s~%s %-8s %s\n' "$C_DIM" "$C_RESET" "$batch" "$(batch_title "$batch")"
      skip=$((skip + 1))
    else
      printf '  %s?%s %-8s %s (%s)\n' "$C_YELLOW" "$C_RESET" "$batch" "$(batch_title "$batch")" "$result"
      other=$((other + 1))
    fi
  done

  printf '\n  success=%d  failed=%d  skipped=%d  other=%d\n' "$success" "$fail" "$skip" "$other"
  printf '  run_id=%s\n' "$RUN_ID"
  printf '  worktree=%s\n' "${WORKTREE:-n/a}"
  printf '  branch=%s\n' "${BRANCH:-n/a}"
  printf '  logs=%s\n' "$LOG_ROOT/$RUN_ID"
}

# ── Main ──

preflight_check

if (( LIST_ONLY )); then
  list_batches; exit 0
fi

if (( DO_CLEANUP )); then
  cleanup_old_runs "$CLEANUP_KEEP"
  show_disk_usage
  exit 0
fi

mapfile -t SELECTED < <(select_batches)

if [[ ${#SELECTED[@]} -eq 0 ]]; then
  log_warn "runner" "No batches selected (is batches.toml populated?)"
  exit 0
fi

if [[ -n "$CONTINUE_RUN" ]]; then
  load_run
elif (( DRY_RUN )); then
  RUN_ID="dry-$(date +%Y%m%d-%H%M%S)"
  ensure_dir "$LOG_ROOT/$RUN_ID" "$(run_prompts_dir "$RUN_ID")" "$(run_cumulative_dir "$RUN_ID")"
  : > "$(run_status_file "$RUN_ID")"
  WORKTREE="$ROKO_ROOT"
  BRANCH="(dry-run)"
else
  create_run
fi

log_info "runner" "Model: $CONV_MODEL | Reasoning: $CONV_REASONING | Timeout: ${CONV_TIMEOUT}s"
log_info "runner" "Batches: ${#SELECTED[@]} | Gates: $(( SKIP_TRACK_GATES ? 0 : 1 )) | Pause: $PAUSE_BETWEEN_TRACKS"

# Execute track by track
any_failed=0
mapfile -t TRACKS < <(get_ordered_tracks)

for track in "${TRACKS[@]}"; do
  run_track "$track" || any_failed=1
done

# End-of-run test gate
if (( ! SKIP_TEST_GATE && ! DRY_RUN && ! VERIFY_ONLY && ! any_failed )); then
  run_test_gate "$RUN_ID" "$WORKTREE" || any_failed=1
fi

print_summary

# Post-run disk report
if [[ -n "${RUN_ID:-}" && "$DRY_RUN" -eq 0 ]]; then
  local target_dir
  target_dir="$(run_target_dir "$RUN_ID")"
  if [[ -d "$target_dir" ]]; then
    local target_size
    target_size="$(du -sm "$target_dir" 2>/dev/null | awk '{print $1}')"
    log_info "disk" "Target dir: ${target_size}MB ($target_dir)"
    log_info "disk" "To reclaim: bash tmp/runners/converge-followup/run.sh --cleanup"
  fi
  local free_mb
  free_mb="$(get_free_mb /)"
  log_info "disk" "Free space: ${free_mb}MB"
  if (( free_mb < CONV_MIN_FREE_MB )); then
    log_warn "disk" "LOW SPACE — run --cleanup to free target dirs and old worktrees"
  fi
fi

exit "$any_failed"
