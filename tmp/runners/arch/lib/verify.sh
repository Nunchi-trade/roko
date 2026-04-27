#!/usr/bin/env bash

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

write_failure_summary() {
  local batch="$1"
  local run_id="$2"
  local note="$3"
  local log_file failure_file
  log_file=$(run_log_file "$run_id" "$batch")
  failure_file=$(run_failure_file "$run_id" "$batch")

  {
    echo "$note"
    echo
    echo "Recent log tail:"
    tail -40 "$log_file" 2>/dev/null || true
  } > "$failure_file"
}

backup_worktree_state() {
  local run_id="$1" batch="$2" attempt="$3" worktree="$4" label="$5"
  local backup_dir prefix
  backup_dir="$(run_backups_dir "$run_id")"
  prefix="$backup_dir/${batch}-attempt-${attempt}-${label}"
  ensure_dir "$backup_dir"

  git -C "$worktree" status --short -- . ':(exclude).cargo-target' ':(exclude)target' \
    > "${prefix}.status"
  git -C "$worktree" diff -- . ':(exclude).cargo-target' ':(exclude)target' \
    > "${prefix}.patch"
  {
    echo "run_id=$run_id"
    echo "batch=$batch"
    echo "attempt=$attempt"
    echo "label=$label"
    echo "captured_at=$(date -Iseconds)"
    echo "worktree=$worktree"
  } > "${prefix}.meta"
}

reset_runner_worktree() {
  local worktree="$1"
  git -C "$worktree" reset --hard HEAD >/dev/null 2>&1 || true
  git -C "$worktree" clean -fd >/dev/null 2>&1 || true
}

verify_batch() {
  local batch="$1"
  local run_id="$2"
  local worktree="$3"
  local attempt="${4:-?}"
  local log_file
  log_file=$(run_log_file "$run_id" "$batch")
  local target_dir
  target_dir=$(batch_target_dir "$run_id" "$batch" "verify" "$attempt")
  rm -rf "$target_dir"
  mkdir -p "$target_dir"

  # Level 1: Structural checks (grep for expected symbols)
  local structural_failed=0
  while IFS= read -r cmd; do
    [[ -z "$cmd" ]] && continue
    record_status "$run_id" "$batch" "$attempt" "structural_check" "$cmd"
    echo "[structural] $cmd" >> "$log_file"
    if ! ( cd "$worktree" && eval "$cmd" ) >> "$log_file" 2>&1; then
      record_status "$run_id" "$batch" "$attempt" "structural_failed" "$cmd"
      log_err "$batch" "Structural check failed: $cmd"
      structural_failed=1
    fi
  done < <(batch_structural_checks "$batch")

  if (( structural_failed )); then
    write_failure_summary "$batch" "$run_id" "Structural verification failed — expected symbols not found."
    return 1
  fi

  # Level 2: Compilation checks
  while IFS= read -r cmd; do
    [[ -z "$cmd" ]] && continue
    record_status "$run_id" "$batch" "$attempt" "verify_running" "$cmd"
    echo "[verify] CARGO_TARGET_DIR=$target_dir $cmd" >> "$log_file"
    if ! (
      cd "$worktree" &&
      env CARGO_TARGET_DIR="$target_dir" bash -lc "$cmd"
    ) >> "$log_file" 2>&1; then
      record_status "$run_id" "$batch" "$attempt" "verify_failed" "$cmd"
      log_err "$batch" "Verify failed: $cmd"
      write_failure_summary "$batch" "$run_id" "Verification failed for command: $cmd"
      return 1
    fi
  done < <(batch_verify_commands "$batch")

  # Level 3: Anti-pattern checks (banned patterns must NOT appear)
  local antipattern_failed=0
  while IFS= read -r cmd; do
    [[ -z "$cmd" ]] && continue
    record_status "$run_id" "$batch" "$attempt" "antipattern_check" "$cmd"
    echo "[antipattern] $cmd" >> "$log_file"
    if ! ( cd "$worktree" && eval "$cmd" ) >> "$log_file" 2>&1; then
      record_status "$run_id" "$batch" "$attempt" "antipattern_failed" "$cmd"
      log_err "$batch" "Anti-pattern check failed: $cmd"
      antipattern_failed=1
    fi
  done < <(batch_antipattern_checks "$batch")

  if (( antipattern_failed )); then
    write_failure_summary "$batch" "$run_id" "Anti-pattern violation detected."
    return 1
  fi

  record_status "$run_id" "$batch" "$attempt" "verify_succeeded" "all verification checks passed"
  log_ok "$batch" "Verification passed (structural + compilation + anti-pattern)"
  return 0
}

commit_batch_if_needed() {
  local batch="$1"
  local worktree="$2"
  local run_id="${3:-}"
  local attempt="${4:-?}"
  local title
  title=$(batch_title "$batch")

  # Never stage build artifacts
  rm -rf "$worktree/.cargo-target" "$worktree/target"

  git -C "$worktree" add -A
  if git -C "$worktree" diff --cached --quiet; then
    if [[ -n "$run_id" ]]; then
      record_status "$run_id" "$batch" "$attempt" "commit_noop" "no staged changes after verify"
    fi
    log_warn "$batch" "No changes staged after successful verification"
    return 10
  fi

  git -C "$worktree" commit -m "$(cat <<EOF
arch(${batch}): ${title}

Automated implementation via tmp/runners/arch/run-arch.sh
EOF
)" >/dev/null
  if [[ -n "$run_id" ]]; then
    record_status "$run_id" "$batch" "$attempt" "commit_succeeded" "$(git -C "$worktree" rev-parse --short HEAD)"
  fi
  log_ok "$batch" "Committed: $(git -C "$worktree" log --oneline -1)"
  return 0
}
