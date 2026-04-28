#!/usr/bin/env bash

set -euo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

source "$SCRIPT_DIR/lib/common.sh"
source "$SCRIPT_DIR/lib/spawn.sh"
source "$SCRIPT_DIR/lib/verify.sh"

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

print_usage() {
  cat <<'EOF'
run-converge.sh — overnight Codex runner for runtime convergence + demo

Usage:
  bash tmp/runners/converge/run-converge.sh
  bash tmp/runners/converge/run-converge.sh --only F01,F02,F03
  bash tmp/runners/converge/run-converge.sh --group foundation
  bash tmp/runners/converge/run-converge.sh --continue last
  bash tmp/runners/converge/run-converge.sh --dry-run --only S01

Options:
  --only LIST         Comma-separated batch ids (F01,S01,E01,...)
  --group NAME        Run all batches in a group (foundation,services,engine,
                      wiring,observability,retirement,demo,tests)
  --continue RUN      Continue a prior run id, or 'last'
  --dry-run           Show what would run (preview prompts)
  --force             Re-run even successful batches
  --verify-only       Skip Codex, only run verify gates
  --list              Show batch manifest
  --model MODEL       Override model (default: gpt-5.5)
  --reasoning LEVEL   Override reasoning (default: high)
  --timeout SECONDS   Per-batch timeout (default: 5400)
  --retries N         Automatic retries per batch (default: 2)
  --base-ref REF      Base git ref for a new worktree (default: HEAD)
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --only) IFS=',' read -r -a SELECTED_BATCHES <<< "$2"; shift 2 ;;
    --only=*) IFS=',' read -r -a SELECTED_BATCHES <<< "${1#*=}"; shift ;;
    --group) GROUP_FILTER="$2"; shift 2 ;;
    --group=*) GROUP_FILTER="${1#*=}"; shift ;;
    --continue) CONTINUE_RUN="$2"; shift 2 ;;
    --continue=*) CONTINUE_RUN="${1#*=}"; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    --force) FORCE=1; shift ;;
    --verify-only) VERIFY_ONLY=1; shift ;;
    --list) LIST_ONLY=1; shift ;;
    --model) CONV_MODEL="$2"; shift 2 ;;
    --model=*) CONV_MODEL="${1#*=}"; shift ;;
    --reasoning) CONV_REASONING="$2"; shift 2 ;;
    --reasoning=*) CONV_REASONING="${1#*=}"; shift ;;
    --timeout) CONV_TIMEOUT="$2"; shift 2 ;;
    --timeout=*) CONV_TIMEOUT="${1#*=}"; shift ;;
    --retries) CONV_MAX_RETRIES="$2"; shift 2 ;;
    --retries=*) CONV_MAX_RETRIES="${1#*=}"; shift ;;
    --base-ref) CONV_BASE_REF="$2"; shift 2 ;;
    --base-ref=*) CONV_BASE_REF="${1#*=}"; shift ;;
    -h|--help) print_usage; exit 0 ;;
    *) log_err "cli" "Unknown argument: $1"; print_usage; exit 1 ;;
  esac
done

select_batches() {
  if [[ -n "$GROUP_FILTER" ]]; then
    batches_for_group "$GROUP_FILTER"
    return
  fi

  if [[ ${#SELECTED_BATCHES[@]} -eq 0 ]]; then
    printf '%s\n' "${ALL_BATCHES[@]}"
    return
  fi

  local wanted=()
  local raw candidate found
  for raw in "${SELECTED_BATCHES[@]}"; do
    found=0
    for candidate in "${ALL_BATCHES[@]}"; do
      if [[ "$candidate" == "$raw" ]]; then
        wanted+=("$candidate")
        found=1
        break
      fi
    done
    if (( found == 0 )); then
      log_err "cli" "Unknown batch: $raw"
      exit 1
    fi
  done

  for candidate in "${ALL_BATCHES[@]}"; do
    for raw in "${wanted[@]}"; do
      if [[ "$candidate" == "$raw" ]]; then
        echo "$candidate"
      fi
    done
  done
}

list_batches() {
  printf '%s%-5s %-55s %-12s %s%s\n' "$C_BOLD" "ID" "TITLE" "GROUP" "DEPS" "$C_RESET"
  local batch
  for batch in "${ALL_BATCHES[@]}"; do
    printf '%-5s %-55s %-12s %s\n' \
      "$batch" \
      "$(batch_title "$batch")" \
      "$(batch_group "$batch")" \
      "$(batch_deps "$batch")"
  done
}

create_run() {
  RUN_ID="run-$(date +%Y%m%d-%H%M%S)"
  WORKTREE="$WORKTREE_ROOT/converge-$RUN_ID"
  BRANCH="codex/converge-$RUN_ID"

  ensure_dir "$LOG_ROOT/$RUN_ID"
  ensure_dir "$(run_prompts_dir "$RUN_ID")"
  : > "$(run_status_file "$RUN_ID")"
  ln -sfn "$LOG_ROOT/$RUN_ID" "$LOG_ROOT/latest"
  git -C "$ROKO_ROOT" worktree add -b "$BRANCH" "$WORKTREE" "$CONV_BASE_REF" >/dev/null

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
}

create_dry_run() {
  RUN_ID="dry-run-$(date +%Y%m%d-%H%M%S)"
  WORKTREE="$WORKTREE_ROOT/converge-$RUN_ID"
  BRANCH="(not-created)"
  ensure_dir "$LOG_ROOT/$RUN_ID"
  ensure_dir "$(run_prompts_dir "$RUN_ID")"
  : > "$(run_status_file "$RUN_ID")"
  ln -sfn "$LOG_ROOT/$RUN_ID" "$LOG_ROOT/latest"
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
}

load_run() {
  if [[ "$CONTINUE_RUN" == "last" ]]; then
    CONTINUE_RUN="$(latest_run_id || true)"
  fi
  if [[ -z "$CONTINUE_RUN" ]]; then
    log_err "cli" "No prior run available to continue"
    exit 1
  fi
  local manifest
  manifest="$(run_manifest_file "$CONTINUE_RUN")"
  if [[ ! -f "$manifest" ]]; then
    log_err "cli" "Missing manifest for run: $CONTINUE_RUN"
    exit 1
  fi
  # shellcheck disable=SC1090
  source "$manifest"
  RUN_ID="$CONTINUE_RUN"
  if [[ ! -d "$WORKTREE" ]]; then
    log_err "cli" "Worktree missing for run $RUN_ID: $WORKTREE"
    exit 1
  fi
  ln -sfn "$LOG_ROOT/$RUN_ID" "$LOG_ROOT/latest"
}

batch_status() {
  local batch="$1"
  local result_file
  result_file="$(run_result_file "$RUN_ID" "$batch")"
  [[ -f "$result_file" ]] && cat "$result_file" || true
}

deps_satisfied() {
  local dep status
  local -a deps=()
  IFS=' ' read -r -a deps <<< "$(batch_deps "$1")"
  for dep in "${deps[@]}"; do
    [[ -z "$dep" ]] && continue
    status="$(batch_status "$dep")"
    success_status "$status" || return 1
  done
  return 0
}

deps_terminal_failure() {
  local dep status
  local -a deps=()
  IFS=' ' read -r -a deps <<< "$(batch_deps "$1")"
  for dep in "${deps[@]}"; do
    [[ -z "$dep" ]] && continue
    status="$(batch_status "$dep")"
    terminal_failure_status "$status" && return 0
  done
  return 1
}

resume_preserved_batch() {
  local batch="$1"
  local current_batch result
  [[ -n "$CONTINUE_RUN" ]] || return 1
  current_batch="$(current_batch_name "$RUN_ID" 2>/dev/null || true)"
  [[ "$current_batch" == "$batch" ]] || return 1
  result="$(batch_status "$batch")"
  [[ -z "$result" ]] || return 1
  worktree_dirty "$WORKTREE"
}

run_one_batch() {
  local batch="$1"
  local result_file log_file failure_file
  result_file="$(run_result_file "$RUN_ID" "$batch")"
  log_file="$(run_log_file "$RUN_ID" "$batch")"
  failure_file="$(run_failure_file "$RUN_ID" "$batch")"

  local existing
  existing="$(batch_status "$batch")"
  if [[ -n "$existing" ]] && success_status "$existing" && (( FORCE == 0 )); then
    log_info "$batch" "Already successful; skipping"
    return 0
  fi

  if (( DRY_RUN == 1 )); then
    compose_prompt_snapshot "$batch" "$RUN_ID" "dry-run" "$failure_file" >/dev/null
    record_status "$RUN_ID" "$batch" "dry-run" "dry_run" "batch preview only"
    log_info "$batch" "[DRY RUN] $(batch_title "$batch")"
    log_info "$batch" "Prompt: $(batch_prompt_file "$batch")"
    log_info "$batch" "Verify commands:"
    batch_verify_commands "$batch" | sed 's/^/  /'
    log_info "$batch" "Structural checks:"
    batch_structural_checks "$batch" | sed 's/^/  /'
    log_info "$batch" "Anti-pattern checks:"
    batch_antipattern_checks "$batch" | sed 's/^/  /'
    echo "dry_run" > "$result_file"
    return 0
  fi

  if (( VERIFY_ONLY == 1 )); then
    if verify_batch "$batch" "$RUN_ID" "$WORKTREE"; then
      echo "verified" > "$result_file"
      return 0
    fi
    echo "verify_failed" > "$result_file"
    return 1
  fi

  local attempt spawn_rc commit_rc
  local preserve_dirty_resume=0
  local preserve_failed_state=0
  local start_attempt=1

  if resume_preserved_batch "$batch"; then
    preserve_dirty_resume=1
    start_attempt="$(current_batch_attempt "$RUN_ID" 2>/dev/null || echo 1)"
    log_warn "$batch" "Resuming interrupted batch from preserved dirty worktree state"
  fi

  : > "$failure_file"
  for attempt in $(seq "$start_attempt" "$CONV_MAX_RETRIES"); do
    set_current_batch "$RUN_ID" "$batch" "$attempt"
    log_header "$batch ATTEMPT $attempt/$CONV_MAX_RETRIES"
    if (( preserve_dirty_resume == 1 && attempt == start_attempt )); then
      record_status "$RUN_ID" "$batch" "$attempt" "attempt_resumed" "reusing dirty worktree state from interrupted run"
      log_info "$batch" "Keeping current worktree changes; skipping reset before restart"
    elif (( preserve_failed_state == 1 && attempt == start_attempt + 1 )); then
      record_status "$RUN_ID" "$batch" "$attempt" "attempt_preserved_retry" "reusing dirty worktree state from failed prior attempt"
      log_info "$batch" "Keeping failed-attempt changes for retry; skipping reset"
    else
      if worktree_dirty "$WORKTREE"; then
        backup_worktree_state "$RUN_ID" "$batch" "$attempt" "$WORKTREE" "pre-reset"
        record_status "$RUN_ID" "$batch" "$attempt" "attempt_backed_up" "saved dirty worktree before reset"
      fi
      record_status "$RUN_ID" "$batch" "$attempt" "attempt_started" "$(batch_title "$batch")"
      reset_runner_worktree "$WORKTREE"
    fi

    spawn_rc=0
    if spawn_batch "$batch" "$RUN_ID" "$WORKTREE" "$attempt" "$failure_file"; then
      if verify_batch "$batch" "$RUN_ID" "$WORKTREE" "$attempt"; then
        commit_rc=0
        if commit_batch_if_needed "$batch" "$WORKTREE" "$RUN_ID" "$attempt"; then
          echo "success" > "$result_file"
          record_status "$RUN_ID" "$batch" "$attempt" "attempt_succeeded" "verified and committed"
        else
          commit_rc=$?
          if [[ "$commit_rc" -eq 10 ]]; then
            echo "success_noop" > "$result_file"
            record_status "$RUN_ID" "$batch" "$attempt" "attempt_succeeded" "verified with no new changes"
          else
            echo "verify_failed" > "$result_file"
            record_status "$RUN_ID" "$batch" "$attempt" "attempt_failed" "commit step failed"
            write_failure_summary "$batch" "$RUN_ID" "Commit step failed."
          fi
        fi
        clear_current_batch "$RUN_ID"
        return 0
      fi
      echo "verify_failed" > "$result_file"
      record_status "$RUN_ID" "$batch" "$attempt" "attempt_failed" "verification failed"
      if (( attempt < CONV_MAX_RETRIES )) && worktree_dirty "$WORKTREE"; then
        backup_worktree_state "$RUN_ID" "$batch" "$attempt" "$WORKTREE" "verify-failed"
        record_status "$RUN_ID" "$batch" "$attempt" "attempt_backed_up" "saved failed verify state for retry"
        preserve_failed_state=1
      fi
    else
      spawn_rc=$?
      if [[ "$spawn_rc" -eq 124 ]]; then
        echo "timeout" > "$result_file"
        record_status "$RUN_ID" "$batch" "$attempt" "attempt_failed" "codex timed out"
        write_failure_summary "$batch" "$RUN_ID" "Codex timed out."
      else
        echo "spawn_failed" > "$result_file"
        record_status "$RUN_ID" "$batch" "$attempt" "attempt_failed" "codex exited unsuccessfully"
        write_failure_summary "$batch" "$RUN_ID" "Codex exited unsuccessfully."
      fi
      if (( attempt < CONV_MAX_RETRIES )) && worktree_dirty "$WORKTREE"; then
        backup_worktree_state "$RUN_ID" "$batch" "$attempt" "$WORKTREE" "spawn-failed"
        record_status "$RUN_ID" "$batch" "$attempt" "attempt_backed_up" "saved failed spawn state for retry"
        preserve_failed_state=1
      fi
    fi

    if (( attempt < CONV_MAX_RETRIES )); then
      if (( preserve_failed_state == 1 )); then
        log_warn "$batch" "Retrying with preserved failed-attempt changes"
      else
        log_warn "$batch" "Retrying after failure"
      fi
    fi
  done

  clear_current_batch "$RUN_ID"

  return 1
}

print_summary() {
  local batch result success=0 fail=0 other=0
  log_header "RUN SUMMARY ($RUN_ID)"
  for batch in "${SELECTED[@]}"; do
    result="$(batch_status "$batch")"
    result="${result:-pending}"
    printf '  %-5s %-16s %s\n' "$batch" "$result" "$(batch_title "$batch")"
    if success_status "$result"; then
      success=$((success + 1))
    elif terminal_failure_status "$result"; then
      fail=$((fail + 1))
    else
      other=$((other + 1))
    fi
  done
  printf '\n'
  printf '  success=%d  failed=%d  other=%d\n' "$success" "$fail" "$other"
  printf '  run_id=%s\n' "$RUN_ID"
  printf '  worktree=%s\n' "$WORKTREE"
  printf '  branch=%s\n' "$BRANCH"
  printf '  logs=%s\n' "$LOG_ROOT/$RUN_ID"
}

# ---------- main ----------

preflight_check

if (( LIST_ONLY == 1 )); then
  list_batches
  exit 0
fi

mapfile -t SELECTED < <(select_batches)

if [[ -n "$CONTINUE_RUN" ]]; then
  load_run
  log_info "runner" "Continuing run $RUN_ID in $WORKTREE"
else
  if (( DRY_RUN == 1 )); then
    create_dry_run
    log_info "runner" "Created dry-run manifest $RUN_ID"
  else
    create_run
    log_info "runner" "Created run $RUN_ID in $WORKTREE"
  fi
fi

log_info "runner" "Model: $CONV_MODEL (reasoning: $CONV_REASONING)"
log_info "runner" "Selected batches: $(printf '%s,' "${SELECTED[@]}" | sed 's/,$//')"

batch_failed=0
for batch in "${SELECTED[@]}"; do
  if deps_satisfied "$batch"; then
    run_one_batch "$batch" || batch_failed=1
  elif deps_terminal_failure "$batch"; then
    log_warn "$batch" "Blocked by failed dependency: $(batch_deps "$batch")"
    echo "blocked" > "$(run_result_file "$RUN_ID" "$batch")"
  else
    log_warn "$batch" "Skipping; dependencies not yet satisfied: $(batch_deps "$batch")"
    echo "blocked" > "$(run_result_file "$RUN_ID" "$batch")"
  fi
done

print_summary
exit "$batch_failed"
