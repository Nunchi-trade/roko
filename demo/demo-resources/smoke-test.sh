#!/usr/bin/env bash
# Compatibility entrypoint for the reusable roko smoke test.
# Usage: bash smoke-test.sh [base-url]

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
echo -e "\033[0;36m▸ roko smoke test\033[0m"
exec bash "$DIR/bin/roko-smoke.sh" "$@"
