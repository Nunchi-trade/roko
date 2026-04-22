#!/usr/bin/env bash
# Compatibility entrypoint for the reusable roko smoke test.
# Usage: bash smoke-test.sh [base-url]

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
exec bash "$DIR/bin/roko-smoke.sh" "$@"
