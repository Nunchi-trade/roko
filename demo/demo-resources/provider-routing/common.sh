#!/usr/bin/env bash
set -euo pipefail

# ── Colors ────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
WHITE='\033[1;37m'
DIM='\033[2m'
ITALIC='\033[3m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# ── Basic logging ─────────────────────────────────────────────────
info()  { echo -e "${BLUE}[info]${NC}  $*"; }
ok()    { echo -e "${GREEN}  ✓${NC} $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC}  $*"; }
err()   { echo -e "${RED}[err]${NC}   $*"; }
fail()  { echo -e "${RED}  ✗${NC} $*"; }
header(){ echo -e "\n${BOLD}═══ $* ═══${NC}\n"; }

# ── Presentation helpers ──────────────────────────────────────────

banner() {
    echo -e "${CYAN}"
    cat <<'BANNER'
                ██████╗  ██████╗ ██╗  ██╗ ██████╗
                ██╔══██╗██╔═══██╗██║ ██╔╝██╔═══██╗
                ██████╔╝██║   ██║█████╔╝ ██║   ██║
                ██╔══██╗██║   ██║██╔═██╗ ██║   ██║
                ██║  ██║╚██████╔╝██║  ██╗╚██████╔╝
                ╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═╝ ╚═════╝
BANNER
    echo -e "${DIM}         agents that build themselves${NC}"
    echo ""
}

# step N "Title" — numbered phase header with box drawing
step() {
    local num="$1"; shift
    local title="Phase ${num}  $*"
    local inner_w=70
    local pad=$(( inner_w - ${#title} - 2 ))
    (( pad < 0 )) && pad=0
    echo ""
    printf "  ${MAGENTA}┌"; printf '─%.0s' $(seq 1 $((inner_w + 2))); printf "┐${NC}\n"
    printf "  ${MAGENTA}│${NC} ${BOLD}${WHITE}Phase ${num}${NC}  ${BOLD}%s${NC}%*s ${MAGENTA}│${NC}\n" "$*" "$pad" ""
    printf "  ${MAGENTA}└"; printf '─%.0s' $(seq 1 $((inner_w + 2))); printf "┘${NC}\n"
    echo ""
}

# narrate "investor-facing tagline"
narrate() {
    echo -e "  ${DIM}${ITALIC}$*${NC}"
    echo ""
}

# hr — horizontal rule
hr() {
    printf "  ${DIM}"
    printf '─%.0s' $(seq 1 72)
    printf "${NC}\n"
}

# countdown N "message" — visible pause with countdown
countdown() {
    local secs="${1:-3}"; shift
    local msg="${1:-Watch the TUI...}"
    echo ""
    for ((s=secs; s>0; s--)); do
        printf "\r  ${DIM}${ITALIC}%s %d${NC}  " "$msg" "$s"
        sleep 1
    done
    printf "\r%-60s\r" " "
}

# spinner PID "message" — braille-frame animated spinner (background-safe)
spinner() {
    local pid="$1"; shift
    local msg="${1:-working}"
    local frames=('⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧' '⠇' '⠏')
    local i=0
    while kill -0 "$pid" 2>/dev/null; do
        printf "\r  ${CYAN}${frames[$i]}${NC} ${DIM}%s${NC}" "$msg"
        i=$(( (i + 1) % ${#frames[@]} ))
        sleep 0.1
    done
    printf "\r%-60s\r" " "
}

# progress_bar current total "label"
progress_bar() {
    local cur="$1" total="$2" label="${3:-}"
    local width=40
    local filled=$(( cur * width / total ))
    local empty=$(( width - filled ))
    local pct=$(( cur * 100 / total ))
    local bar=""
    for ((b=0; b<filled; b++)); do bar+="█"; done
    for ((b=0; b<empty; b++)); do bar+="░"; done
    printf "\r  ${CYAN}%s${NC} ${DIM}%3d%%${NC} %s" "$bar" "$pct" "$label"
    if (( cur >= total )); then echo ""; fi
}

# ── Box-drawing table renderer ────────────────────────────────────
# Usage:
#   table_set_widths 20 12 30       (optional — explicit column widths)
#   table_header "Col1" "Col2" "Col3"
#   table_row "val1" "val2" "val3"
#   table_footer
#
# If table_set_widths is called before table_header, those widths are used.
# Otherwise widths are auto-calculated from header names (min 14).
# Values are truncated to fit columns — no overflow.

_TABLE_WIDTHS=()
_TABLE_COLS=0
_TABLE_EXPLICIT_WIDTHS=()

table_set_widths() {
    _TABLE_EXPLICIT_WIDTHS=("$@")
}

table_header() {
    _TABLE_COLS=$#
    _TABLE_WIDTHS=()
    local cols=("$@")

    if (( ${#_TABLE_EXPLICIT_WIDTHS[@]} > 0 )); then
        _TABLE_WIDTHS=("${_TABLE_EXPLICIT_WIDTHS[@]}")
        _TABLE_EXPLICIT_WIDTHS=()
    else
        for col in "${cols[@]}"; do
            local w=${#col}
            (( w < 14 )) && w=14
            _TABLE_WIDTHS+=("$w")
        done
    fi

    # Top border
    printf "  ${DIM}┌"
    for ((c=0; c<_TABLE_COLS; c++)); do
        printf '─%.0s' $(seq 1 $((_TABLE_WIDTHS[c] + 2)))
        if (( c < _TABLE_COLS - 1 )); then printf "┬"; fi
    done
    printf "┐${NC}\n"

    # Header row
    printf "  ${DIM}│${NC}"
    for ((c=0; c<_TABLE_COLS; c++)); do
        printf " ${BOLD}%-${_TABLE_WIDTHS[$c]}s${NC} ${DIM}│${NC}" "${cols[$c]}"
    done
    printf "\n"

    # Separator
    printf "  ${DIM}├"
    for ((c=0; c<_TABLE_COLS; c++)); do
        printf '─%.0s' $(seq 1 $((_TABLE_WIDTHS[c] + 2)))
        if (( c < _TABLE_COLS - 1 )); then printf "┼"; fi
    done
    printf "┤${NC}\n"
}

# _truncate string maxlen — truncate with ellipsis if needed (plain text only)
_truncate() {
    local s="$1" max="$2"
    if (( ${#s} > max )); then
        echo "${s:0:$((max-1))}…"
    else
        echo "$s"
    fi
}

table_row() {
    local cols=("$@")
    printf "  ${DIM}│${NC}"
    for ((c=0; c<_TABLE_COLS; c++)); do
        local val="${cols[$c]:-}"
        local w="${_TABLE_WIDTHS[$c]}"
        # Only truncate if value has no ANSI codes (simple heuristic)
        if [[ "$val" != *$'\033'* ]]; then
            val=$(_truncate "$val" "$w")
        fi
        printf " %-${w}s ${DIM}│${NC}" "$val"
    done
    printf "\n"
}

table_footer() {
    printf "  ${DIM}└"
    for ((c=0; c<_TABLE_COLS; c++)); do
        printf '─%.0s' $(seq 1 $((_TABLE_WIDTHS[c] + 2)))
        if (( c < _TABLE_COLS - 1 )); then printf "┴"; fi
    done
    printf "┘${NC}\n"
}

# ── Cumulative stats tracker ──────────────────────────────────────
_STAT_AGENTS=0
_STAT_JOBS=0
_STAT_GATES_PASS=0
_STAT_GATES_FAIL=0
_STAT_RUNS=0
_STAT_START_TIME="${EPOCHSECONDS:-$(date +%s)}"

stat_add() {
    case "$1" in
        agents)     _STAT_AGENTS=$((_STAT_AGENTS + ${2:-1}));;
        jobs)       _STAT_JOBS=$((_STAT_JOBS + ${2:-1}));;
        gates_pass) _STAT_GATES_PASS=$((_STAT_GATES_PASS + ${2:-1}));;
        gates_fail) _STAT_GATES_FAIL=$((_STAT_GATES_FAIL + ${2:-1}));;
        runs)       _STAT_RUNS=$((_STAT_RUNS + ${2:-1}));;
    esac
}

show_stats() {
    local now="${EPOCHSECONDS:-$(date +%s)}"
    local elapsed=$(( now - _STAT_START_TIME ))
    echo ""
    printf "  ${DIM}"; printf '┈%.0s' $(seq 1 72); printf "${NC}\n"
    printf "  ${DIM}agents${NC} %-4d  ${DIM}jobs${NC} %-4d  ${DIM}gates${NC} ${GREEN}%d${NC}${DIM}✓${NC} ${RED}%d${NC}${DIM}✗${NC}  ${DIM}runs${NC} %-4d  ${DIM}elapsed${NC} %ds\n" \
        "$_STAT_AGENTS" "$_STAT_JOBS" "$_STAT_GATES_PASS" "$_STAT_GATES_FAIL" "$_STAT_RUNS" "$elapsed"
    printf "  ${DIM}"; printf '┈%.0s' $(seq 1 72); printf "${NC}\n"
}

# ── Locate roko binary ───────────────────────────────────────────
find_roko() {
    # Prefer the compiled binary over shell functions/aliases
    local repo_root
    repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
    if [[ -x "${repo_root}/target/release/roko" ]]; then
        echo "${repo_root}/target/release/roko"
    elif [[ -x "${repo_root}/target/debug/roko" ]]; then
        echo "${repo_root}/target/debug/roko"
    elif [[ -x "${CARGO_TARGET_DIR:-target}/release/roko" ]]; then
        echo "${CARGO_TARGET_DIR:-target}/release/roko"
    elif [[ -x "${CARGO_TARGET_DIR:-target}/debug/roko" ]]; then
        echo "${CARGO_TARGET_DIR:-target}/debug/roko"
    elif command -v roko &>/dev/null; then
        echo "roko"
    else
        err "roko binary not found. Run: cargo build -p roko-cli"
        exit 1
    fi
}

ROKO=$(find_roko)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEMO_CONFIG="${SCRIPT_DIR}/roko.toml"

# ── Workspace setup ──────────────────────────────────────────────
setup_workspace() {
    local workspace="${ROKO_WORKDIR:-${1:-$(mktemp -d /tmp/roko-demo-XXXXXX)}}"
    mkdir -p "$workspace"
    # Only copy config if workspace doesn't already have one
    if [[ ! -f "$workspace/roko.toml" ]]; then
        cp "$DEMO_CONFIG" "$workspace/roko.toml"
    fi

    # Source .env if it exists
    if [[ -f "${SCRIPT_DIR}/.env" ]]; then
        set -a
        source "${SCRIPT_DIR}/.env"
        set +a
    fi

    echo "$workspace"
}

# ── Cleanup ──────────────────────────────────────────────────────
cleanup_workspace() {
    local workspace="$1"
    # Only clean up temp directories, not the project workspace
    if [[ -d "$workspace" && "$workspace" == /tmp/roko-demo-* ]]; then
        rm -rf "$workspace"
    fi
}

# ── Serve integration ────────────────────────────────────────────
# When ROKO_SERVE_URL is set, use the serve API for runs instead of local CLI.
ROKO_SERVE_URL="${ROKO_SERVE_URL:-}"

# Run a prompt through roko — uses serve API if available, local CLI otherwise.
# When using the serve API, polls until the job completes so learning data is
# written before the next iteration starts.
roko_run() {
    local prompt="$1"
    local model="${2:-}"
    local workdir="${3:-$PWD}"

    if [[ -n "$ROKO_SERVE_URL" ]]; then
        # Use serve API — events go through live StateHub to TUI
        local body
        if [[ -n "$model" ]]; then
            body=$(printf '{"prompt":"%s","model":"%s"}' "$prompt" "$model")
        else
            body=$(printf '{"prompt":"%s"}' "$prompt")
        fi
        local response
        response=$(curl -s -X POST "${ROKO_SERVE_URL}/api/run" \
            -H 'Content-Type: application/json' \
            -d "$body" 2>/dev/null)
        local run_id
        run_id=$(echo "$response" | python3 -c "import sys,json; print(json.load(sys.stdin).get('id',''))" 2>/dev/null)
        if [[ -z "$run_id" ]]; then
            echo "$response"
            return 1
        fi

        # Poll until finished
        local status=""
        local max_polls=120  # 2 minutes max
        local poll=0
        while (( poll < max_polls )); do
            local status_json
            status_json=$(curl -s "${ROKO_SERVE_URL}/api/run/${run_id}/status" 2>/dev/null)
            local finished
            finished=$(echo "$status_json" | python3 -c "import sys,json; print(json.load(sys.stdin).get('finished', False))" 2>/dev/null)
            if [[ "$finished" == "True" ]]; then
                local success
                success=$(echo "$status_json" | python3 -c "import sys,json; print(json.load(sys.stdin).get('success', False))" 2>/dev/null)
                [[ "$success" == "True" ]]
                return $?
            fi
            sleep 1
            ((poll++))
        done
        return 1  # timed out
    else
        # Local CLI
        local args=("$prompt" --workdir "$workdir")
        if [[ -n "$model" ]]; then
            args+=(--model "$model")
        fi
        "$ROKO" run "${args[@]}" 2>/dev/null
    fi
}

# ── Provider availability check ─────────────────────────────────
check_provider_key() {
    local env_var="$1"
    if [[ -z "${!env_var:-}" ]]; then
        return 1
    fi
    return 0
}
