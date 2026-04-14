#!/usr/bin/env bash

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

: "${UX_MODEL:=gpt-5.4}"
: "${UX_REASONING:=high}"
: "${UX_TIMEOUT:=5400}"

do_timeout() {
  local seconds="$1"
  shift
  if command -v timeout >/dev/null 2>&1; then
    timeout "$seconds" "$@"
  elif command -v gtimeout >/dev/null 2>&1; then
    gtimeout "$seconds" "$@"
  else
    "$@"
  fi
}

compose_prompt_snapshot() {
  local batch="$1"
  local run_id="$2"
  local attempt="$3"
  local failure_file="$4"
  local out
  out="$(run_prompt_snapshot "$run_id" "$batch")"

  {
    echo "# UX Refactoring Batch $batch"
    echo
    echo "Run id: $run_id"
    echo "Attempt: $attempt"
    echo "Model: $UX_MODEL"
    echo "Reasoning: $UX_REASONING"
    echo
    if [[ -s "$failure_file" ]]; then
      echo "## Previous attempt failure context"
      echo
      cat "$failure_file"
      echo
      echo "Use that context to avoid repeating the same failure."
      echo
    fi
    cat "$(batch_prompt_file "$batch")"
  } > "$out"

  echo "$out"
}

spawn_batch() {
  local batch="$1"
  local run_id="$2"
  local worktree="$3"
  local attempt="$4"
  local failure_file="$5"

  local prompt_snapshot
  prompt_snapshot=$(compose_prompt_snapshot "$batch" "$run_id" "$attempt" "$failure_file")
  local log_file
  log_file=$(run_log_file "$run_id" "$batch")
  local last_message_file
  last_message_file=$(run_last_message_file "$run_id" "$batch")

  local start_ts
  start_ts=$(date +%s)
  local exit_code=0

  {
    echo "=== Batch: $batch ($(batch_title "$batch")) ==="
    echo "=== Started: $(date -Iseconds) ==="
    echo "=== Worktree: $worktree ==="
    echo "=== Model: $UX_MODEL ==="
    echo "=== Reasoning: $UX_REASONING ==="
    echo "=== Timeout: $UX_TIMEOUT ==="
    echo "=== Prompt snapshot: $prompt_snapshot ==="
    echo
  } > "$log_file"

  do_timeout "$UX_TIMEOUT" \
    env CARGO_TARGET_DIR="$worktree/.cargo-target" \
    codex exec \
      --model "$UX_MODEL" \
      --sandbox workspace-write \
      --full-auto \
      -c "model_reasoning_effort=$UX_REASONING" \
      --cd "$worktree" \
      -o "$last_message_file" \
      - \
      < "$prompt_snapshot" >> "$log_file" 2>&1 || exit_code=$?

  local end_ts
  end_ts=$(date +%s)
  local elapsed=$((end_ts - start_ts))

  {
    echo
    echo "=== Finished: $(date -Iseconds) ==="
    echo "=== Duration: $(fmt_duration "$elapsed") ==="
    echo "=== Exit code: $exit_code ==="
  } >> "$log_file"

  if [[ "$exit_code" -eq 0 ]]; then
    log_ok "$batch" "Codex completed in $(fmt_duration "$elapsed")"
    return 0
  fi

  if [[ "$exit_code" -eq 124 ]]; then
    log_err "$batch" "Timed out after $(fmt_duration "$UX_TIMEOUT")"
    return 124
  fi

  log_err "$batch" "Codex exited with code $exit_code"
  return "$exit_code"
}
