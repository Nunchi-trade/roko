#!/usr/bin/env bash
# roko-demo.sh - Compatibility wrapper for bin/roko-demo.
# Usage: roko-demo.sh <workflow> [args...]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKFLOW="${1:-help}"
if [[ $# -gt 0 ]]; then
    shift
fi

case "$WORKFLOW" in
    help|--help|-h)
        exec bash "$SCRIPT_DIR/roko-demo" help
        ;;
    smoke)
        exec bash "$SCRIPT_DIR/roko-demo" dashboard-smoke "$@"
        ;;
    all)
        exec bash "$SCRIPT_DIR/roko-demo" all "$@"
        ;;
    *)
        exec bash "$SCRIPT_DIR/roko-demo" run "$WORKFLOW" "$@"
        ;;
esac
