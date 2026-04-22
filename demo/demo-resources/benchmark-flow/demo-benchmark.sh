#!/usr/bin/env bash
# demo-benchmark.sh — Run the reusable SWE-bench proxy + C-factor demo.
# Usage: bash demo-benchmark.sh [workdir]
# Exit: 0 if all benchmark control batches behave as expected.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../bin/common.sh
source "$SCRIPT_DIR/../bin/common.sh"

require_roko
require_python
require_cmd git

WORKDIR="${1:-$(mktemp -d "${TMPDIR:-/tmp}/roko-bench-demo.XXXXXX")}"
mkdir -p "$WORKDIR"

log "SWE-bench proxy demo workspace: $WORKDIR"

run_bench() {
    local label="$1"
    shift
    echo ""
    log "$label"
    "$ROKO" bench swe --batch-size 2 --workdir "$WORKDIR" "$@"
}

run_bench "Positive control: gold oracle patches" \
    --agent-mode gold \
    --export-predictions "$WORKDIR/predictions-gold.jsonl"

run_bench "Negative control: empty patches" \
    --agent-mode empty

COMMAND_AGENT="$PYTHON -c 'import sys,json; print(json.load(sys.stdin)[\"patch\"], end=\"\")'"
run_bench "Command adapter: oracle command reads instance JSON on stdin" \
    --agent-mode command \
    --agent-command "$COMMAND_AGENT" \
    --export-predictions "$WORKDIR/predictions-command.jsonl"

SCORES="$WORKDIR/.roko/bench/scores.jsonl"
EPISODES="$WORKDIR/.roko/learn/episodes.jsonl"
CFACTOR="$WORKDIR/.roko/learn/c-factor.jsonl"

[[ -f "$SCORES" ]] || die "missing scores file: $SCORES"
[[ -f "$EPISODES" ]] || die "missing episodes file: $EPISODES"
[[ -f "$CFACTOR" ]] || die "missing C-factor file: $CFACTOR"

echo ""
log "Validating score and C-factor pattern"
"$PYTHON" - "$SCORES" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
rows = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
if len(rows) < 3:
    raise SystemExit(f"expected at least 3 score rows, got {len(rows)}")

gold, empty, command = rows[-3:]
assert gold["agent_mode"] == "gold", gold
assert empty["agent_mode"] == "empty", empty
assert command["agent_mode"] == "command", command
assert gold["resolved"] == gold["total"] == 2, gold
assert empty["resolved"] == 0 and empty["total"] == 2, empty
assert command["resolved"] == command["total"] == 2, command
assert empty["cfactor_before"] is not None and empty["cfactor_after"] is not None, empty
assert empty["cfactor_after"] < empty["cfactor_before"], empty
assert command["cfactor_after"] is not None and command["cfactor_before"] is not None, command
assert command["cfactor_after"] > command["cfactor_before"], command

print("  ok latest rows: gold 2/2, empty 0/2, command 2/2")
print(
    "  ok C-factor: "
    f"gold->{gold['cfactor_after']:.3f}, "
    f"empty {empty['cfactor_before']:.3f}->{empty['cfactor_after']:.3f}, "
    f"command {command['cfactor_before']:.3f}->{command['cfactor_after']:.3f}"
)
PY

echo ""
log "Artifact counts"
wc -l "$SCORES" "$EPISODES" "$CFACTOR" "$WORKDIR/predictions-command.jsonl"

echo ""
log "Current status snapshot"
"$ROKO" status --workdir "$WORKDIR" --cfactor

echo ""
log "Benchmark demo passed"
cat <<EOF

Artifacts:
  scores:      $SCORES
  runs:        $WORKDIR/.roko/bench/runs
  episodes:    $EPISODES
  c-factor:    $CFACTOR
  predictions: $WORKDIR/predictions-command.jsonl

EOF
