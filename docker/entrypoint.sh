#!/usr/bin/env bash
set -euo pipefail

STATE_DIR="${MIRAGE_STATE_DIR:-/workspace/.roko/state}"
# Parent of state dir (e.g. /data/.roko/state → /data/.roko)
ROKO_ROOT="$(dirname "${STATE_DIR}")"

# Railway (and other PaaS) mount volumes as root. Fix ownership so the
# non-root `mirage` user can write snapshots, then drop privileges.
if [ "$(id -u)" = "0" ]; then
  mkdir -p "${STATE_DIR}"
  chown -R mirage:mirage "${ROKO_ROOT}" 2>/dev/null || true
  exec gosu mirage "$@"
fi

# Already running as mirage (local docker / docker-compose) — just exec.
exec "$@"
