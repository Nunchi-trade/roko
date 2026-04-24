#!/usr/bin/env bash
# run-tui-demo.sh — Run all demo workflows against a live roko serve.
# Populates every TUI panel: agents, plans, tasks, gates, output, jobs,
# episodes, diagnoses, efficiency, event log.
#
# Usage:
#   roko serve --tui          # Terminal 1
#   bash run-tui-demo.sh      # Terminal 2
#
set -euo pipefail

BASE="${ROKO_SERVE_URL:-http://127.0.0.1:6677}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ROKO="${REPO_ROOT}/target/debug/roko"
if [[ ! -x "$ROKO" ]]; then
  ROKO="${REPO_ROOT}/target/release/roko"
fi
if [[ ! -x "$ROKO" ]] && command -v roko &>/dev/null; then
  ROKO="roko"
fi

# ── Colors & formatting ───────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'
BLUE='\033[0;34m'; CYAN='\033[0;36m'; MAGENTA='\033[0;35m'
WHITE='\033[1;37m'; DIM='\033[2m'; ITALIC='\033[3m'
BOLD='\033[1m'; NC='\033[0m'

W=74  # consistent content width

ok()     { echo -e "  ${GREEN}✓${NC} $*"; }
info()   { echo -e "  ${BLUE}ℹ${NC} $*"; }
fail()   { echo -e "  ${RED}✗${NC} $*"; }

step() {
    local num="$1"; shift
    local pad=$(( W - ${#num} - ${#@} - 9 ))
    (( pad < 0 )) && pad=0
    echo ""
    printf "  ${MAGENTA}┌"; printf '─%.0s' $(seq 1 $W); printf "┐${NC}\n"
    printf "  ${MAGENTA}│${NC} ${BOLD}${WHITE}Phase ${num}${NC}  ${BOLD}%s${NC}%*s${MAGENTA}│${NC}\n" "$*" "$pad" ""
    printf "  ${MAGENTA}└"; printf '─%.0s' $(seq 1 $W); printf "┘${NC}\n"
    echo ""
}

narrate() {
    echo -e "  ${DIM}${ITALIC}$*${NC}"
    echo ""
}

countdown() {
    local secs="${1:-3}"; shift
    local msg="${1:-Watch the TUI...}"
    echo ""
    for ((s=secs; s>0; s--)); do
        printf "\r  ${DIM}${ITALIC}%s %d${NC}  " "$msg" "$s"
        sleep 1
    done
    printf "\r%-80s\r" " "
}

hr() {
    printf "  ${DIM}"; printf '─%.0s' $(seq 1 $W); printf "${NC}\n"
}

# ── Stats tracker ─────────────────────────────────────────────────
_S_AGENTS=0; _S_JOBS=0; _S_GP=0; _S_GF=0; _S_RUNS=0
_S_START=$(date +%s)

show_stats() {
    local now; now=$(date +%s)
    local elapsed=$(( now - _S_START ))
    echo ""
    printf "  ${DIM}"; printf '┈%.0s' $(seq 1 $W); printf "${NC}\n"
    printf "  ${DIM}agents${NC} %-4d  ${DIM}jobs${NC} %-4d  ${DIM}gates${NC} ${GREEN}%d${NC}${DIM}✓${NC} ${RED}%d${NC}${DIM}✗${NC}  ${DIM}runs${NC} %-4d  ${DIM}elapsed${NC} %ds\n" \
        "$_S_AGENTS" "$_S_JOBS" "$_S_GP" "$_S_GF" "$_S_RUNS" "$elapsed"
    printf "  ${DIM}"; printf '┈%.0s' $(seq 1 $W); printf "${NC}\n"
}

# ── Box-drawing table ─────────────────────────────────────────────
_TW=(); _TC=0; _TW_EXPLICIT=()

table_widths() { _TW_EXPLICIT=("$@"); }

table_header() {
    _TC=$#; _TW=()
    local cols=("$@")
    if (( ${#_TW_EXPLICIT[@]} > 0 )); then
        _TW=("${_TW_EXPLICIT[@]}"); _TW_EXPLICIT=()
    else
        for col in "${cols[@]}"; do
            local w=${#col}; (( w < 14 )) && w=14; _TW+=("$w")
        done
    fi
    printf "  ${DIM}┌"
    for ((c=0; c<_TC; c++)); do
        printf '─%.0s' $(seq 1 $((_TW[c] + 2)))
        (( c < _TC - 1 )) && printf "┬"
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
        (( c < _TC - 1 )) && printf "┼"
    done
    printf "┤${NC}\n"
}

_trunc() {
    local s="$1" max="$2"
    if (( ${#s} > max )); then echo "${s:0:$((max-1))}…"; else echo "$s"; fi
}

table_row() {
    local cols=("$@")
    printf "  ${DIM}│${NC}"
    for ((c=0; c<_TC; c++)); do
        local val="${cols[$c]:-}"
        local w="${_TW[$c]}"
        # Only truncate plain text (skip ANSI-colored values)
        [[ "$val" != *$'\033'* ]] && val=$(_trunc "$val" "$w")
        printf " %-${w}s ${DIM}│${NC}" "$val"
    done
    printf "\n"
}

table_footer() {
    printf "  ${DIM}└"
    for ((c=0; c<_TC; c++)); do
        printf '─%.0s' $(seq 1 $((_TW[c] + 2)))
        (( c < _TC - 1 )) && printf "┴"
    done
    printf "┘${NC}\n"
}

api() { curl -s -X "$1" "${BASE}$2" -H 'Content-Type: application/json' ${3:+-d "$3"}; }

wait_for_serve() {
  for i in $(seq 1 10); do
    if curl -s "${BASE}/api/health" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  echo "ERROR: roko serve not responding at ${BASE}"
  exit 1
}

# ═════════════════════════════════════════════════════════════════════
#  Banner
# ═════════════════════════════════════════════════════════════════════

echo ""
echo -e "${CYAN}"
cat <<'BANNER'
                ██████╗  ██████╗ ██╗  ██╗ ██████╗
                ██╔══██╗██╔═══██╗██║ ██╔╝██╔═══██╗
                ██████╔╝██║   ██║█████╔╝ ██║   ██║
                ██╔══██╗██║   ██║██╔═██╗ ██║   ██║
                ██║  ██║╚██████╔╝██║  ██╗╚██████╔╝
                ╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═╝ ╚═════╝
BANNER
echo -e "${DIM}            agents that build themselves${NC}"
echo ""
hr
echo -e "  ${DIM}Target${NC}  ${WHITE}${BASE}${NC}"
echo -e "  ${DIM}Mode${NC}    live demo against roko serve + TUI"
hr
echo ""

wait_for_serve
ok "serve is up and healthy"
countdown 2 "Starting demo in"

# ═════════════════════════════════════════════════════════════════════
#  Phase 1: Agent Fleet
# ═════════════════════════════════════════════════════════════════════

step 1 "Register Agent Fleet"
narrate "Deploying 7 specialized agents — zero human coordination"

AGENTS=(
  '{"agent_id":"agent-rustsmith","label":"rustsmith","capabilities":["coding","review"],"skills":["rust","systems","performance"],"tier":"Expert","reputation":96}'
  '{"agent_id":"agent-ethdev","label":"ethdev","capabilities":["coding","research"],"skills":["solidity","evm","defi","security"],"tier":"Expert","reputation":94}'
  '{"agent_id":"agent-fullstack","label":"fullstack","capabilities":["coding","tasks"],"skills":["typescript","react","nodejs","api"],"tier":"Trusted","reputation":88}'
  '{"agent_id":"agent-researcher","label":"researcher","capabilities":["research","tasks"],"skills":["analysis","ml","data-science","papers"],"tier":"Trusted","reputation":91}'
  '{"agent_id":"agent-auditor","label":"auditor","capabilities":["review","security"],"skills":["audit","formal-verification","exploits"],"tier":"Expert","reputation":99}'
  '{"agent_id":"agent-devops","label":"devops","capabilities":["tasks","monitoring"],"skills":["kubernetes","terraform","ci-cd","observability"],"tier":"Trusted","reputation":85}'
  '{"agent_id":"agent-strategist","label":"strategist","capabilities":["research","tasks"],"skills":["mechanism-design","game-theory","tokenomics"],"tier":"Expert","reputation":93}'
)

table_widths 18 9 30 10
table_header "Agent" "Tier" "Skills" "Rep"
for agent in "${AGENTS[@]}"; do
  name=$(echo "$agent" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['agent_id'])")
  tier=$(echo "$agent" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['tier'])")
  skills=$(echo "$agent" | python3 -c "import sys,json; d=json.load(sys.stdin); print(', '.join(d['skills'][:3]))")
  rep=$(echo "$agent" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['reputation'])")
  api POST /api/agents/register "$agent" >/dev/null
  table_row "$name" "$tier" "$skills" "${rep}/100"
  ((_S_AGENTS++)) || true
  sleep 0.3
done
table_footer

echo ""
ok "7 agents deployed and ready"
show_stats
countdown 3 "Watch the TUI update..."

# ═════════════════════════════════════════════════════════════════════
#  Phase 2: DeFi Yield Strategy
# ═════════════════════════════════════════════════════════════════════

step 2 "DeFi Yield Strategy"
narrate "Full job lifecycle: research to delivery — autonomous end-to-end"

JID=$(api POST /api/jobs '{"title":"Design adaptive yield farming strategy for Aave v4","description":"Model risk-adjusted yield across pools, accounting for IL, gas costs, and protocol incentives.","reward":"3200 KORAI","committed_candidates":["agent-strategist","agent-researcher"]}' | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "created job ${DIM}${JID:0:8}...${NC}"
((_S_JOBS++)) || true
sleep 0.5

api POST "/api/jobs/$JID/assign" '{"agent_id":"agent-strategist"}' >/dev/null
ok "assigned → ${BOLD}agent-strategist${NC}"
sleep 0.5

api POST "/api/jobs/$JID/start" >/dev/null
ok "agent working..."
sleep 1

api POST "/api/jobs/$JID/submit" '{"result_summary":"Delivered 3-pool rotation strategy. Backtested over 180 days: 12.4% APY risk-adjusted, max drawdown -3.2%.","artifacts":[{"type":"file","path":"strategy/aave-v4-yield.toml"},{"type":"file","path":"backtest/results.csv"}],"gate_results":[{"gate":"backtest","passed":true},{"gate":"risk-model","passed":true},{"gate":"peer-review","passed":true}]}' >/dev/null
ok "submitted — 3 gates passed"
((_S_GP+=3)) || true
sleep 0.5

api POST "/api/jobs/$JID/evaluate" '{"accepted":true,"feedback":"Strong risk modeling. Strategy approved for paper-trading phase."}' >/dev/null
echo ""
echo -e "  ${GREEN}${BOLD}  ✓ ACCEPTED${NC} — approved for paper-trading"
echo ""
hr
echo -e "  ${DIM}Result${NC}  12.4% APY risk-adjusted · max drawdown -3.2%"
echo -e "  ${DIM}Gates${NC}   backtest ${GREEN}✓${NC}   risk-model ${GREEN}✓${NC}   peer-review ${GREEN}✓${NC}"
hr

show_stats
countdown 3 "Watch the TUI update..."

# ═════════════════════════════════════════════════════════════════════
#  Phase 3: Security Audit
# ═════════════════════════════════════════════════════════════════════

step 3 "Security Audit"
narrate "The system catches what humans miss — gate-driven quality enforcement"

JID=$(api POST /api/jobs '{"title":"Audit BountyMarket.sol for reentrancy and access control","description":"Formal verification of resolve() and assign() paths.","reward":"5000 KORAI"}' | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
ok "created audit job ${DIM}${JID:0:8}...${NC}"
((_S_JOBS++)) || true
sleep 0.3

api POST "/api/jobs/$JID/assign" '{"agent_id":"agent-auditor"}' >/dev/null
api POST "/api/jobs/$JID/start" >/dev/null
ok "assigned → ${BOLD}agent-auditor${NC} (reputation: 99)"
sleep 1

api POST "/api/jobs/$JID/submit" '{"result_summary":"Found 2 medium-severity issues: assign() front-running, resolve() reentrancy via evaluator callback.","gate_results":[{"gate":"slither","passed":true},{"gate":"certora-verify","passed":false},{"gate":"coverage","passed":true}]}' >/dev/null
((_S_GP+=2)) || true
((_S_GF+=1)) || true
echo ""
hr
echo -e "  ${BOLD}Findings${NC}"
echo -e "    ${YELLOW}⚠${NC}  assign() front-running vulnerability"
echo -e "    ${YELLOW}⚠${NC}  resolve() reentrancy via evaluator callback"
echo ""
echo -e "  ${DIM}Gates${NC}   slither ${GREEN}✓${NC}   certora-verify ${RED}✗${NC}   coverage ${GREEN}✓${NC}"
hr
sleep 0.5

api POST "/api/jobs/$JID/evaluate" '{"accepted":false,"feedback":"Formal verification found counterexample. Fix resolve() before re-submission."}' >/dev/null
echo ""
echo -e "  ${RED}${BOLD}  ✗ REJECTED${NC} — formal verification failed, resubmission required"
echo ""
narrate "Gate failures trigger automatic replanning — the system self-corrects"

show_stats
countdown 3 "Watch the TUI update..."

# ═════════════════════════════════════════════════════════════════════
#  Phase 4: Rapid Pipeline
# ═════════════════════════════════════════════════════════════════════

step 4 "Rapid Job Pipeline"
narrate "5 tasks in 30 seconds — parallel pipeline at scale"

TASKS=(
  "Implement WebSocket event streaming"
  "Write Prometheus alerting rules"
  "Build Grafana dashboard for C-factor"
  "Add circuit breaker to Ollama"
  "Refactor CascadeRouter for neuro"
)
AGENTS_POOL=("agent-rustsmith" "agent-fullstack" "agent-devops" "agent-rustsmith" "agent-researcher")

table_widths 36 18 7 8
table_header "Task" "Agent" "Gates" "Result"
for i in "${!TASKS[@]}"; do
  task="${TASKS[$i]}"
  agent="${AGENTS_POOL[$i]}"

  JID=$(api POST /api/jobs "{\"title\":\"$task\",\"description\":\"Quick task\",\"reward\":\"$((1000 + i * 500)) KORAI\"}" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
  api POST "/api/jobs/$JID/assign" "{\"agent_id\":\"$agent\"}" >/dev/null
  api POST "/api/jobs/$JID/start" >/dev/null
  ((_S_JOBS++)) || true
  sleep 0.8

  PASS=$([ $((i % 4)) -ne 3 ] && echo "true" || echo "false")
  api POST "/api/jobs/$JID/submit" "{\"result_summary\":\"Completed: $task\",\"gate_results\":[{\"gate\":\"compile\",\"passed\":true},{\"gate\":\"test\",\"passed\":$PASS},{\"gate\":\"clippy\",\"passed\":true}]}" >/dev/null
  api POST "/api/jobs/$JID/evaluate" "{\"accepted\":$PASS,\"feedback\":\"$([ "$PASS" = "true" ] && echo "Clean implementation" || echo "Tests failing — needs fix")\"}" >/dev/null

  if [ "$PASS" = "true" ]; then
    table_row "$task" "$agent" "3/3" "$(echo -e "${GREEN}PASS${NC}")"
    ((_S_GP+=3)) || true
  else
    table_row "$task" "$agent" "2/3" "$(echo -e "${RED}FAIL${NC}")"
    ((_S_GP+=2)) || true
    ((_S_GF+=1)) || true
  fi
  sleep 0.3
done
table_footer

show_stats
countdown 3 "Watch the TUI update..."

# ═════════════════════════════════════════════════════════════════════
#  Phase 5: Agent Matchmaking
# ═════════════════════════════════════════════════════════════════════

step 5 "Agent Matchmaking"
narrate "Right agent, right task, every time — skill-based routing"

table_widths 34 24 12
table_header "Job" "Required Skills" "Candidates"
for query in \
  '{"title":"Optimize signal router hot path","skills":["rust","performance"],"reward":"2000 KORAI","minTier":"Expert"}' \
  '{"title":"Audit token bridge contract","skills":["security","solidity"],"reward":"4000 KORAI","minTier":"Expert"}' \
  '{"title":"Build monitoring dashboard","skills":["observability","typescript"],"reward":"1500 KORAI","minTier":"Trusted"}' \
; do
  title=$(echo "$query" | python3 -c "import sys,json; print(json.load(sys.stdin)['title'])")
  skills=$(echo "$query" | python3 -c "import sys,json; print(', '.join(json.load(sys.stdin)['skills']))")
  result=$(api POST /api/jobs/match "$query")
  count=$(echo "$result" | python3 -c "import sys,json; print(len(json.load(sys.stdin).get('candidates',[])))" 2>/dev/null || echo "?")
  table_row "$title" "$skills" "${count} matched"
  sleep 0.3
done
table_footer

countdown 2 "Watch the TUI update..."

# ═════════════════════════════════════════════════════════════════════
#  Phase 6: Live Prompt Runs
# ═════════════════════════════════════════════════════════════════════

step 6 "Live Prompt Runs"
narrate "Watch agents think in real-time — from prompt to verified output"

PROMPTS=(
  "Write a one-line Python function to reverse a string"
  "Explain the CAP theorem in 2 sentences"
  "Write a Rust match expression for HTTP status code categories"
)

for prompt in "${PROMPTS[@]}"; do
  short="${prompt:0:50}"
  RID=$(api POST /api/run "{\"prompt\":\"$prompt\"}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('id','?'))" 2>/dev/null)
  ok "dispatched: ${DIM}${short}...${NC} → ${CYAN}${RID:0:8}${NC}"
  ((_S_RUNS++)) || true
  sleep 2
done

countdown 5 "Agents working..."

show_stats
countdown 3 "Watch the TUI update..."

# ═════════════════════════════════════════════════════════════════════
#  Phase 7: Graceful Cancellation
# ═════════════════════════════════════════════════════════════════════

step 7 "Graceful Cancellation"
narrate "Production-grade lifecycle management — clean teardown"

JID=$(api POST /api/jobs '{"title":"Cancelled: prototype was superseded","description":"No longer needed","reward":"500 KORAI"}' | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
api POST "/api/jobs/$JID/assign" '{"agent_id":"agent-fullstack"}' >/dev/null
api POST "/api/jobs/$JID/cancel" >/dev/null
((_S_JOBS++)) || true
ok "created → assigned → cancelled ${DIM}(clean lifecycle)${NC}"

# ═════════════════════════════════════════════════════════════════════
#  Summary
# ═════════════════════════════════════════════════════════════════════

echo ""
echo ""
printf "  ${CYAN}${BOLD}╔"; printf '═%.0s' $(seq 1 $W); printf "╗${NC}\n"
printf "  ${CYAN}${BOLD}║${NC}%-*s${CYAN}${BOLD}║${NC}\n" $W "$(printf '%*s' $(( (W + 14) / 2 )) 'Demo Complete')"
printf "  ${CYAN}${BOLD}╚"; printf '═%.0s' $(seq 1 $W); printf "╝${NC}\n"
echo ""

# Fetch live dashboard state for final summary
SNAP=$(curl -s "${BASE}/api/projections/dashboard" 2>/dev/null)

plans_active=$(echo "$SNAP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',{}).get('stats',{}).get('plans_active',0))" 2>/dev/null || echo "—")
plans_done=$(echo "$SNAP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',{}).get('stats',{}).get('plans_completed',0))" 2>/dev/null || echo "—")
tasks_active=$(echo "$SNAP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',{}).get('stats',{}).get('tasks_active',0))" 2>/dev/null || echo "—")
tasks_done=$(echo "$SNAP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',{}).get('stats',{}).get('tasks_completed',0))" 2>/dev/null || echo "—")
agents_n=$(echo "$SNAP" | python3 -c "import sys,json; print(len(json.load(sys.stdin).get('state',{}).get('agents',{})))" 2>/dev/null || echo "—")
agents_active=$(echo "$SNAP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',{}).get('stats',{}).get('agents_active',0))" 2>/dev/null || echo "—")
gates_pass=$(echo "$SNAP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',{}).get('stats',{}).get('gates_passed',0))" 2>/dev/null || echo "—")
gates_fail=$(echo "$SNAP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',{}).get('stats',{}).get('gates_failed',0))" 2>/dev/null || echo "—")
episodes=$(echo "$SNAP" | python3 -c "import sys,json; print(json.load(sys.stdin).get('state',{}).get('stats',{}).get('episodes_total',0))" 2>/dev/null || echo "—")
cost=$(echo "$SNAP" | python3 -c "import sys,json; print(f\"\${json.load(sys.stdin).get('state',{}).get('stats',{}).get('cost_usd_total',0):.3f}\")" 2>/dev/null || echo "—")
events=$(echo "$SNAP" | python3 -c "import sys,json; print(len(json.load(sys.stdin).get('state',{}).get('event_log',[])))" 2>/dev/null || echo "—")
diagnoses=$(echo "$SNAP" | python3 -c "import sys,json; print(len(json.load(sys.stdin).get('state',{}).get('diagnoses',[])))" 2>/dev/null || echo "—")

NOW=$(date +%s)
ELAPSED=$(( NOW - _S_START ))

table_widths 22 46
table_header "Metric" "Value"
table_row "Agents registered" "${agents_n} (${agents_active} active)"
table_row "Plans" "${plans_active} active / ${plans_done} completed"
table_row "Tasks" "${tasks_active} active / ${tasks_done} completed"
table_row "Gates" "${gates_pass} passed / ${gates_fail} failed"
table_row "Episodes" "$episodes"
table_row "Cost" "$cost"
table_row "Event log" "${events} entries"
table_row "Diagnoses" "$diagnoses"
table_row "Total time" "${ELAPSED}s"
table_footer

echo ""
echo -e "  ${GREEN}${BOLD}✓${NC} ${BOLD}All TUI tabs populated${NC} — check F1 through F9"
echo ""
echo -e "  ${DIM}${ITALIC}Agents that build themselves. Autonomously.${NC}"
echo ""
