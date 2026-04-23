#!/usr/bin/env bash
# 02-model-comparison.sh — Run the same prompt through each available model
source "$(dirname "$0")/common.sh"

PROMPT="${1:-Write a function that checks if a number is prime. Keep it under 10 lines.}"
RESULTS_FILE="comparison-results.jsonl"

header "Model Comparison"
info "Prompt: $PROMPT"

WORKSPACE=$(setup_workspace)
trap "cleanup_workspace '$WORKSPACE'" EXIT
cd "$WORKSPACE"

"$ROKO" init 2>/dev/null || true

# Models to test (key from roko.toml).
# Keys are resolved by roko internally (secrets store, env, config) —
# we just try each model and handle failure gracefully.
MODELS=("claude-haiku" "kimi-k2-5" "glm-5-1" "llama3-2")

declare -a RESULT_ROWS

for model in "${MODELS[@]}"; do
    info "Testing model: $model"
    start_time=$(date +%s%N)

    if output=$(roko_run "$PROMPT" "$model" "$WORKSPACE" 2>&1); then
        end_time=$(date +%s%N)
        elapsed_ms=$(( (end_time - start_time) / 1000000 ))
        status="PASS"
        ok "$model completed in ${elapsed_ms}ms"
    else
        end_time=$(date +%s%N)
        elapsed_ms=$(( (end_time - start_time) / 1000000 ))
        status="FAIL"
        warn "$model failed after ${elapsed_ms}ms"
    fi

    # Write JSONL record
    echo "{\"model\":\"$model\",\"elapsed_ms\":$elapsed_ms,\"status\":\"$status\",\"timestamp\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"}" >> "$RESULTS_FILE"
    RESULT_ROWS+=("$(printf '%-16s %-10s %-10s %s' "$model" "${elapsed_ms}ms" "—" "$status")")
done

header "Results"
printf '%-16s %-10s %-10s %s\n' "Model" "Latency" "Cost" "Status"
echo "────────────────────────────────────────────────────"
for row in "${RESULT_ROWS[@]}"; do
    echo "$row"
done
echo "────────────────────────────────────────────────────"
info "Raw results: $WORKSPACE/$RESULTS_FILE"

header "Done"
