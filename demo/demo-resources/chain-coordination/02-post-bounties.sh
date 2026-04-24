#!/usr/bin/env bash
# 02-post-bounties.sh — Mint DAEJI, approve BountyMarket, post 3 bounties.
source "$(dirname "$0")/common.sh"

require_cast
require_python

header "Bounty Posting"

# ── 1. Register workers in WorkerRegistry (required for BountyMarket.assign) ─
info "Registering workers in WorkerRegistry (bond = 1000 DAEJI each)..."
BOND=$(ether_to_wei 1000)
MINT_AMOUNT=$(ether_to_wei 50000)

for PK in "$DEPLOYER_PK" "$ACCOUNT1_PK" "$ACCOUNT2_PK"; do
    ADDR=$(cast wallet address "$PK")
    # Mint tokens for bonding
    cast_send "$DAEJI" "mint(address,uint256)" "$ADDR" "$MINT_AMOUNT" \
        --private-key "$DEPLOYER_PK" > /dev/null
    # Approve WorkerRegistry to pull bond
    cast_send "$DAEJI" "approve(address,uint256)" "$WORKER_REGISTRY" "$BOND" \
        --private-key "$PK" > /dev/null
    # Register as worker
    cast_send "$WORKER_REGISTRY" "register(uint256)" "$BOND" \
        --private-key "$PK" > /dev/null
    ok "Worker $ADDR registered with $BOND wei bond"
done

# ── 2. Mint DAEJI to poster (Account #0) for bounties ────────────────────────
echo
info "Minting DAEJI to deployer for bounty funding..."
BOUNTY_FUND=$(ether_to_wei 100000)
cast_send "$DAEJI" "mint(address,uint256)" "$DEPLOYER_ADDR" "$BOUNTY_FUND" \
    --private-key "$DEPLOYER_PK" > /dev/null

BALANCE=$(cast_call "$DAEJI" "balanceOf(address)(uint256)" "$DEPLOYER_ADDR")
ok "Deployer DAEJI balance: $BALANCE"

# ── 3. Approve BountyMarket to spend DAEJI ───────────────────────────────────
echo
info "Approving BountyMarket to spend DAEJI..."
APPROVAL=$(ether_to_wei 100000)
cast_send "$DAEJI" "approve(address,uint256)" "$BOUNTY_MARKET" "$APPROVAL" \
    --private-key "$DEPLOYER_PK" > /dev/null
ok "BountyMarket approved for $APPROVAL wei"

# ── 4. Post 3 bounties ───────────────────────────────────────────────────────
echo
DEADLINE=$(future_deadline 7200)  # 2 hours from now

# Bounty 1: perps-liquidate (10 ETH equivalent, minTier=3 Trusted)
info "Posting bounty #1: perps-liquidate (10 DAEJI, Trusted tier)..."
BOUNTY1=$(ether_to_wei 10)
TX1=$(cast_send "$BOUNTY_MARKET" \
    "postJob(bytes32,uint256,uint64,uint8)(uint256)" \
    "$JOB_PERPS_LIQUIDATE" "$BOUNTY1" "$DEADLINE" 3 \
    --private-key "$DEPLOYER_PK")
ok "Bounty #0 posted (perps-liquidate, 10 DAEJI)"
dim "  specHash: $JOB_PERPS_LIQUIDATE"

# Bounty 2: oracle-update (1 ETH equivalent, minTier=3 Trusted)
info "Posting bounty #2: oracle-update (1 DAEJI, Trusted tier)..."
BOUNTY2=$(ether_to_wei 1)
TX2=$(cast_send "$BOUNTY_MARKET" \
    "postJob(bytes32,uint256,uint64,uint8)(uint256)" \
    "$JOB_ORACLE_UPDATE" "$BOUNTY2" "$DEADLINE" 3 \
    --private-key "$DEPLOYER_PK")
ok "Bounty #1 posted (oracle-update, 1 DAEJI)"
dim "  specHash: $JOB_ORACLE_UPDATE"

# Bounty 3: funding-window (5 ETH equivalent, minTier=3 Trusted)
info "Posting bounty #3: funding-window (5 DAEJI, Trusted tier)..."
BOUNTY3=$(ether_to_wei 5)
TX3=$(cast_send "$BOUNTY_MARKET" \
    "postJob(bytes32,uint256,uint64,uint8)(uint256)" \
    "$JOB_FUNDING_WINDOW" "$BOUNTY3" "$DEADLINE" 3 \
    --private-key "$DEPLOYER_PK")
ok "Bounty #2 posted (funding-window, 5 DAEJI)"
dim "  specHash: $JOB_FUNDING_WINDOW"

# ── 5. Verify ────────────────────────────────────────────────────────────────
echo
info "Verifying bounty state..."
NEXT_ID=$(cast_call "$BOUNTY_MARKET" "nextJobId()(uint256)")
ok "nextJobId = $NEXT_ID (3 bounties created)"

for ID in 0 1 2; do
    STATE=$(cast_call "$BOUNTY_MARKET" "stateOf(uint256)(uint8)" "$ID")
    dim "  Job #$ID state = $STATE (2 = Funded)"
done

header "Done"
ok "3 bounties posted and funded on BountyMarket"
