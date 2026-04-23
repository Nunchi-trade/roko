#!/usr/bin/env bash
# Summarize latest benchmark score rows and learning/knowledge artifact counts.
# Usage: bash summarize-bench.sh [--workdir DIR]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../bin/common.sh
source "$SCRIPT_DIR/../bin/common.sh"

WORKDIR="$ROKO_REPO_ROOT"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --workdir)
            WORKDIR="$2"
            shift 2
            ;;
        -h|--help)
            sed -n '1,18p' "$0"
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

require_python

"$PYTHON" - "$WORKDIR" <<'PY'
import json
import sys
from pathlib import Path

workdir = Path(sys.argv[1]).resolve()
bench_dir = workdir / ".roko" / "bench"
score_paths = sorted(bench_dir.glob("scores*.jsonl"))

print(f"Benchmark summary for {workdir}")
if not score_paths:
    print("  no score files found")
else:
    for path in score_paths:
        rows = []
        for line in path.read_text().splitlines():
            line = line.strip()
            if not line:
                continue
            rows.append(json.loads(line))
        if not rows:
            continue
        row = rows[-1]
        before = row.get("cfactor_before")
        after = row.get("cfactor_after")
        if before is None or after is None:
            delta = "n/a"
        else:
            delta = f"{after - before:+.3f}"
        print(
            "  "
            f"{path.name}: "
            f"{row.get('agent_mode')} "
            f"{row.get('resolved')}/{row.get('total')} "
            f"pass={row.get('pass_rate', 0.0) * 100:.1f}% "
            f"format={row.get('format_valid')} "
            f"apply={row.get('apply_check')} "
            f"tests={row.get('tests_passed')} "
            f"cfactor_delta={delta} "
            f"run={row.get('run_id')}"
        )

artifact_paths = [
    workdir / ".roko" / "learn" / "episodes.jsonl",
    workdir / ".roko" / "learn" / "task-metrics.jsonl",
    workdir / ".roko" / "learn" / "efficiency.jsonl",
    workdir / ".roko" / "learn" / "c-factor.jsonl",
    workdir / ".roko" / "neuro" / "knowledge.jsonl",
]
print("Artifact counts:")
for path in artifact_paths:
    if path.exists():
        count = sum(1 for line in path.read_text().splitlines() if line.strip())
    else:
        count = 0
    print(f"  {path.relative_to(workdir)}: {count}")
PY

if [[ -x "$ROKO" ]]; then
    echo ""
    "$ROKO" knowledge stats --workdir "$WORKDIR" || true
fi
