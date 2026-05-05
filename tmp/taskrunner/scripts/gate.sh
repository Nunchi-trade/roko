#!/usr/bin/env bash
# Run a wave gate: build + test + clippy on the main branch after merges.
#
# Usage:
#   ./scripts/gate.sh wave-0
#   ./scripts/gate.sh wave-1 --fix   # also run cargo fmt

set -euo pipefail
cd "$(dirname "$0")/.."

WAVE="${1:?Usage: gate.sh <wave-name> [--fix]}"
FIX="${2:-}"
PROJECT_ROOT="$(cd ../.. && pwd)"
TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
REPORT="audits/gate-${WAVE}-${TIMESTAMP}.md"

cd "$PROJECT_ROOT"

echo "═══════════════════════════════════════════════"
echo "  GATE: ${WAVE}"
echo "  Time: ${TIMESTAMP}"
echo "═══════════════════════════════════════════════"
echo ""

# Optional: format first
if [[ "$FIX" == "--fix" ]]; then
    echo "--- cargo fmt ---"
    cargo +nightly fmt --all 2>&1 || true
    echo ""
fi

# Build
echo "--- cargo build --workspace ---"
BUILD_OUTPUT=$(cargo build --workspace 2>&1) && BUILD_OK=true || BUILD_OK=false
echo "$BUILD_OUTPUT" | tail -5

# Test
echo ""
echo "--- cargo test --workspace ---"
TEST_OUTPUT=$(cargo test --workspace 2>&1) && TEST_OK=true || TEST_OK=false
echo "$TEST_OUTPUT" | tail -10

# Clippy
echo ""
echo "--- cargo clippy --workspace --no-deps -- -D warnings ---"
CLIPPY_OUTPUT=$(cargo clippy --workspace --no-deps -- -D warnings 2>&1) && CLIPPY_OK=true || CLIPPY_OK=false
echo "$CLIPPY_OUTPUT" | tail -10

# Report
PASS=true
[[ "$BUILD_OK" == "false" ]] && PASS=false
[[ "$TEST_OK" == "false" ]] && PASS=false
[[ "$CLIPPY_OK" == "false" ]] && PASS=false

echo ""
echo "═══════════════════════════════════════════════"
if [[ "$PASS" == "true" ]]; then
    echo "  GATE PASSED ✓"
else
    echo "  GATE FAILED ✗"
    [[ "$BUILD_OK" == "false" ]] && echo "    Build: FAILED"
    [[ "$TEST_OK" == "false" ]] && echo "    Test:  FAILED"
    [[ "$CLIPPY_OK" == "false" ]] && echo "    Clippy: FAILED"
fi
echo "═══════════════════════════════════════════════"

# Write report
cd "$(dirname "$0")/.."
mkdir -p audits
cat > "$REPORT" << EOF
# Gate Report: ${WAVE}

**Time**: ${TIMESTAMP}
**Result**: $(if [[ "$PASS" == "true" ]]; then echo "PASSED"; else echo "FAILED"; fi)

## Build
**Status**: $(if [[ "$BUILD_OK" == "true" ]]; then echo "OK"; else echo "FAILED"; fi)

## Test
**Status**: $(if [[ "$TEST_OK" == "true" ]]; then echo "OK"; else echo "FAILED"; fi)

## Clippy
**Status**: $(if [[ "$CLIPPY_OK" == "true" ]]; then echo "OK"; else echo "FAILED"; fi)
EOF

echo "Report: ${REPORT}"
exit $([[ "$PASS" == "true" ]] && echo 0 || echo 1)
