#!/usr/bin/env bash
# roko-smoke.sh - Compatibility wrapper for the reusable dashboard smoke test.
# Usage: roko-smoke.sh [serve-url]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=common.sh
source "$SCRIPT_DIR/common.sh"

BASE_URL="${1:-$ROKO_SERVE_URL}"

bash "$SCRIPT_DIR/roko-demo" dashboard-smoke "$BASE_URL"

log "Checking local CLI commands"
require_roko
WORKDIR="$(with_temp_workspace)"
(
    cd "$WORKDIR"
    "$ROKO" init >/dev/null 2>&1
    "$ROKO" status --quiet >/dev/null 2>&1
    "$ROKO" doctor >/dev/null 2>&1
)
log "CLI smoke passed"
