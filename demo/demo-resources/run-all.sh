#!/usr/bin/env bash
# Run the non-interactive reusable demo checks against an existing roko-serve.
# Usage: bash run-all.sh [base-url]

set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
BASE="${1:-http://127.0.0.1:6677}"

# ── Colors & formatting ───────────────────────────────────────────
CYAN='\033[0;36m'; GREEN='\033[0;32m'; RED='\033[0;31m'
DIM='\033[2m'; ITALIC='\033[3m'; BOLD='\033[1m'; NC='\033[0m'
MAGENTA='\033[0;35m'; WHITE='\033[1;37m'

echo -e "${CYAN}"
cat <<'BANNER'
                ██████╗  ██████╗ ██╗  ██╗ ██████╗
                ██╔══██╗██╔═══██╗██║ ██╔╝██╔═══██╗
                ██████╔╝██║   ██║█████╔╝ ██║   ██║
                ██╔══██╗██║   ██║██╔═██╗ ██║   ██║
                ██║  ██║╚██████╔╝██║  ██╗╚██████╔╝
                ╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═╝ ╚═════╝
BANNER
echo -e "${DIM}         full test suite${NC}"
echo ""

PASS=0
FAIL=0
TOTAL=0
START_TIME=$(date +%s)
declare -a SUITE_NAMES
declare -a SUITE_RESULTS

run_suite() {
    local name="$1"
    shift
    ((TOTAL++)) || true
    SUITE_NAMES+=("$name")

    echo ""
    echo -e "  ${MAGENTA}▸${NC} ${BOLD}[$TOTAL]${NC} $name"

    if "$@"; then
        echo -e "  ${GREEN}  ✓${NC} $name"
        PASS=$((PASS + 1))
        SUITE_RESULTS+=("PASS")
    else
        echo -e "  ${RED}  ✗${NC} $name" >&2
        FAIL=$((FAIL + 1))
        SUITE_RESULTS+=("FAIL")
    fi
}

run_suite "doctor" bash "$DIR/bin/roko-demo" doctor
run_suite "benchmark flow" bash "$DIR/bin/roko-demo" bench
run_suite "seed agents" bash "$DIR/bin/roko-demo" seed-agents "$BASE"
run_suite "dashboard smoke" bash "$DIR/bin/roko-demo" dashboard-smoke "$BASE"
run_suite "workflow registry" bash "$DIR/bin/roko-demo" list

if [[ "${RUN_OLLAMA_BENCH:-0}" == "1" ]]; then
    run_suite "ollama coding benchmark" bash "$DIR/coding-agent-benchmarks/run-ollama-bench.sh"
fi

NOW=$(date +%s)
ELAPSED=$(( NOW - START_TIME ))

echo ""
echo ""

# Box-drawing table helpers (inline for standalone script)
_TW=(); _TC=0
_table_header() {
    _TC=$#; _TW=()
    local cols=("$@")
    for col in "${cols[@]}"; do
        local w=${#col}; (( w < 14 )) && w=14; _TW+=("$w")
    done
    printf "  ${DIM}┌"
    for ((c=0; c<_TC; c++)); do
        printf '─%.0s' $(seq 1 $((_TW[c] + 2)))
        if (( c < _TC - 1 )); then printf "┬"; fi
    done
    printf "┐${NC}\n"
    printf "  ${DIM}│${NC}"
    for ((c=0; c<_TC; c++)); do
        printf " ${BOLD}%-${_TW[$c]}s${NC} ${DIM}│${NC}" "${cols[$c]}"
    done
    printf "\n"
    printf "  ${DIM}├"
    for ((c=0; c<_TC; c++)); do
        printf '─%.0s' $(seq 1 $((_TW[c] + 2)))
        if (( c < _TC - 1 )); then printf "┼"; fi
    done
    printf "┤${NC}\n"
}
_table_row() {
    local cols=("$@")
    printf "  ${DIM}│${NC}"
    for ((c=0; c<_TC; c++)); do
        printf " %-${_TW[$c]}s ${DIM}│${NC}" "${cols[$c]:-}"
    done
    printf "\n"
}
_table_footer() {
    printf "  ${DIM}└"
    for ((c=0; c<_TC; c++)); do
        printf '─%.0s' $(seq 1 $((_TW[c] + 2)))
        if (( c < _TC - 1 )); then printf "┴"; fi
    done
    printf "┘${NC}\n"
}

_table_header "Suite" "Result"
for i in "${!SUITE_NAMES[@]}"; do
    if [[ "${SUITE_RESULTS[$i]}" == "PASS" ]]; then
        _table_row "${SUITE_NAMES[$i]}" "$(echo -e "${GREEN}PASS${NC}")"
    else
        _table_row "${SUITE_NAMES[$i]}" "$(echo -e "${RED}FAIL${NC}")"
    fi
done
_table_footer

echo ""
echo -e "  ${BOLD}Result:${NC} ${GREEN}$PASS passed${NC}, ${RED}$FAIL failed${NC}   ${DIM}(${ELAPSED}s)${NC}"
echo ""

[[ "$FAIL" -eq 0 ]]
