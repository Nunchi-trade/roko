#!/usr/bin/env bash
# dag.sh — dependency-aware parallel scheduler
#
# The DAG scheduler finds batches whose dependencies are all satisfied,
# dispatches up to N in parallel on separate worktrees, merges completed
# batches back to the run branch, and repeats until done.
#
# Key design:
#   - Each concurrent batch gets its own sub-worktree (forked from run branch HEAD)
#   - After codex completes + AP checks pass, the sub-worktree is merged back
#   - Merges are serialized via flock to prevent race conditions
#   - Cumulative context is snapshotted after each merge
#   - Cargo checks are deferred to wave gates (not per-batch)
#   - Disk space is checked before each dispatch wave

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
source "$(dirname "${BASH_SOURCE[0]}")/spawn.sh"

# ── Worktree helpers ──

# Sub-worktree for a single batch execution (forked from main run branch)
batch_worktree_path() {
  local run_id="$1" batch="$2"
  echo "$WORKTREE_ROOT/${RUNNER_NAME}-${run_id}-${batch}"
}

create_batch_worktree() {
  local run_id="$1" batch="$2" main_branch="$3"
  local wt_path branch_name
  wt_path="$(batch_worktree_path "$run_id" "$batch")"
  branch_name="codex/${RUNNER_NAME}-${run_id}-${batch}"

  if [[ -d "$wt_path" ]]; then
    # Reuse existing (retry scenario) — reset to current main branch state
    git -C "$wt_path" fetch origin >/dev/null 2>&1 || true
    git -C "$wt_path" reset --hard "$main_branch" >/dev/null 2>&1 || true
  else
    git -C "$ROKO_ROOT" worktree add -b "$branch_name" "$wt_path" "$main_branch" >/dev/null 2>&1 || {
      # Branch might already exist from a prior attempt — remove stale and retry
      git -C "$ROKO_ROOT" branch -D "$branch_name" >/dev/null 2>&1 || true
      git -C "$ROKO_ROOT" worktree add -b "$branch_name" "$wt_path" "$main_branch" >/dev/null 2>&1
    }
  fi
  echo "$wt_path"
}

remove_batch_worktree() {
  local run_id="$1" batch="$2"
  local wt_path branch_name
  wt_path="$(batch_worktree_path "$run_id" "$batch")"
  branch_name="codex/${RUNNER_NAME}-${run_id}-${batch}"
  if [[ -d "$wt_path" ]]; then
    git -C "$ROKO_ROOT" worktree remove --force "$wt_path" 2>/dev/null || rm -rf "$wt_path"
  fi
  # Never delete branches — preserve for inspection and manual merge
  # git -C "$ROKO_ROOT" branch -D "$branch_name" 2>/dev/null || true
}

# ── Anti-pattern checks (fast, no cargo) ──

run_ap_checks() {
  (( ${SKIP_AP_CHECKS:-0} )) && return 0
  local batch="$1" worktree="$2" run_id="$3"
  local scope_files failed=0
  scope_files="$(batch_scope "$batch")"

  for f in $scope_files; do
    local fp="$worktree/$f"
    [[ -f "$fp" ]] || continue

    # AP-1: Stubs that silently pass
    if grep -qn 'Verdict::pass.*stub\|Verdict::pass.*always\|Verdict::pass.*noop\|Verdict::pass.*todo\|Verdict::pass.*placeholder' "$fp" 2>/dev/null; then
      log_err "$batch" "AP-1: silent-pass stub in $f"; failed=1
    fi

    # AP-2: block_on in async
    if grep -qn 'futures::executor::block_on\|futures::executor::LocalPool' "$fp" 2>/dev/null; then
      log_err "$batch" "AP-2: block_on in $f"; failed=1
    fi

    # AP-3: Duplicate traits (vs foundation.rs)
    if [[ "$f" != *"foundation.rs" ]]; then
      local traits
      traits="$(grep -oP 'pub trait \K\w+' "$fp" 2>/dev/null || true)"
      for t in $traits; do
        if grep -q "pub trait $t" "$worktree/crates/roko-core/src/foundation.rs" 2>/dev/null; then
          log_err "$batch" "AP-3: dup trait $t in $f"; failed=1
        fi
      done
    fi

    # AP-5: Shell out to CLI
    if grep -qn 'Command::new.*"claude"\|Command::new.*"codex"' "$fp" 2>/dev/null; then
      log_err "$batch" "AP-5: shell out to CLI in $f"; failed=1
    fi

    # AP-6: Inline prompt strings
    if grep -qn 'format!.*"You are a\|format!.*"You are an' "$fp" 2>/dev/null; then
      log_err "$batch" "AP-6: inline prompt in $f"; failed=1
    fi

    # AP-7: std::sync::Mutex + .await
    if grep -q 'std::sync::Mutex' "$fp" 2>/dev/null; then
      if awk '/fn /{fl=0;fa=0} /\.lock\(\)/{fl=1} /\.await/{fa=1} fl&&fa{exit 1}' "$fp" 2>/dev/null; then
        : # ok
      else
        log_err "$batch" "AP-7: std Mutex + await in $f"; failed=1
      fi
    fi
  done

  return "$failed"
}

# ── Portable lock (mkdir-based, works on macOS and Linux) ──

acquire_lock() {
  local lock_dir="$1" timeout="${2:-120}"
  local waited=0
  while ! mkdir "$lock_dir" 2>/dev/null; do
    if (( waited >= timeout )); then
      return 1
    fi
    sleep 1
    waited=$((waited + 1))
  done
  # Store PID for stale-lock detection
  echo $$ > "$lock_dir/pid"
  return 0
}

release_lock() {
  local lock_dir="$1"
  rm -rf "$lock_dir"
}

# ── Merge batch back to main run branch (serialized via lock) ──

merge_batch_to_main() {
  local batch="$1" run_id="$2" main_worktree="$3"
  local batch_wt
  batch_wt="$(batch_worktree_path "$run_id" "$batch")"
  local batch_branch="codex/${RUNNER_NAME}-${run_id}-${batch}"
  local lock_dir
  lock_dir="$(run_merge_lock "$run_id").d"

  # Commit in the batch worktree first (outside lock — no contention)
  git -C "$batch_wt" add -A
  if git -C "$batch_wt" diff --cached --quiet 2>/dev/null; then
    log_warn "$batch" "No changes to merge"
    return 10  # noop
  fi

  local title
  title="$(batch_title "$batch")"
  git -C "$batch_wt" commit -m "$(printf '%s(%s): %s' "$RUNNER_NAME" "$batch" "$title")" >/dev/null 2>&1

  # Serialize merges with mkdir lock to prevent concurrent git operations
  acquire_lock "$lock_dir" 120 || {
    log_err "$batch" "Failed to acquire merge lock"
    return 1
  }

  local merge_rc=0
  {
    git -C "$main_worktree" merge --no-edit "$batch_branch" >/dev/null 2>&1 || {
      log_err "$batch" "Merge conflict — aborting merge"
      git -C "$main_worktree" merge --abort 2>/dev/null || true
      merge_rc=1
    }

    if (( merge_rc == 0 )); then
      local hash
      hash="$(git -C "$main_worktree" rev-parse --short HEAD)"
      log_ok "$batch" "Merged $hash"

      # Snapshot cumulative context while holding the lock (reads main worktree)
      snapshot_cumulative_context "$run_id" "$batch" "$main_worktree"
    fi
  }

  release_lock "$lock_dir"
  return "$merge_rc"
}

# ── Wave gate (cargo check + clippy, once per group) ──

run_wave_gate() {
  local wave="$1" run_id="$2" worktree="$3"
  local target_dir
  target_dir="$(run_target_dir "$run_id")"
  local gate_log="$LOG_ROOT/$run_id/gate-${wave}.log"

  log_header "WAVE GATE: $wave"

  log_info "gate" "cargo check --workspace"
  local output exit_code=0
  output="$(cd "$worktree" && env CARGO_TARGET_DIR="$target_dir" \
    cargo check --workspace 2>&1)" || exit_code=$?
  echo "$output" > "$gate_log"

  if (( exit_code != 0 )); then
    log_err "gate" "Workspace check failed after wave $wave"
    echo "$output" | grep '^error' | head -5 >&2
    return 1
  fi
  log_ok "gate" "Workspace compiles"

  log_info "gate" "cargo clippy --workspace"
  exit_code=0
  output="$(cd "$worktree" && env CARGO_TARGET_DIR="$target_dir" \
    cargo clippy --workspace --no-deps -- -D warnings 2>&1)" || exit_code=$?
  echo "$output" >> "$gate_log"

  if (( exit_code != 0 )); then
    log_err "gate" "Clippy failed after wave $wave"
    echo "$output" | grep '^error' | head -5 >&2
    return 1
  fi
  log_ok "gate" "Clippy clean"

  record_status "$run_id" "gate:$wave" "1" "gate_ok" "check+clippy"
  return 0
}

# ── End-of-run test gate ──

run_test_gate() {
  local run_id="$1" worktree="$2"
  local target_dir
  target_dir="$(run_target_dir "$run_id")"

  log_header "TEST GATE"
  log_info "test" "cargo test --workspace"

  local output exit_code=0
  output="$(cd "$worktree" && env CARGO_TARGET_DIR="$target_dir" \
    cargo test --workspace 2>&1)" || exit_code=$?
  echo "$output" > "$LOG_ROOT/$run_id/gate-test.log"

  if (( exit_code != 0 )); then
    log_err "test" "Tests failed"
    echo "$output" | grep -E '^test .* FAILED|^failures:' | head -20 >&2
    return 1
  fi

  local count
  count="$(echo "$output" | grep -oP '\d+ passed' | tail -1)"
  log_ok "test" "All tests passed ($count)"
  return 0
}

# ── DAG scheduler ──

# Returns batch IDs that are ready to run (all deps satisfied, not yet done)
dag_ready_batches() {
  local run_id="$1"
  shift
  local -a candidates=("$@")

  for batch in "${candidates[@]}"; do
    # Skip already completed/failed
    local result_file
    result_file="$(run_result_file "$run_id" "$batch")"
    if [[ -f "$result_file" ]]; then
      local status; status="$(cat "$result_file")"
      success_status "$status" && continue
      terminal_failure "$status" && continue
      [[ "$status" == "in_progress" ]] && continue
    fi

    # Check deps
    local -a deps=()
    IFS=' ' read -r -a deps <<< "$(batch_deps "$batch")"
    local all_met=1
    for dep in "${deps[@]}"; do
      [[ -z "$dep" ]] && continue
      local dep_file
      dep_file="$(run_result_file "$run_id" "$dep")"
      if [[ ! -f "$dep_file" ]] || ! success_status "$(cat "$dep_file")"; then
        all_met=0; break
      fi
    done

    (( all_met )) && echo "$batch"
  done
}

# Check if any dep has terminally failed (so this batch is blocked forever)
dag_blocked_batches() {
  local run_id="$1"
  shift
  local -a candidates=("$@")

  for batch in "${candidates[@]}"; do
    local result_file
    result_file="$(run_result_file "$run_id" "$batch")"
    [[ -f "$result_file" ]] && continue  # already has a result

    local -a deps=()
    IFS=' ' read -r -a deps <<< "$(batch_deps "$batch")"
    for dep in "${deps[@]}"; do
      [[ -z "$dep" ]] && continue
      local dep_file
      dep_file="$(run_result_file "$run_id" "$dep")"
      if [[ -f "$dep_file" ]] && terminal_failure "$(cat "$dep_file")"; then
        echo "$batch"; break
      fi
    done
  done
}

# Count batches with a given status category
dag_count() {
  local run_id="$1" category="$2"
  shift 2
  local -a batches=("$@")
  local count=0
  for batch in "${batches[@]}"; do
    local rf; rf="$(run_result_file "$run_id" "$batch")"
    [[ -f "$rf" ]] || continue
    local s; s="$(cat "$rf")"
    case "$category" in
      success) success_status "$s" && count=$((count+1)) ;;
      failed)  terminal_failure "$s" && count=$((count+1)) ;;
    esac
  done
  echo "$count"
}

# Check if all batches in a group are completed (for gate triggering)
dag_group_complete() {
  local run_id="$1" group="$2"
  shift 2
  local -a batches=("$@")
  for batch in "${batches[@]}"; do
    local grp
    grp="$(batch_group "$batch")"
    [[ "$grp" == "$group" ]] || continue
    local rf
    rf="$(run_result_file "$run_id" "$batch")"
    if [[ ! -f "$rf" ]]; then
      return 1  # pending
    fi
    local s
    s="$(cat "$rf")"
    if [[ "$s" == "in_progress" ]]; then
      return 1  # still running
    fi
    # success or failed — either way, this batch is done
  done
  return 0  # all batches in this group have a terminal result
}
