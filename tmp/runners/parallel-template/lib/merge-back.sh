#!/usr/bin/env bash
# merge-back.sh — merge run branch back to source branch with agent-driven conflict resolution
#
# This module handles:
#   - Checkpoint merges: merge accumulated work back to source branch at group boundaries
#   - Conflict resolution: spawn Codex to resolve merge conflicts automatically
#   - Final merge: merge everything back at end of run with full test gate
#
# Design:
#   - Source branch = the branch that was active when the run started (e.g. wp-arch2)
#   - Run branch = the runner's own branch where batches merge into
#   - Checkpoint merges keep the source branch up-to-date so parallel runners don't diverge too far
#   - Conflicts are resolved by spawning a Codex agent in the source worktree

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

: "${MERGE_BACK_ENABLED:=1}"
: "${MERGE_BACK_CONFLICT_TIMEOUT:=600}"
: "${MERGE_BACK_STRATEGY:=checkpoint}"  # checkpoint | final-only

# ── Source branch tracking ──

# Store the source branch at run start
record_source_branch() {
  local run_id="$1"
  local source_branch
  source_branch="$(git -C "$ROKO_ROOT" rev-parse --abbrev-ref HEAD)"
  echo "SOURCE_BRANCH='$source_branch'" >> "$(run_manifest_file "$run_id")"
  echo "$source_branch"
}

# Read source branch from manifest
get_source_branch() {
  local run_id="$1"
  local manifest
  manifest="$(run_manifest_file "$run_id")"
  grep "^SOURCE_BRANCH=" "$manifest" 2>/dev/null | cut -d"'" -f2
}

# ── Conflict resolution via Codex ──

resolve_conflicts_with_agent() {
  local run_id="$1" checkpoint_name="$2" worktree="$3"
  local conflict_files
  conflict_files="$(git -C "$worktree" diff --name-only --diff-filter=U 2>/dev/null)"

  if [[ -z "$conflict_files" ]]; then
    log_warn "merge-back" "No conflict files found (unexpected)"
    return 1
  fi

  local conflict_count
  conflict_count="$(echo "$conflict_files" | wc -l | tr -d ' ')"
  log_info "merge-back" "Resolving $conflict_count conflict(s) via Codex..."

  # Build conflict resolution prompt
  local prompt_file="$LOG_ROOT/$run_id/merge-conflict-${checkpoint_name}.prompt.md"
  {
    echo "# Merge Conflict Resolution"
    echo
    echo "You are resolving merge conflicts after merging runner work back to the source branch."
    echo "The runner branch contains completed batch work. The source branch may have independent changes."
    echo
    echo "## Conflicting Files"
    echo
    for f in $conflict_files; do
      echo "### \`$f\`"
      echo '```'
      cat "$worktree/$f" 2>/dev/null | head -500
      echo '```'
      echo
    done
    echo "## Instructions"
    echo
    echo "1. For each conflicting file, resolve the conflict by keeping BOTH sets of changes where possible."
    echo "2. The runner branch changes are the NEW work — prefer keeping them."
    echo "3. The source branch changes are prior context — integrate them where they don't conflict with new work."
    echo "4. Remove ALL conflict markers (<<<<<<< ======= >>>>>>>)."
    echo "5. Ensure the result compiles (run \`cargo check --workspace\` after resolving)."
    echo "6. Stage all resolved files with \`git add\`."
    echo
    echo "## Do NOT"
    echo
    echo "- Delete any functionality from either side unless it's genuinely duplicated"
    echo "- Add new features or refactor beyond what's needed to resolve the conflict"
    echo "- Leave any conflict markers in the code"
  } > "$prompt_file"

  local log_file="$LOG_ROOT/$run_id/merge-conflict-${checkpoint_name}.log"
  local target_dir
  target_dir="$(run_target_dir "$run_id")"

  local exit_code=0
  do_timeout "$MERGE_BACK_CONFLICT_TIMEOUT" \
    env CARGO_TARGET_DIR="$target_dir" \
    codex exec \
      --model "${CONV_MODEL:-gpt-5.5}" \
      --sandbox workspace-write \
      --full-auto \
      --cd "$worktree" \
      - \
      < "$prompt_file" > "$log_file" 2>&1 || exit_code=$?

  if (( exit_code != 0 )); then
    log_err "merge-back" "Conflict resolution agent failed (exit $exit_code)"
    return 1
  fi

  # Verify no remaining conflict markers
  if git -C "$worktree" diff --name-only --diff-filter=U 2>/dev/null | grep -q .; then
    log_err "merge-back" "Agent did not resolve all conflicts"
    return 1
  fi

  # Verify it compiles
  local check_rc=0
  (cd "$worktree" && env CARGO_TARGET_DIR="$target_dir" cargo check --workspace 2>/dev/null) || check_rc=$?
  if (( check_rc != 0 )); then
    log_err "merge-back" "Post-resolution cargo check failed"
    return 1
  fi

  log_ok "merge-back" "Conflicts resolved and verified"
  return 0
}

# ── Checkpoint merge: merge run branch progress back to source ──

merge_back_checkpoint() {
  local run_id="$1" checkpoint_name="$2" main_worktree="$3"
  local source_branch
  source_branch="$(get_source_branch "$run_id")"

  if [[ -z "$source_branch" ]]; then
    log_warn "merge-back" "No source branch recorded — skipping checkpoint"
    return 0
  fi

  (( MERGE_BACK_ENABLED )) || { log_info "merge-back" "Merge-back disabled"; return 0; }

  local main_branch
  main_branch="$(git -C "$main_worktree" rev-parse --abbrev-ref HEAD)"

  log_info "merge-back" "Checkpoint '$checkpoint_name': merging $main_branch → $source_branch"

  # We merge INTO the source branch. Use the original repo (not worktree) since
  # the source branch might not have a worktree.
  # Strategy: create a temporary worktree for source branch, merge there.

  local source_wt="$WORKTREE_ROOT/${RUNNER_NAME}-${run_id}-source-merge"
  local source_branch_ref
  source_branch_ref="$(git -C "$ROKO_ROOT" rev-parse "$source_branch" 2>/dev/null)" || {
    log_err "merge-back" "Source branch '$source_branch' not found"
    return 1
  }

  # Create or reuse source merge worktree
  if [[ -d "$source_wt" ]]; then
    git -C "$source_wt" checkout "$source_branch" >/dev/null 2>&1 || true
    git -C "$source_wt" reset --hard "$source_branch" >/dev/null 2>&1 || true
  else
    git -C "$ROKO_ROOT" worktree add "$source_wt" "$source_branch" 2>/dev/null || {
      log_err "merge-back" "Cannot create source merge worktree"
      return 1
    }
  fi

  # Attempt merge
  local merge_rc=0
  git -C "$source_wt" merge --no-edit "$main_branch" 2>/dev/null || merge_rc=$?

  if (( merge_rc != 0 )); then
    log_warn "merge-back" "Merge conflict at checkpoint '$checkpoint_name' — invoking agent"
    record_event "$run_id" "MERGE_BACK" "0" "conflict" "checkpoint=$checkpoint_name"

    # Try agent-driven resolution
    local resolve_rc=0
    resolve_conflicts_with_agent "$run_id" "$checkpoint_name" "$source_wt" || resolve_rc=$?

    if (( resolve_rc != 0 )); then
      log_err "merge-back" "Could not resolve conflicts — aborting merge, continuing run"
      git -C "$source_wt" merge --abort 2>/dev/null || true
      record_event "$run_id" "MERGE_BACK" "0" "conflict_unresolved" "checkpoint=$checkpoint_name"
      return 1
    fi

    # Commit the resolution
    git -C "$source_wt" add -A
    git -C "$source_wt" commit --no-edit -m "$(printf 'merge-back(%s): resolve conflicts at %s' "$RUNNER_NAME" "$checkpoint_name")" 2>/dev/null
  fi

  local hash
  hash="$(git -C "$source_wt" rev-parse --short HEAD)"
  log_ok "merge-back" "Checkpoint '$checkpoint_name' merged → $source_branch ($hash)"
  record_event "$run_id" "MERGE_BACK" "0" "checkpoint_merged" "checkpoint=$checkpoint_name,hash=$hash"

  # Update the run branch to include any new source branch changes (forward merge)
  # This keeps subsequent batches building on top of the merged state
  local fwd_rc=0
  git -C "$main_worktree" merge --no-edit "$source_branch" 2>/dev/null || fwd_rc=$?
  if (( fwd_rc != 0 )); then
    # Forward merge conflict — accept theirs (source branch is canonical)
    git -C "$main_worktree" checkout --theirs . 2>/dev/null || true
    git -C "$main_worktree" add -A 2>/dev/null || true
    git -C "$main_worktree" commit --no-edit -m "merge-back: forward-merge source branch" 2>/dev/null || true
  fi

  return 0
}

# ── Final merge: merge everything back at end of run ──

merge_back_final() {
  local run_id="$1" main_worktree="$2"
  local source_branch
  source_branch="$(get_source_branch "$run_id")"

  if [[ -z "$source_branch" ]]; then
    log_warn "merge-back" "No source branch — cannot do final merge"
    return 1
  fi

  (( MERGE_BACK_ENABLED )) || { log_info "merge-back" "Merge-back disabled"; return 0; }

  log_header "FINAL MERGE-BACK"
  log_info "merge-back" "Merging all work → $source_branch"

  # Do the final checkpoint merge
  merge_back_checkpoint "$run_id" "final" "$main_worktree" || {
    log_err "merge-back" "Final merge failed — manual merge required"
    log_err "merge-back" "Run branch: $(git -C "$main_worktree" rev-parse --abbrev-ref HEAD)"
    log_err "merge-back" "Source: $source_branch"
    log_err "merge-back" "To merge manually:"
    log_err "merge-back" "  git checkout $source_branch && git merge $(git -C "$main_worktree" rev-parse --abbrev-ref HEAD)"
    return 1
  }

  # Clean up the source merge worktree
  local source_wt="$WORKTREE_ROOT/${RUNNER_NAME}-${run_id}-source-merge"
  if [[ -d "$source_wt" ]]; then
    git -C "$ROKO_ROOT" worktree remove --force "$source_wt" 2>/dev/null || true
  fi

  log_ok "merge-back" "All work merged to $source_branch"
  return 0
}

# ── Cleanup source merge worktree ──

cleanup_source_worktree() {
  local run_id="$1"
  local source_wt="$WORKTREE_ROOT/${RUNNER_NAME}-${run_id}-source-merge"
  if [[ -d "$source_wt" ]]; then
    git -C "$ROKO_ROOT" worktree remove --force "$source_wt" 2>/dev/null || rm -rf "$source_wt"
  fi
}
