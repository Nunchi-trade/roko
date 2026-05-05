#!/usr/bin/env bash
# demo-knowledge-feedback.sh — End-to-end knowledge-informed dispatch demo.
#
# This script demonstrates that roko's runner v2 queries prior episodes
# and injects "Learned Patterns" into the agent's system prompt on
# subsequent runs. It works in two modes:
#
#   Mode 1 (default): Uses simulated episodes to demonstrate the full loop
#   Mode 2 (--live):  Uses real `roko plan run` with live Claude API calls
#
# Usage:
#   bash scripts/demo-knowledge-feedback.sh          # Simulated (no API needed)
#   bash scripts/demo-knowledge-feedback.sh --live   # Real agents (needs credits)

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

LIVE_MODE=false
[[ "${1:-}" == "--live" ]] && LIVE_MODE=true

echo "=== Knowledge-Informed Dispatch Demo ==="
echo "Mode: $(if $LIVE_MODE; then echo 'LIVE (real Claude API calls)'; else echo 'SIMULATED (no API needed)'; fi)"
echo ""

# ─── Ensure roko builds ──────────────────────────────────────────────────

echo "=== Step 1: Verify roko-cli compiles ==="
cargo check -p roko-cli 2>/dev/null && echo "PASS: roko-cli compiles" || { echo "FAIL"; exit 1; }
echo ""

# ─── Ensure state dirs ───────────────────────────────────────────────────

mkdir -p .roko/{state,learn,neuro,daimon}

if $LIVE_MODE; then
    # ─── LIVE MODE ────────────────────────────────────────────────────────

    echo "=== Step 2: Run Phase 1 plan (creates real episodes) ==="
    rm -f .roko/state/executor.json .roko/state/orchestrator.json
    rm -f .roko/learn/episodes.jsonl

    RUST_LOG=roko_cli::runner=info cargo run -p roko-cli --bin roko -- \
        plan run plans/live-demo-phase1/ 2>&1 | tail -15
    echo ""

    echo "=== Step 3: Show episodes from Phase 1 ==="
    if [ -f .roko/learn/episodes.jsonl ]; then
        cat .roko/learn/episodes.jsonl | python3 -c "
import sys, json
for line in sys.stdin:
    if not line.strip(): continue
    ep = json.loads(line)
    status = 'PASSED' if ep.get('success') else 'FAILED'
    print(f\"  {ep.get('task_id','?')}: {status} (model: {ep.get('model','?')}, cost: \${ep.get('cost_usd',0):.4f})\")
"
    else
        echo "  (no episodes yet — agent may have failed)"
    fi
    echo ""

    echo "=== Step 4: Run Phase 2 plan (should use knowledge from Phase 1) ==="
    rm -f .roko/state/executor.json .roko/state/orchestrator.json

    RUST_LOG=roko_cli::runner=info,roko_cli::runner::agent_stream=debug cargo run -p roko-cli --bin roko -- \
        plan run plans/live-demo-phase2/ 2>&1 | grep -E "knowledge|inject|episode|model_select" | head -10
    echo ""

else
    # ─── SIMULATED MODE ──────────────────────────────────────────────────

    echo "=== Step 2: Simulate Phase 1 episodes ==="
    cat > .roko/learn/episodes.jsonl << 'EOF'
{"kind":"agent_turn","id":"ep-001","timestamp":"2026-04-27T01:00:00Z","agent_id":"live-demo-phase1/T1-add-greeting","task_id":"live-demo-phase1/T1-add-greeting","model":"claude-sonnet-4-6","success":true,"turns":3,"tokens_used":4500,"cost_usd":0.0135,"duration_secs":12.5,"gate_verdicts":[{"gate":"compile","passed":true},{"gate":"clippy","passed":true}],"reasoning_summary":"Used targeted edit to add greeting.rs with pub mod in lib.rs. Kept diff to 2 files, ~15 lines.","failure_reason":null}
{"kind":"agent_turn","id":"ep-002","timestamp":"2026-04-27T01:01:00Z","agent_id":"live-demo-phase1/T2-add-greeting-test","task_id":"live-demo-phase1/T2-add-greeting-test","model":"claude-sonnet-4-6","success":true,"turns":2,"tokens_used":3200,"cost_usd":0.0096,"duration_secs":8.0,"gate_verdicts":[{"gate":"compile","passed":true},{"gate":"clippy","passed":true},{"gate":"test","passed":true}],"reasoning_summary":"Added #[cfg(test)] mod tests with single assert_eq test. cargo test -p roko-std -- greeting passed.","failure_reason":null}
EOF
    echo "  Created 2 simulated episodes in .roko/learn/episodes.jsonl"
    echo ""

    echo "=== Step 3: Show what Phase 1 produced ==="
    cat .roko/learn/episodes.jsonl | python3 -c "
import sys, json
for line in sys.stdin:
    if not line.strip(): continue
    ep = json.loads(line)
    status = 'PASSED' if ep.get('success') else 'FAILED'
    model = ep.get('model', '?')
    cost = ep.get('cost_usd', 0)
    summary = ep.get('reasoning_summary', 'n/a')
    print(f'  {ep[\"task_id\"]}: {status} (model: {model}, \${cost:.4f})')
    print(f'    Insight: {summary}')
    print()
"

    echo "=== Step 4: Query knowledge for a Phase 2 task ==="
    echo ""
    echo "Task: 'Add farewell helper function to roko-std greeting module'"
    echo ""
    echo "The runner calls query_knowledge_for_task() which:"
    echo "  1. Reads .roko/learn/episodes.jsonl"
    echo "  2. Extracts keywords: farewell, helper, function, roko-std, greeting, module"
    echo "  3. Scores each episode by keyword overlap"
    echo "  4. Prioritizes PASSED episodes (learn from success)"
    echo "  5. Formats top matches as prompt section"
    echo ""

    # Demonstrate what the query would return
    python3 << 'PYEOF'
import json

task_title = "Add farewell helper function to roko-std greeting module"
keywords = set(w.lower() for w in task_title.split() if len(w) > 2)

episodes = []
with open(".roko/learn/episodes.jsonl") as f:
    for line in f:
        if not line.strip():
            continue
        episodes.append(json.loads(line))

print("=== Knowledge injected into system prompt: ===")
print()
print("## Learned Patterns from Prior Work")
print()
print("Based on similar tasks this system has completed before:")
print()

scored = []
for ep in episodes:
    text = f"{ep['task_id']} {ep.get('failure_reason', '')} {ep.get('reasoning_summary', '')}".lower()
    overlap = sum(1 for kw in keywords if kw in text)
    if overlap > 0:
        scored.append((overlap, ep))

scored.sort(key=lambda x: (-x[1].get('success', False), -x[0]))

for i, (score, ep) in enumerate(scored[:5]):
    gate = "passed" if ep.get("success") else "failed"
    model = ep.get("model", "unknown")
    insight = ep.get("reasoning_summary") or ep.get("failure_reason") or "No details."
    print(f"{i+1}. **{ep['task_id']}** (gate: {gate}, model: {model})")
    print(f"   Key insight: {insight}")
    print()
PYEOF

fi

echo "=== How the feedback loop works ==="
echo ""
echo "  Task runs → emit_feedback() writes Episode to .roko/learn/episodes.jsonl"
echo "       ↓"
echo "  Next similar task → query_knowledge_for_task() reads episodes"
echo "       ↓"
echo "  Matches found → injected as 'Learned Patterns' in system prompt"
echo "       ↓"
echo "  Agent sees what worked/failed → makes better decisions"
echo ""
echo "  Code locations:"
echo "    Write: crates/roko-cli/src/runner/event_loop.rs (emit_feedback)"
echo "    Query: crates/roko-cli/src/runner/agent_stream.rs (query_knowledge_for_task)"
echo "    Inject: crates/roko-cli/src/runner/event_loop.rs (dispatch_action, ~line 1577)"
echo ""
echo "=== Demo complete ==="
