#!/usr/bin/env bash
# Run benchmark harness controls for the coding-agent benchmark workflow.
# Usage: bash run-controls.sh [--workdir DIR] [--batch-size N]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../bin/common.sh
source "$SCRIPT_DIR/../bin/common.sh"

WORKDIR="$ROKO_REPO_ROOT"
BATCH_SIZE="${BENCH_BATCH_SIZE:-2}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --workdir)
            WORKDIR="$2"
            shift 2
            ;;
        --batch-size)
            BATCH_SIZE="$2"
            shift 2
            ;;
        -h|--help)
            sed -n '1,20p' "$0"
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

require_roko
require_python
require_cmd git

mkdir -p "$WORKDIR/.roko/bench"

log "Benchmark control workdir: $WORKDIR"

log "Positive control: gold oracle patches"
"$ROKO" bench swe \
    --batch-size "$BATCH_SIZE" \
    --workdir "$WORKDIR" \
    --agent-mode gold \
    --report "$WORKDIR/.roko/bench/scores-gold.jsonl" \
    --export-predictions "$WORKDIR/.roko/bench/predictions-gold.jsonl"

log "Negative control: empty patches"
"$ROKO" bench swe \
    --batch-size "$BATCH_SIZE" \
    --workdir "$WORKDIR" \
    --agent-mode empty \
    --report "$WORKDIR/.roko/bench/scores-empty.jsonl" \
    --export-predictions "$WORKDIR/.roko/bench/predictions-empty.jsonl"

log "Validating controls"
"$PYTHON" - "$WORKDIR/.roko/bench/scores-gold.jsonl" "$WORKDIR/.roko/bench/scores-empty.jsonl" <<'PY'
import json
import sys
from pathlib import Path

gold_path = Path(sys.argv[1])
empty_path = Path(sys.argv[2])


def latest(path):
    rows = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    if not rows:
        raise SystemExit(f"no score rows found in {path}")
    return rows[-1]


gold = latest(gold_path)
empty = latest(empty_path)
expected_total = gold["total"]

assert gold["agent_mode"] == "gold", gold
assert expected_total > 0, gold
assert gold["resolved"] == expected_total, gold
assert gold["format_valid"] == expected_total, gold
assert gold["apply_check"] == expected_total, gold
assert gold["tests_passed"] == expected_total, gold

assert empty["agent_mode"] == "empty", empty
assert empty["total"] == expected_total, empty
assert empty["resolved"] == 0, empty
assert empty["format_valid"] == 0, empty
assert empty["apply_check"] == 0, empty
assert empty["tests_passed"] == 0, empty

print(f"  ok gold {expected_total}/{expected_total}, empty 0/{expected_total}")
PY

log "Controls complete"
bash "$SCRIPT_DIR/summarize-bench.sh" --workdir "$WORKDIR"
