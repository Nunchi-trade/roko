# x402 Micropayments for Self-Funding Agents

> x402 is a protocol built on HTTP's 402 Payment Required status code. Agents pay for services with a signed ERC-3009 `transferWithAuthorization` header — no API keys, no accounts, no invoicing. Payment is as fast as the HTTP request that carries it. Enables agent-to-agent commerce at the speed of HTTP.


> **Implementation**: Deferred

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [02-korai-token-economics.md](./02-korai-token-economics.md), [10-spore-job-market.md](./10-spore-job-market.md)
**Key sources**: `refactoring-prd/09-innovations.md` §VIII, `bardo-backup/tmp/agent-chain-new/12-agent-economy.md`

---

## Abstract

The x402 protocol enables micropayments between agents using HTTP's 402 Payment Required status code — a status code that has existed in the HTTP specification since 1997 but was "reserved for future use" until agent commerce gave it a reason to exist. x402 was developed by Coinbase and the Linux Foundation as a standard for machine-to-machine payments.

The core insight: agent-to-agent commerce has fundamentally different requirements than human-to-human commerce. Agents transact at high frequency — hundreds or thousands of requests per hour. They cannot fill out forms, respond to CAPTCHAs, or wait for invoice approvals. They need payment that is as fast and frictionless as the HTTP requests that carry it. x402 achieves this: payment is a header on the request that was already being sent.

---

## Protocol Flow

### Step-by-Step

```
1. Agent sends POST request to an MCP service endpoint
   POST /tools/call
   Content-Type: application/json
   {"tool": "execute_plan", "args": {...}}

2. Server responds HTTP 402 Payment Required
   HTTP/1.1 402 Payment Required
   X-Payment-Required: {
     "amount": "500000000000000000",   // 0.5 KORAI in wei
     "asset": "KORAI",
     "chain": "korai-mainnet",
     "recipient": "0xSERVICE_ADDRESS",
     "validUntil": 1712700000
   }

3. Agent signs ERC-3009 transferWithAuthorization
   - Gasless, off-chain signature
   - Authorizes transfer of 0.5 KORAI to service address
   - Valid for limited time window

4. Agent retries request with payment header
   POST /tools/call
   Content-Type: application/json
   X-Payment: {
     "authorization": "0xSIGNED_ERC3009_AUTH",
     "from": "0xAGENT_ADDRESS",
     "amount": "500000000000000000",
     "nonce": "0xRANDOM_NONCE"
   }
   {"tool": "execute_plan", "args": {...}}

5. Server validates
   - Verify signature against agent's on-chain balance
   - Confirm balance covers the amount
   - Execute the ERC-3009 transfer (or batch at epoch)

6. Server executes work and returns result
   HTTP/1.1 200 OK
   X-Payment-Receipt: {
     "txHash": "0xSETTLEMENT_TX",
     "workProductHash": "0xDELIVERABLE_HASH"
   }
   {"result": {...}}
```

### Key Properties

- **No API keys**: The cryptographic signature is both authentication and payment
- **No accounts**: No registration, no monthly billing, no invoicing
- **No credit checks**: The signature is verified against on-chain balance
- **Atomic**: Payment and service delivery are coupled in a single HTTP round-trip
- **Composable**: Any MCP service can be payment-gated without modifying its core logic

---

## ERC-3009: transferWithAuthorization

ERC-3009 is an Ethereum standard that enables gasless token transfers. Instead of the sender executing an on-chain transaction (which costs gas), the sender signs an off-chain authorization that a third party can later submit on-chain.

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

The signed authorization says: "I (from) authorize the transfer of (value) tokens to (to), valid between (validAfter) and (validBefore), with this unique nonce." Anyone can submit this authorization on-chain, but the tokens move only from `from` to `to`.

### Why ERC-3009?

| Alternative | Problem |
|---|---|
| Direct on-chain transfer | Requires gas from the sender; adds latency for on-chain confirmation |
| Payment channels (Lightning-like) | Requires channel setup, capacity lockup, and channel management |
| ERC-20 approve + transferFrom | Two-step process; approve is an on-chain transaction |
| ERC-3009 | Single off-chain signature; gasless for the sender; batched settlement |

ERC-3009 hits the sweet spot: the agent's payment is a single off-chain signature that can be verified instantly and settled on-chain in batches. The service provider bears the gas cost of settlement (amortized across many payments).

---

## Use Cases in the Korai Ecosystem

### Agent Paying for MCP Services

The primary use case: an agent discovers a specialized MCP service (PRD generator, code reviewer, security auditor) and pays for it per-call.

```
Agent                     MCP Service (Security Auditor)
  │                                │
  ├── POST /tools/call ───────────→│
  │                                │
  │←── 402 Payment Required ───────┤  "0.5 KORAI for security audit"
  │                                │
  ├── POST /tools/call ───────────→│  + X-Payment header
  │    + ERC-3009 signature        │
  │                                │
  │←── 200 OK ─────────────────────┤  Audit results
  │    + X-Payment-Receipt         │
```

### Agent Paying for Knowledge Queries

Querying the Korai knowledge base costs KORAI (see [02-korai-token-economics.md](./02-korai-token-economics.md)). x402 enables pay-per-query:

```
Agent                     Korai Knowledge Node
  │                                │
  ├── korai_queryKnowledge ───────→│
  │                                │
  │←── 402: 0.01 KORAI ───────────┤
  │                                │
  ├── + X-Payment ────────────────→│
  │                                │
  │←── Top-K knowledge results ────┤
```

### Agent Self-Funding Loop

The most powerful pattern: an agent earns KORAI by completing jobs and spends KORAI on services that help it complete more jobs:

```
1. Agent completes a coding job → earns 500 KORAI
2. Agent pays 50 KORAI for a security audit of its work (x402)
3. Agent pays 10 KORAI for knowledge queries to improve next task (x402)
4. Agent pays 5 KORAI to post a knowledge entry from this task (x402)
5. Net: 435 KORAI profit, plus knowledge contribution that earns future rewards
```

The agent is economically autonomous: it earns, spends, invests in knowledge, and compounds its capabilities over time.

---

## Batch Settlement

Individual x402 payments are small (0.01-5 KORAI). Settling each one as a separate on-chain transaction would be gas-wasteful. Instead, service providers batch settlements:

```
1. Collect ERC-3009 authorizations during an epoch
2. At epoch boundary, submit a batch settlement transaction:
   - Single on-chain tx executes all pending transferWithAuthorization calls
   - Gas cost amortized across N payments
3. Emit settlement receipt for each payment
```

Typical batch sizes: 50-200 payments per settlement transaction. At 50 payments per batch, the per-payment gas cost is approximately 1/50th of a single on-chain transfer.

---

## Security Considerations

### Double-Spend Prevention

The ERC-3009 nonce prevents double-spending. Each authorization has a unique nonce; the on-chain contract records used nonces and rejects duplicates. A malicious agent cannot reuse a signed authorization.

### Balance Verification

The service provider verifies the agent's on-chain balance before executing work. If the agent's balance drops between verification and settlement (e.g., another service provider settles first), the settlement transaction reverts for that specific authorization. The service provider bears this risk, mitigated by:
- Short validity windows (validBefore close to current time)
- Balance checks at verification time
- Credit scoring based on agent history

### Replay Protection

ERC-3009 authorizations include `validAfter` and `validBefore` timestamps. An authorization is only valid within its time window. An old authorization captured by a network observer cannot be replayed after `validBefore`.

---

## Academic Foundations

- x402 Protocol. Coinbase/Linux Foundation (2025). — The HTTP-native micropayment protocol for machine-to-machine commerce.
- ERC-3009. "Transfer With Authorization." Ethereum Improvement Proposals. — The gasless transfer standard enabling off-chain signatures.
- ERC-4337. "Account Abstraction Using Alt Mempool." Ethereum Improvement Proposals. — Account abstraction enabling flexible wallet implementations; complements x402 by allowing agents to use smart contract wallets.

---

## Current Status and Gaps

**Scaffold:**
- x402 protocol specification published by Coinbase/Linux Foundation
- ERC-3009 and ERC-4337 widely implemented in Solidity libraries

**Not yet built (Tier 6):**
- x402 client library for Roko agents (§L1)
- x402 server middleware for MCP services (§L2)
- Batch settlement contract for Korai (§L3)
- Balance verification and credit scoring (§L4)
- Integration with Korai token contract (§L5)
- Self-funding agent loop example (§L6)

---

## Cross-references

- See [02-korai-token-economics.md](./02-korai-token-economics.md) for the KORAI token used in payments
- See [10-spore-job-market.md](./10-spore-job-market.md) for the marketplace where agents earn KORAI
- See [17-chain-client-wallet-traits.md](./17-chain-client-wallet-traits.md) for the wallet traits that sign ERC-3009 authorizations
- See topic [05-tools](../18-tools/INDEX.md) for MCP service definitions
