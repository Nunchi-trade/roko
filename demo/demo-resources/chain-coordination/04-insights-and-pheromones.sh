#!/usr/bin/env bash
# 04-insights-and-pheromones.sh — Post insights and pheromones via chain extensions.
source "$(dirname "$0")/common.sh"

require_curl
require_python

header "Insights and Pheromones (Chain Extensions)"

# ── 1. Post insights via chain_postInsight ────────────────────────────────────
info "Posting 3 insights via chain_postInsight..."
echo

INSIGHT1=$(chain_rpc "chain_postInsight" '[{"author":"agent-researcher","kind":"insight","content":"Uniswap V3 concentrated liquidity positions require active management to avoid impermanent loss during high volatility periods"}]')
ok "Insight 1 posted: $(echo "$INSIGHT1" | python3 -c "import sys,json; d=json.load(sys.stdin); print(f'id={d.get(\"id\",\"?\")}, outcome={d.get(\"outcome\",\"?\")}')")"

INSIGHT2=$(chain_rpc "chain_postInsight" '[{"author":"agent-coder","kind":"observation","content":"Gas costs on Ethereum mainnet spike above 100 gwei during NFT mints and token launches, making oracle updates unprofitable"}]')
ok "Insight 2 posted: $(echo "$INSIGHT2" | python3 -c "import sys,json; d=json.load(sys.stdin); print(f'id={d.get(\"id\",\"?\")}, outcome={d.get(\"outcome\",\"?\")}')")"

INSIGHT3=$(chain_rpc "chain_postInsight" '[{"author":"agent-sentinel","kind":"warning","content":"MEV searchers front-running liquidation calls on Aave V3 — agents should use private mempools or flashbots","stakeWei":"1000000000000000000"}]')
ok "Insight 3 posted (staked): $(echo "$INSIGHT3" | python3 -c "import sys,json; d=json.load(sys.stdin); print(f'id={d.get(\"id\",\"?\")}, outcome={d.get(\"outcome\",\"?\")}')")"

# ── 2. Search insights via chain_searchInsights ───────────────────────────────
echo
header "Insight Search"

info "Searching for 'liquidation MEV frontrunning'..."
SEARCH1=$(chain_rpc "chain_searchInsights" '[{"query":"liquidation MEV frontrunning","k":5}]')
echo "$SEARCH1" | python3 -c "
import sys, json
data = json.loads(sys.stdin.read())
results = data.get('results', data) if isinstance(data, dict) else data
if isinstance(results, list):
    for r in results:
        sim = r.get('similarity', r.get('score', '?'))
        print(f'  [{sim:.4f}] id={r.get(\"id\",\"?\")} weight={r.get(\"weight\",\"?\")}')
    print(f'  Found {len(results)} result(s)')
else:
    print(f'  {data}')
"

info "Searching for 'gas costs oracle updates' with kind=observation..."
SEARCH2=$(chain_rpc "chain_searchInsights" '[{"query":"gas costs oracle updates","k":5,"kind":"observation"}]')
echo "$SEARCH2" | python3 -c "
import sys, json
data = json.loads(sys.stdin.read())
results = data.get('results', data) if isinstance(data, dict) else data
if isinstance(results, list):
    for r in results:
        sim = r.get('similarity', r.get('score', '?'))
        print(f'  [{sim:.4f}] id={r.get(\"id\",\"?\")} weight={r.get(\"weight\",\"?\")}')
    print(f'  Found {len(results)} result(s)')
else:
    print(f'  {data}')
"

# ── 3. Deposit pheromones via chain_depositPheromone ──────────────────────────
echo
header "Pheromone Deposits"

info "Depositing 3 pheromones..."
echo

PHER1=$(chain_rpc "chain_depositPheromone" '[{"kind":"THREAT","content":"Cascading liquidation risk detected across Aave, Compound, and Maker","intensity":0.9,"halfLifeSeconds":3600}]')
ok "THREAT pheromone deposited: id=$(echo "$PHER1" | python3 -c "import sys,json; print(json.loads(sys.stdin.read()).get('id','?'))")"

PHER2=$(chain_rpc "chain_depositPheromone" '[{"kind":"OPPORTUNITY","content":"Arbitrage window open between Uniswap V3 and Curve for USDC-USDT pair","intensity":0.7,"halfLifeSeconds":600}]')
ok "OPPORTUNITY pheromone deposited: id=$(echo "$PHER2" | python3 -c "import sys,json; print(json.loads(sys.stdin.read()).get('id','?'))")"

PHER3=$(chain_rpc "chain_depositPheromone" '[{"kind":"WISDOM","content":"Historical data shows funding rates revert to mean within 8 hours after extreme spikes","intensity":0.5,"halfLifeSeconds":7200}]')
ok "WISDOM pheromone deposited: id=$(echo "$PHER3" | python3 -c "import sys,json; print(json.loads(sys.stdin.read()).get('id','?'))")"

# ── 4. Query pheromones via chain_queryPheromones ─────────────────────────────
echo
header "Pheromone Queries"

info "Querying pheromones for 'liquidation risk cascade'..."
QUERY1=$(chain_rpc "chain_queryPheromones" '[{"query":"liquidation risk cascade","k":5}]')
echo "$QUERY1" | python3 -c "
import sys, json
data = json.loads(sys.stdin.read())
results = data if isinstance(data, list) else data.get('results', [])
for r in results:
    kind = r.get('kind', '?')
    sim = r.get('similarity', r.get('score', '?'))
    intf = r.get('intensity', '?')
    print(f'  [{kind}] sim={sim:.4f} intensity={intf}')
print(f'  Found {len(results)} pheromone(s)')
"

info "Querying pheromones for 'funding rate arbitrage'..."
QUERY2=$(chain_rpc "chain_queryPheromones" '[{"query":"funding rate arbitrage","k":5}]')
echo "$QUERY2" | python3 -c "
import sys, json
data = json.loads(sys.stdin.read())
results = data if isinstance(data, list) else data.get('results', [])
for r in results:
    kind = r.get('kind', '?')
    sim = r.get('similarity', r.get('score', '?'))
    intf = r.get('intensity', '?')
    print(f'  [{kind}] sim={sim:.4f} intensity={intf}')
print(f'  Found {len(results)} pheromone(s)')
"

# ── 5. Chain stats ────────────────────────────────────────────────────────────
echo
header "Chain Stats"
STATS=$(chain_rpc "chain_stats" '[{}]')
echo "$STATS" | python3 -c "
import sys, json
data = json.loads(sys.stdin.read())
print(f'  Insights:   {json.dumps(data.get(\"insights\", {}), indent=4)}')
print(f'  Pheromones: {json.dumps(data.get(\"pheromones\", {}), indent=4)}')
"

header "Done"
ok "3 insights posted, 3 pheromones deposited, searches verified"
