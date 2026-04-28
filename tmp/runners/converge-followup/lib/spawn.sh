#!/usr/bin/env bash
# spawn.sh — prompt composition with cumulative context + codex invocation

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

: "${CONV_MODEL:=gpt-5.5}"
: "${CONV_REASONING:=high}"
: "${CONV_TIMEOUT:=5400}"

do_timeout() {
  local seconds="$1"; shift
  if command -v timeout >/dev/null 2>&1; then timeout "$seconds" "$@"
  elif command -v gtimeout >/dev/null 2>&1; then gtimeout "$seconds" "$@"
  else "$@"
  fi
}

# ── Cumulative context ──
#
# After each successful batch, we snapshot which files were changed and their
# contents. The NEXT batch's prompt includes all of this, so Codex knows what
# prior batches actually produced — not what they were supposed to produce.

snapshot_cumulative_context() {
  local run_id="$1" batch="$2" worktree="$3"
  local cum_dir
  cum_dir="$(run_cumulative_dir "$run_id")"
  ensure_dir "$cum_dir"

  # Record which files this batch changed (from last commit)
  git -C "$worktree" diff --name-only HEAD~1 HEAD 2>/dev/null \
    > "$cum_dir/${batch}.files" || true

  # Snapshot the full content of changed files
  local changed_file
  while IFS= read -r changed_file; do
    [[ -z "$changed_file" ]] && continue
    local full_path="$worktree/$changed_file"
    if [[ -f "$full_path" ]]; then
      ensure_dir "$(dirname "$cum_dir/${batch}/${changed_file}")"
      cp "$full_path" "$cum_dir/${batch}/${changed_file}"
    fi
  done < "$cum_dir/${batch}.files"
}

# Build the cumulative context section for the prompt.
# This tells Codex: "here's what previous batches in this run actually changed"
build_cumulative_section() {
  local run_id="$1" current_batch="$2" worktree="$3"
  local cum_dir
  cum_dir="$(run_cumulative_dir "$run_id")"
  [[ -d "$cum_dir" ]] || return 0

  # Collect all files changed by prior batches (deduplicated, latest version)
  local -A seen_files=()
  local prior_batch
  for prior_batch in "${ALL_BATCHES[@]}"; do
    [[ "$prior_batch" == "$current_batch" ]] && break
    local files_list="$cum_dir/${prior_batch}.files"
    [[ -f "$files_list" ]] || continue

    while IFS= read -r f; do
      [[ -z "$f" ]] && continue
      seen_files["$f"]="$prior_batch"
    done < "$files_list"
  done

  if [[ ${#seen_files[@]} -eq 0 ]]; then
    return 0
  fi

  echo "## Files modified by prior batches in this run"
  echo
  echo "These are the ACTUAL current contents after previous batches ran."
  echo "Your changes must be compatible with this code."
  echo

  local file last_batch
  for file in "${!seen_files[@]}"; do
    last_batch="${seen_files[$file]}"
    local full_path="$worktree/$file"
    if [[ -f "$full_path" ]]; then
      local lines
      lines=$(wc -l < "$full_path" | tr -d ' ')
      # For large files (>500 lines), only show the first 100 and last 100
      if (( lines > 500 )); then
        echo "### \`$file\` (last modified by $last_batch, $lines lines — truncated)"
        echo
        echo '```rust'
        head -100 "$full_path"
        echo "// ... ($((lines - 200)) lines omitted) ..."
        tail -100 "$full_path"
        echo '```'
      else
        echo "### \`$file\` (last modified by $last_batch)"
        echo
        echo '```rust'
        cat "$full_path"
        echo '```'
      fi
      echo
    fi
  done
}

# ── Prompt composition ──

compose_prompt() {
  local batch="$1" run_id="$2" attempt="$3" worktree="$4"
  local out
  out="$(run_prompt_snapshot "$run_id" "$batch")"
  ensure_dir "$(dirname "$out")"

  {
    echo "# Batch $batch — $(batch_title "$batch")"
    echo
    echo "Run: $run_id | Attempt: $attempt | Model: $CONV_MODEL"
    echo

    # Context pack (rules, architecture, anti-patterns)
    for ctx_file in "$CONTEXT_DIR"/*.md; do
      [[ -f "$ctx_file" ]] || continue
      echo "---"
      echo
      cat "$ctx_file"
      echo
    done

    # Cumulative context: what prior batches actually produced
    echo "---"
    echo
    build_cumulative_section "$run_id" "$batch" "$worktree"

    # Live file contents for write-scope files
    local scope_files also_read_files
    scope_files="$(batch_scope "$batch")"
    also_read_files="$(batch_also_read "$batch")"
    local all_files="$scope_files $also_read_files"

    if [[ -n "${all_files// /}" ]]; then
      echo "---"
      echo
      echo "## Current file contents (live from worktree)"
      echo
      for f in $all_files; do
        local full_path="$worktree/$f"
        if [[ -f "$full_path" ]]; then
          local lines
          lines=$(wc -l < "$full_path" | tr -d ' ')
          if (( lines > 800 )); then
            echo "### \`$f\` ($lines lines — truncated)"
            echo
            echo '```rust'
            head -200 "$full_path"
            echo "// ... ($((lines - 400)) lines omitted) ..."
            tail -200 "$full_path"
            echo '```'
          else
            echo "### \`$f\`"
            echo
            echo '```rust'
            cat "$full_path"
            echo '```'
          fi
          echo
        else
          echo "### \`$f\` — does not exist yet (create it)"
          echo
        fi
      done
    fi

    # Prior failure context (structured, not raw log tail)
    local failure_file
    failure_file="$(run_failure_file "$run_id" "$batch")"
    if [[ -s "$failure_file" ]]; then
      echo "---"
      echo
      echo "## Previous attempt failed"
      echo
      cat "$failure_file"
      echo
      echo "Fix the issues above. Do not repeat the same mistakes."
      echo
    fi

    # The batch-specific prompt
    echo "---"
    echo
    local prompt_file
    prompt_file="$(batch_prompt_file "$batch")"
    if [[ -f "$prompt_file" ]]; then
      cat "$prompt_file"
    else
      echo "**ERROR: missing prompt file: $prompt_file**"
    fi
  } > "$out"

  echo "$out"
}

# ── Spawn ──

spawn_batch() {
  local batch="$1" run_id="$2" worktree="$3" attempt="$4"

  local prompt_snapshot
  prompt_snapshot="$(compose_prompt "$batch" "$run_id" "$attempt" "$worktree")"

  local log_file
  log_file="$(run_log_file "$run_id" "$batch")"

  # Share a single target dir across the whole run for incremental compilation
  local target_dir
  target_dir="$(run_target_dir "$run_id")"
  mkdir -p "$target_dir"

  {
    echo "=== Batch: $batch ($(batch_title "$batch")) ==="
    echo "=== Started: $(date -Iseconds) ==="
    echo "=== Worktree: $worktree ==="
    echo "=== Model: $CONV_MODEL ==="
    echo "=== Prompt: $prompt_snapshot ==="
    echo
  } > "$log_file"

  record_status "$run_id" "$batch" "$attempt" "spawn_started" "codex exec"
  record_event "$run_id" "$batch" "$attempt" "spawn_started"

  local start_ts exit_code=0
  start_ts=$(date +%s)

  do_timeout "$CONV_TIMEOUT" \
    env CARGO_TARGET_DIR="$target_dir" \
    codex exec \
      --model "$CONV_MODEL" \
      --sandbox workspace-write \
      --full-auto \
      -c "model_reasoning_effort=$CONV_REASONING" \
      --cd "$worktree" \
      - \
      < "$prompt_snapshot" >> "$log_file" 2>&1 || exit_code=$?

  local elapsed=$(( $(date +%s) - start_ts ))

  printf '\n=== Finished: %s ===\n=== Duration: %s ===\n=== Exit: %d ===\n' \
    "$(date -Iseconds)" "$(fmt_duration "$elapsed")" "$exit_code" >> "$log_file"

  if [[ "$exit_code" -eq 0 ]]; then
    record_status "$run_id" "$batch" "$attempt" "spawn_ok" "$(fmt_duration "$elapsed")"
    record_event "$run_id" "$batch" "$attempt" "spawn_ok" "duration=${elapsed}s"
    log_ok "$batch" "Codex completed in $(fmt_duration "$elapsed")"
    return 0
  elif [[ "$exit_code" -eq 124 ]]; then
    record_status "$run_id" "$batch" "$attempt" "spawn_timeout" "$(fmt_duration "$CONV_TIMEOUT")"
    log_err "$batch" "Timed out"
    return 124
  else
    record_status "$run_id" "$batch" "$attempt" "spawn_failed" "exit $exit_code"
    log_err "$batch" "Codex exit $exit_code"
    return "$exit_code"
  fi
}
