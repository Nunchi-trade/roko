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
    A1|A2)
      cat <<'EOF'
- explorer: mirage/serve dashboard data paths and API surface
- worker: frontend-facing dashboard state or route wiring in the owned scope
- worker: tests and verification fixes for touched backend/cli paths
EOF
      ;;
    B1|B2)
      cat <<'EOF'
- explorer: demo manifest/scenario/runtime wiring and missing contracts
- worker: contracts and forge tests
- worker: `crates/roko-demo` scenario/runtime/event streaming implementation
EOF
      ;;
    C1|C2)
      cat <<'EOF'
- explorer: `roko-agent-server` and existing serve/mirage integration seams
- worker: new crate or backend aggregator implementation
- worker: migration cleanup, muxing, and compile/test follow-through
EOF
      ;;
    D1)
      cat <<'EOF'
- explorer: current gaps in attestation/lineage/tiering/VCG/router paths
- worker: `crates/roko-core` + `crates/roko-chain` completion
- worker: `crates/roko-neuro` + `crates/roko-daimon` completion
- worker: `crates/roko-agent` provider/tool-loop/runtime wiring
EOF
      ;;
    E1)
      cat <<'EOF'
- explorer: live routing/replanning/prompt composition call graph
- worker: `crates/roko-learn` feedback signals and storage wiring
- worker: `crates/roko-conductor` + `crates/roko-compose` prompt/routing integration
- worker: `crates/roko-orchestrator` + `crates/roko-cli` replanning/runtime wiring
EOF
      ;;
    D2)
      cat <<'EOF'
- explorer: orchestrator/runtime/dreams ownership seams
- worker: `crates/roko-orchestrator` DAG/executor changes
- worker: `crates/roko-agent` + `crates/roko-runtime` supervision/runtime changes
- worker: `crates/roko-dreams` middle-layer dream capabilities
EOF
      ;;
    D3)
      cat <<'EOF'
- explorer: long-horizon learning, daemon, and safety seams already present
- worker: `crates/roko-neuro` + `crates/roko-learn` learning/pheromone work
- worker: `crates/roko-daimon` + `crates/roko-dreams` heartbeat/deployment work
- worker: `crates/roko-compose` + `crates/roko-cli` prompt/daemon integration
EOF
      ;;
    F1)
      cat <<'EOF'
- explorer: current TUI navigation/view gaps and missing serve routes
- worker: `crates/roko-cli/src/tui` interactive views and data loading
- worker: `crates/roko-serve/src/routes` endpoint additions and tests
EOF
      ;;
    F2)
      cat <<'EOF'
- explorer: daemon/PRD/MCP/playbook wiring seams
- worker: `crates/roko-cli` daemon, tracing, and promote-hook work
- worker: `crates/roko-mcp-*` / `crates/roko-index` code-intelligence surface
- worker: `crates/roko-learn` playbook/cost aggregation integration
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
