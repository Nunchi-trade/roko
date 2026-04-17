#!/usr/bin/env bash
set -euo pipefail

resolve_first_existing() {
  local candidate
  for candidate in "$@"; do
    if [[ -f "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

CHECKLIST="$(resolve_first_existing \
  "$REPO_ROOT/bardo-backup/tmp/roko-progress/MORI-PARITY-CHECKLIST.md" \
  "$REPO_ROOT/../bardo-backup/tmp/roko-progress/MORI-PARITY-CHECKLIST.md" \
  "$REPO_ROOT/../../../../bardo-backup/tmp/roko-progress/MORI-PARITY-CHECKLIST.md")"

APPENDIX="$(resolve_first_existing \
  "$REPO_ROOT/tmp/ux-followup-runner/context-pack/05-MORI-REFERENCE-APPENDIX.md" \
  "$REPO_ROOT/../../../tmp/ux-followup-runner/context-pack/05-MORI-REFERENCE-APPENDIX.md")"

OUTPUT="$REPO_ROOT/tmp/mori-parity-verified.md"

exec python3 tools/mori-parity-check/verify.py \
  --repo-root "$REPO_ROOT" \
  --checklist "$CHECKLIST" \
  --appendix "$APPENDIX" \
  --output "$OUTPUT" \
  "$@"
