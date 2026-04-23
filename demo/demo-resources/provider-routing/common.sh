#!/usr/bin/env bash
set -euo pipefail

# ── Colors ────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m' # No Color

info()  { echo -e "${BLUE}[info]${NC}  $*"; }
ok()    { echo -e "${GREEN}[ok]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC}  $*"; }
err()   { echo -e "${RED}[err]${NC}   $*"; }
header(){ echo -e "\n${BOLD}═══ $* ═══${NC}\n"; }

# ── Locate roko binary ───────────────────────────────────────────
find_roko() {
    # Prefer the compiled binary over shell functions/aliases
    local repo_root
    repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
    if [[ -x "${repo_root}/target/release/roko" ]]; then
        echo "${repo_root}/target/release/roko"
    elif [[ -x "${repo_root}/target/debug/roko" ]]; then
        echo "${repo_root}/target/debug/roko"
    elif [[ -x "${CARGO_TARGET_DIR:-target}/release/roko" ]]; then
        echo "${CARGO_TARGET_DIR:-target}/release/roko"
    elif [[ -x "${CARGO_TARGET_DIR:-target}/debug/roko" ]]; then
        echo "${CARGO_TARGET_DIR:-target}/debug/roko"
    elif command -v roko &>/dev/null; then
        echo "roko"
    else
        err "roko binary not found. Run: cargo build -p roko-cli"
        exit 1
    fi
}

ROKO=$(find_roko)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEMO_CONFIG="${SCRIPT_DIR}/roko.toml"

# ── Workspace setup ──────────────────────────────────────────────
setup_workspace() {
    local workspace="${ROKO_WORKDIR:-${1:-$(mktemp -d /tmp/roko-demo-XXXXXX)}}"
    mkdir -p "$workspace"
    # Only copy config if workspace doesn't already have one
    if [[ ! -f "$workspace/roko.toml" ]]; then
        cp "$DEMO_CONFIG" "$workspace/roko.toml"
    fi

    # Source .env if it exists
    if [[ -f "${SCRIPT_DIR}/.env" ]]; then
        set -a
        source "${SCRIPT_DIR}/.env"
        set +a
    fi

    echo "$workspace"
}

# ── Cleanup ──────────────────────────────────────────────────────
cleanup_workspace() {
    local workspace="$1"
    # Only clean up temp directories, not the project workspace
    if [[ -d "$workspace" && "$workspace" == /tmp/roko-demo-* ]]; then
        rm -rf "$workspace"
    fi
}

# ── Serve integration ────────────────────────────────────────────
# When ROKO_SERVE_URL is set, use the serve API for runs instead of local CLI.
ROKO_SERVE_URL="${ROKO_SERVE_URL:-}"

# Run a prompt through roko — uses serve API if available, local CLI otherwise.
roko_run() {
    local prompt="$1"
    local model="${2:-}"
    local workdir="${3:-$PWD}"

    if [[ -n "$ROKO_SERVE_URL" ]]; then
        # Use serve API — events go through live StateHub to TUI
        local body
        if [[ -n "$model" ]]; then
            body=$(printf '{"prompt":"%s","model":"%s"}' "$prompt" "$model")
        else
            body=$(printf '{"prompt":"%s"}' "$prompt")
        fi
        curl -s -X POST "${ROKO_SERVE_URL}/api/run" \
            -H 'Content-Type: application/json' \
            -d "$body" 2>/dev/null
    else
        # Local CLI
        local args=("$prompt" --workdir "$workdir")
        if [[ -n "$model" ]]; then
            args+=(--model "$model")
        fi
        "$ROKO" run "${args[@]}" 2>/dev/null
    fi
}

# ── Provider availability check ─────────────────────────────────
check_provider_key() {
    local env_var="$1"
    if [[ -z "${!env_var:-}" ]]; then
        return 1
    fi
    return 0
}
