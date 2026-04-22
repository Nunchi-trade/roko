#!/usr/bin/env bash
# roko-down.sh — Stop roko-serve and any running agent sidecars.
# Usage: roko-down.sh [--workdir DIR]

set -euo pipefail
source "$(dirname "$0")/common.sh"

WORKDIR="${1:-$(pwd)}"
PID_FILE="$WORKDIR/.roko/serve.pid"

if [[ -f "$PID_FILE" ]]; then
    PID=$(cat "$PID_FILE")
    if kill -0 "$PID" 2>/dev/null; then
        log "Stopping roko-serve (pid $PID)..."
        kill "$PID" 2>/dev/null || true
        wait "$PID" 2>/dev/null || true
        log "Stopped."
    else
        log "roko-serve not running (stale pid $PID)."
    fi
    rm -f "$PID_FILE"
else
    # Fallback: kill by port.
    PIDS=$(lsof -ti :6677 2>/dev/null || true)
    if [[ -n "$PIDS" ]]; then
        log "Stopping processes on :6677..."
        echo "$PIDS" | xargs kill 2>/dev/null || true
    else
        log "No roko-serve found."
    fi
fi
