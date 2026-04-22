#!/usr/bin/env bash
set -euo pipefail

STATE_DIR="${MIRAGE_STATE_DIR:-/workspace/.roko/state}"
VOLUME_ROOT="${RAILWAY_VOLUME_MOUNT_PATH:-}"

if [ -n "${VOLUME_ROOT}" ]; then
  VOLUME_ROOT="${VOLUME_ROOT%/}"
  case "${STATE_DIR}" in
    "${VOLUME_ROOT}"|"${VOLUME_ROOT}"/*) ;;
    *)
      STATE_DIR="${VOLUME_ROOT}/state"
      export MIRAGE_STATE_DIR="${STATE_DIR}"
      ;;
  esac
fi

# Parent of state dir (e.g. /data/.roko/state -> /data/.roko)
ROKO_ROOT="$(dirname "${STATE_DIR}")"

# Railway (and other PaaS) mount volumes as root. Fix ownership so the
# non-root `mirage` user can write snapshots, then drop privileges.
if [ "$(id -u)" = "0" ]; then
  mkdir -p "${STATE_DIR}"
  chown -R mirage:mirage "${ROKO_ROOT}" 2>/dev/null || true
  exec gosu mirage "$@"
fi

# Already running as mirage (local docker / docker-compose) -- just exec.
exec "$@"
