#!/usr/bin/env bash
# roko-up.sh — Start roko-serve, seed agents, verify everything is ready.
# Usage: roko-up.sh [--port PORT] [--workdir DIR] [--no-seed]
#
# Idempotent: safe to run multiple times. Checks if serve is already running.

set -euo pipefail
source "$(dirname "$0")/common.sh"
require_roko
require_python

PORT=6677
WORKDIR=""
SEED=true

while [[ $# -gt 0 ]]; do
    case "$1" in
        --port)    PORT="$2"; shift 2 ;;
        --workdir) WORKDIR="$2"; shift 2 ;;
        --no-seed) SEED=false; shift ;;
        *)         die "unknown flag: $1" ;;
    esac
done

BASE="http://127.0.0.1:${PORT}"

# If serve is already running, skip startup.
if http_get_json "${BASE}/api/health" >/dev/null 2>&1; then
    log "roko-serve already running on :${PORT}"
else
    if [[ -z "$WORKDIR" ]]; then
        WORKDIR="$(pwd)"
    fi

    # Ensure workspace is initialized.
    mkdir -p "$WORKDIR"
    if [[ ! -f "$WORKDIR/roko.toml" ]]; then
        log "Initializing workspace at $WORKDIR..."
        (cd "$WORKDIR" && "$ROKO" init) >/dev/null 2>&1
    fi

    log "Starting roko-serve on :${PORT}..."
    "$ROKO" serve --workdir "$WORKDIR" --port "$PORT" > "${WORKDIR}/.roko/serve.log" 2>&1 &
    SERVE_PID=$!
    echo "$SERVE_PID" > "${WORKDIR}/.roko/serve.pid"

    if ! wait_for_http "${BASE}/api/health" 15; then
        die "roko-serve failed to start. Check ${WORKDIR}/.roko/serve.log"
    fi
    log "roko-serve ready (pid $SERVE_PID)"
fi

# Seed demo agents.
if $SEED; then
    log "Seeding demo agents..."
    bash "$DEMO_RESOURCES_DIR/bin/roko-demo" seed-agents "${BASE}"
fi

log "Ready. Endpoints:"
log "  Health:    ${BASE}/api/health"
log "  Agents:    ${BASE}/api/managed-agents"
log "  Match:     POST ${BASE}/api/jobs/match"
log "  Jobs:      ${BASE}/api/jobs"
log "  Dashboard: http://localhost:5173 (if running)"
