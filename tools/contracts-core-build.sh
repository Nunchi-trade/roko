#!/usr/bin/env bash
#
# Build the Nunchi-trade/contracts-core submodule so roko can consume
# compiled ABIs + bytecode for the agents package.
#
# Roko uses contracts-core (pinned at commit a818863) as the single source of
# truth for Solidity contracts. See tmp/phase-2-wiring-plan.md (PR #29) for
# context.
#
# Run this once after cloning, and again after pulling a new contracts-core
# pin:
#
#   ./tools/contracts-core-build.sh
#
# Prereqs:
#   - forge (foundry) — install via `curl -L https://foundry.paradigm.xyz | bash`
#   - soldeer support (bundled with recent foundry)
#
# After this runs, artifacts are at:
#   contracts-core/out/<Name>.sol/<Name>.json
#
# which crates/roko-demo/src/deploy.rs::ContractArtifact::load() reads via the
# `contracts_dir` arg (currently pointed at roko's own contracts/ — migration
# to contracts-core happens in P2.1 / P2.4).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONTRACTS_CORE_DIR="$REPO_ROOT/contracts-core"

if [ ! -d "$CONTRACTS_CORE_DIR" ]; then
  echo "ERROR: contracts-core submodule not found at $CONTRACTS_CORE_DIR" >&2
  echo "Run: git submodule update --init --recursive" >&2
  exit 1
fi

if [ ! -f "$CONTRACTS_CORE_DIR/foundry.toml" ]; then
  echo "ERROR: contracts-core appears empty at $CONTRACTS_CORE_DIR" >&2
  echo "Run: git submodule update --init --recursive" >&2
  exit 1
fi

cd "$CONTRACTS_CORE_DIR"

if [ ! -d dependencies ]; then
  echo "==> Installing contracts-core soldeer dependencies..."
  forge soldeer install
fi

echo "==> Building contracts-core agents package (FOUNDRY_PROFILE=agents)..."
FOUNDRY_PROFILE=agents forge build

echo ""
echo "✓ contracts-core built"
echo "  Artifacts: $CONTRACTS_CORE_DIR/out/"
echo ""
echo "Pinned commit:"
git -C "$CONTRACTS_CORE_DIR" rev-parse --short HEAD
