#!/usr/bin/env bash
# run-parallel.sh — run converge-followup batches using the parallel template
#
# This is a thin wrapper that sets RUNNER_NAME + RUNNER_ROOT and delegates
# to the parallel-template runner. All batches.toml, prompts/, and context-pack/
# are read from this directory.
#
# Usage:
#   bash tmp/runners/converge-followup/run-parallel.sh [OPTIONS]
#
# Options: same as parallel-template/run-parallel.sh (--parallel N, --list, etc.)
#
# Examples:
#   bash tmp/runners/converge-followup/run-parallel.sh --list
#   bash tmp/runners/converge-followup/run-parallel.sh --dry-run
#   bash tmp/runners/converge-followup/run-parallel.sh --parallel 4
#   bash tmp/runners/converge-followup/run-parallel.sh --group A --parallel 2
#   bash tmp/runners/converge-followup/run-parallel.sh --continue

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE_DIR="$(cd "$SCRIPT_DIR/../parallel-template" && pwd)"

if [[ ! -f "$TEMPLATE_DIR/run-parallel.sh" ]]; then
  echo "ERROR: parallel-template not found at $TEMPLATE_DIR" >&2
  exit 1
fi

export RUNNER_NAME="converge-followup"
export RUNNER_ROOT="$SCRIPT_DIR"
export LOG_ROOT="$SCRIPT_DIR/logs"
export PROMPTS_DIR="$SCRIPT_DIR/prompts"
export CONTEXT_DIR="$SCRIPT_DIR/context-pack"

# ── Speed mode: skip all intermediate checks, only final check at end ──
export SKIP_AP_CHECKS=1

exec bash "$TEMPLATE_DIR/run-parallel.sh" --no-gate --no-merge-back "$@"
