#!/usr/bin/env bash
# common.sh — Shared helpers for chain-coordination demo scripts.
#
# Sources: contract addresses, cast wrappers, JSON-RPC helpers, color logging.
# Usage:  source "$(dirname "$0")/common.sh"

set -euo pipefail

# ── Paths ─────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEMO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$DEMO_DIR/../.." && pwd)"

# ── RPC ───────────────────────────────────────────────────────────
RPC_URL="${RPC_URL:-http://127.0.0.1:8545}"

# ── Contract addresses (local mirage deployment) ──────────────────
DAEJI="0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"
AGENT_REGISTRY="0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"
WORKER_REGISTRY="0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"
BOUNTY_MARKET="0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9"
CONSORTIUM_VALIDATOR="0x5FC8d32690cc91D4c39d9d3abcBD16989F875707"
INSIGHT_BOARD="0xa513E6E4b8f2a923D98304ec87F64353C4D5C853"
ISFR="0xA51c1fc2f0D1a1b8494Ed1FE312d7C3a78Ed91C0"

# ── Anvil default accounts ────────────────────────────────────────
DEPLOYER_ADDR="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
DEPLOYER_PK="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

ACCOUNT1_ADDR="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
ACCOUNT1_PK="0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"

ACCOUNT2_ADDR="0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
ACCOUNT2_PK="0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"

# ── Job type hashes (from JobTypeRegistry) ────────────────────────
JOB_PERPS_LIQUIDATE="0xc4700ff6808b71edc77db1eeabc54c98bc47e0b4ff4ee423a63dbe9422f84450"
JOB_ORACLE_UPDATE="0x5cd23da0cd7fe7e15997bc1bfed5abf553a6f57779b5f3282c9df24e9823b7a7"
JOB_FUNDING_WINDOW="0x76060bb820d8017a6b4576d901cf163d073ecc23b5ae315825a8041c28f33ada"

# ── Colors ────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

info()   { echo -e "${BLUE}[info]${NC}  $*"; }
ok()     { echo -e "${GREEN}[ok]${NC}    $*"; }
warn()   { echo -e "${YELLOW}[warn]${NC}  $*"; }
err()    { echo -e "${RED}[err]${NC}   $*"; }
header() { echo -e "\n${BOLD}=== $* ===${NC}\n"; }
dim()    { echo -e "${DIM}$*${NC}"; }

# ── Dependency checks ─────────────────────────────────────────────
require_cmd() {
    if ! command -v "$1" &>/dev/null; then
        err "Required command not found: $1"
        exit 1
    fi
}

require_cast()   { require_cmd cast; }
require_curl()   { require_cmd curl; }
require_python() { require_cmd python3; }

# ── Cast wrappers ─────────────────────────────────────────────────
# cast_send CONTRACT SIG ARGS... --private-key PK
cast_send() {
    cast send --rpc-url "$RPC_URL" "$@" 2>&1
}

# cast_call CONTRACT SIG ARGS...
cast_call() {
    cast call --rpc-url "$RPC_URL" "$@" 2>&1
}

# ── JSON-RPC helper ───────────────────────────────────────────────
# chain_rpc METHOD PARAMS_JSON → prints result field
chain_rpc() {
    local method="$1"
    local params="${2:-[]}"
    local response
    response=$(curl -sf -X POST "$RPC_URL" \
        -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":$params,\"id\":1}")
    local error
    error=$(echo "$response" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('error',{}).get('message',''))" 2>/dev/null)
    if [[ -n "$error" ]]; then
        err "RPC $method failed: $error"
        return 1
    fi
    echo "$response" | python3 -c "import sys,json; print(json.dumps(json.load(sys.stdin)['result']))" 2>/dev/null
}

# chain_rpc_raw METHOD PARAMS_JSON → prints full response
chain_rpc_raw() {
    local method="$1"
    local params="${2:-[]}"
    curl -sf -X POST "$RPC_URL" \
        -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$method\",\"params\":$params,\"id\":1}"
}

# ── Timestamp helper ──────────────────────────────────────────────
# Returns a deadline N seconds in the future.
future_deadline() {
    local offset="${1:-3600}"
    echo $(( $(date +%s) + offset ))
}

# ── Wei conversion ────────────────────────────────────────────────
# ether_to_wei ETHER → prints wei string
ether_to_wei() {
    python3 -c "print(int(float('$1') * 10**18))"
}
