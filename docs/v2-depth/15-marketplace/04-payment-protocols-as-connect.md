# Payment Protocols as Connect Cells

> Depth for [21-MARKETPLACE.md](../../unified/21-MARKETPLACE.md). Covers the 3-layer payment stack (x402, MPP, ERC-8183), Shared Payment Tokens, multi-rail support, cost transparency, and the self-funding agent loop -- all expressed as Connect Cells with economic semantics.

---

## 1. Three Payment Layers

The agent economy requires payment at three scales, each implemented as a Connect Cell with different lifecycle characteristics (see [02-CELL.md](../../unified/02-CELL.md) for Connect protocol):

```
Layer 3: ERC-8183 Escrow (Trustless Job Payments)
  $10-$500 per job. On-chain deposit. Milestone-based release. 48h dispute.
  Connect Cell: stateful, long-lived, milestone-gated

Layer 2: MPP Sessions (Pre-Funded Streaming)
  $5-$50 per session. Off-chain vouchers. 1h default TTL.
  Connect Cell: stateful, session-scoped, streaming draws

Layer 1: x402 (Per-Request Micropayments)
  $0.001-$1 per request. No session state. ERC-3009 signed auth.
  Connect Cell: stateless, per-request, challenge-response
```

### 1.1 When to Use Which

| Scenario | Protocol | Why |
|---|---|---|
| Single inference call | x402 | No session overhead |
| 5-20 draft iterations | x402 | Each independent, total < $1 |
| 50-200 call build run | MPP | Session avoids re-signing each request |
| Trustless job between strangers | ERC-8183 | On-chain escrow protects both parties |
| Long-running agent operation | MPP | Pre-funded with auto-extend |

---

## 2. x402: Stateless Connect Cell

x402 (Hammond 2025, Coinbase / Linux Foundation) is built on HTTP 402 Payment Required. Each request is independent -- no session state, no deposits, no prior trust.

### 2.1 Protocol Flow

```
1. Client sends request without payment
   POST /v1/messages

2. Server responds with payment requirement
   HTTP 402 Payment Required
   X-Payment-Required: {
     "amount": "35000",        // $0.035 USDC
     "asset": "0x8335...913",  // USDC on Base
     "chain_id": 8453,         // Base L2
     "recipient": "0xGateway",
     "expiry": <now + 60s>,
     "nonce": <32 random bytes>
   }

3. Client signs ERC-3009 transferWithAuthorization, resends
   POST /v1/messages
   X-Payment: { "intent": "charge", "authorization": { ... } }

4. Server verifies off-chain (no RPC call), processes, returns
   HTTP 200 OK
   Payment-Receipt: {"receipt_id":"...","amount_charged":"32400"}
   X-Roko-Cost: 0.032400
```

### 2.2 Key Properties

| Property | Value |
|---|---|
| Payment token | USDC on Base (chain ID 8453) |
| Minimum payment | ~$0.001 (1,000 base units) |
| Verification | Off-chain ecrecover (no RPC) |
| Settlement | Batched on-chain every 10 min or 100 authorizations |
| Replay protection | Per-request 32-byte nonce |
| Expiry | 60 seconds default |

### 2.3 Settlement Batching

The gateway accumulates signed ERC-3009 authorizations without executing on-chain immediately. Batch settlement: ~65,000 gas per transfer x N transfers. At N=100, total ~$0.001 on Base L2. Per-request gas cost: ~$0.00001.

---

## 3. MPP: Stateful Session Connect Cell

MPP (Machine Payment Protocol) provides session-based streaming payments. An agent deposits once and draws against a balance across many requests.

### 3.1 Session Lifecycle

```
OPEN:   POST /v1/mpp/sessions
        Body: { "authorization": { ERC-3009 signed $10 deposit } }
        Response: { "session_id": "abc", "funded_amount": 10000000, "status": "active" }

DRAW:   POST /v1/messages
        X-Payment: { "intent": "session", "session_id": "abc", "session_op": "draw" }
        Response headers: X-Roko-Cost: 0.0234, X-Roko-Session-Balance: 9.9766

TOP-UP: POST /v1/mpp/sessions/abc/top-up
        Body: { "authorization": { ERC-3009 signed $5 } }

CLOSE:  DELETE /v1/mpp/sessions/abc
        Response: { "total_drawn": 12250000, "refund_amount": 2750000 }
```

### 3.2 Session States

```
Active --> Exhausted --> Active (via top-up)
                     \-> Expired (TTL elapsed)
                     \-> Settled (explicitly closed)
```

### 3.3 Cost Transparency Headers

Every MPP response includes full cost breakdown:

```
X-Roko-Cost: 0.0234           # Actual cost this request
X-Roko-Naive-Cost: 0.0380     # What it would cost without caching
X-Roko-Savings: 0.0146        # Cache savings
X-Roko-Cache-Status: semantic-hit
X-Roko-Provider: anthropic
X-Roko-Tokens-In: 45000
X-Roko-Tokens-Out: 1200
X-Roko-Session-Cost: 4.83     # Cumulative session cost
X-Roko-Session-Savings: 2.32  # Cumulative savings
X-Roko-Session-Balance: 5.17  # Remaining balance
```

These headers implement the Observe protocol -- read-only cost observation for every transaction.

---

## 4. ERC-8183: Milestone-Based Escrow Connect Cell

ERC-8183 provides trustless job payments with on-chain escrow. This is the heaviest payment layer, used for jobs between strangers where mutual trust is absent.

### 4.1 Escrow Lifecycle

```
POST JOB:    Requester deposits budget --> escrow. State = Open.
CLAIM JOB:   Winning agent locks domain stake. State = Claimed.
SUBMIT:      Agent submits delivery_hash + evidence. State = PendingVerification.
APPROVE:     Gate pipeline passes --> payment released, stake unlocked.
REJECT:      Gate fails --> slash applied, budget refunded.
TIMEOUT:     Deadline passes --> 2% slash, budget refunded.
DISPUTE:     Agent disputes within 24h --> 3-agent panel votes.
```

### 4.2 DVP (Delivery vs. Payment)

Atomic settlement -- neither party can cheat:
- Delivery accepted AND payment transferred in single atomic operation
- No state where delivery is accepted but payment pending
- No state where payment sent but delivery unverified

---

## 5. Shared Payment Tokens (SPTs)

SPTs are budget-delegation Signals -- an orchestrator creates SPT Signals and distributes to sub-agents. Each SPT is a scoped, time-limited authorization to spend from the orchestrator's budget.

```rust
pub struct SharedPaymentToken {
    pub spt_id: Uuid,
    pub parent_session_id: Uuid,   // MPP session this draws from
    pub max_amount: u64,           // hard spending ceiling (USDC)
    pub expires_at: u64,           // unix timestamp
    pub scoped_to: Vec<ServiceEndpoint>, // allowed services
    pub drawn: u64,                // current amount spent
    pub holder: AgentId,           // sub-agent holding this SPT
}
```

### 5.1 Budget Delegation Example

```
Orchestrator budget: $15.25 (MPP session)
  +-- Implementer SPT: $8.00 max
  |   scoped_to: [inference_gateway, mcp_tools]
  |   expires_at: 2h from now
  +-- Reviewer SPT: $3.00 max
  |   scoped_to: [inference_gateway]
  +-- AutoFixer SPT: $2.00 max
  |   scoped_to: [inference_gateway]
  +-- Reserve: $2.25 (held by conductor, no SPT)
```

### 5.2 Constraints

| Constraint | Value | Rationale |
|---|---|---|
| max_amount | Hard ceiling | Prevents runaway spending |
| expires_at | 2-4h typical | Limits exposure window |
| scoped_to | Service endpoint list | Prevents unauthorized use |
| max single agent | 60% of total | No single agent gets majority |

When a sub-agent exhausts its SPT: (1) report to conductor, (2) conductor options: reallocate from reserve, downgrade model tier, escalate to user for top-up. If unresolved in 5 minutes, task is paused (not cancelled).

---

## 6. Multi-Rail Support

MPP supports multiple payment rails through the same session interface:

| Rail | Use Case | Settlement |
|---|---|---|
| USDC on Base | Primary, x402-compatible | Sub-second on Base L2 |
| USDC on other L2s | Cross-chain agents | Bridge + settle |
| Credit card (Stripe) | Human buyers, non-crypto | Stripe instant payout |
| Lightning | Bitcoin-native agents | Sub-second |
| KORAI | Korai chain native token | 400ms block time |

A Route Cell selects the payment provider based on cost, speed, and counterparty capability.

---

## 7. The Self-Funding Agent Loop

The most powerful application: agents that earn revenue from work and spend it on inference, closing the economic loop.

```
EARN:   Knowledge sales + Job completions + Verification + Oracle provision
  |
  v
STORE:  Balance Signal (USDC in agent wallet)
  |
  v
SPEND:  Inference (x402 to gateway) + MCP tools + Knowledge purchases + KORAI staking
  |
  v
PRODUCE: Better knowledge + Better task performance + Better predictions
  |
  v
EARN MORE (Loop repeats)
```

### 7.1 Break-Even Analysis

**Minimum Viable Agent** (Worker tier, sonnet-tier inference):

```
Revenue per day:
  10 knowledge sales * $0.20 avg     = $2.00
  2 job completions * $2.00 avg      = $4.00
  50 verifications * $0.01            = $0.50
  Total:                                $6.50/day

Costs per day:
  30 sonnet calls * $0.07             = $2.10
  MCP tools                           = $0.30
  Knowledge purchases                 = $0.20
  Compute (hosting)                   = $1.50
  KORAI demurrage (5K stake)          = $0.01
  Total:                                $4.11/day

Net:                                   +$2.39/day
```

### 7.2 Bootstrap Period

| Starting Condition | Months to Break-Even | Bootstrap Cost |
|---|---|---|
| New Edge, no reputation | 2-3 months | $100-200 |
| Worker with 0.5 reputation | 1 month | $50-100 |
| Sovereign with 0.8 reputation | < 1 week | $20-50 |

The key variable is reputation. Higher reputation means more job wins, which means more revenue, which means faster self-sustainability.

---

## 8. Revenue Split and Fee Structure

### 8.1 Gateway Spread Model

```
total_charge = provider_cost * (1.0 + spread_pct)

Spread by tier:
  None (new):      20%
  Basic (5+ builds): 18%
  Verified (25+, >90% pass): 15%
  Trusted (100+, >95% pass): 12%
  Sovereign (500+): 8%
```

### 8.2 Protocol Fee Distribution

```
All fees --> 40% burned (deflationary)
         --> 40% Knowledge Vault (staking pool)
         --> 20% Protocol Treasury (development)
```

---

## What This Enables

- **Self-funding agents**: Agents that earn more than they spend, operating without human capital injection
- **Composable payment layers**: x402 for micro, MPP for sessions, ERC-8183 for jobs -- each optimized for its scale
- **Budget delegation**: Orchestrators can safely distribute spending authority to sub-agents via SPTs
- **Full cost transparency**: Every request carries cost headers, enabling cost optimization
- **Multi-rail flexibility**: Same interface works across USDC, Stripe, Lightning, and KORAI

## Feedback Loops

1. **Revenue-capability Loop**: Revenue funds better inference, producing higher-quality work, earning more revenue
2. **Reputation-spread Loop**: Higher reputation reduces gateway spread (20% to 8%), increasing margin, funding more operations
3. **Cache-savings Loop**: Repeated similar queries hit cache, reducing costs, enabling more iterations within budget
4. **SPT-delegation Loop**: Budget delegation enables parallel agent work, increasing throughput, completing jobs faster, winning more jobs

## Open Questions

1. **Settlement timing**: Batch settlement every 10 minutes means the gateway carries credit risk for up to 10 minutes of accumulated authorizations. Is this acceptable at scale?
2. **SPT revocation**: If the orchestrator needs to revoke an SPT mid-task (e.g., agent misbehaving), what is the mechanism? Current design has no revocation path.
3. **Cross-rail arbitrage**: With multiple payment rails, price differences between USDC/KORAI/Lightning could create arbitrage. Is this a feature or a bug?
4. **Stripe minimum**: Stripe's $0.50 minimum makes low-value Tier 3 transactions impractical via fiat. Should the system auto-batch small Stripe purchases?

## Implementation Tasks

1. **Implement `X402Client`** Connect Cell in `crates/roko-agent/src/` with automatic 402 challenge-response handling
2. **Implement `MppSessionCell`** Connect Cell with session lifecycle (open, draw, top-up, close, settle) in `crates/roko-serve/`
3. **Implement `SharedPaymentToken`** Signal type and delegation logic in `crates/roko-core/src/`
4. **Wire cost transparency headers** into all HTTP responses in `crates/roko-serve/src/routes/`
5. **Implement settlement batching** for x402 authorizations in `crates/roko-serve/` or `crates/roko-chain/`
6. **Add multi-rail Route Cell** that selects payment provider based on cost/speed in `crates/roko-agent/`
7. **Implement self-funding balance tracking** as a Store Cell in `crates/roko-learn/` or `crates/roko-core/`

---

*Absorbs: `docs/14-identity-economy/07-mpp-machine-payment-protocol.md`, `docs/14-identity-economy/08-x402-micropayments.md`, `docs/14-identity-economy/13-isfr-clearing-settlement.md` (payment aspects). On-chain payment mechanics covered in [18-registries/06-payments-and-settlement.md](../18-registries/06-payments-and-settlement.md). This doc covers off-chain payment protocol dynamics, SPTs, and the self-funding loop.*
