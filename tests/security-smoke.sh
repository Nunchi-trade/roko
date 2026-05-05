#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT=6677
PASS=0
FAIL=0
WORKDIR=""
HOME_DIR=""
SERVER_PID=""
PUBLIC_PID=""
SERVER_LOG=""
PUBLIC_LOG=""

pass() {
  printf '  PASS: %s\n' "$1"
  PASS=$((PASS + 1))
}

fail() {
  printf '  FAIL: %s\n' "$1"
  FAIL=$((FAIL + 1))
}

summary() {
  echo ""
  echo "=== Results: $PASS passed, $FAIL failed ==="
}

die() {
  summary
  exit 1
}

process_state() {
  ps -o stat= -p "$1" 2>/dev/null | tr -d '[:space:]'
}

cleanup_pid() {
  local pid="${1:-}"
  if [[ -n "$pid" ]]; then
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
  fi
}

cleanup() {
  # kill background SERVER_PID and PUBLIC_PID, then remove the temp dir.
  cleanup_pid "${PUBLIC_PID:-}"
  cleanup_pid "${SERVER_PID:-}"
  if [[ -n "${WORKDIR:-}" && -d "${WORKDIR:-}" ]]; then
    rm -rf "$WORKDIR"
  fi
}

trap cleanup EXIT INT TERM

if [[ -n "${ROKO_BIN:-}" ]]; then
  if [[ ! -x "$ROKO_BIN" ]]; then
    fail "ROKO_BIN is not executable: $ROKO_BIN"
    die
  fi
  ROKO_BIN="$(cd "$(dirname "$ROKO_BIN")" && pwd)/$(basename "$ROKO_BIN")"
  ROKO_CMD=("$ROKO_BIN")
elif [[ -x "$ROOT/target/debug/roko" ]]; then
  ROKO_CMD=("$ROOT/target/debug/roko")
elif [[ -x "$ROOT/target/release/roko" ]]; then
  ROKO_CMD=("$ROOT/target/release/roko")
else
  fail "missing built roko binary; build target/debug/roko first or set ROKO_BIN"
  die
fi

if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  fail "TCP port $PORT is already in use; stop the existing listener and rerun."
  die
fi

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/roko-security-smoke.XXXXXX")"
HOME_DIR="$WORKDIR/home"
mkdir -p "$WORKDIR/.roko" "$HOME_DIR"

cat >"$WORKDIR/roko.toml" <<'TOML'
[agent]
model = "test-model"

[server]
cors_origins = ["http://allowed.example"]
TOML

cd "$WORKDIR"

echo "=== Security Smoke Test ==="
echo ""

# Test 1: Default bind is localhost.
echo "[1] Default bind is localhost"
SERVER_LOG="$WORKDIR/default-server.log"
env -u PORT HOME="$HOME_DIR" "${ROKO_CMD[@]}" serve --workdir "$WORKDIR" >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!

deadline=$((SECONDS + 30))
while (( SECONDS < deadline )); do
  if curl --noproxy '*' -fsS --connect-timeout 1 --max-time 2 "http://127.0.0.1:${PORT}/health" >/dev/null; then
    break
  fi

  state="$(process_state "$SERVER_PID")"
  if [[ -z "$state" || "$state" == Z* ]]; then
    fail "Server did not become healthy on 127.0.0.1:${PORT}"
    tail -n 40 "$SERVER_LOG" | tr -d '\r' || true
    die
  fi

  sleep 0.25
done

if ! curl --noproxy '*' -fsS --connect-timeout 1 --max-time 2 "http://127.0.0.1:${PORT}/health" >/dev/null; then
  fail "Server did not respond on 127.0.0.1:${PORT}/health"
  tail -n 40 "$SERVER_LOG" | tr -d '\r' || true
  die
fi

listener_info="$(lsof -nP -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null || true)"
if [[ "$listener_info" == *"127.0.0.1:${PORT}"* || "$listener_info" == *"localhost:${PORT}"* ]]; then
  pass "Server listens on localhost at ${PORT}"
else
  fail "Server listener is not loopback-only on port ${PORT}"
  printf '%s\n' "$listener_info"
  die
fi

echo ""

# Test 2: Terminal disabled by default.
echo "[2] Terminal disabled by default"
terminal_resp="$WORKDIR/terminal-response.json"
: >"$terminal_resp"
terminal_code="$(curl --noproxy '*' -sS --connect-timeout 1 --max-time 2 \
  -X POST \
  -H 'content-type: application/json' \
  -d '{}' \
  -o "$terminal_resp" \
  -w '%{http_code}' \
  "http://127.0.0.1:${PORT}/api/terminal/sessions" || true)"
terminal_body="$(tr -d '\r' <"$terminal_resp")"
if [[ "$terminal_code" == "403" && "$terminal_body" == *'"error":"Terminal disabled"'* && "$terminal_body" == *'"hint":"Set serve.terminal_enabled=true or use --enable-terminal"'* ]]; then
  pass "Terminal creation is rejected with a disabled hint"
else
  fail "Terminal creation was not rejected as expected (HTTP ${terminal_code:-000})"
  printf '  Body: %s\n' "$terminal_body"
  die
fi
rm -f "$terminal_resp"

echo ""

# Test 3: CORS restricts arbitrary origins.
echo "[3] CORS restricts arbitrary origins"
allowed_headers="$WORKDIR/cors-allowed.headers"
evil_headers="$WORKDIR/cors-evil.headers"
: >"$allowed_headers"
: >"$evil_headers"
curl --noproxy '*' -sS --connect-timeout 1 --max-time 2 -X OPTIONS \
  -H 'Origin: http://allowed.example' \
  -H 'Access-Control-Request-Method: GET' \
  -D "$allowed_headers" \
  -o /dev/null \
  "http://127.0.0.1:${PORT}/api/health" >/dev/null || true
curl --noproxy '*' -sS --connect-timeout 1 --max-time 2 -X OPTIONS \
  -H 'Origin: http://evil.example' \
  -H 'Access-Control-Request-Method: GET' \
  -D "$evil_headers" \
  -o /dev/null \
  "http://127.0.0.1:${PORT}/api/health" >/dev/null || true

allowed_origin="$(awk '
  {
    line = tolower($0)
    if (line ~ /^access-control-allow-origin:/) {
      sub(/\r$/, "", $0)
      sub(/^[^:]+:[[:space:]]*/, "", $0)
      print $0
      exit
    }
  }
' "$allowed_headers")"
evil_origin="$(awk '
  {
    line = tolower($0)
    if (line ~ /^access-control-allow-origin:/) {
      sub(/\r$/, "", $0)
      sub(/^[^:]+:[[:space:]]*/, "", $0)
      print $0
      exit
    }
  }
' "$evil_headers")"

if [[ "$allowed_origin" == "http://allowed.example" && -z "$evil_origin" ]]; then
  pass "Allowed origin is reflected and arbitrary origin is denied"
else
  fail "CORS allowlist did not reject arbitrary origins"
  printf '  allowed_origin=%s\n' "${allowed_origin:-<missing>}"
  printf '  evil_origin=%s\n' "${evil_origin:-<missing>}"
  die
fi
rm -f "$allowed_headers" "$evil_headers"

# Stop the default server before probing the public-bind gate.
cleanup_pid "$SERVER_PID"
SERVER_PID=""

echo ""

# Test 4: Public bind without auth is rejected.
echo "[4] Public bind without auth is rejected"
PUBLIC_LOG="$WORKDIR/public-bind.log"
env -u PORT HOME="$HOME_DIR" "${ROKO_CMD[@]}" serve --workdir "$WORKDIR" --bind 0.0.0.0 >"$PUBLIC_LOG" 2>&1 &
PUBLIC_PID=$!

deadline=$((SECONDS + 20))
while true; do
  state="$(process_state "$PUBLIC_PID")"
  if [[ -z "$state" || "$state" == Z* ]]; then
    break
  fi

  if (( SECONDS >= deadline )); then
    fail "Public bind stayed up on 0.0.0.0 without auth"
    tail -n 40 "$PUBLIC_LOG" | tr -d '\r' || true
    cleanup_pid "$PUBLIC_PID"
    PUBLIC_PID=""
    die
  fi

  sleep 0.25
done

public_exit=0
if wait "$PUBLIC_PID"; then
  public_exit=0
else
  public_exit=$?
fi
PUBLIC_PID=""
public_output="$(tail -n 40 "$PUBLIC_LOG" 2>/dev/null | tr -d '\r')"
if [[ "$public_exit" -ne 0 && "$public_output" == *"Public bind requires"* && "$public_output" == *"acknowledge_public_risk"* && "$public_output" == *"auth.enabled = true"* ]]; then
  pass "Public bind is rejected unless auth or explicit acknowledgement is enabled"
else
  fail "Public bind did not fail with the expected safety message"
  printf '  exit_code=%s\n' "$public_exit"
  printf '  output=%s\n' "$public_output"
  die
fi

echo ""
summary
if (( FAIL > 0 )); then
  exit 1
fi
exit 0
