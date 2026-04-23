#!/usr/bin/env bash
# 03-learning-loop.sh — Run N iterations to observe cascade router learning
source "$(dirname "$0")/common.sh"

ITERATIONS="${1:-30}"
BATCH_SIZE=10

header "Learning Loop Demo"
info "Iterations: $ITERATIONS (reporting every $BATCH_SIZE)"

WORKSPACE=$(setup_workspace)
trap "cleanup_workspace '$WORKSPACE'" EXIT
cd "$WORKSPACE"

"$ROKO" init 2>/dev/null || true

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

for ((i=1; i<=ITERATIONS; i++)); do
    prompt_idx=$(( (i - 1) % ${#PROMPTS[@]} ))
    prompt="${PROMPTS[$prompt_idx]}"

    # Cycle through available models (let cascade router decide if configured)
    info "[$i/$ITERATIONS] $prompt"

    if roko_run "$prompt" "" "$WORKSPACE"; then
        ((completed++)) || true
    else
        ((failed++)) || true
    fi

    # Report every BATCH_SIZE iterations
    if (( i % BATCH_SIZE == 0 )); then
        header "Progress Report (after $i iterations)"
        info "Completed: $completed | Failed: $failed"
        echo
        "$ROKO" learn router --workdir "$WORKSPACE" 2>/dev/null || warn "Could not read router state"
        echo
    fi
done

header "Final Report"
info "Total: $ITERATIONS | Completed: $completed | Failed: $failed"
echo
"$ROKO" learn router --workdir "$WORKSPACE" 2>/dev/null || true
echo

header "Learning State Files"
ls -la "$WORKSPACE/.roko/learn/" 2>/dev/null || warn "No learning data found"

header "Done"
info "Inspect router state: cat $WORKSPACE/.roko/learn/cascade-router.json | python3 -m json.tool"
info "Visualize: python3 $(dirname "$0")/04-visualize-learning.py $WORKSPACE/.roko/learn/"
