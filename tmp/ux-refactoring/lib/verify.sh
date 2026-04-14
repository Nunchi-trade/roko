#!/usr/bin/env bash

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

write_failure_summary() {
  local batch="$1"
  local run_id="$2"
  local note="$3"
  local log_file
  log_file=$(run_log_file "$run_id" "$batch")
  local failure_file
  failure_file=$(run_failure_file "$run_id" "$batch")

  {
    echo "$note"
    echo
    echo "Recent log tail:"
    tail -40 "$log_file" 2>/dev/null || true
  } > "$failure_file"
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
  local log_file
  log_file=$(run_log_file "$run_id" "$batch")

  while IFS= read -r cmd; do
    [[ -z "$cmd" ]] && continue
    echo "[verify] $cmd" >> "$log_file"
    if ! (
      cd "$worktree" &&
      env CARGO_TARGET_DIR="$worktree/.cargo-target" bash -lc "$cmd"
    ) >> "$log_file" 2>&1; then
      log_err "$batch" "Verify failed: $cmd"
      write_failure_summary "$batch" "$run_id" "Verification failed for command: $cmd"
      return 1
    fi
  done < <(batch_verify_commands "$batch")

  log_ok "$batch" "Verification passed"
  return 0
}

commit_batch_if_needed() {
  local batch="$1"
  local worktree="$2"
  local title
  title=$(batch_title "$batch")

  git -C "$worktree" add -A
  if git -C "$worktree" diff --cached --quiet; then
    log_warn "$batch" "No changes staged after successful verification"
    return 10
  fi

  git -C "$worktree" commit -m "$(cat <<EOF
ux-refactoring(${batch}): ${title}

Automated implementation via tmp/ux-refactoring/run-ux-refactoring.sh
EOF
)" >/dev/null
  log_ok "$batch" "Committed: $(git -C "$worktree" log --oneline -1)"
  return 0
}
