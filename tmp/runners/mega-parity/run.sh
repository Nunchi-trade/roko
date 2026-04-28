#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export RUNNER_NAME="mega-parity"
export RUNNER_ROOT="$SCRIPT_DIR"
export LOG_ROOT="$SCRIPT_DIR/logs"
export PROMPTS_DIR="$SCRIPT_DIR/prompts"
export CONTEXT_DIR="$SCRIPT_DIR/context-pack"

# Default to high parallelism — override with --parallel N
: "${PARALLEL:=8}"
export PARALLEL

exec bash "$SCRIPT_DIR/../parallel-template/run-parallel.sh" "$@"
