#!/usr/bin/env bash
set -euo pipefail

# Railway (and other PaaS) mount volumes as root. Fix ownership so the
# non-root `mirage` user can write snapshots, then drop privileges.
if [ "$(id -u)" = "0" ]; then
  chown -R mirage:mirage /workspace/.roko 2>/dev/null || true
  exec gosu mirage "$@"
fi

# Already running as mirage (local docker / docker-compose) — just exec.
exec "$@"
