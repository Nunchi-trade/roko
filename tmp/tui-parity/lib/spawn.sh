#!/usr/bin/env bash

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

: "${UX_MODEL:=gpt-5.4}"
: "${UX_REASONING:=high}"
: "${UX_TIMEOUT:=5400}"

emit_delegation_guidance() {
  local batch="$1"
  cat <<EOF
## Delegation Requirement

You are explicitly authorized to use multiple subagents for this batch.
Use them aggressively where it helps, but keep the immediate blocking work local.

Required delegation behavior:

- Before coding, form a short plan and identify 2-4 concrete sidecar subtasks.
- Spawn explorers for targeted codebase questions and workers for bounded code edits.
- Each subagent gets the same context pack plus its specific task.
- Give each worker a disjoint write scope and tell them they are not alone in the codebase.
- Do not wait idly for subagents if you can make progress locally.
- If subagents are unavailable in this environment, continue locally without failing.

Suggested parallel split for batch \`$batch\`:
EOF

  case "$batch" in
    T1)
      cat <<'EOF'
- explorer: read StateHub, DashboardSnapshot, EventBus APIs and usage in orchestrate.rs
- worker: add watch::Receiver to App + new_connected() constructor + main_loop streaming
- worker: add update_from_dashboard_snapshot() to TuiState + tests
EOF
      ;;
    T2)
      cat <<'EOF'
- explorer: read Mori agent_output.rs SegmentKind parsing patterns
- worker: create segment.rs with SegmentKind enum + parser + CachedRender
- worker: integrate segment rendering into agents_view.rs right panel
EOF
      ;;
    T3)
      cat <<'EOF'
- explorer: read approval flow in Mori (ApprovalRequested event, pending state)
- worker: create approval_ipc.rs with channels + integrate into App event loop
- worker: wire PlanRunner to send approval requests when approval_required
EOF
      ;;
    T4)
      cat <<'EOF'
- explorer: read ProcessSupervisor in roko-runtime, sysinfo crate API
- worker: add ProcessMetrics struct + sys metrics collection thread
- worker: render Procs sub-tab as table with sparklines
EOF
      ;;
    T5)
      cat <<'EOF'
- explorer: read parallel_pool.rs, wave_progress.rs, AgentRow/Wave structs
- worker: bridge parallel_pool to AgentRow data + add context gauge
- worker: populate execution_waves + wire wave ribbon + nav keys
EOF
      ;;
    T6)
      cat <<'EOF'
- explorer: read efficiency events format, cascade router state
- worker: add RouteMetrics struct + populate from snapshot
- worker: render metrics bar above agent output + model/tier columns
EOF
      ;;
    T7)
      cat <<'EOF'
- explorer: find all dead fields/structs (agents_by_id, token_burn_history, etc.)
- worker: remove dead fields from state.rs + update all callers
- worker: clean up dead code in views/* and widgets/*
EOF
      ;;
    T8)
      cat <<'EOF'
- explorer: read Mori nerv_viz.rs patterns + existing postfx pipeline
- worker: implement nerv_viz effect with braille visualization
- worker: implement particle_system + EffectsConfig persistence + v key cycling
EOF
      ;;
    *)
      cat <<'EOF'
- explorer: targeted read-only architecture questions
- worker: first bounded implementation slice
- worker: second bounded implementation slice
EOF
      ;;
  esac

  echo
}

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
  ensure_dir "$(dirname "$out")"

  {
    echo "# TUI Parity Batch $batch"
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
    emit_delegation_guidance "$batch"
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
  local target_dir
  target_dir=$(batch_target_dir "$run_id" "$batch" "codex" "$attempt")
  : > "$last_message_file"
  rm -rf "$target_dir"
  mkdir -p "$target_dir"

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
    echo "=== Cargo target: $target_dir ==="
    echo "=== Prompt snapshot: $prompt_snapshot ==="
    echo
  } > "$log_file"

  record_status "$run_id" "$batch" "$attempt" "spawn_started" "codex exec started"

  do_timeout "$UX_TIMEOUT" \
    env CARGO_TARGET_DIR="$target_dir" \
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
    record_status "$run_id" "$batch" "$attempt" "spawn_succeeded" "codex exec completed"
    log_ok "$batch" "Codex completed in $(fmt_duration "$elapsed")"
    return 0
  fi

  if [[ "$exit_code" -eq 124 ]]; then
    record_status "$run_id" "$batch" "$attempt" "spawn_timed_out" "codex exec timed out"
    log_err "$batch" "Timed out after $(fmt_duration "$UX_TIMEOUT")"
    return 124
  fi

  record_status "$run_id" "$batch" "$attempt" "spawn_failed" "codex exec exited with $exit_code"
  log_err "$batch" "Codex exited with code $exit_code"
  return "$exit_code"
}
