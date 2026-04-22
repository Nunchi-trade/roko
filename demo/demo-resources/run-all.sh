#!/usr/bin/env bash
# Run the non-interactive reusable demo checks against an existing roko-serve.
# Usage: bash run-all.sh [base-url]

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
BASE="${1:-http://127.0.0.1:6677}"

PASS=0
FAIL=0

run_suite() {
    local name="$1"
    shift
    printf '\n==> %s\n' "$name"
    if "$@"; then
        printf '  ok %s\n' "$name"
        PASS=$((PASS + 1))
    else
        printf '  fail %s\n' "$name" >&2
        FAIL=$((FAIL + 1))
    fi
}

run_suite "doctor" bash "$DIR/bin/roko-demo" doctor
run_suite "benchmark flow" bash "$DIR/bin/roko-demo" bench
run_suite "seed agents" bash "$DIR/bin/roko-demo" seed-agents "$BASE"
run_suite "dashboard smoke" bash "$DIR/bin/roko-demo" dashboard-smoke "$BASE"
run_suite "workflow registry" bash "$DIR/bin/roko-demo" list

printf '\n==> Result: %s passed, %s failed\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
