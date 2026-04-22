#!/usr/bin/env bash
# Shared helpers for tmp/demo-resources automation scripts.

set -euo pipefail

DEMO_RESOURCES_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROKO_REPO_ROOT="$(cd "$DEMO_RESOURCES_DIR/../.." && pwd)"

ROKO="${ROKO:-$ROKO_REPO_ROOT/target/debug/roko}"
PYTHON="${PYTHON:-python3}"
ROKO_SERVE_URL="${ROKO_SERVE_URL:-http://127.0.0.1:6677}"

api_url() {
    local base="${1:-$ROKO_SERVE_URL}"
    base="${base%/}"
    if [[ "$base" == */api ]]; then
        printf '%s' "$base"
    else
        printf '%s/api' "$base"
    fi
}

log() {
    printf '==> %s\n' "$*"
}

warn() {
    printf 'WARN: %s\n' "$*" >&2
}

die() {
    printf 'ERROR: %s\n' "$*" >&2
    exit 1
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

require_python() {
    require_cmd "$PYTHON"
}

require_roko() {
    if [[ ! -x "$ROKO" ]]; then
        die "roko binary not found at $ROKO; run: cargo build -p roko-cli"
    fi
}

http_json() {
    local method="$1"
    local url="$2"
    local payload="${3:-}"
    local timeout="${HTTP_TIMEOUT_SECS:-20}"
    require_python
    "$PYTHON" - "$method" "$url" "$payload" "$timeout" <<'PY'
import sys
import urllib.error
import urllib.request

method, url, payload, timeout_s = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4])
data = payload.encode("utf-8") if payload else None
request = urllib.request.Request(url, data=data, method=method)
request.add_header("accept", "application/json")
if data is not None:
    request.add_header("content-type", "application/json")
try:
    with urllib.request.urlopen(request, timeout=timeout_s) as response:
        sys.stdout.write(response.read().decode("utf-8"))
except urllib.error.HTTPError as error:
    body = error.read().decode("utf-8", errors="replace")
    sys.stderr.write(f"HTTP {error.code} {method} {url}\n{body}\n")
    sys.exit(22)
except urllib.error.URLError as error:
    sys.stderr.write(f"request failed: {method} {url}: {error}\n")
    sys.exit(7)
PY
}

http_get_json() {
    http_json GET "$1"
}

http_post_json() {
    http_json POST "$1" "$2"
}

json_eval() {
    local expr="$1"
    require_python
    "$PYTHON" -c '
import json, sys
expr = sys.argv[1]
data = json.load(sys.stdin)
safe_names = {
    "data": data, "len": len, "str": str, "int": int,
    "any": any, "all": all, "sum": sum, "min": min, "max": max,
    "sorted": sorted, "isinstance": isinstance, "list": list, "dict": dict,
}
print(eval(expr, {"__builtins__": {}}, safe_names))
' "$expr"
}

wait_for_http() {
    local url="$1"
    local timeout="${2:-30}"
    local start
    start=$(date +%s)
    while true; do
        if http_get_json "$url" >/dev/null 2>&1; then
            return 0
        fi
        if (( $(date +%s) - start >= timeout )); then
            return 1
        fi
        sleep 1
    done
}

with_temp_workspace() {
    mktemp -d "${TMPDIR:-/tmp}/roko-demo.XXXXXX"
}

start_roko_serve_bg() {
    local workdir="$1"
    local port="${2:-6677}"
    local log_path="${3:-$workdir/roko-serve.log}"
    require_roko
    mkdir -p "$workdir"
    "$ROKO" serve --workdir "$workdir" --bind 127.0.0.1 --port "$port" >"$log_path" 2>&1 &
    printf '%s' "$!"
}

stop_pid() {
    local pid="${1:-}"
    [[ -n "$pid" ]] || return 0
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
}

run_script() {
    local relative_path="$1"
    shift || true
    local script="$DEMO_RESOURCES_DIR/$relative_path"
    [[ -f "$script" ]] || die "unknown demo script: $relative_path"
    bash "$script" "$@"
}

free_port() {
    require_python
    "$PYTHON" - <<'PY'
import socket

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}
