# 08 — x402 Micropayments

> x402 is the Coinbase / Linux Foundation protocol for HTTP-native micropayments. Agents
> pay for services by attaching ERC-3009 signed USDC authorizations to HTTP requests.
> No session state, no deposits, no trust required. This document specifies the protocol
> flow, the self-funding agent loop, Rust implementation patterns, and integration with
> the Roko agent runtime.


> **Implementation**: Deferred

---

## 1. Protocol Overview

x402 is built on HTTP 402 Payment Required — a status code that has existed since HTTP/1.1
(RFC 7231, 1999) but was "reserved for future use" for 25 years. The x402 protocol
(Hammond 2025) finally gives it a purpose: machine-to-machine payments at the HTTP layer.

### 1.1 Core Flow

```
Step 1: Client sends request without payment
  POST /v1/messages
  Body: { "model": "claude-sonnet-4-20250514", "messages": [...] }

Step 2: Server responds with payment requirement
  HTTP 402 Payment Required
  X-Payment-Required: {
    "amount": "35000",           // $0.035 USDC (6 decimals)
    "asset": "0x833589...913",   // USDC on Base
    "chain_id": 8453,            // Base L2
    "recipient": "0xGateway...", // Gateway wallet
    "expiry": 1711234567,        // 60 seconds from now
    "nonce": "0xa1b2c3...",      // 32 random bytes (replay protection)
    "intent": "charge"
  }

Step 3: Client signs ERC-3009 authorization and resends
  POST /v1/messages
  X-Payment: {
    "intent": "charge",
    "authorization": {
      "from": "0xClientWallet",
      "to": "0xGateway...",
      "value": "35000",
      "valid_after": "0",
      "valid_before": "1711234567",
      "nonce": "0xa1b2c3...",
      "v": 28, "r": "0x...", "s": "0x..."
    }
  }
  Body: { "model": "claude-sonnet-4-20250514", "messages": [...] }

Step 4: Server verifies off-chain and processes
  - Reconstruct EIP-712 typed data hash
  - ecrecover → verify signer matches "from"
  - Check: to == gateway wallet, value >= minimum, time in window
  - Process request
  - Return response with receipt headers

  HTTP 200 OK
  Payment-Receipt: {"receipt_id":"...","amount_charged":"32400"}
  X-Roko-Cost: 0.032400
  X-Roko-Provider-Cost: 0.027000
  X-Roko-Spread: 0.005400
```

### 1.2 Key Properties

| Property | Value |
|---|---|
| **Payment token** | USDC on Base (chain ID 8453) |
| **Token address** | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` |
| **Decimals** | 6 (1,000,000 base units = $1.00) |
| **Minimum payment** | ~$0.001 (1,000 base units) |
| **Verification** | Off-chain (no RPC call needed) |
| **Settlement** | Batched on-chain (operator settles periodically) |
| **Replay protection** | Per-request 32-byte nonce |
| **Expiry** | 60 seconds default |

### 1.3 ERC-3009: transferWithAuthorization

The payment primitive is ERC-3009's `transferWithAuthorization` — a gasless token transfer
where the sender signs an EIP-712 typed data message off-chain, and a relayer submits it
on-chain later:

```solidity
function transferWithAuthorization(
    address from,
    address to,
    uint256 value,
    uint256 validAfter,
    uint256 validBefore,
    bytes32 nonce,
    uint8 v, bytes32 r, bytes32 s
) external;
```

The EIP-712 domain for USDC on Base:
- name: `"USD Coin"`
- version: `"2"`
- chainId: `8453`
- verifyingContract: `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`

---

## 2. The Self-Funding Agent Loop

The most powerful application of x402 is the self-funding agent — an agent that earns
revenue from its work and uses that revenue to pay for its own inference and tools.

### 2.1 The Loop

```
Agent earns revenue:
  ├── Knowledge marketplace sales (USDC via x402)
  ├── Job completion payments (USDC via ERC-8183)
  ├── Verification services (USDC via x402)
  └── Oracle provision (USDC via x402)
        │
        ▼
Agent spends revenue:
  ├── Inference calls (USDC via x402 to gateway)
  ├── MCP tool usage (USDC via x402 to tool providers)
  ├── Knowledge purchases (USDC via x402 to sellers)
  └── KORAI staking (USDC → KORAI swap → stake)
        │
        ▼
Agent produces more work:
  ├── Better knowledge (higher-value marketplace listings)
  ├── Better task performance (higher reputation → more jobs)
  └── Better predictions (higher accuracy → more oracle demand)
        │
        ▼
Cycle repeats (self-sustaining)
```

### 2.2 Self-Sustainability Threshold

An agent becomes self-sustaining when its revenue exceeds its costs:

```
Revenue per day:
  Knowledge sales:        $2.00/day (20 sales × $0.10 avg)
  Job completions:        $5.00/day (5 jobs × $1.00 avg)
  Verification services:  $1.00/day (100 verifications × $0.01)
  Total daily revenue:    $8.00/day

Cost per day:
  Inference (sonnet-tier): $3.50/day (50 calls × $0.07 avg)
  MCP tools:               $0.50/day
  Knowledge purchases:     $0.30/day
  KORAI demurrage:         $0.01/day
  Total daily cost:        $4.31/day

Net:                       +$3.69/day (self-sustaining)
```

### 2.3 Cold Start Problem

A new agent has no revenue stream. The operator must fund the agent's initial operations
until it builds enough reputation and knowledge to become self-sustaining.

**Bootstrap funding model**:
1. Operator deposits $50-100 USDC into the agent's wallet.
2. Agent uses funds for initial knowledge acquisition and task completion.
3. As reputation builds, job win rate increases.
4. Typically reaches self-sustainability within 30-60 days of active operation.

---

## 3. Rust Implementation

### 3.1 x402 Client

```rust
use alloy::primitives::{Address, U256, Bytes};
use alloy::signers::local::PrivateKeySigner;

/// x402 payment client for a Roko agent.
/// Handles the 402 challenge-response flow automatically.
pub struct X402Client {
    /// The agent's signing key for ERC-3009 authorizations.
    signer: PrivateKeySigner,

    /// The agent's USDC balance (tracked locally, verified periodically).
    balance: u64,

    /// HTTP client for API calls.
    http: reqwest::Client,
}

impl X402Client {
    /// Send a request with automatic x402 payment handling.
    /// If the server returns 402, sign the payment and retry.
    pub async fn send_with_payment(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, X402Error> {
        // Step 1: Try without payment
        let response = self.http.post(url)
            .json(body)
            .send()
            .await?;

        if response.status() != reqwest::StatusCode::PAYMENT_REQUIRED {
            return Ok(response); // No payment needed
        }

        // Step 2: Parse payment requirement
        let payment_req: PaymentRequired = response.json().await?;

        // Step 3: Check balance
        if payment_req.amount > self.balance {
            return Err(X402Error::InsufficientBalance {
                required: payment_req.amount,
                available: self.balance,
            });
        }

        // Step 4: Sign ERC-3009 authorization
        let authorization = self.sign_erc3009(
            payment_req.recipient,
            payment_req.amount,
            payment_req.nonce,
            payment_req.expiry,
        ).await?;

        // Step 5: Resend with payment
        let response = self.http.post(url)
            .header("X-Payment", serde_json::to_string(&PaymentCredential {
                intent: "charge".to_string(),
                authorization,
            })?)
            .json(body)
            .send()
            .await?;

        // Step 6: Track cost from receipt
        if let Some(receipt) = response.headers().get("Payment-Receipt") {
            let receipt: PaymentReceipt = serde_json::from_str(
                receipt.to_str()?
            )?;
            self.track_cost(receipt.amount_charged);
        }

        Ok(response)
    }

    /// Sign an ERC-3009 transferWithAuthorization message.
    async fn sign_erc3009(
        &self,
        to: Address,
        value: u64,
        nonce: Bytes,
        valid_before: u64,
    ) -> Result<Erc3009Authorization, X402Error> {
        use alloy::sol_types::SolStruct;

        // Construct EIP-712 typed data
        let domain = alloy::sol_types::eip712_domain! {
            name: "USD Coin",
            version: "2",
            chain_id: 8453,
            verifying_contract: address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"),
        };

        let transfer = TransferWithAuthorization {
            from: self.signer.address(),
            to,
            value: U256::from(value),
            validAfter: U256::ZERO,
            validBefore: U256::from(valid_before),
            nonce: nonce.into(),
        };

        let signature = self.signer.sign_typed_data(
            &transfer,
            &domain,
        ).await?;

        Ok(Erc3009Authorization {
            from: self.signer.address(),
            to,
            value,
            valid_after: 0,
            valid_before,
            nonce,
            v: signature.v().y_parity_byte() + 27,
            r: signature.r(),
            s: signature.s(),
        })
    }
}
```

### 3.2 x402 Server (Gateway Side)

```rust
/// x402 payment verification middleware.
/// Verifies ERC-3009 signatures without on-chain RPC calls.
pub async fn verify_x402_payment(
    payment: &PaymentCredential,
    expected_recipient: Address,
    minimum_amount: u64,
) -> Result<PaymentContext, X402Error> {
    let auth = &payment.authorization;

    // Verify recipient
    if auth.to != expected_recipient {
        return Err(X402Error::WrongRecipient);
    }

    // Verify amount
    if auth.value < minimum_amount {
        return Err(X402Error::InsufficientPayment);
    }

    // Verify time window
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    if now < auth.valid_after || now > auth.valid_before {
        return Err(X402Error::ExpiredPayment);
    }

    // Verify signature (off-chain ecrecover)
    let domain = usdc_base_domain();
    let transfer = TransferWithAuthorization {
        from: auth.from,
        to: auth.to,
        value: U256::from(auth.value),
        validAfter: U256::from(auth.valid_after),
        validBefore: U256::from(auth.valid_before),
        nonce: auth.nonce.clone(),
    };

    let digest = transfer.eip712_signing_hash(&domain);
    let recovered = alloy::primitives::Signature::from_rs_and_parity(
        auth.r, auth.s, auth.v as u64 - 27
    )?.recover_address_from_prehash(&digest)?;

    if recovered != auth.from {
        return Err(X402Error::InvalidSignature);
    }

    Ok(PaymentContext {
        payer: auth.from,
        authorized_amount: auth.value,
        nonce: auth.nonce.clone(),
    })
}
```

---

## 4. Cost Estimation

The gateway estimates cost before issuing the 402 challenge. The estimate uses model-
specific pricing and a heuristic output multiplier:

### 4.1 Model Pricing Table

| Model | Input $/1M tokens | Output $/1M tokens | Cached $/1M tokens |
|---|---|---|---|
| claude-opus-4-6 | $15.00 | $75.00 | $1.50 |
| claude-sonnet-4-20250514 | $3.00 | $15.00 | $0.30 |
| claude-haiku-4-5-20251001 | $0.80 | $4.00 | $0.08 |
| o3 | $10.00 | $40.00 | $5.00 |
| o4-mini | $1.10 | $4.40 | $0.55 |

### 4.2 Heuristic Output Estimation

The gateway doesn't know output tokens in advance. It uses a heuristic:

```
estimated_output = base_output × (1 + log2(input_tokens / 1000))

base_output by model:
  haiku:  1,024 tokens
  sonnet: 2,048 tokens
  opus:   4,096 tokens
```

The estimate is deliberately conservative (overestimates). The receipt shows actual cost,
and only actual cost is drawn.

---

## 5. Settlement and Batching

### 5.1 Off-Chain Accumulation

The gateway accumulates signed ERC-3009 authorizations without executing them on-chain
immediately. This avoids per-request gas costs.

### 5.2 Batch Settlement

Periodically (every 10 minutes or every 100 accumulated authorizations, whichever comes
first), the gateway batches outstanding authorizations into a single on-chain transaction:

```solidity
// Batch execute multiple transferWithAuthorization calls
function batchSettle(
    TransferAuth[] calldata auths
) external {
    for (uint i = 0; i < auths.length; i++) {
        USDC.transferWithAuthorization(
            auths[i].from,
            auths[i].to,
            auths[i].value,
            auths[i].validAfter,
            auths[i].validBefore,
            auths[i].nonce,
            auths[i].v,
            auths[i].r,
            auths[i].s
        );
    }
}
```

Gas cost: ~65,000 per transfer × N transfers. At N=100, total gas ≈ 6.5M gas. At Base L2
gas prices (~0.01 gwei), total settlement cost ≈ $0.001. Gas cost per request ≈ $0.00001.

### 5.3 Actual vs. Authorized

The `amount_charged` in the receipt reflects actual cost (from real token usage), which
may be less than the authorized amount. The difference is never drawn. Since ERC-3009
authorizations aren't executed until batch settlement, overpayment simply expires unused.

---

## 6. Security Considerations

### 6.1 Replay Protection

Each x402 request generates a fresh 32-byte nonce. The gateway tracks used nonces in a
bloom filter (fast) backed by a database (definitive). Replayed authorizations with the
same nonce are rejected.

### 6.2 Underpayment Protection

The gateway only processes the request after verifying that the authorized amount meets
the estimated cost. If the authorized amount is less than the minimum, the request is
rejected with a new 402 challenge showing the correct amount.

### 6.3 Private Key Security

Agent wallets should be managed via:

- **Hardware Security Module (HSM)** — For production Sovereign agents.
- **AWS KMS / GCP CloudHSM** — For cloud-deployed agents.
- **Software keystore** — For development and Edge-tier agents.
- **ERC-4337 smart wallet** — For agents with social recovery requirements.

Never store raw private keys in environment variables or configuration files. Use the
system's secure keystore.

---

## 7. Orchestration-as-a-Service (OaaS)

x402 enables Orchestration-as-a-Service: any operator can run a Roko-like orchestration
engine as a service, and any agent can pay to use it.

### 7.1 OaaS Flow

```
Agent → POST /api/orchestrate
  Body: { "task": "Build OAuth2 service for Rust/Axum" }

OaaS → 402 Payment Required
  Amount: $0.05 (initial analysis)

Agent → POST /api/orchestrate (with x402 payment)
  → OaaS analyzes, generates PRD, creates plan
  → Returns proposal with cost estimate

Agent → POST /api/accept (with x402 or MPP session for build cost)
  → OaaS executes plan: spawns sub-agents, runs gates, validates
  → Returns completed build artifacts

Settlement:
  Analysis: $0.05 (x402)
  Build: $14.30 (MPP session)
  Total: $14.35
```

### 7.2 Competitive Dynamics

Multiple OaaS providers compete on cost, quality, and speed:

```
Provider Alpha: $42, 3h ETA, 94% gate pass rate, 312 builds
Provider Beta:  $38, 4h ETA, 89% gate pass rate, 87 builds
Provider Gamma: $45, 2h ETA, 97% gate pass rate, 1,204 builds
```

The agent (or its owner) selects based on cost/quality/speed tradeoff. This creates a
permissionless compute economy where agents buy and sell cognitive labor at HTTP speed.

---

## 8. Implementation Status

> **Implementation status (2026-04-12)**: x402 protocol flow is fully specified. ERC-3009
> signature generation and verification are implemented in Rust. Cost estimation heuristics
> are defined. Settlement batching is designed. Self-funding agent loop is specified. OaaS
> integration is designed. Not yet integrated into the Roko runtime. Current agent dispatch
> uses API keys, not x402.

---

## 9. Academic Citations

- Hammond 2025 — x402: HTTP Payments Protocol (Coinbase / Linux Foundation)
- ERC-3009 — transferWithAuthorization (gasless token transfers)
- RFC 7231 — HTTP 402 Payment Required (status code specification)
- Tang et al. 2025 — Micropayment protocols for AI agent commerce
- Google A2A 2025 — Agent-to-Agent communication protocol

---

*Generated from: bardo-backup/prd/shared/x402-protocol.md, bardo-backup/tmp/death/payments/04-payment-mechanics.md,
bardo-backup/tmp/agent-chain/13-orchestration-as-a-service.md, refactoring-prd/09-innovations.md §VIII.
Naming renames applied: mori→Roko, bardo→Roko, golem→agent. Mortality framing removed.*
