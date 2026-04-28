#!/usr/bin/env bash

set -uo pipefail

: "${ROKO_ROOT:=/Users/will/dev/nunchi/roko/roko}"
: "${CONV_ROOT:=$ROKO_ROOT/tmp/runners/converge}"
: "${LOG_ROOT:=$CONV_ROOT/logs}"
: "${PROMPTS_DIR:=$CONV_ROOT/prompts}"
: "${CONTEXT_DIR:=$CONV_ROOT/context-pack}"
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
  # Track F: Foundation Fixes
  "F01" "F02" "F03" "F04" "F05" "F06"
  # Track S: Service Enhancement
  "S01" "S02" "S03" "S04" "S05"
  "S06" "S07" "S08" "S09"
  "S10" "S11"
  "S12" "S13"
  # Track E: Engine Enhancement
  "E01" "E02" "E03" "E04" "E05" "E06" "E07" "E08"
  # Track W: Wiring
  "W01" "W02" "W03" "W04" "W05" "W06" "W07" "W08"
  # Track O: Observability
  "O01" "O02" "O03" "O04" "O05" "O06"
  # Track R: Retirement
  "R01" "R02" "R03" "R04" "R05"
  # Track C: CLI + Demo
  "C01" "C02" "C03" "C04" "C05" "C06"
  "C07" "C08" "C09" "C10" "C11" "C12"
  # Track T: Integration Tests
  "T01" "T02" "T03" "T04" "T05"
  # Track D: Daimon Refactor
  "D01" "D02" "D03" "D04"
  # Track G: Gateway Consolidation
  "G01" "G02" "G03" "G04" "G05" "G06" "G07" "G08" "G09"
  # Track K: Knowledge Feedback Loop
  "K01" "K02" "K03" "K04" "K05"
  # Track X: Security Hardening
  "X01" "X02"
  # Track L: Layering Firewall
  "L01" "L02" "L03" "L04"
)

# ---------- batch groups (for --group filter) ----------

GROUPS_FOUNDATION=( "F01" "F02" "F03" "F04" "F05" "F06" )
GROUPS_SERVICES=( "S01" "S02" "S03" "S04" "S05" "S06" "S07" "S08" "S09" "S10" "S11" "S12" "S13" )
GROUPS_ENGINE=( "E01" "E02" "E03" "E04" "E05" "E06" "E07" "E08" )
GROUPS_WIRING=( "W01" "W02" "W03" "W04" "W05" "W06" "W07" "W08" )
GROUPS_OBSERVABILITY=( "O01" "O02" "O03" "O04" "O05" "O06" )
GROUPS_RETIREMENT=( "R01" "R02" "R03" "R04" "R05" )
GROUPS_DEMO=( "C01" "C02" "C03" "C04" "C05" "C06" "C07" "C08" "C09" "C10" "C11" "C12" )
GROUPS_TESTS=( "T01" "T02" "T03" "T04" "T05" )
GROUPS_DAIMON=( "D01" "D02" "D03" "D04" )
GROUPS_GATEWAY=( "G01" "G02" "G03" "G04" "G05" "G06" "G07" "G08" "G09" )
GROUPS_KNOWLEDGE=( "K01" "K02" "K03" "K04" "K05" )
GROUPS_SECURITY=( "X01" "X02" )
GROUPS_LAYERING=( "L01" "L02" "L03" "L04" )

batches_for_group() {
  case "$1" in
    foundation|F) printf '%s\n' "${GROUPS_FOUNDATION[@]}" ;;
    services|S)   printf '%s\n' "${GROUPS_SERVICES[@]}" ;;
    engine|E)     printf '%s\n' "${GROUPS_ENGINE[@]}" ;;
    wiring|W)     printf '%s\n' "${GROUPS_WIRING[@]}" ;;
    observability|O) printf '%s\n' "${GROUPS_OBSERVABILITY[@]}" ;;
    retirement|R) printf '%s\n' "${GROUPS_RETIREMENT[@]}" ;;
    demo|C)       printf '%s\n' "${GROUPS_DEMO[@]}" ;;
    tests|T)      printf '%s\n' "${GROUPS_TESTS[@]}" ;;
    daimon|D)     printf '%s\n' "${GROUPS_DAIMON[@]}" ;;
    gateway|G)    printf '%s\n' "${GROUPS_GATEWAY[@]}" ;;
    knowledge|K)  printf '%s\n' "${GROUPS_KNOWLEDGE[@]}" ;;
    security|X)   printf '%s\n' "${GROUPS_SECURITY[@]}" ;;
    layering|L)   printf '%s\n' "${GROUPS_LAYERING[@]}" ;;
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
tmp_target_root()       { echo "${TMPDIR:-/tmp}/roko-converge-targets"; }
batch_target_dir()      { echo "$(tmp_target_root)/$1/shared"; }

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
    # Track F: Foundation Fixes
    F01) echo "Invert crate dep: roko-core must not depend on roko-runtime" ;;
    F02) echo "Remove local trait copies from effect_driver, use roko-core" ;;
    F03) echo "Remove local RuntimeEvent from effect_driver, use roko-core" ;;
    F04) echo "Update JsonlLogger to use roko-core EventConsumer + all variants" ;;
    F05) echo "Update RuntimeProjection to parse all RuntimeEvent variants" ;;
    F06) echo "Remove WorkflowEvent/WorkflowEventConsumer, use roko-core" ;;
    # Track S: Service Enhancement
    S01) echo "ModelCallService: wire to existing provider dispatch" ;;
    S02) echo "ModelCallService: add Anthropic API + OpenAI-compat backends" ;;
    S03) echo "ModelCallService: integrate CascadeRouter for model selection" ;;
    S04) echo "ModelCallService: add cost tracking + cost prediction" ;;
    S05) echo "ModelCallService: add MCP config passthrough" ;;
    S06) echo "PromptAssemblyService: wire full 9-layer SystemPromptBuilder" ;;
    S07) echo "PromptAssemblyService: add ContextSource for neuro/knowledge" ;;
    S08) echo "PromptAssemblyService: add ContextSource for episodes + playbooks" ;;
    S09) echo "PromptAssemblyService: add section effectiveness scoring" ;;
    S10) echo "FeedbackService: add episode recording sink" ;;
    S11) echo "FeedbackService: add CascadeRouter bandit observation" ;;
    S12) echo "GateService: add remaining rungs 3-6" ;;
    S13) echo "GateService: add adaptive threshold integration" ;;
    # Track E: Engine Enhancement
    E01) echo "PipelineStateV2: add TOML workflow config loading" ;;
    E02) echo "PipelineStateV2: add checkpoint serialization + resume" ;;
    E03) echo "EffectDriver: wire real agent spawn via ModelCallService" ;;
    E04) echo "EffectDriver: implement commit effect (git add + commit)" ;;
    E05) echo "EffectDriver: implement checkpoint effect" ;;
    E06) echo "WorkflowEngine: add progress callback + event emission" ;;
    E07) echo "WorkflowEngine: add cancellation via CancellationToken" ;;
    E08) echo "WorkflowEngine: add resume from checkpoint" ;;
    # Track W: Wiring
    W01) echo "Add --engine v2/legacy flag to roko run CLI" ;;
    W02) echo "Wire WorkflowEngine as default for roko run" ;;
    W03) echo "Wire WorkflowEngine adapter construction in run.rs" ;;
    W04) echo "Wire WorkflowEngine to roko plan run entry point" ;;
    W05) echo "Wire ACP bridge_events to use WorkflowEngine" ;;
    W06) echo "Wire unified.rs oneshot to use ModelCallService" ;;
    W07) echo "Wire roko-serve background tasks to WorkflowEngine events" ;;
    W08) echo "Wire roko-serve SSE to WorkflowEngine RuntimeEvent stream" ;;
    # Track O: Observability
    O01) echo "Wire JsonlLogger as WorkflowEngine EventConsumer" ;;
    O02) echo "Wire RuntimeProjection to roko-serve dashboard routes" ;;
    O03) echo "Bridge WorkflowEngine events to StateHub (TUI)" ;;
    O04) echo "Create CLI progress printer (Clack-style output)" ;;
    O05) echo "Wire CLI progress printer into cmd_run engine path" ;;
    O06) echo "Wire efficiency event summary to CLI post-run output" ;;
    # Track R: Retirement
    R01) echo "Add legacy-orchestrate feature to roko-cli Cargo.toml" ;;
    R02) echo "Feature-gate orchestrate.rs behind legacy-orchestrate" ;;
    R03) echo "Feature-gate dispatch_helpers + agent_spawn behind legacy" ;;
    R04) echo "Ensure cargo check passes without legacy-orchestrate" ;;
    R05) echo "Ensure cargo check passes with legacy-orchestrate" ;;
    # Track C: CLI + Demo
    C01) echo "Create output_format.rs: Clack-style symbols + ANSI colors" ;;
    C02) echo "Identity line: agent name + model + routing decision" ;;
    C03) echo "Cost prediction line: estimated tokens + cost" ;;
    C04) echo "Knowledge loading line: facts loaded + confidence" ;;
    C05) echo "Cost actual + delta: real vs predicted after execution" ;;
    C06) echo "Gate results: formatted pass/fail with timing" ;;
    C07) echo "--share flag: generate share token + store run data" ;;
    C08) echo "Share endpoint: GET /api/shared/:token in roko-serve" ;;
    C09) echo "Agent list: formatted output for roko agent list" ;;
    C10) echo "Replay: formatted output for roko replay" ;;
    C11) echo "Dashboard SPA: wire knowledge + learning + agents APIs" ;;
    C12) echo "Dashboard SPA: add share page at /share/:token" ;;
    # Track T: Tests
    T01) echo "Integration test: WorkflowEngine express workflow" ;;
    T02) echo "Integration test: WorkflowEngine standard with gate" ;;
    T03) echo "Integration test: WorkflowEngine checkpoint + resume" ;;
    T04) echo "Integration test: CLI --engine v2 flag parses" ;;
    T05) echo "Integration test: share URL generation + retrieval" ;;
    # Track D: Daimon Refactor
    D01) echo "Extract AffectPolicy trait to roko-core foundation" ;;
    D02) echo "Implement DaimonPolicy wrapping DaimonState" ;;
    D03) echo "Wire AffectPolicy into WorkflowEngine + EffectDriver" ;;
    D04) echo "Wire DaimonPolicy into CLI run path" ;;
    # Track G: Gateway Consolidation
    G01) echo "Service contract: unified request/response types" ;;
    G02) echo "Durable gateway event writer + projection" ;;
    G03) echo "ProviderCallCell: move provider execution into cell" ;;
    G04) echo "HTTP gateway adapter: wire routes to ModelCallService" ;;
    G05) echo "CLI runner adapter: ensure EffectDriver uses ModelCallService" ;;
    G06) echo "Domain caller migration (research, dreams, neuro)" ;;
    G07) echo "Cache cell + budget cell in ModelCallService" ;;
    G08) echo "Thinking cap + convergence detection cells" ;;
    G09) echo "Wire force_backend to CascadeRouter learning" ;;
    # Track K: Knowledge Feedback Loop
    K01) echo "Add knowledge-aware routing method to CascadeRouter" ;;
    K02) echo "Wire knowledge query into ModelCallService routing" ;;
    K03) echo "Inject knowledge into prompt assembly" ;;
    K04) echo "Record knowledge usage in episode metadata" ;;
    K05) echo "Knowledge confidence update loop" ;;
    # Track X: Security Hardening
    X01) echo "Fix contract fail-open to fail-closed" ;;
    X02) echo "Consolidate stream JSON parsers" ;;
    # Track L: Layering Firewall
    L01) echo "Add layer metadata to all Cargo.toml files" ;;
    L02) echo "Create layer-check binary" ;;
    L03) echo "Configure cargo-deny" ;;
    L04) echo "Add layer-check to CI workflow" ;;
    *) return 1 ;;
  esac
}

batch_deps() {
  case "$1" in
    # Track F
    F01) echo "" ;;
    F02) echo "F01" ;;
    F03) echo "F02" ;;
    F04) echo "F03" ;;
    F05) echo "F03" ;;
    F06) echo "F03" ;;
    # Track S
    S01) echo "F02" ;;
    S02) echo "S01" ;;
    S03) echo "S01" ;;
    S04) echo "S01" ;;
    S05) echo "S01" ;;
    S06) echo "F02" ;;
    S07) echo "S06" ;;
    S08) echo "S06" ;;
    S09) echo "S06" ;;
    S10) echo "F02" ;;
    S11) echo "S10" ;;
    S12) echo "F02" ;;
    S13) echo "S12" ;;
    # Track E
    E01) echo "F06" ;;
    E02) echo "E01" ;;
    E03) echo "F03 S01" ;;
    E04) echo "F03" ;;
    E05) echo "E02 E04" ;;
    E06) echo "F06 E03" ;;
    E07) echo "E06" ;;
    E08) echo "E02 E06" ;;
    # Track W
    W01) echo "E06" ;;
    W02) echo "W01" ;;
    W03) echo "W02 S01 S06 S10 S12" ;;
    W04) echo "W03" ;;
    W05) echo "E06" ;;
    W06) echo "S01" ;;
    W07) echo "E06" ;;
    W08) echo "W07" ;;
    # Track O
    O01) echo "F04 W02" ;;
    O02) echo "F05" ;;
    O03) echo "W02" ;;
    O04) echo "" ;;
    O05) echo "O04 W02" ;;
    O06) echo "O05 S04" ;;
    # Track R
    R01) echo "" ;;
    R02) echo "R01 W04" ;;
    R03) echo "R02" ;;
    R04) echo "R03" ;;
    R05) echo "R04" ;;
    # Track C
    C01) echo "" ;;
    C02) echo "C01" ;;
    C03) echo "C02 S04" ;;
    C04) echo "C02" ;;
    C05) echo "C03" ;;
    C06) echo "C01" ;;
    C07) echo "W02" ;;
    C08) echo "C07" ;;
    C09) echo "C01" ;;
    C10) echo "C01" ;;
    C11) echo "O02" ;;
    C12) echo "C08" ;;
    # Track T
    T01) echo "E06" ;;
    T02) echo "T01" ;;
    T03) echo "E08" ;;
    T04) echo "W01" ;;
    T05) echo "C08" ;;
    # Track D
    D01) echo "" ;;
    D02) echo "D01" ;;
    D03) echo "D01 E06" ;;
    D04) echo "D02 D03 W02" ;;
    # Track G
    G01) echo "" ;;
    G02) echo "G01" ;;
    G03) echo "G01 S01" ;;
    G04) echo "G03" ;;
    G05) echo "G03 W02" ;;
    G06) echo "G03" ;;
    G07) echo "G03" ;;
    G08) echo "G03" ;;
    G09) echo "G03 S03" ;;
    # Track K
    K01) echo "S03" ;;
    K02) echo "K01 G03" ;;
    K03) echo "S07" ;;
    K04) echo "S10 K01" ;;
    K05) echo "K04" ;;
    # Track X
    X01) echo "" ;;
    X02) echo "" ;;
    # Track L
    L01) echo "F01" ;;
    L02) echo "L01" ;;
    L03) echo "" ;;
    L04) echo "L02" ;;
    *) echo "" ;;
  esac
}

batch_group() {
  case "$1" in
    F*) echo "foundation" ;;
    S*) echo "services" ;;
    E*) echo "engine" ;;
    W*) echo "wiring" ;;
    O*) echo "observability" ;;
    R*) echo "retirement" ;;
    C*) echo "demo" ;;
    T*) echo "tests" ;;
    D*) echo "daimon" ;;
    G*) echo "gateway" ;;
    K*) echo "knowledge" ;;
    X*) echo "security" ;;
    L*) echo "layering" ;;
    *) echo "misc" ;;
  esac
}

batch_verify_commands() {
  case "$1" in
    F01|F02|F03)
      cat <<'EOF'
cargo check -p roko-core
cargo check -p roko-runtime
EOF
      ;;
    F04|F05|F06)
      cat <<'EOF'
cargo check -p roko-runtime
EOF
      ;;
    S01|S02|S03|S04|S05)
      cat <<'EOF'
cargo check -p roko-agent
EOF
      ;;
    S06|S07|S08|S09)
      cat <<'EOF'
cargo check -p roko-compose
EOF
      ;;
    S10|S11)
      cat <<'EOF'
cargo check -p roko-learn
EOF
      ;;
    S12|S13)
      cat <<'EOF'
cargo check -p roko-gate
EOF
      ;;
    E01|E02)
      cat <<'EOF'
cargo check -p roko-runtime
cargo test -p roko-runtime --lib -- pipeline_state
EOF
      ;;
    E03|E04|E05)
      cat <<'EOF'
cargo check -p roko-runtime
EOF
      ;;
    E06|E07|E08)
      cat <<'EOF'
cargo check -p roko-runtime
EOF
      ;;
    W01|W02|W03|W04)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    W05)
      cat <<'EOF'
cargo check -p roko-acp
EOF
      ;;
    W06)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    W07|W08)
      cat <<'EOF'
cargo check -p roko-serve
EOF
      ;;
    O01|O03|O05|O06)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    O02)
      cat <<'EOF'
cargo check -p roko-serve
EOF
      ;;
    O04)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    R01|R02|R03|R04)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    R05)
      cat <<'EOF'
cargo check -p roko-cli --features legacy-orchestrate
EOF
      ;;
    C01|C02|C03|C04|C05|C06|C09|C10)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    C07)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    C08)
      cat <<'EOF'
cargo check -p roko-serve
EOF
      ;;
    C11|C12)
      echo "echo 'SPA check skipped — no cargo gate for JS/TS'"
      ;;
    T01|T02|T03)
      cat <<'EOF'
cargo test -p roko-runtime --lib -- workflow_engine
EOF
      ;;
    T04)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    T05)
      cat <<'EOF'
cargo check -p roko-serve
EOF
      ;;
    # Track D: Daimon
    D01)
      cat <<'EOF'
cargo check -p roko-core
EOF
      ;;
    D02)
      cat <<'EOF'
cargo check -p roko-daimon
EOF
      ;;
    D03)
      cat <<'EOF'
cargo check -p roko-runtime
EOF
      ;;
    D04)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    # Track G: Gateway
    G01)
      cat <<'EOF'
cargo check -p roko-core
EOF
      ;;
    G02)
      cat <<'EOF'
cargo check -p roko-agent
EOF
      ;;
    G03|G07|G08|G09)
      cat <<'EOF'
cargo check -p roko-agent
EOF
      ;;
    G04)
      cat <<'EOF'
cargo check -p roko-serve
EOF
      ;;
    G05)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    G06)
      cat <<'EOF'
cargo check -p roko-dreams
cargo check -p roko-neuro
cargo check -p roko-std
EOF
      ;;
    # Track K: Knowledge
    K01)
      cat <<'EOF'
cargo check -p roko-learn
EOF
      ;;
    K02)
      cat <<'EOF'
cargo check -p roko-agent
EOF
      ;;
    K03)
      cat <<'EOF'
cargo check -p roko-compose
EOF
      ;;
    K04)
      cat <<'EOF'
cargo check -p roko-learn
EOF
      ;;
    K05)
      cat <<'EOF'
cargo check -p roko-neuro
EOF
      ;;
    # Track X: Security
    X01)
      cat <<'EOF'
cargo check -p roko-agent
EOF
      ;;
    X02)
      cat <<'EOF'
cargo check -p roko-agent
EOF
      ;;
    # Track L: Layering
    L01)
      cat <<'EOF'
cargo check --workspace
EOF
      ;;
    L02)
      cat <<'EOF'
cargo check -p roko-cli
EOF
      ;;
    L03)
      echo "test -f deny.toml"
      ;;
    L04)
      echo "test -f .github/workflows/ci.yml"
      ;;
    *)
      return 1
      ;;
  esac
}

# ---------- structural checks ----------

batch_structural_checks() {
  case "$1" in
    F01) cat <<'EOF'
! grep -q 'roko-runtime' crates/roko-core/Cargo.toml
EOF
      ;;
    F02) cat <<'EOF'
grep -q 'use roko_core::foundation' crates/roko-runtime/src/effect_driver.rs
! grep -q 'pub trait ModelCaller' crates/roko-runtime/src/effect_driver.rs
EOF
      ;;
    F03) cat <<'EOF'
grep -q 'roko_core.*RuntimeEvent' crates/roko-runtime/src/effect_driver.rs
! grep -q 'pub enum RuntimeEvent' crates/roko-runtime/src/effect_driver.rs
EOF
      ;;
    F04) cat <<'EOF'
grep -q 'use roko_core' crates/roko-runtime/src/jsonl_logger.rs
EOF
      ;;
    F05) cat <<'EOF'
grep -q 'AgentOutput\|FeedbackRecorded\|StateCheckpointed' crates/roko-runtime/src/projection.rs
EOF
      ;;
    F06) cat <<'EOF'
grep -q 'use roko_core::foundation::EventConsumer' crates/roko-runtime/src/workflow_engine.rs
! grep -q 'pub trait WorkflowEventConsumer' crates/roko-runtime/src/workflow_engine.rs
EOF
      ;;
    S01) cat <<'EOF'
grep -q 'adapter_for_kind\|create_agent_for_model\|InferenceProvider' crates/roko-agent/src/model_call_service.rs
EOF
      ;;
    S03) cat <<'EOF'
grep -q 'CascadeRouter' crates/roko-agent/src/model_call_service.rs
EOF
      ;;
    S04) cat <<'EOF'
grep -q 'cost_usd\|cost_predict\|CostTable' crates/roko-agent/src/model_call_service.rs
EOF
      ;;
    S06) cat <<'EOF'
grep -q 'SystemPromptBuilder' crates/roko-compose/src/prompt_assembly_service.rs
EOF
      ;;
    S10) cat <<'EOF'
grep -q 'episode\|Episode' crates/roko-learn/src/feedback_service.rs
EOF
      ;;
    S12) cat <<'EOF'
grep -q 'rung.*[3-6]\|DiffGate\|diff' crates/roko-gate/src/gate_service.rs
EOF
      ;;
    E01) cat <<'EOF'
grep -q 'toml\|Toml\|TOML\|from_toml\|WorkflowConfig' crates/roko-runtime/src/pipeline_state.rs
EOF
      ;;
    E02) cat <<'EOF'
grep -q 'Serialize\|Deserialize\|checkpoint\|resume' crates/roko-runtime/src/pipeline_state.rs
EOF
      ;;
    E03) cat <<'EOF'
grep -q 'model_caller\|ModelCaller' crates/roko-runtime/src/effect_driver.rs
EOF
      ;;
    E06) cat <<'EOF'
grep -q 'EventConsumer\|event_consumer\|consumers' crates/roko-runtime/src/workflow_engine.rs
EOF
      ;;
    W01) cat <<'EOF'
grep -q 'engine\|Engine\|--engine\|legacy' crates/roko-cli/src/run.rs
EOF
      ;;
    W02) cat <<'EOF'
grep -q 'run_with_workflow_engine\|WorkflowEngine' crates/roko-cli/src/run.rs
EOF
      ;;
    O04) cat <<'EOF'
grep -q 'pub struct\|pub fn' crates/roko-cli/src/output_format.rs
EOF
      ;;
    R01) cat <<'EOF'
grep -q 'legacy-orchestrate' crates/roko-cli/Cargo.toml
EOF
      ;;
    R02) cat <<'EOF'
grep -q 'cfg.*feature.*legacy.orchestrate' crates/roko-cli/src/orchestrate.rs
EOF
      ;;
    C01) cat <<'EOF'
grep -q 'pub struct\|pub fn' crates/roko-cli/src/output_format.rs
EOF
      ;;
    C07) cat <<'EOF'
grep -q 'share\|Share' crates/roko-cli/src/run.rs
EOF
      ;;
    C08) cat <<'EOF'
grep -q 'shared\|share' crates/roko-serve/src/routes/shared_runs.rs
EOF
      ;;
    T01) cat <<'EOF'
grep -q '#\[tokio::test\]' crates/roko-runtime/src/workflow_engine.rs
EOF
      ;;
    # Track D
    D01) cat <<'EOF'
grep -q 'pub trait AffectPolicy' crates/roko-core/src/foundation.rs
grep -q 'pub struct AffectContext' crates/roko-core/src/foundation.rs
grep -q 'pub struct NoOpAffectPolicy' crates/roko-core/src/foundation.rs
grep -q 'BehavioralState' crates/roko-core/src/foundation.rs
EOF
      ;;
    D02) cat <<'EOF'
grep -q 'pub struct DaimonPolicy' crates/roko-daimon/src/policy.rs
grep -q 'impl AffectPolicy for DaimonPolicy' crates/roko-daimon/src/policy.rs
grep -q 'pub mod policy' crates/roko-daimon/src/lib.rs
EOF
      ;;
    D03) cat <<'EOF'
grep -q 'affect_policy\|AffectPolicy' crates/roko-runtime/src/workflow_engine.rs
grep -q 'affect_policy\|AffectPolicy' crates/roko-runtime/src/effect_driver.rs
EOF
      ;;
    D04) cat <<'EOF'
grep -q 'DaimonPolicy\|daimon' crates/roko-cli/src/run.rs
EOF
      ;;
    # Track G
    G01) cat <<'EOF'
grep -q 'CallerIdentity\|CachePolicy\|TokenBudget' crates/roko-core/src/foundation.rs
EOF
      ;;
    G02) cat <<'EOF'
grep -q 'pub struct GatewayEventWriter' crates/roko-agent/src/gateway_events.rs
grep -q 'pub mod gateway_events' crates/roko-agent/src/lib.rs
EOF
      ;;
    G03) cat <<'EOF'
grep -q 'ProviderCallCell\|provider_call_cell' crates/roko-agent/src/model_call_service.rs
EOF
      ;;
    G07) cat <<'EOF'
grep -q 'CacheCell\|BudgetCell\|cache\|budget' crates/roko-agent/src/model_call_service.rs
EOF
      ;;
    G08) cat <<'EOF'
grep -q 'ThinkingCap\|ConvergenceDetect\|convergence' crates/roko-agent/src/model_call_service.rs
EOF
      ;;
    G09) cat <<'EOF'
grep -q 'force_backend\|forced_observation\|record_forced' crates/roko-agent/src/model_call_service.rs
EOF
      ;;
    # Track K
    K01) cat <<'EOF'
grep -q 'route_with_knowledge' crates/roko-learn/src/cascade_router.rs
EOF
      ;;
    K02) cat <<'EOF'
grep -q 'knowledge_store\|KnowledgeStore' crates/roko-agent/src/model_call_service.rs
EOF
      ;;
    K03) cat <<'EOF'
grep -q 'knowledge\|KnowledgeStore' crates/roko-compose/src/prompt_assembly_service.rs
EOF
      ;;
    K05) cat <<'EOF'
grep -q 'update_confidence\|record_usage' crates/roko-neuro/src/lib.rs
EOF
      ;;
    # Track X
    X01) cat <<'EOF'
! grep -q 'AgentContract::permissive' crates/roko-agent/src/safety/mod.rs
grep -q 'RestrictedFallback\|restricted' crates/roko-agent/src/safety/mod.rs
EOF
      ;;
    X02) cat <<'EOF'
grep -q 'StreamJsonParser\|pub trait.*StreamJson' crates/roko-agent/src/streaming.rs
EOF
      ;;
    # Track L
    L01) cat <<'EOF'
grep -q 'package.metadata.roko' crates/roko-core/Cargo.toml
grep -q 'package.metadata.roko' crates/roko-agent/Cargo.toml
grep -q 'package.metadata.roko' crates/roko-cli/Cargo.toml
EOF
      ;;
    L02) cat <<'EOF'
grep -q 'layer.check\|layer_check' crates/roko-cli/src/layer_check.rs
EOF
      ;;
    L03) cat <<'EOF'
grep -q 'advisories\|licenses' deny.toml
EOF
      ;;
    *) ;;
  esac
}

# ---------- anti-pattern checks ----------

batch_antipattern_checks() {
  case "$1" in
    S01|S02|S03|S04|S05)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-agent/src/model_call_service.rs
! grep -rn 'format!.*You are' crates/roko-agent/src/model_call_service.rs
EOF
      ;;
    S06|S07|S08|S09)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-compose/src/prompt_assembly_service.rs
! grep -rn 'format!.*You are.*the' crates/roko-compose/src/prompt_assembly_service.rs
EOF
      ;;
    E03|E04|E05)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-runtime/src/effect_driver.rs
! grep -rn 'if.*phase.*==' crates/roko-runtime/src/effect_driver.rs
EOF
      ;;
    E06|E07|E08)
      cat <<'EOF'
! grep -rn 'Command::new' crates/roko-runtime/src/workflow_engine.rs
EOF
      ;;
    F02|F03)
      cat <<'EOF'
! grep -n 'pub trait ModelCaller' crates/roko-runtime/src/effect_driver.rs
! grep -n 'pub trait PromptAssembler' crates/roko-runtime/src/effect_driver.rs
! grep -n 'pub trait FeedbackSink' crates/roko-runtime/src/effect_driver.rs
! grep -n 'pub trait GateRunner' crates/roko-runtime/src/effect_driver.rs
EOF
      ;;
    W01|W02|W03|W04)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-cli/src/run.rs
EOF
      ;;
    W05)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-acp/src/bridge_events.rs
! grep -rn 'Command::new.*claude' crates/roko-acp/src/runner.rs
EOF
      ;;
    # Track D
    D03|D04)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-runtime/src/effect_driver.rs
EOF
      ;;
    # Track G
    G03|G07|G08)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-agent/src/model_call_service.rs
! grep -rn 'format!.*You are' crates/roko-agent/src/model_call_service.rs
EOF
      ;;
    G06)
      cat <<'EOF'
! grep -rn 'Command::new.*claude' crates/roko-dreams/src/runner.rs
! grep -rn 'Command::new.*claude' crates/roko-neuro/src/episode_completion.rs
EOF
      ;;
    # Track X
    X01)
      cat <<'EOF'
! grep -n 'permissive' crates/roko-agent/src/safety/mod.rs
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

  require_file "$CONV_ROOT/BATCHES.md"

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
