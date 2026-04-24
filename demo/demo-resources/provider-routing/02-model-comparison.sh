#!/usr/bin/env bash
# 02-model-comparison.sh — Run the same prompt through each available model
source "$(dirname "$0")/common.sh"

PROMPT="${1:-Write a function that checks if a number is prime. Keep it under 10 lines.}"
RESULTS_FILE="comparison-results.jsonl"

banner
step 2 "Model Comparison"
narrate "Same prompt, different models — find the fastest, most capable option"

info "Prompt: ${DIM}${PROMPT}${NC}"
echo

WORKSPACE=$(setup_workspace)
trap "cleanup_workspace '$WORKSPACE'" EXIT
cd "$WORKSPACE"

"$ROKO" init 2>/dev/null || true

# Models to test (key from roko.toml).
# Keys are resolved by roko internally (secrets store, env, config) —
# we just try each model and handle failure gracefully.
MODELS=("claude-haiku" "kimi-k2-5" "glm-5-1" "llama3-2")

declare -a STATUS_LIST
declare -a LATENCY_LIST
declare -a MODEL_LIST

best_model=""
best_time=999999

for model in "${MODELS[@]}"; do
    info "Testing model: ${BOLD}$model${NC}"
    start_time=$(date +%s%N)

    if output=$(roko_run "$PROMPT" "$model" "$WORKSPACE" 2>&1); then
        end_time=$(date +%s%N)
        elapsed_ms=$(( (end_time - start_time) / 1000000 ))
        status="PASS"
        if (( elapsed_ms < best_time )); then
            best_time=$elapsed_ms
            best_model=$model
        fi
    else
        end_time=$(date +%s%N)
        elapsed_ms=$(( (end_time - start_time) / 1000000 ))
        status="FAIL"
    fi

    # Write JSONL record
    echo "{\"model\":\"$model\",\"elapsed_ms\":$elapsed_ms,\"status\":\"$status\",\"timestamp\":\"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"}" >> "$RESULTS_FILE"
    MODEL_LIST+=("$model")
    LATENCY_LIST+=("$elapsed_ms")
    STATUS_LIST+=("$status")
done

echo
table_set_widths 18 12 10 10
table_header "Model" "Latency" "Status" "Rating"
for i in "${!MODEL_LIST[@]}"; do
    model="${MODEL_LIST[$i]}"
    ms="${LATENCY_LIST[$i]}"
    stat="${STATUS_LIST[$i]}"

    # Color-code latency
    if [[ "$stat" == "FAIL" ]]; then
        color_stat=$(echo -e "${RED}FAIL${NC}")
        rating="—"
    elif (( ms < 2000 )); then
        color_stat=$(echo -e "${GREEN}PASS${NC}")
        rating=$(echo -e "${GREEN}★★★${NC}")
    elif (( ms < 5000 )); then
        color_stat=$(echo -e "${GREEN}PASS${NC}")
        rating=$(echo -e "${YELLOW}★★☆${NC}")
    else
        color_stat=$(echo -e "${GREEN}PASS${NC}")
        rating=$(echo -e "${RED}★☆☆${NC}")
    fi

    table_row "$model" "${ms}ms" "$color_stat" "$rating"
done
table_footer

if [[ -n "$best_model" ]]; then
    echo
    echo -e "  ${GREEN}${BOLD}★ Winner: ${best_model}${NC} ${DIM}(${best_time}ms)${NC}"
fi

echo
info "Raw results: $WORKSPACE/$RESULTS_FILE"
echo
hr
echo -e "  ${DIM}${ITALIC}Model comparison complete.${NC}"
echo
