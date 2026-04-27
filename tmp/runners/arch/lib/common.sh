#!/usr/bin/env bash

set -uo pipefail

: "${ROKO_ROOT:=/Users/will/dev/nunchi/roko/roko}"
: "${ARCH_ROOT:=$ROKO_ROOT/tmp/runners/arch}"
: "${LOG_ROOT:=$ARCH_ROOT/logs}"
: "${PROMPTS_DIR:=$ARCH_ROOT/prompts}"
: "${CONTEXT_DIR:=$ARCH_ROOT/context-pack}"
: "${WORKTREE_ROOT:=$ROKO_ROOT/.roko/worktrees}"

if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
  C_RESET=$'\e[0m'
  C_BOLD=$'\e[1m'
  C_DIM=$'\e[2m'
  C_RED=$'\e[31m'
  C_GREEN=$'\e[32m'
  C_YELLOW=$'\e[33m'
  C_BLUE=$'\e[34m'
  C_MAGENTA=$'\e[35m'
  C_CYAN=$'\e[36m'
else
  C_RESET='' C_BOLD='' C_DIM='' C_RED='' C_GREEN='' C_YELLOW='' C_BLUE='' C_MAGENTA='' C_CYAN=''
fi

log_info()   { printf '%s[INFO]%s  %s%-10s%s %s\n' "$C_BLUE"   "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2"; }
log_ok()     { printf '%s[OK]%s    %s%-10s%s %s\n' "$C_GREEN"  "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2"; }
log_warn()   { printf '%s[WARN]%s  %s%-10s%s %s\n' "$C_YELLOW" "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2" >&2; }
log_err()    { printf '%s[ERR]%s   %s%-10s%s %s\n' "$C_RED"    "$C_RESET" "$C_DIM" "$1" "$C_RESET" "$2" >&2; }
log_header() { printf '\n%s=== %s ===%s\n\n' "$C_BOLD$C_MAGENTA" "$1" "$C_RESET"; }

# ---------- batch registry (dependency order) ----------

ALL_BATCHES=(
  "P0A"
  "P0B"
  "P0C"
  "P1A"
  "P1B"
  "P1C"
  "P1D"
  "P2A"
  "P2B"
  "P2C"
  "P2D"
  "P3A"
  "P3B"
  "P3C"
  "P4A"
  "P4B"
)

# ---------- batch groups (for --group filter) ----------

GROUPS_PHASE0=( "P0A" "P0B" "P0C" )
GROUPS_PHASE1=( "P1A" "P1B" "P1C" "P1D" )
GROUPS_PHASE2=( "P2A" "P2B" "P2C" "P2D" )
GROUPS_PHASE3=( "P3A" "P3B" "P3C" )
GROUPS_PHASE4=( "P4A" "P4B" )

batches_for_group() {
  case "$1" in
    phase0) printf '%s\n' "${GROUPS_PHASE0[@]}" ;;
    phase1) printf '%s\n' "${GROUPS_PHASE1[@]}" ;;
    phase2) printf '%s\n' "${GROUPS_PHASE2[@]}" ;;
    phase3) printf '%s\n' "${GROUPS_PHASE3[@]}" ;;
    phase4) printf '%s\n' "${GROUPS_PHASE4[@]}" ;;
    *) log_err "group" "Unknown group: $1"; return 1 ;;
  esac
}

# ---------- status helpers ----------

success_status() {
  case "${1:-}" in
    success|success_noop|verified|skipped) return 0 ;;
    *) return 1 ;;
  esac
}

terminal_failure_status() {
  case "${1:-}" in
    spawn_failed|verify_failed|timeout|blocked) return 0 ;;
    *) return 1 ;;
  esac
}

fmt_duration() {
  local s="${1:-0}"
  local h=$((s / 3600))
  local m=$(((s % 3600) / 60))
  local sec=$((s % 60))
  if (( h > 0 )); then
    printf '%dh %dm %ds' "$h" "$m" "$sec"
  elif (( m > 0 )); then
    printf '%dm %ds' "$m" "$sec"
  else
    printf '%ds' "$sec"
  fi
}

# ---------- file helpers ----------

ensure_dir() { mkdir -p "$1"; }

require_file() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    log_err "bootstrap" "Missing file: $path"
    exit 1
  fi
}

latest_run_id() {
  [[ -d "$LOG_ROOT" ]] || return 1
  find "$LOG_ROOT" -maxdepth 2 -name manifest.env -exec dirname {} \; \
    | xargs -n 1 basename \
    | sort \
    | tail -1
}

worktree_dirty() {
  local worktree="$1"
  git -C "$worktree" status --porcelain=v1 -uall \
    | grep -Ev '^[ MADRCU?!]{2} (\.cargo-target/|target/)' \
    | grep -q .
}

# ---------- run file paths ----------

run_manifest_file()     { echo "$LOG_ROOT/$1/manifest.env"; }
run_result_file()       { echo "$LOG_ROOT/$1/$2.result"; }
run_log_file()          { echo "$LOG_ROOT/$1/$2.log"; }
run_prompts_dir()       { echo "$LOG_ROOT/$1/prompts"; }
run_prompt_snapshot()   { echo "$(run_prompts_dir "$1")/$2.prompt.md"; }
run_last_message_file() { echo "$LOG_ROOT/$1/$2.last.txt"; }
run_failure_file()      { echo "$LOG_ROOT/$1/$2.failure.txt"; }
run_status_file()       { echo "$LOG_ROOT/$1/status.tsv"; }
run_current_batch_file(){ echo "$LOG_ROOT/$1/current-batch.env"; }
run_backups_dir()       { echo "$LOG_ROOT/$1/backups"; }
batch_prompt_file()     { echo "$PROMPTS_DIR/$1.prompt.md"; }
tmp_target_root()       { echo "${TMPDIR:-/tmp}/roko-arch-targets"; }
batch_target_dir()      { echo "$(tmp_target_root)/$1/$2/$3-attempt-$4"; }

# ---------- current batch tracking ----------

current_batch_value() {
  local run_id="$1" key="$2" file
  file="$(run_current_batch_file "$run_id")"
  [[ -f "$file" ]] || return 1
  awk -F= -v key="$key" '$1 == key { gsub(/\047/, "", $2); print $2 }' "$file"
}
current_batch_name()    { current_batch_value "$1" "BATCH"; }
current_batch_attempt() { current_batch_value "$1" "ATTEMPT"; }

record_status() {
  local run_id="$1" batch="$2" attempt="$3" status="$4" note="${5:-}"
  printf '%s\t%s\t%s\t%s\t%s\n' \
    "$(date -Iseconds)" "$batch" "$attempt" "$status" "$note" \
    >> "$(run_status_file "$run_id")"
}

set_current_batch() {
  local run_id="$1" batch="$2" attempt="$3"
  cat > "$(run_current_batch_file "$run_id")" <<EOF
BATCH='$batch'
ATTEMPT='$attempt'
UPDATED_AT='$(date -Iseconds)'
EOF
}

clear_current_batch() { rm -f "$(run_current_batch_file "$1")"; }

# ---------- batch metadata ----------

batch_title() {
  case "$1" in
    P0A) echo "RuntimeEvent types" ;;
    P0B) echo "Foundation traits (6 traits)" ;;
    P0C) echo "EventBus RuntimeEvent support" ;;
    P1A) echo "ModelCallService" ;;
    P1B) echo "PromptAssemblyService" ;;
    P1C) echo "FeedbackService" ;;
    P1D) echo "GateService" ;;
    P2A) echo "PipelineState v2 (config-driven)" ;;
    P2B) echo "TaskScheduler (pure DAG)" ;;
    P2C) echo "EffectDriver" ;;
    P2D) echo "WorkflowEngine facade" ;;
    P3A) echo "AcpAdapter (EventConsumer → ACP)" ;;
    P3B) echo "SseAdapter + REST panel endpoints" ;;
    P3C) echo "JsonlLogger + RuntimeProjection" ;;
    P4A) echo "Wire CLI entry points" ;;
    P4B) echo "Wire ACP entry points" ;;
    *) return 1 ;;
  esac
}

batch_deps() {
  case "$1" in
    P0B) echo "P0A" ;;
    P0C) echo "P0A" ;;
    P1A) echo "P0A P0B" ;;
    P1B) echo "P0A P0B" ;;
    P1C) echo "P0A P0B" ;;
    P1D) echo "P0A P0B" ;;
    P2A) echo "P0A" ;;
    P2B) echo "" ;;
    P2C) echo "P1A P1B P1C P1D P2A P2B" ;;
    P2D) echo "P2C" ;;
    P3A) echo "P0B P0C" ;;
    P3B) echo "P0B P0C" ;;
    P3C) echo "P0B P0C" ;;
    P4A) echo "P2D" ;;
    P4B) echo "P2D P3A" ;;
    *) echo "" ;;
  esac
}

batch_group() {
  case "$1" in
    P0*) echo "phase0" ;;
    P1*) echo "phase1" ;;
    P2*) echo "phase2" ;;
    P3*) echo "phase3" ;;
    P4*) echo "phase4" ;;
    *) echo "misc" ;;
  esac
}

batch_verify_commands() {
  case "$1" in
    P0A)
      cat <<'EOF'
cargo check -p roko-core
EOF
      ;;
    P0B)
      cat <<'EOF'
cargo check -p roko-core
EOF
      ;;
    P0C)
      cat <<'EOF'
cargo check -p roko-runtime
EOF
      ;;
    P1A)
      cat <<'EOF'
cargo check -p roko-agent
EOF
      ;;
    P1B)
      cat <<'EOF'
cargo check -p roko-compose
EOF
      ;;
    P1C)
      cat <<'EOF'
cargo check -p roko-learn
EOF
      ;;
    P1D)
      cat <<'EOF'
cargo check -p roko-gate
EOF
      ;;
    P2A)
      cat <<'EOF'
cargo check -p roko-runtime
cargo test -p roko-runtime --lib -- pipeline_state
EOF
      ;;
    P2B)
      cat <<'EOF'
cargo check -p roko-runtime
cargo test -p roko-runtime --lib -- task_scheduler
EOF
      ;;
    P2C)
      cat <<'EOF'
cargo check -p roko-runtime
EOF
      ;;
    P2D)
      cat <<'EOF'
cargo check -p roko-runtime
EOF
      ;;
    P3A)
      cat <<'EOF'
cargo check -p roko-acp
EOF
      ;;
    P3B)
      cat <<'EOF'
cargo check -p roko-serve
EOF
      ;;
    P3C)
      cat <<'EOF'
cargo check -p roko-runtime
EOF
      ;;
    P4A)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    P4B)
      cat <<'EOF'
cargo check -p roko-acp
EOF
      ;;
    *)
      return 1
      ;;
  esac
}

# ---------- anti-pattern verification ----------

batch_antipattern_checks() {
  case "$1" in
    P1A)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-agent/src/model_call_service.rs
! grep -rn 'format!.*You are' crates/roko-agent/src/model_call_service.rs
EOF
      ;;
    P1B)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-compose/src/prompt_assembly_service.rs
! grep -rn 'format!.*You are.*the' crates/roko-compose/src/prompt_assembly_service.rs
EOF
      ;;
    P2C)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-runtime/src/effect_driver.rs
! grep -rn 'if.*phase.*==' crates/roko-runtime/src/effect_driver.rs
EOF
      ;;
    P2D)
      cat <<'EOF'
! grep -rn 'Command::new' crates/roko-runtime/src/workflow_engine.rs
EOF
      ;;
    P4A)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-cli/src/run.rs
EOF
      ;;
    P4B)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-acp/src/bridge_events.rs
! grep -rn 'Command::new.*claude' crates/roko-acp/src/runner.rs
EOF
      ;;
    *) ;;
  esac
}

batch_structural_checks() {
  case "$1" in
    P0A)
      cat <<'EOF'
grep -q 'pub enum RuntimeEvent' crates/roko-core/src/runtime_event.rs
grep -q 'pub mod runtime_event' crates/roko-core/src/lib.rs
EOF
      ;;
    P0B)
      cat <<'EOF'
grep -q 'pub trait ModelCaller' crates/roko-core/src/foundation.rs
grep -q 'pub trait PromptAssembler' crates/roko-core/src/foundation.rs
grep -q 'pub trait FeedbackSink' crates/roko-core/src/foundation.rs
grep -q 'pub trait GateRunner' crates/roko-core/src/foundation.rs
grep -q 'pub trait EventConsumer' crates/roko-core/src/foundation.rs
grep -q 'pub trait EffectExecutor' crates/roko-core/src/foundation.rs
grep -q 'pub mod foundation' crates/roko-core/src/lib.rs
EOF
      ;;
    P0C)
      cat <<'EOF'
grep -q 'RuntimeEvent' crates/roko-runtime/src/event_bus.rs
EOF
      ;;
    P1A)
      cat <<'EOF'
grep -q 'pub struct ModelCallService' crates/roko-agent/src/model_call_service.rs
grep -q 'pub mod model_call_service' crates/roko-agent/src/lib.rs
EOF
      ;;
    P1B)
      cat <<'EOF'
grep -q 'pub struct PromptAssemblyService' crates/roko-compose/src/prompt_assembly_service.rs
grep -q 'pub mod prompt_assembly_service' crates/roko-compose/src/lib.rs
EOF
      ;;
    P1C)
      cat <<'EOF'
grep -q 'pub struct FeedbackService' crates/roko-learn/src/feedback_service.rs
grep -q 'pub mod feedback_service' crates/roko-learn/src/lib.rs
EOF
      ;;
    P1D)
      cat <<'EOF'
grep -q 'pub struct GateService' crates/roko-gate/src/gate_service.rs
grep -q 'pub mod gate_service' crates/roko-gate/src/lib.rs
EOF
      ;;
    P2A)
      cat <<'EOF'
grep -q 'pub struct PipelineStateV2' crates/roko-runtime/src/pipeline_state.rs
grep -q 'pub mod pipeline_state' crates/roko-runtime/src/lib.rs
EOF
      ;;
    P2B)
      cat <<'EOF'
grep -q 'pub struct TaskScheduler' crates/roko-runtime/src/task_scheduler.rs
grep -q 'pub mod task_scheduler' crates/roko-runtime/src/lib.rs
EOF
      ;;
    P2C)
      cat <<'EOF'
grep -q 'pub struct EffectDriver' crates/roko-runtime/src/effect_driver.rs
grep -q 'pub mod effect_driver' crates/roko-runtime/src/lib.rs
EOF
      ;;
    P2D)
      cat <<'EOF'
grep -q 'pub struct WorkflowEngine' crates/roko-runtime/src/workflow_engine.rs
grep -q 'pub mod workflow_engine' crates/roko-runtime/src/lib.rs
EOF
      ;;
    P3A)
      cat <<'EOF'
grep -q 'pub struct AcpAdapter' crates/roko-acp/src/acp_adapter.rs
grep -q 'pub mod acp_adapter' crates/roko-acp/src/lib.rs
EOF
      ;;
    P3B)
      cat <<'EOF'
grep -q 'pub struct SseAdapter\|pub fn sse_' crates/roko-serve/src/adapters.rs
EOF
      ;;
    P3C)
      cat <<'EOF'
grep -q 'pub struct JsonlLogger' crates/roko-runtime/src/jsonl_logger.rs
grep -q 'pub struct RuntimeProjection' crates/roko-runtime/src/projection.rs
EOF
      ;;
    P4A)
      cat <<'EOF'
grep -q 'WorkflowEngine\|workflow_engine' crates/roko-cli/src/run.rs
EOF
      ;;
    P4B)
      cat <<'EOF'
grep -q 'WorkflowEngine\|workflow_engine' crates/roko-acp/src/runner.rs
EOF
      ;;
    *) ;;
  esac
}

# ---------- preflight ----------

preflight_check() {
  local errors=0
  log_header "PREFLIGHT"

  if command -v codex >/dev/null 2>&1; then
    log_ok "preflight" "codex CLI: $(command -v codex)"
  else
    log_err "preflight" "codex CLI not found"
    errors=$((errors + 1))
  fi

  if git -C "$ROKO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    log_ok "preflight" "git repo detected"
  else
    log_err "preflight" "ROKO_ROOT is not a git repo: $ROKO_ROOT"
    errors=$((errors + 1))
  fi

  ensure_dir "$LOG_ROOT"
  ensure_dir "$WORKTREE_ROOT"

  require_file "$ARCH_ROOT/BATCHES.md"

  local batch
  for batch in "${ALL_BATCHES[@]}"; do
    require_file "$(batch_prompt_file "$batch")"
  done

  local ctx
  for ctx in 00-RULES.md 01-ARCHITECTURE.md 02-EXISTING-CODE.md 03-ANTI-PATTERNS.md; do
    require_file "$CONTEXT_DIR/$ctx"
  done

  local dirty_count
  dirty_count=$(git -C "$ROKO_ROOT" status --porcelain | wc -l | tr -d ' ')
  if (( dirty_count > 0 )); then
    log_warn "preflight" "main repo has $dirty_count uncommitted change(s); worktree starts from committed HEAD only"
  else
    log_ok "preflight" "main repo is clean"
  fi

  return "$errors"
}
