#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
ARTIFACT_ROOT="${ROKO_PROOF_ARTIFACT_ROOT:-/tmp/roko-mori-proof-$(date +%Y%m%d-%H%M%S)}"
ROKO_BIN="${ROKO_BIN:-$ROOT/target/debug/roko}"
TIMEOUT_SECS="${ROKO_PROOF_TIMEOUT_SECS:-420}"
PROVIDER_LIST="${ROKO_PROOF_PROVIDERS:-auto}"

mkdir -p "$ARTIFACT_ROOT"

log() {
  printf '[proof] %s\n' "$*"
}

fail() {
  printf '[proof] ERROR: %s\n' "$*" >&2
  exit 1
}

json_assert() {
  local name="$1"
  local expression="$2"
  local file="$3"
  python3 - "$name" "$expression" "$file" <<'PY'
import json
import os
import sys

name, expression, path = sys.argv[1:4]
with open(path, "r", encoding="utf-8") as fh:
    data = json.load(fh)
safe = {
    "data": data,
    "len": len,
    "isinstance": isinstance,
    "dict": dict,
    "list": list,
    "any": any,
    "all": all,
}
if not eval(expression, {"__builtins__": {}}, safe):
    raise SystemExit(f"{name}: assertion failed: {expression}\n{json.dumps(data)[:1200]}")
PY
}

jsonl_assert() {
  local name="$1"
  local expression="$2"
  local file="$3"
  python3 - "$name" "$expression" "$file" <<'PY'
import json
import os
import sys

name, expression, path = sys.argv[1:4]
records = []
with open(path, "r", encoding="utf-8") as fh:
    for line_no, line in enumerate(fh, 1):
        if not line.strip():
            continue
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError as err:
            raise SystemExit(f"{name}: invalid JSONL at line {line_no}: {err}") from err
safe = {
    "records": records,
    "len": len,
    "isinstance": isinstance,
    "dict": dict,
    "list": list,
    "any": any,
    "all": all,
}
if not eval(expression, {"__builtins__": {}}, safe):
    raise SystemExit(f"{name}: assertion failed: {expression}\n{json.dumps(records[:8])[:1200]}")
PY
}

run_limited() {
  if command -v timeout >/dev/null 2>&1; then
    timeout "$TIMEOUT_SECS" "$@"
  elif command -v gtimeout >/dev/null 2>&1; then
    gtimeout "$TIMEOUT_SECS" "$@"
  elif [ -x /opt/homebrew/bin/timeout ]; then
    /opt/homebrew/bin/timeout "$TIMEOUT_SECS" "$@"
  else
    "$@"
  fi
}

ensure_roko() {
  if [ -x "$ROKO_BIN" ]; then
    log "using roko binary: $ROKO_BIN"
    return
  fi
  log "building roko binary because $ROKO_BIN does not exist"
  cargo build -p roko-cli --bin roko >/dev/null
  [ -x "$ROKO_BIN" ] || fail "roko binary was not produced at $ROKO_BIN"
}

provider_program() {
  case "$1" in
    claude)
      command -v claude || true
      ;;
    codex)
      command -v codex || true
      ;;
    openai|anthropic|moonshot|zai|perplexity)
      true
      ;;
    *)
      command -v "$1" || true
      ;;
  esac
}

supported_by_runner() {
  case "$1" in
    claude|codex|anthropic|openai|moonshot|zai|perplexity) return 0 ;;
    *) return 1 ;;
  esac
}

provider_runtime() {
  case "$1" in
    claude|codex)
      printf '%s\n' "cli_stream"
      ;;
    anthropic|openai|moonshot|zai|perplexity)
      printf '%s\n' "agent_result_bridge"
      ;;
    *)
      printf '%s\n' "unsupported"
      ;;
  esac
}

provider_requires_binary() {
  case "$1" in
    claude|codex) return 0 ;;
    *) return 1 ;;
  esac
}

provider_credential_env() {
  case "$1" in
    anthropic)
      printf '%s\n' "ANTHROPIC_API_KEY"
      ;;
    openai)
      printf '%s\n' "OPENAI_API_KEY"
      ;;
    moonshot)
      printf '%s\n' "MOONSHOT_API_KEY"
      ;;
    zai)
      printf '%s\n' "ZAI_API_KEY"
      ;;
    perplexity)
      printf '%s\n' "PERPLEXITY_API_KEY"
      ;;
    *)
      true
      ;;
  esac
}

provider_model() {
  case "$1" in
    claude)
      printf '%s\n' "${ROKO_PROOF_CLAUDE_MODEL:-claude-sonnet-4-6}"
      ;;
    codex)
      printf '%s\n' "${ROKO_PROOF_CODEX_MODEL:-claude-sonnet-4-6}"
      ;;
    anthropic)
      printf '%s\n' "${ROKO_PROOF_ANTHROPIC_MODEL:-claude-sonnet-4-6}"
      ;;
    openai)
      printf '%s\n' "${ROKO_PROOF_OPENAI_MODEL:-gpt-4o-mini}"
      ;;
    moonshot)
      printf '%s\n' "${ROKO_PROOF_MOONSHOT_MODEL:-kimi-k2.5}"
      ;;
    zai)
      printf '%s\n' "${ROKO_PROOF_ZAI_MODEL:-glm-5.1}"
      ;;
    perplexity)
      printf '%s\n' "${ROKO_PROOF_PERPLEXITY_MODEL:-sonar}"
      ;;
    *)
      printf '%s\n' "${ROKO_PROOF_MODEL:-$1}"
      ;;
  esac
}

provider_model_key() {
  case "$1" in
    anthropic|openai|moonshot|zai|perplexity)
      printf 'proof-%s\n' "$1"
      ;;
    *)
      provider_model "$1"
      ;;
  esac
}

classify_provider_failure_file() {
  if grep -Eiq 'rate[ _-]?limit|rate_limited|too many requests|quota exceeded|429' "$@"; then
    printf '%s\n' "rate_limited"
    return
  fi
  if grep -Eiq 'api[ _-]?key|auth|credential|login|log in|not authenticated|unauthorized|forbidden|401|403|invalid key|invalid token' "$@"; then
    printf '%s\n' "auth_failed"
    return
  fi
  printf '%s\n' "unsupported"
}

record_provider_status() {
  local provider="$1"
  local status="$2"
  local program="$3"
  local model="$4"
  local runtime="$5"
  local reason="$6"
  local credential_env="$7"
  local artifacts="${8:-}"
  python3 - "$ARTIFACT_ROOT/provider-matrix.jsonl" "$ARTIFACT_ROOT/unsupported-providers.jsonl" "$provider" "$status" "$program" "$model" "$runtime" "$reason" "$credential_env" "$artifacts" <<'PY'
import json
import os
import sys
from pathlib import Path

matrix_path, unsupported_path = sys.argv[1:3]
provider, status, program, model, runtime, reason, credential_env, artifacts = sys.argv[3:11]
record = {
    "provider": provider,
    "status": status,
    "runtime": runtime,
    "program": program or None,
    "model": model or None,
    "reason": reason or None,
    "credential_env": credential_env or None,
    "credential_present": bool(credential_env and os.environ.get(credential_env)),
    "mocked": False,
    "artifacts": artifacts or None,
}
for path in (matrix_path,):
    with open(path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(record, sort_keys=True) + "\n")
if status != "proved":
    with open(unsupported_path, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(record, sort_keys=True) + "\n")
PY
}

providers_to_check() {
  if [ "$PROVIDER_LIST" = "auto" ]; then
    printf '%s\n' claude codex anthropic openai moonshot zai perplexity
  else
    printf '%s\n' "$PROVIDER_LIST" | tr ',' '\n' | sed '/^$/d'
  fi
}

write_workspace() {
  local workdir="$1"
  local provider="$2"
  local program="$3"
  local model="$4"
  local model_key
  model_key="$(provider_model_key "$provider")"

  mkdir -p "$workdir/plans/proof"
  (
    cd "$workdir"
    git init -q
    cargo init --bin --quiet
    printf 'fn main() { println!("roko proof"); }\n' > src/main.rs
    git add -A
    git commit -q -m 'proof fixture' --allow-empty
  )

  case "$provider" in
    claude|codex)
      cat > "$workdir/roko.toml" <<EOF
[agent]
command = "$program"
default_model = "$model"
EOF
      ;;
    anthropic)
      cat > "$workdir/roko.toml" <<EOF
schema_version = 2

[agent]
default_model = "$model_key"

[providers.anthropic]
kind = "anthropic_api"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"
timeout_ms = 180000

[models.$model_key]
provider = "anthropic"
slug = "$model"
context_window = 200000
max_output = 8192
supports_tools = true
tool_format = "anthropic_blocks"
EOF
      ;;
    openai)
      cat > "$workdir/roko.toml" <<EOF
schema_version = 2

[agent]
default_model = "$model_key"

[providers.openai]
kind = "openai_compat"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
timeout_ms = 180000

[models.$model_key]
provider = "openai"
slug = "$model"
context_window = 128000
max_output = 8192
supports_tools = true
tool_format = "openai_json"
EOF
      ;;
    moonshot)
      cat > "$workdir/roko.toml" <<EOF
schema_version = 2

[agent]
default_model = "$model_key"

[providers.moonshot]
kind = "openai_compat"
base_url = "https://api.moonshot.ai/v1"
api_key_env = "MOONSHOT_API_KEY"
timeout_ms = 180000

[models.$model_key]
provider = "moonshot"
slug = "$model"
context_window = 256000
max_output = 65535
supports_tools = true
supports_thinking = true
supports_vision = true
supports_partial = true
tool_format = "openai_json"
EOF
      ;;
    zai)
      cat > "$workdir/roko.toml" <<EOF
schema_version = 2

[agent]
default_model = "$model_key"

[providers.zai]
kind = "openai_compat"
base_url = "https://api.z.ai/api/paas/v4"
api_key_env = "ZAI_API_KEY"
timeout_ms = 180000

[models.$model_key]
provider = "zai"
slug = "$model"
context_window = 200000
max_output = 131072
supports_tools = true
supports_thinking = true
supports_web_search = true
supports_mcp_tools = true
tool_format = "openai_json"
EOF
      ;;
    perplexity)
      cat > "$workdir/roko.toml" <<EOF
schema_version = 2

[agent]
default_model = "$model_key"

[providers.perplexity]
kind = "perplexity_api"
base_url = "https://api.perplexity.ai"
api_key_env = "PERPLEXITY_API_KEY"
timeout_ms = 180000

[models.$model_key]
provider = "perplexity"
slug = "$model"
context_window = 128000
max_output = 8192
supports_tools = false
tool_format = "openai_json"
EOF
      ;;
    *)
      cat > "$workdir/roko.toml" <<EOF
[agent]
command = "$program"
default_model = "$model"
EOF
      ;;
  esac

  cat >> "$workdir/roko.toml" <<EOF

[executor]
task_timeout_secs = 180

[budget]
max_plan_usd = 20.0
max_turn_usd = 10.0

[gates]
clippy_enabled = false
skip_tests = true
EOF

  cat > "$workdir/plans/proof/tasks.toml" <<EOF
[meta]
plan = "mori-runtime-proof-$provider"
status = "ready"
max_parallel = 1
skip_enrichment = true

[[task]]
id = "T1"
title = "Create durable proof marker and survive one forced retry"
description = "Create proof.txt with exact lines provider=$provider and retry-fixed=$provider. The first verify command intentionally fails once to prove retry wiring, then the second attempt must pass without mocks."
role = "implementer"
tier = "focused"
status = "ready"
files = ["proof.txt"]
depends_on = []
verify = [
  { phase = "retry-proof", command = "if [ \"\${ROKO_GATE_RUNG:-}\" = \"0\" ] && [ -n \"\${ROKO_GATE_ATTEMPT_SENTINEL:-}\" ]; then mkdir -p \"\$(dirname \"\$ROKO_GATE_ATTEMPT_SENTINEL\")\"; if [ ! -f \"\$ROKO_GATE_ATTEMPT_SENTINEL\" ]; then touch \"\$ROKO_GATE_ATTEMPT_SENTINEL\"; echo forced first retry >&2; exit 1; fi; fi; grep -q '^provider=$provider$' proof.txt && grep -q '^retry-fixed=$provider$' proof.txt", fail_msg = "proof.txt must contain provider and retry-fixed markers after the forced retry", timeout_ms = 30000 },
]

[[task]]
id = "T2"
title = "Append dependent task marker"
description = "Append exact line dependent-task=$provider to proof.txt. This task depends on T1, so successful completion proves task dependency ordering."
role = "implementer"
tier = "focused"
status = "ready"
files = ["proof.txt"]
depends_on = ["T1"]
verify = [
  { phase = "dependency-proof", command = "grep -q '^provider=$provider$' proof.txt && grep -q '^retry-fixed=$provider$' proof.txt && grep -q '^dependent-task=$provider$' proof.txt", fail_msg = "proof.txt must contain all proof markers", timeout_ms = 30000 },
]
EOF
}

free_port() {
  python3 - <<'PY'
import socket
sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}

curl_json() {
  local url="$1"
  local out="$2"
  curl -fsS "$url" -o "$out"
}

wait_for_health() {
  local base_url="$1"
  local deadline="$2"
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if curl -fsS "$base_url/health" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

validate_runtime_artifacts() {
  local provider="$1"
  local model="$2"
  local workdir="$3"
  local outdir="$4"
  python3 - "$provider" "$model" "$workdir" "$outdir" <<'PY'
import json
import shutil
import sys
from collections import Counter
from pathlib import Path

provider, model, workdir, outdir = sys.argv[1:5]
workdir = Path(workdir)
outdir = Path(outdir)


def fail(message):
    raise SystemExit(f"{provider}: {message}")


def read_jsonl(rel, min_records=1):
    path = workdir / rel
    if not path.exists():
        fail(f"{rel} missing")
    records = []
    with path.open("r", encoding="utf-8") as fh:
        for line_no, line in enumerate(fh, 1):
            if not line.strip():
                continue
            try:
                records.append(json.loads(line))
            except json.JSONDecodeError as err:
                fail(f"{rel} has invalid JSONL at line {line_no}: {err}")
    if len(records) < min_records:
        fail(f"{rel} has {len(records)} records, expected at least {min_records}")
    return records


def require_any(records, predicate, message):
    if not any(predicate(record) for record in records):
        fail(message)


events = read_jsonl(".roko/events.jsonl")
event_types = Counter(record.get("type") for record in events)
required_event_types = {
    "resume.marker",
    "run.started",
    "run.completed",
    "plan.started",
    "plan.completed",
    "task.attempt.started",
    "task.attempt.completed",
    "agent.dispatch.started",
    "agent.dispatch.completed",
    "agent.started",
    "agent.exited",
    "agent.completed",
    "prompt.assembled",
    "gate.dispatch.started",
    "gate.completed",
    "retry.decision",
}
missing = sorted(required_event_types - set(event_types))
if missing:
    fail(f"runner event log missing event types: {', '.join(missing)}")

require_any(
    events,
    lambda record: record.get("type") == "run.completed"
    and record.get("outcome") == "succeeded",
    "run.completed succeeded event missing",
)
require_any(
    events,
    lambda record: record.get("type") == "agent.dispatch.completed"
    and record.get("outcome") == "spawned",
    "agent dispatch spawn completion missing",
)
require_any(
    events,
    lambda record: record.get("type") == "prompt.assembled"
    and record.get("estimated_tokens", 0) > 0
    and len(record.get("included_sections", [])) > 0,
    "prompt diagnostics event missing included sections and token estimate",
)
require_any(
    events,
    lambda record: record.get("type") == "gate.completed"
    and record.get("passed") is False
    and "forced first retry" in record.get("output", ""),
    "forced retry gate failure missing",
)
require_any(
    events,
    lambda record: record.get("type") == "gate.completed"
    and record.get("passed") is True,
    "passing gate completion missing",
)
require_any(
    events,
    lambda record: record.get("type") == "retry.decision"
    and record.get("action") == "retry_after_backoff"
    and record.get("next_attempt") is not None,
    "retry-after-backoff decision missing",
)
require_any(
    events,
    lambda record: record.get("type") == "resume.marker"
    and record.get("marker", {}).get("current_plan_ids"),
    "resume marker lacks current plan ids",
)

for rel in [".roko/state/executor.json", ".roko/state/orchestrator.json"]:
    path = workdir / rel
    if not path.exists():
        fail(f"{rel} missing")
    try:
        json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        fail(f"{rel} is invalid JSON: {err}")

expected_provider_labels = {
    "claude": {"claude", "claude-cli", "claude_cli"},
    "codex": {"codex", "codex-cli", "codex_cli"},
    "anthropic": {"anthropic", "anthropic_api"},
    "openai": {"openai", "openai_compat"},
    "moonshot": {"moonshot", "openai_compat"},
    "zai": {"zai", "openai_compat"},
    "perplexity": {"perplexity", "perplexity_api"},
}.get(provider, {provider})
started_provider_labels = {
    (record.get("event") or {}).get("provider")
    for record in events
    if record.get("type") == "agent.started"
}
started_provider_labels.discard(None)
if expected_provider_labels.isdisjoint(started_provider_labels):
    fail(
        "agent.started did not report expected provider label; "
        f"expected one of {sorted(expected_provider_labels)}, got {sorted(started_provider_labels)}"
    )

root_episodes = read_jsonl(".roko/episodes.jsonl")
learn_episodes = read_jsonl(".roko/learn/episodes.jsonl")
efficiency = read_jsonl(".roko/learn/efficiency.jsonl")
efficiency_summaries = read_jsonl(".roko/learn/efficiency-summaries.jsonl")
gate_outcomes = read_jsonl(".roko/learn/gate-outcomes.jsonl")
retry_outcomes = read_jsonl(".roko/learn/retry-outcomes.jsonl")
provider_model_outcomes = read_jsonl(".roko/learn/provider-model-outcomes.jsonl")
knowledge_lifecycle = read_jsonl(".roko/neuro/knowledge-lifecycle.jsonl")

for label, records in {
    ".roko/episodes.jsonl": root_episodes,
    ".roko/learn/episodes.jsonl": learn_episodes,
}.items():
    require_any(
        records,
        lambda record: record.get("kind") == "runner_task_gate",
        f"{label} has no runner_task_gate episode",
    )
    require_any(records, lambda record: record.get("success") is False, f"{label} has no failed retry episode")
    require_any(records, lambda record: record.get("success") is True, f"{label} has no successful episode")

require_any(efficiency, lambda record: record.get("gate_passed") is False, "efficiency log has no failed gate event")
require_any(efficiency, lambda record: record.get("gate_passed") is True, "efficiency log has no passing gate event")
require_any(
    efficiency_summaries,
    lambda record: record.get("provider") and record.get("model"),
    "efficiency summaries lack provider/model evidence",
)
require_any(gate_outcomes, lambda record: record.get("passed") is False, "gate outcomes have no failed gate")
require_any(gate_outcomes, lambda record: record.get("passed") is True, "gate outcomes have no passing gate")
require_any(
    retry_outcomes,
    lambda record: record.get("status") in {"scheduled", "started", "succeeded", "exhausted", "not_retryable", "cancelled"},
    "retry outcomes have no normalized retry status",
)
require_any(
    provider_model_outcomes,
    lambda record: record.get("provider") and record.get("model") and record.get("task_id"),
    "provider/model outcomes lack provider, model, or task_id",
)
require_any(
    knowledge_lifecycle,
    lambda record: record.get("record_id") and record.get("episode_id"),
    "knowledge lifecycle lacks episode-linked receipt",
)

durable_dir = outdir / "durable"
durable_dir.mkdir(parents=True, exist_ok=True)
for rel in [
    ".roko/events.jsonl",
    ".roko/episodes.jsonl",
    ".roko/learn/episodes.jsonl",
    ".roko/learn/efficiency.jsonl",
    ".roko/learn/efficiency-summaries.jsonl",
    ".roko/learn/gate-outcomes.jsonl",
    ".roko/learn/retry-outcomes.jsonl",
    ".roko/learn/provider-model-outcomes.jsonl",
    ".roko/neuro/knowledge-lifecycle.jsonl",
    ".roko/state/executor.json",
    ".roko/state/orchestrator.json",
]:
    source = workdir / rel
    dest = durable_dir / rel.replace(".roko/", "").replace("/", "__")
    shutil.copy2(source, dest)

proof = {
    "provider": provider,
    "model": model,
    "event_type_counts": dict(sorted(event_types.items())),
    "root_episodes": len(root_episodes),
    "learn_episodes": len(learn_episodes),
    "efficiency_events": len(efficiency),
    "efficiency_summaries": len(efficiency_summaries),
    "gate_outcomes": len(gate_outcomes),
    "retry_outcomes": len(retry_outcomes),
    "provider_model_outcomes": len(provider_model_outcomes),
    "knowledge_lifecycle_records": len(knowledge_lifecycle),
    "executor_snapshot": str(workdir / ".roko/state/executor.json"),
    "orchestrator_snapshot": str(workdir / ".roko/state/orchestrator.json"),
    "mocked": False,
}
(outdir / "runtime-artifact-proof.json").write_text(
    json.dumps(proof, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY
}

validate_http_artifacts() {
  local provider="$1"
  local outdir="$2"
  python3 - "$provider" "$outdir" <<'PY'
import json
import sys
from pathlib import Path

provider, outdir = sys.argv[1:3]
http = Path(outdir) / "http"


def fail(message):
    raise SystemExit(f"{provider}: {message}")


def load(name):
    path = http / name
    if not path.exists():
        fail(f"HTTP artifact {name} missing")
    with path.open("r", encoding="utf-8") as fh:
        return json.load(fh)


def projection(name):
    doc = load(name)
    return doc.get("data", doc.get("state", doc))


catalog = load("projection-catalog.json")
catalog_names = {entry.get("name") for entry in catalog.get("projections", [])}
required = {"execution_trace", "runtime_feedback", "cost_state", "provider_state", "retry_state"}
missing = sorted(required - catalog_names)
if missing:
    fail(f"projection catalog missing: {', '.join(missing)}")

episodes = load("episodes.json")
if not isinstance(episodes, list) or len(episodes) < 1:
    fail("/api/episodes returned no episodes")

gates = load("gates-history.json")
if not isinstance(gates, dict) or len(gates.get("history", [])) < 2:
    fail("/api/gates/history returned insufficient gate history")

events = projection("projection-events.json")
if len(events.get("items", [])) < 1:
    fail("/api/projections/events returned no event log items")

cost_state = projection("projection-cost-state.json")
if len(cost_state.get("records", [])) < 1 and cost_state.get("efficiency_records", 0) < 1:
    fail("/api/projections/cost_state has no cost or efficiency evidence")

provider_state = projection("projection-provider-state.json")
if len(provider_state.get("outcomes", [])) < 1:
    fail("/api/projections/provider_state has no provider/model outcomes")

retry_state = projection("projection-retry-state.json")
if len(retry_state.get("attempts", [])) < 1:
    fail("/api/projections/retry_state has no retry attempts")

trace = projection("projection-execution-trace.json")
proof = trace.get("proof", {})
for key in [
    "has_plan_or_task_state",
    "has_provider_state",
    "has_gate_state",
    "has_retry_state",
    "has_episode_state",
    "has_cost_state",
]:
    if proof.get(key) is not True:
        fail(f"/api/projections/execution_trace proof flag {key} was not true")

runtime_feedback = projection("projection-runtime-feedback.json")
if runtime_feedback.get("efficiency_events", {}).get("total", 0) < 1:
    fail("/api/projections/runtime_feedback has no efficiency events")
if len(runtime_feedback.get("providers", {}).get("outcomes", [])) < 1:
    fail("/api/projections/runtime_feedback has no provider outcomes")
if len(runtime_feedback.get("retries", {}).get("attempts", [])) < 1:
    fail("/api/projections/runtime_feedback has no retry attempts")

summary = {
    "provider": provider,
    "projection_catalog_entries": sorted(catalog_names),
    "http_artifacts": sorted(path.name for path in http.glob("*.json")),
    "checked_projection_endpoints": sorted(required),
}
(Path(outdir) / "http-proof.json").write_text(
    json.dumps(summary, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
PY
}

prove_resume_from_snapshot() {
  local provider="$1"
  local workdir="$2"
  local outdir="$3"

  [ -s "$workdir/.roko/state/executor.json" ] || fail "$provider: executor snapshot missing before resume proof"
  [ -s "$workdir/.roko/state/orchestrator.json" ] || fail "$provider: orchestrator snapshot missing before resume proof"

  log "$provider: proving resume from durable snapshot"
  set +e
  (
    cd "$ROOT"
    run_limited "$ROKO_BIN" --json --color never plan run "$workdir/plans" --workdir "$workdir" --max-retries 1
  ) >"$outdir/resume.stdout" 2>"$outdir/resume.stderr"
  local status="$?"
  set -e
  if [ "$status" -ne 0 ]; then
    tail -80 "$outdir/resume.stderr" >&2 || true
    fail "$provider: resume proof run failed"
  fi

  python3 - "$provider" "$workdir" "$outdir" <<'PY'
import json
import sys
from pathlib import Path

provider, workdir, outdir = sys.argv[1:4]
workdir = Path(workdir)
outdir = Path(outdir)
events = [
    json.loads(line)
    for line in (workdir / ".roko/events.jsonl").read_text(encoding="utf-8").splitlines()
    if line.strip()
]
markers = [event for event in events if event.get("type") == "resume.marker"]
if not markers:
    raise SystemExit(f"{provider}: no resume.marker events after resume proof")
latest = markers[-1]
marker = latest.get("marker", {})
if marker.get("outcome") != "resumed":
    raise SystemExit(f"{provider}: latest resume outcome was {marker.get('outcome')!r}, expected 'resumed'")
if not marker.get("snapshot_plan_ids") or not marker.get("current_plan_ids"):
    raise SystemExit(f"{provider}: resumed marker lacks snapshot/current plan ids")
snapshot_path = marker.get("snapshot_path")
if not snapshot_path or not Path(snapshot_path).exists():
    raise SystemExit(f"{provider}: resumed marker snapshot path does not exist: {snapshot_path}")
summary = {
    "provider": provider,
    "resume_marker": marker,
    "resume_events": len(markers),
    "snapshot_path": snapshot_path,
    "executor_snapshot": str(workdir / ".roko/state/executor.json"),
    "orchestrator_snapshot": str(workdir / ".roko/state/orchestrator.json"),
    "mocked": False,
}
(outdir / "resume-proof.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

prove_http_surfaces() {
  local provider="$1"
  local workdir="$2"
  local outdir="$3"
  local port
  port="$(free_port)"
  local base_url="http://127.0.0.1:$port"
  local server_log="$outdir/server.log"

  "$ROKO_BIN" serve --workdir "$workdir" --bind 127.0.0.1 --port "$port" --color never >"$server_log" 2>&1 &
  local server_pid="$!"
  trap 'kill "$server_pid" >/dev/null 2>&1 || true' RETURN

  wait_for_health "$base_url" "$(( $(date +%s) + 45 ))" || {
    cat "$server_log" >&2 || true
    fail "$provider: roko serve did not become healthy"
  }

  mkdir -p "$outdir/http"
  curl_json "$base_url/api/health" "$outdir/http/api-health.json"
  curl_json "$base_url/api/executor/state" "$outdir/http/executor-state.json"
  curl_json "$base_url/api/episodes" "$outdir/http/episodes.json"
  curl_json "$base_url/api/gates/history?limit=50" "$outdir/http/gates-history.json"
  curl_json "$base_url/api/gates/summary" "$outdir/http/gates-summary.json"
  curl_json "$base_url/api/learn/efficiency" "$outdir/http/learn-efficiency.json"
  curl_json "$base_url/api/statehub/snapshot" "$outdir/http/statehub-snapshot.json"
  curl_json "$base_url/api/statehub/events?limit=100" "$outdir/http/statehub-events.json"
  curl_json "$base_url/api/projections/catalog" "$outdir/http/projection-catalog.json"
  curl_json "$base_url/api/projections/events?limit=100" "$outdir/http/projection-events.json"
  curl_json "$base_url/api/projections/gate_state" "$outdir/http/projection-gate-state.json"
  curl_json "$base_url/api/projections/executor_state" "$outdir/http/projection-executor-state.json"
  curl_json "$base_url/api/projections/cost_meter" "$outdir/http/projection-cost-meter.json"
  curl_json "$base_url/api/projections/execution_trace?limit=250" "$outdir/http/projection-execution-trace.json"
  curl_json "$base_url/api/projections/runtime_feedback?limit=250" "$outdir/http/projection-runtime-feedback.json"
  curl_json "$base_url/api/projections/cost_state?limit=250" "$outdir/http/projection-cost-state.json"
  curl_json "$base_url/api/projections/provider_state?limit=250" "$outdir/http/projection-provider-state.json"
  curl_json "$base_url/api/projections/retry_state?limit=250" "$outdir/http/projection-retry-state.json"
  curl_json "$base_url/api/knowledge?q=proof&limit=5" "$outdir/http/knowledge.json"

  json_assert "$provider episodes" "isinstance(data, list) and len(data) >= 1" "$outdir/http/episodes.json"
  json_assert "$provider gates" "isinstance(data, dict) and len(data.get('history', [])) >= 2" "$outdir/http/gates-history.json"
  json_assert "$provider statehub" "isinstance(data, dict) and ('stats' in data or 'stats' in data.get('data', {}))" "$outdir/http/statehub-snapshot.json"
  json_assert "$provider projection events" "isinstance(data, dict) and len(data.get('data', {}).get('items', [])) >= 1" "$outdir/http/projection-events.json"
  validate_http_artifacts "$provider" "$outdir"

  kill "$server_pid" >/dev/null 2>&1 || true
  wait "$server_pid" >/dev/null 2>&1 || true
  trap - RETURN
}

prove_provider() {
  local provider="$1"
  local program="$2"
  local model="$3"
  local outdir="$ARTIFACT_ROOT/providers/$provider"
  local workdir="$outdir/work"
  mkdir -p "$outdir"

  write_workspace "$workdir" "$provider" "$program" "$model"

  log "$provider: running real plan with $program"
  set +e
  (
    cd "$ROOT"
    run_limited "$ROKO_BIN" --json --color never plan run "$workdir/plans" --workdir "$workdir" --max-retries 1
  ) >"$outdir/run.stdout" 2>"$outdir/run.stderr"
  local status="$?"
  set -e

  if [ "$status" -ne 0 ]; then
    printf '%s\n' "$provider: plan run failed with status $status" >"$outdir/FAILED"
    local classified_status
    classified_status="$(classify_provider_failure_file "$outdir/run.stderr" "$outdir/run.stdout")"
    if [ "$classified_status" = "auth_failed" ] || [ "$classified_status" = "rate_limited" ]; then
      printf '%s\n' "$classified_status" >"$outdir/SKIPPED_STATUS"
      log "$provider: skipped after real dispatch attempt because provider returned $classified_status"
      return 78
    fi
    tail -80 "$outdir/run.stderr" >&2 || true
    fail "$provider: plan run failed"
  fi

  grep -q "\"succeeded\": true" "$outdir/run.stdout" || fail "$provider: JSON report did not succeed"
  grep -q "^provider=$provider$" "$workdir/proof.txt" || fail "$provider: provider marker missing"
  grep -q "^retry-fixed=$provider$" "$workdir/proof.txt" || fail "$provider: retry marker missing"
  grep -q "^dependent-task=$provider$" "$workdir/proof.txt" || fail "$provider: dependent task marker missing"
  grep -q '"type":"gate.completed"' "$workdir/.roko/events.jsonl" || fail "$provider: gate.completed missing"
  grep -q '"type":"run.completed"' "$workdir/.roko/events.jsonl" || fail "$provider: run.completed missing"
  grep -q 'forced first retry' "$workdir/.roko/events.jsonl" || fail "$provider: forced retry evidence missing"
  validate_runtime_artifacts "$provider" "$model" "$workdir" "$outdir"

  cp "$workdir/.roko/events.jsonl" "$outdir/events.jsonl"
  cp "$workdir/proof.txt" "$outdir/proof.txt"

  prove_resume_from_snapshot "$provider" "$workdir" "$outdir"
  cp "$workdir/.roko/events.jsonl" "$outdir/events.jsonl"

  prove_http_surfaces "$provider" "$workdir" "$outdir"

  python3 - "$provider" "$program" "$model" "$workdir" "$outdir" <<'PY'
import json
import sys
from pathlib import Path

provider, program, model, workdir, outdir = sys.argv[1:6]


def last_json_object(text):
    decoder = json.JSONDecoder()
    latest = None
    for idx, char in enumerate(text):
        if char != "{":
            continue
        try:
            value, end = decoder.raw_decode(text[idx:])
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict) and {"succeeded", "total_tasks", "plans"}.issubset(value):
            latest = value
    if latest is None:
        raise SystemExit("no JSON object found in run.stdout")
    return latest


run = last_json_object(Path(outdir, "run.stdout").read_text(encoding="utf-8"))
events = Path(outdir, "events.jsonl").read_text(encoding="utf-8").splitlines()
runtime_proof_path = Path(outdir, "runtime-artifact-proof.json")
http_proof_path = Path(outdir, "http-proof.json")
summary = {
    "provider": provider,
    "program": program,
    "model": model,
    "workspace": workdir,
    "succeeded": run.get("succeeded"),
    "tasks_completed": run.get("tasks_completed"),
    "tasks_failed": run.get("tasks_failed"),
    "total_agent_calls": run.get("total_agent_calls"),
    "event_count": len(events),
    "has_retry_evidence": any("forced first retry" in line for line in events),
    "has_gate_completed": any('"type":"gate.completed"' in line for line in events),
    "has_run_completed": any('"type":"run.completed"' in line for line in events),
    "runtime_artifact_proof": json.loads(runtime_proof_path.read_text(encoding="utf-8")),
    "resume_proof": json.loads(Path(outdir, "resume-proof.json").read_text(encoding="utf-8")),
    "http_proof": json.loads(http_proof_path.read_text(encoding="utf-8")),
    "http_artifacts": sorted(p.name for p in Path(outdir, "http").glob("*.json")),
    "mocked": False,
}
Path(outdir, "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
print(json.dumps(summary, indent=2))
PY
}

prove_merge_backend() {
  local outdir="$ARTIFACT_ROOT/runner-merge-proof"
  mkdir -p "$outdir"

  log "merge: proving real git merge success and conflict evidence via runner backend tests"
  set +e
  (
    cd "$ROOT"
    cargo test -p roko-cli --lib runner::merge -- --nocapture
  ) >"$outdir/cargo-test.stdout" 2>"$outdir/cargo-test.stderr"
  local status="$?"
  set -e
  if [ "$status" -ne 0 ]; then
    tail -120 "$outdir/cargo-test.stderr" >&2 || true
    tail -120 "$outdir/cargo-test.stdout" >&2 || true
    fail "merge backend proof failed"
  fi

  grep -Eq 'git_backend_merges_existing_branch.*ok' "$outdir/cargo-test.stdout" \
    || fail "merge success test result missing from merge proof output"
  grep -Eq 'git_backend_reports_conflict_and_aborts.*ok' "$outdir/cargo-test.stdout" \
    || fail "merge conflict test result missing from merge proof output"

  python3 - "$outdir" <<'PY'
import json
import sys
from pathlib import Path

outdir = Path(sys.argv[1])
stdout = (outdir / "cargo-test.stdout").read_text(encoding="utf-8")
summary = {
    "merge_success": "git_backend_merges_existing_branch" in stdout,
    "merge_conflict_failure_evidence": "git_backend_reports_conflict_and_aborts" in stdout,
    "backend": "GitMergeBackend",
    "mocked": False,
    "stdout": str(outdir / "cargo-test.stdout"),
    "stderr": str(outdir / "cargo-test.stderr"),
}
(outdir / "merge-proof.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
print(json.dumps(summary, indent=2, sort_keys=True))
PY
}

main() {
  ensure_roko
  prove_merge_backend

  local manifest="$ARTIFACT_ROOT/manifest.jsonl"
  : > "$manifest"
  : > "$ARTIFACT_ROOT/provider-matrix.jsonl"
  : > "$ARTIFACT_ROOT/unsupported-providers.jsonl"

  while IFS= read -r provider; do
    [ -n "$provider" ] || continue
    local model
    model="$(provider_model "$provider")"
    local runtime
    runtime="$(provider_runtime "$provider")"
    local credential_env
    credential_env="$(provider_credential_env "$provider")"
    local program
    program="$(provider_program "$provider")"
    if provider_requires_binary "$provider" && [ -z "$program" ]; then
      record_provider_status "$provider" "unsupported" "$program" "$model" "$runtime" "required provider binary was not found on PATH" "$credential_env"
      log "$provider: skipped, binary not found"
      continue
    fi
    if ! supported_by_runner "$provider"; then
      record_provider_status "$provider" "unsupported" "$program" "$model" "$runtime" "proof harness has no write-capable runner dispatch path for this provider" "$credential_env"
      log "$provider: skipped, active runner adapter is not implemented for proof execution"
      continue
    fi
    if [ -n "$credential_env" ] && [ -z "${!credential_env:-}" ]; then
      record_provider_status "$provider" "missing_credentials" "$program" "$model" "$runtime" "required credential environment variable is unset" "$credential_env"
      log "$provider: skipped, $credential_env is not set"
      continue
    fi
    if prove_provider "$provider" "$program" "$model"; then
      record_provider_status "$provider" "proved" "$program" "$model" "$runtime" "real provider completed proof run" "$credential_env" "$ARTIFACT_ROOT/providers/$provider"
      printf '{"provider":"%s","status":"proved","artifacts":"%s"}\n' "$provider" "$ARTIFACT_ROOT/providers/$provider" >> "$manifest"
    else
      local proof_status="$?"
      if [ "$proof_status" -eq 78 ]; then
        local classified_status
        classified_status="$(cat "$ARTIFACT_ROOT/providers/$provider/SKIPPED_STATUS" 2>/dev/null || printf '%s\n' auth_failed)"
        record_provider_status "$provider" "$classified_status" "$program" "$model" "$runtime" "real provider dispatch reported $classified_status" "$credential_env" "$ARTIFACT_ROOT/providers/$provider"
        continue
      fi
      fail "$provider: proof failed with status $proof_status"
    fi
  done < <(providers_to_check)

  python3 - "$ARTIFACT_ROOT" <<'PY'
import json
import os
import sys
from pathlib import Path

root = Path(sys.argv[1])
proved = []
for summary in root.glob("providers/*/summary.json"):
    proved.append(json.loads(summary.read_text(encoding="utf-8")))
unsupported = []
unsupported_path = root / "unsupported-providers.jsonl"
if unsupported_path.exists():
    for line in unsupported_path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            unsupported.append(json.loads(line))
matrix = []
matrix_path = root / "provider-matrix.jsonl"
if matrix_path.exists():
    for line in matrix_path.read_text(encoding="utf-8").splitlines():
        if line.strip():
            matrix.append(json.loads(line))
allowed_statuses = {"proved", "missing_credentials", "auth_failed", "rate_limited", "unsupported"}
bad_statuses = sorted(
    {
        record.get("status")
        for record in matrix
        if record.get("status") not in allowed_statuses
    }
)
if bad_statuses:
    raise SystemExit(f"provider matrix contains unsupported statuses: {bad_statuses}")
if os.environ.get("ROKO_PROOF_PROVIDERS", "auto") == "auto":
    expected_auto = {"claude", "codex", "anthropic", "openai", "moonshot", "zai", "perplexity"}
    observed = {record.get("provider") for record in matrix}
    if not expected_auto.issubset(observed):
        missing = sorted(expected_auto - observed)
        raise SystemExit(f"provider matrix missing requested providers: {missing}")
report = {
    "artifact_root": str(root),
    "proved_providers": proved,
    "provider_matrix": matrix,
    "unsupported_or_missing": unsupported,
    "merge_proof": json.loads((root / "runner-merge-proof" / "merge-proof.json").read_text(encoding="utf-8")),
    "mocked": False,
}
(root / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print(json.dumps(report, indent=2))
PY

  log "proof complete: $ARTIFACT_ROOT/report.json"
}

main "$@"
