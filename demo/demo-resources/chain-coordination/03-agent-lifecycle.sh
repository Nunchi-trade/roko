#!/usr/bin/env bash
# 03-agent-lifecycle.sh — Walk a bounty through the full state machine:
#   postJob -> assign -> submit -> resolve (via deployer as resolver)
source "$(dirname "$0")/common.sh"

require_cast
require_python

header "Bounty Lifecycle Demo"

# ── Setup: ensure workers are registered and funded ───────────────────────────
info "Setting up: minting DAEJI, registering workers, posting bounty..."
echo

BOND=$(ether_to_wei 1000)
MINT_AMOUNT=$(ether_to_wei 50000)
DEADLINE=$(future_deadline 7200)

# Mint + register workers if not already registered
for PK in "$DEPLOYER_PK" "$ACCOUNT1_PK"; do
    ADDR=$(cast wallet address "$PK")
    cast_send "$DAEJI" "mint(address,uint256)" "$ADDR" "$MINT_AMOUNT" \
        --private-key "$DEPLOYER_PK" > /dev/null
    cast_send "$DAEJI" "approve(address,uint256)" "$WORKER_REGISTRY" "$BOND" \
        --private-key "$PK" > /dev/null
    # register may fail if already registered — that is fine
    cast_send "$WORKER_REGISTRY" "register(uint256)" "$BOND" \
        --private-key "$PK" > /dev/null 2>&1 || true
done

# Ensure deployer is resolver (it is by default, but be explicit)
info "Ensuring deployer is the BountyMarket resolver..."
cast_send "$BOUNTY_MARKET" "setResolver(address)" "$DEPLOYER_ADDR" \
    --private-key "$DEPLOYER_PK" > /dev/null 2>&1 || true

# Approve + post bounty
BOUNTY_AMT=$(ether_to_wei 5)
cast_send "$DAEJI" "approve(address,uint256)" "$BOUNTY_MARKET" "$BOUNTY_AMT" \
    --private-key "$DEPLOYER_PK" > /dev/null

# ── Step 1: postJob ──────────────────────────────────────────────────────────
header "Step 1: Post Job"
SPEC_HASH=$(cast keccak "oracle-price-feed-update-v1")
cast_send "$BOUNTY_MARKET" \
    "postJob(bytes32,uint256,uint64,uint8)(uint256)" \
    "$SPEC_HASH" "$BOUNTY_AMT" "$DEADLINE" 2 \
    --private-key "$DEPLOYER_PK" > /dev/null

# Figure out the job ID (nextJobId - 1)
NEXT_ID=$(cast_call "$BOUNTY_MARKET" "nextJobId()(uint256)")
JOB_ID=$(( NEXT_ID - 1 ))
STATE=$(cast_call "$BOUNTY_MARKET" "stateOf(uint256)(uint8)" "$JOB_ID")
ok "Job #$JOB_ID posted (state=$STATE, expected 2=Funded)"

# ── Step 2: assign ───────────────────────────────────────────────────────────
header "Step 2: Assign Worker"
info "Assigning Account #1 ($ACCOUNT1_ADDR) to job #$JOB_ID..."
cast_send "$BOUNTY_MARKET" "assign(uint256,address)" "$JOB_ID" "$ACCOUNT1_ADDR" \
    --private-key "$DEPLOYER_PK" > /dev/null

STATE=$(cast_call "$BOUNTY_MARKET" "stateOf(uint256)(uint8)" "$JOB_ID")
ok "Job #$JOB_ID assigned (state=$STATE, expected 3=Assigned)"

# ── Step 3: submit ───────────────────────────────────────────────────────────
header "Step 3: Submit Result"
RESULT_HASH=$(cast keccak "oracle-feed-updated-with-3-sources")
info "Worker submitting result hash..."
cast_send "$BOUNTY_MARKET" "submit(uint256,bytes32)" "$JOB_ID" "$RESULT_HASH" \
    --private-key "$ACCOUNT1_PK" > /dev/null

STATE=$(cast_call "$BOUNTY_MARKET" "stateOf(uint256)(uint8)" "$JOB_ID")
ok "Job #$JOB_ID submitted (state=$STATE, expected 4=Submitted)"

# ── Step 4: resolve ──────────────────────────────────────────────────────────
header "Step 4: Resolve (Accept)"
info "Resolver accepting the result..."
cast_send "$BOUNTY_MARKET" "resolve(uint256,bool)" "$JOB_ID" true \
    --private-key "$DEPLOYER_PK" > /dev/null

STATE=$(cast_call "$BOUNTY_MARKET" "stateOf(uint256)(uint8)" "$JOB_ID")
ok "Job #$JOB_ID resolved (state=$STATE, expected 5=Terminal)"

# ── Step 5: verify full job struct ───────────────────────────────────────────
header "Step 5: Verify Final State"
JOB_DATA=$(cast_call "$BOUNTY_MARKET" \
    "getJob(uint256)((address,uint256,uint64,uint8,bytes32,address,bytes32,uint8,bool))" \
    "$JOB_ID")
info "Full job data:"
dim "  $JOB_DATA"

# Parse accepted flag (last field)
ACCEPTED=$(echo "$JOB_DATA" | python3 -c "
import sys
data = sys.stdin.read().strip()
# The last field in the tuple is the bool 'accepted'
print('true' if 'true' in data.lower() else 'false')
")
ok "Job #$JOB_ID accepted=$ACCEPTED"

# Check worker reputation was updated
WORKER_REP=$(cast_call "$WORKER_REGISTRY" "reputationOf(address)(uint256)" "$ACCOUNT1_ADDR")
ok "Worker reputation after success: $WORKER_REP (scale: 1_000_000 = 1.0)"

header "Done"
ok "Full lifecycle: postJob -> assign -> submit -> resolve(accept)"
echo
info "State transitions: None(0) -> Funded(2) -> Assigned(3) -> Submitted(4) -> Terminal(5)"
