# 07 — MPP: Machine Payment Protocol

> MPP (Machine Payment Protocol) provides session-based streaming payments for agent
> operations. While x402 handles per-request micropayments, MPP handles pre-funded sessions
> where an agent deposits once and draws against a balance across many requests. This
> document specifies the protocol mechanics, session lifecycle, budget delegation via SPTs,
> and multi-rail payment support.

---

## 1. Protocol Stack

Three payment protocols operate at different scales. MPP occupies the middle layer:

```
┌──────────────────────────────────────────────┐
│  ERC-8183 Escrow (Trustless Job Payments)    │
│  $10-$500 per job. On-chain deposit.         │
│  Milestone-based release. 48h dispute window.│
├──────────────────────────────────────────────┤
│  MPP Sessions (Pre-Funded Streaming)         │  ← This document
│  $5-$50 per session. Off-chain vouchers.     │
│  Low per-request overhead. 1h default TTL.   │
├──────────────────────────────────────────────┤
│  x402 (Per-Request Micropayments)            │
│  $0.001-$1 per request. No session state.    │
│  ERC-3009 signed authorization. Stateless.   │
└──────────────────────────────────────────────┘
```

### 1.1 When to Use MPP vs. x402 vs. ERC-8183

| Scenario | Protocol | Why |
|---|---|---|
| Single inference call | x402 | No session overhead for one-off requests |
| Draft iteration (5-20 calls) | x402 | Each call is independent; total < $1 |
| Full build run (50-200 calls) | MPP | Session avoids re-signing each request |
| Trustless job between strangers | ERC-8183 | On-chain escrow protects both parties |
| Long-running agent operation | MPP | Pre-funded session with auto-extend |
| Mid-build scope change | x402 | One-off top-up via independent payment |

### 1.2 Composability

The three protocols compose naturally in a typical workflow:

```
1. DRAFTING: x402 per-request ($0.03/call)
   → User iterates on idea, accumulates context
   → Total: $0.45 for 15 draft iterations

2. PROPOSAL: x402 ($0.05 one-off)
   → Agent generates formal proposal with cost estimate

3. BUILD: MPP session ($20 deposit)
   → User opens session, agent draws per-request
   → 150 requests over 90 minutes
   → Total drawn: $14.30

4. ADJUSTMENT: x402 ($5 one-off)
   → Mid-build scope change, independent payment

5. SETTLEMENT: MPP session close
   → $5.70 refunded (unspent balance)
```

---

## 2. Session Lifecycle

### 2.1 Session Open

```
POST /v1/mpp/sessions
Body: {
  "authorization": {
    "from": "0xClientWallet",
    "to": "0xOperatorWallet",
    "value": "10000000",              // $10.00 USDC (6 decimals)
    "valid_after": "0",
    "valid_before": "1711238167",
    "nonce": "0x...",
    "v": 28, "r": "0x...", "s": "0x..."
  }
}

Response: {
  "session_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "funded_amount": "10000000",        // $10.00
  "expires_at": 1711238167,           // TTL: 1 hour default
  "status": "active"
}
```

The gateway verifies the ERC-3009 signature off-chain (no RPC call needed), creates the
session in a DashMap (in-memory) backed by SQLite (persistent), and returns the session ID.

### 2.2 Session Draw

Subsequent requests include an `X-Payment` header with session credentials:

```
POST /v1/messages
X-Payment: {"intent": "session", "session_id": "a1b2c3d4...", "session_op": "draw"}
Body: { "model": "claude-sonnet-4-20250514", "messages": [...] }

Response headers:
  X-Roko-Cost: 0.0234
  X-Roko-Session-Balance: 9.9766
  X-Roko-Session-Draws: 1
  Payment-Receipt: {"receipt_id":"...","amount_charged":"23400"}
```

No new ERC-3009 signature needed per request. The gateway checks session balance, runs
the handler, computes actual cost from token usage, and deducts from session balance.

### 2.3 Session Top-Up

```
POST /v1/mpp/sessions/{id}/top-up
Body: {
  "authorization": { ... ERC-3009 ... "value": "5000000" }  // +$5.00
}

Response: {
  "session_id": "a1b2c3d4...",
  "previous_balance": "2340000",     // $2.34
  "added": "5000000",                // $5.00
  "new_balance": "7340000"           // $7.34
}
```

### 2.4 Session Close

```
DELETE /v1/mpp/sessions/{id}

Response: {
  "session_id": "a1b2c3d4...",
  "total_funded": "15000000",        // $15.00 (original + top-up)
  "total_drawn": "12250000",         // $12.25
  "refund_amount": "2750000",        // $2.75
  "draw_count": 47,
  "status": "settled"
}
```

The operator batches the on-chain settlement: drawn funds transfer to the operator,
refund to the client. Single on-chain transaction.

### 2.5 Session States

```
Active → Exhausted → Active (via top-up)
                  ↘ Expired (TTL elapsed)
                  ↘ Settled (explicitly closed)

States:
  Active:    accepting draws
  Exhausted: balance = 0, can be revived via top-up
  Expired:   TTL elapsed, no more draws
  Settled:   closed, funds distributed
```

A background task calls `expire_stale()` periodically to transition timed-out sessions.

---

## 3. Shared Payment Tokens (SPTs)

SPTs enable budget delegation — the orchestrator issues scoped authorizations to
sub-agents:

### 3.1 SPT Structure

```rust
/// Shared Payment Token — a scoped, time-limited authorization
/// for a sub-agent to spend from the orchestrator's budget.
pub struct SharedPaymentToken {
    /// Unique identifier for this SPT.
    pub spt_id: Uuid,

    /// Parent session this SPT draws from.
    pub parent_session_id: Uuid,

    /// Maximum amount this SPT can spend (USDC base units).
    pub max_amount: u64,

    /// Expiry timestamp (Unix seconds).
    pub expires_at: u64,

    /// Services this SPT is scoped to.
    /// The sub-agent can only spend at these service endpoints.
    pub scoped_to: Vec<ServiceEndpoint>,

    /// Current amount drawn against this SPT.
    pub drawn: u64,

    /// Agent that holds this SPT.
    pub holder: AgentId,
}
```

### 3.2 Budget Delegation Example

```
Orchestrator budget: $15.25 (MPP session)
  ├── Implementer SPT: $8.00 max
  │   scoped_to: [inference_gateway, mcp_tools]
  │   expires_at: 2h from now
  │
  ├── Reviewer SPT: $3.00 max
  │   scoped_to: [inference_gateway]
  │   expires_at: 2h from now
  │
  ├── AutoFixer SPT: $2.00 max
  │   scoped_to: [inference_gateway]
  │   expires_at: 2h from now
  │
  └── Reserve: $2.25 (held by conductor)
      No SPT issued — conductor allocates on demand
```

### 3.3 SPT Constraints

| Constraint | Value | Rationale |
|---|---|---|
| `max_amount` | Hard ceiling | Prevents runaway spending |
| `expires_at` | Typically 2-4h | Limits exposure window |
| `scoped_to` | Service endpoint list | Prevents unauthorized service use |
| `max_sub_agent_budget_pct` | 60% of total | No single agent gets majority |
| `spt_expiry_hours` | 4h default | Configurable in `roko.toml` |

### 3.4 SPT Exhaustion

When a sub-agent exhausts its SPT:

1. Sub-agent stops and reports to the conductor.
2. Conductor options:
   a. Reallocate from reserve pool.
   b. Downgrade model tier (Opus → Sonnet, ~50% cost reduction).
   c. Escalate to user for top-up.
3. If no resolution within 5 minutes, the task is paused (not cancelled).

---

## 4. Multi-Rail Payment Support

MPP supports multiple payment rails through the same session interface:

### 4.1 Supported Rails

| Rail | Use Case | Settlement |
|---|---|---|
| **USDC on Base** | Primary. Agent-to-agent, x402 compatible | Sub-second on Base L2 |
| **USDC on other L2s** | Cross-chain agents | Bridge + settle |
| **Credit card (Stripe)** | Human buyers, non-crypto-native | Stripe instant payout |
| **Lightning** | Bitcoin-native agents | Sub-second |
| **KORAI** | Korai chain native token | 400ms block time |

### 4.2 Multi-Rail Session

A session can accept multiple rails. The operator specifies accepted rails in
configuration:

```toml
# roko.toml
[billing]
accept_proposals = true
funding_modes = ["escrow", "session", "x402"]

[billing.session]
accepted_rails = ["usdc_base", "stripe", "korai"]
min_deposit = 5.00
max_session_duration_hours = 24
auto_close_on_idle_minutes = 30
```

---

## 5. Cost Transparency

Every MPP response includes cost transparency headers:

```
X-Roko-Cost: 0.0234              # What this request actually cost
X-Roko-Naive-Cost: 0.0380        # What it would have cost without caching
X-Roko-Savings: 0.0146           # Cache savings on this request
X-Roko-Cache-Status: semantic-hit # Cache hit type
X-Roko-Provider: anthropic        # Which provider served this request
X-Roko-Tokens-In: 45000
X-Roko-Tokens-Out: 1200
X-Roko-Session-Cost: 4.83         # Cumulative session cost
X-Roko-Session-Savings: 2.32      # Cumulative session savings
X-Roko-Session-Balance: 5.17      # Remaining session balance
```

Cache hits reduce the provider cost directly, and the client sees the savings. Draft
iterations that hit cache (because the user asks similar questions) cost progressively
less. The system rewards iteration.

---

## 6. Multi-Party Revenue Splitting

When a build involves multiple providers, settlement splits automatically:

```
User pays $15.25 for a build:
  → $10.20 to inference gateway (LLM calls)
  → $3.10 to compute provider (execution machines)
  → $1.00 to deployment service (DNS, TLS setup)
  → $0.95 to operator (platform margin)
```

The split happens atomically in a single settlement transaction. For builds that use
marketplace agents (e.g., a security auditor hired from the knowledge marketplace):

```
  → $2.00 to external security auditor agent
  → $0.20 protocol fee (10% of marketplace transaction)
```

---

## 7. The Spread Model

The gateway's revenue comes from a percentage spread on provider costs:

```
total_charge = provider_cost × (1.0 + spread_pct)
```

Default spread: 20%. Reputation tiers reduce the spread:

| ERC-8004 Tier | Requirements | Spread |
|---|---|---|
| None | New, no history | 20% |
| Basic | 5+ completed builds | 18% |
| Verified | 25+ builds, >90% pass rate | 15% |
| Trusted | 100+ builds, >95% pass rate | 12% |
| Sovereign | 500+ builds | 8% |

---

## 8. Refund Mechanics

### 8.1 Session Refunds

```
refund_amount = funded_amount - drawn_amount
```

Unspent balance returns on session close. The operator batches the on-chain transfer.

### 8.2 Escrow Refunds

Completed milestones stay paid. Failed milestones refund after the 48-hour dispute window.
Cancelled builds: in-progress milestones refund in full, completed milestones are
irreversible.

### 8.3 x402 Overpayment

The pre-authorized value in the ERC-3009 signature is an upper bound. The gateway only
draws actual cost. The difference is never touched (authorization expires).

---

## 9. Configuration Reference

```toml
# roko.toml — billing section

[billing]
accept_proposals = true
funding_modes = ["escrow", "session", "x402"]
min_proposal_value = 1.00
max_proposal_value = 500.00
operator_margin_pct = 15

[billing.escrow]
evaluator = "automated"              # automated | peer | manual
release_per_milestone = true
dispute_window_hours = 48

[billing.session]
min_deposit = 5.00
max_session_duration_hours = 24
auto_close_on_idle_minutes = 30

[billing.delegation]
max_sub_agent_budget_pct = 60
spt_expiry_hours = 4
```

---

## 10. Implementation Status

> **Implementation status (2026-04-12)**: MPP protocol is fully specified. Session lifecycle
> (open, draw, top-up, close, settle) is designed. SPT budget delegation is specified.
> Multi-rail support is designed. Cost transparency headers are defined. Spread model with
> reputation tiers is defined. Not yet implemented in the Roko runtime. Payment currently
> uses API keys, not x402/MPP.

---

## 11. Academic Citations

- Hammond 2025 — x402 protocol specification (Coinbase / Linux Foundation)
- ERC-3009 — transferWithAuthorization (gasless USDC transfers)
- ERC-8183 — Agent-to-agent task escrow
- Ponton et al. 2024 — Machine Payment Protocol specification

---

*Generated from: bardo-backup/tmp/death/14-proposals-and-billing.md (mechanism extracted,
mortality framing removed), bardo-backup/tmp/death/15-cost-tracking.md,
bardo-backup/tmp/death/payments/04-payment-mechanics.md. All naming renames applied:
mori→Roko orchestrator, bardo→Roko, golem→agent.*
