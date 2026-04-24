#!/usr/bin/env bash
# 03-learning-loop.sh — Run N iterations to observe cascade router learning
source "$(dirname "$0")/common.sh"

ITERATIONS="${1:-30}"
BATCH_SIZE=10

banner
step 3 "Learning Loop"
narrate "The system learns which models work best — adaptive routing in action"

info "Iterations: ${BOLD}$ITERATIONS${NC} (reporting every $BATCH_SIZE)"
echo

WORKSPACE=$(setup_workspace)
trap "cleanup_workspace '$WORKSPACE'" EXIT
cd "$WORKSPACE"

"$ROKO" init 2>/dev/null || true

# When using the serve API, learning data lives in the server's workdir.
# Resolve the effective learn dir for reporting.
if [[ -n "$ROKO_SERVE_URL" ]]; then
    # Ask the server for its workdir
    SERVER_WORKDIR=$(curl -s "${ROKO_SERVE_URL}/api/status" 2>/dev/null \
        | python3 -c "import sys,json; print(json.load(sys.stdin).get('workdir',''))" 2>/dev/null)
    if [[ -n "$SERVER_WORKDIR" ]]; then
        LEARN_DIR="${SERVER_WORKDIR}/.roko/learn"
        info "Using serve mode — learning data at: ${DIM}$LEARN_DIR${NC}"
    else
        LEARN_DIR="$WORKSPACE/.roko/learn"
        info "Using serve mode (could not resolve server workdir)"
    fi
    REPORT_WORKDIR="${SERVER_WORKDIR:-$WORKSPACE}"
else
    LEARN_DIR="$WORKSPACE/.roko/learn"
    REPORT_WORKDIR="$WORKSPACE"
fi

# Varied task prompts to exercise different complexity levels
PROMPTS=(
    "Write a one-line Python function to reverse a string"
    "Explain the difference between a mutex and a semaphore in 2 sentences"
    "Write a Rust function to compute fibonacci(n) iteratively"
    "What are the SOLID principles? List them briefly"
    "Write a bash one-liner to count lines in all .rs files"
    "Explain TCP vs UDP in 3 bullet points"
    "Write a Go function to check if a string is a palindrome"
    "What is the CAP theorem? One paragraph"
    "Write a Python decorator that times function execution"
    "Explain the difference between stack and heap memory in 2 sentences"
    "Write a TypeScript function to deep-clone an object"
    "What is eventual consistency? Brief explanation"
    "Write a SQL query to find duplicate rows in a table"
    "Explain what a bloom filter is in 3 sentences"
    "Write a Rust match expression to parse HTTP status codes into categories"
)

completed=0
failed=0

hr

for ((i=1; i<=ITERATIONS; i++)); do
    prompt_idx=$(( (i - 1) % ${#PROMPTS[@]} ))
    prompt="${PROMPTS[$prompt_idx]}"
    short="${prompt:0:55}"

    if roko_run "$prompt" "" "$WORKSPACE"; then
        ((completed++)) || true
        printf "\r  ${GREEN}✓${NC} ${DIM}[%d/%d]${NC} %s\n" "$i" "$ITERATIONS" "$short"
    else
        ((failed++)) || true
        printf "\r  ${RED}✗${NC} ${DIM}[%d/%d]${NC} %s\n" "$i" "$ITERATIONS" "$short"
    fi

    # Progress bar
    progress_bar "$i" "$ITERATIONS" ""

    # Report every BATCH_SIZE iterations
    if (( i % BATCH_SIZE == 0 )); then
        echo ""
        echo ""
        echo -e "  ${BOLD}Progress Report${NC} ${DIM}(after $i iterations)${NC}"
        hr
        echo -e "  ${DIM}Completed:${NC} ${GREEN}$completed${NC}   ${DIM}Failed:${NC} ${RED}$failed${NC}   ${DIM}Success rate:${NC} $(( completed * 100 / i ))%"
        echo
        "$ROKO" learn router --workdir "$REPORT_WORKDIR" 2>/dev/null || warn "Could not read router state"
        hr
        echo
    fi
done

echo ""
echo ""
printf "  ${CYAN}${BOLD}╔"; printf '═%.0s' $(seq 1 58); printf "╗${NC}\n"
printf "  ${CYAN}${BOLD}║${NC}%-58s${CYAN}${BOLD}║${NC}\n" "$(printf '%*s' 36 'Learning Complete')"
printf "  ${CYAN}${BOLD}╚"; printf '═%.0s' $(seq 1 58); printf "╝${NC}\n"
echo ""

table_set_widths 22 30
table_header "Metric" "Value"
table_row "Total iterations" "$ITERATIONS"
table_row "Completed" "$completed"
table_row "Failed" "$failed"
table_row "Success rate" "$(( completed * 100 / ITERATIONS ))%"
table_footer

echo
"$ROKO" learn router --workdir "$REPORT_WORKDIR" 2>/dev/null || true
echo

echo -e "  ${BOLD}Learning State Files${NC}"
hr
ls -la "$LEARN_DIR/" 2>/dev/null || warn "No learning data found"
echo

hr
echo -e "  ${DIM}Inspect:${NC}    cat $LEARN_DIR/cascade-router.json | python3 -m json.tool"
echo -e "  ${DIM}Visualize:${NC}  python3 $(dirname "$0")/04-visualize-learning.py $LEARN_DIR/"
echo ""
echo -e "  ${DIM}${ITALIC}The system improves with every iteration.${NC}"
echo
