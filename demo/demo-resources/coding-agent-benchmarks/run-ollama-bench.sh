#!/usr/bin/env bash
# Run Roko coding-agent benchmark modes through a local Ollama model.
# Usage: bash run-ollama-bench.sh [--model MODEL] [--mode MODE] [--workdir DIR]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../bin/common.sh
source "$SCRIPT_DIR/../bin/common.sh"

MODEL="${BENCH_MODEL:-gemma4:26b-moe-nothink}"
BATCH_SIZE="${BENCH_BATCH_SIZE:-2}"
WORKDIR="$ROKO_REPO_ROOT"
KNOWLEDGE_WORKDIR="$ROKO_REPO_ROOT"
MODES=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --model)
            MODEL="$2"
            shift 2
            ;;
        --batch-size)
            BATCH_SIZE="$2"
            shift 2
            ;;
        --mode)
            MODES+=("$2")
            shift 2
            ;;
        --workdir)
            WORKDIR="$2"
            shift 2
            ;;
        --knowledge-workdir)
            KNOWLEDGE_WORKDIR="$2"
            shift 2
            ;;
        -h|--help)
            sed -n '1,28p' "$0"
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

if [[ "${#MODES[@]}" -eq 0 ]]; then
    MODES=(minimal context neuro)
fi

for mode in "${MODES[@]}"; do
    case "$mode" in
        minimal|context|neuro) ;;
        *) die "unknown mode: $mode" ;;
    esac
done

require_roko
require_python
require_cmd git

if command -v ollama >/dev/null 2>&1; then
    if ! ollama list | awk 'NR > 1 {print $1}' | grep -Fx "$MODEL" >/dev/null 2>&1; then
        warn "Ollama model '$MODEL' was not listed; continuing in case it can be pulled or is aliased"
    fi
else
    warn "ollama command not found; roko run must still be able to reach the configured Ollama provider"
fi

mkdir -p "$WORKDIR/.roko/bench"
MODEL_SLUG="$(printf '%s' "$MODEL" | tr '/:@' '___' | tr -cd '[:alnum:]_.-')"
if [[ -z "$MODEL_SLUG" ]]; then
    MODEL_SLUG="ollama"
fi

log "Ollama coding-agent benchmark workdir: $WORKDIR"
log "Model: $MODEL"
log "Modes: ${MODES[*]}"

for mode in "${MODES[@]}"; do
    printf -v AGENT_COMMAND "%q %q --mode %q --model %q --roko-bin %q --knowledge-workdir %q" \
        "$PYTHON" \
        "$SCRIPT_DIR/roko-ollama-patch-agent.py" \
        "$mode" \
        "$MODEL" \
        "$ROKO" \
        "$KNOWLEDGE_WORKDIR"

    log "Command adapter mode: $mode"
    "$ROKO" bench swe \
        --batch-size "$BATCH_SIZE" \
        --workdir "$WORKDIR" \
        --agent-mode command \
        --agent-command "$AGENT_COMMAND" \
        --report "$WORKDIR/.roko/bench/scores-${MODEL_SLUG}-${mode}.jsonl" \
        --export-predictions "$WORKDIR/.roko/bench/predictions-${MODEL_SLUG}-${mode}.jsonl"
done

log "Ollama benchmark complete"
bash "$SCRIPT_DIR/summarize-bench.sh" --workdir "$WORKDIR"
