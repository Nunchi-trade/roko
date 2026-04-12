# 06 — Wallet Management

> Three custody modes (Delegation, Embedded, LocalKey), WalletHandle abstraction,
> session key management, wallet providers, identity NFT custody.


> **Implementation**: Deferred

---

## Overview

Wallet management is part of the **chain domain plugin** — only agents configured for chain
operations need wallet access. The wallet system supports three custody modes that provide
different security/autonomy tradeoffs, unified behind a `WalletHandle` abstraction that
normalizes all modes to Alloy's `Signer` trait.

---

## Three Custody Modes

### 1. Delegation (Enclave)

Keys live in secure hardware (HSM, TEE, or managed enclave). The agent never touches raw
private keys. Signing requests are sent to the enclave, which returns signed transactions.

```toml
# roko.toml
[wallet]
custody = "delegation"

[wallet.delegation]
# ERC-7710 session key delegation
provider = "enclave"
session_key_ttl = "24h"
max_value_per_session = "10000 USD"
```

**Security properties:**
- Private key never leaves secure hardware
- Session keys (ERC-7715) provide time-limited, value-bounded authorization
- Agent operates with a session key that expires and must be renewed
- Maximum value per session is owner-configurable

**Use case:** Production deployment with real capital.

### 2. Embedded (ERC-4337)

Account abstraction via ERC-4337. The agent operates through a smart contract wallet that
enforces policies on-chain. Providers include Privy, ZeroDev, Safe.

```toml
# roko.toml
[wallet]
custody = "embedded"

[wallet.embedded]
provider = "privy"       # or "zeroDev", "safe"
policy_contract = "0x..."  # PolicyCage address
user_op_bundler = "https://bundler.example.com"
```

**Security properties:**
- On-chain policy enforcement via PolicyCage smart contract
- UserOperation validation before execution
- Multi-sig support (via Safe)
- Gasless transactions (bundler pays gas, agent repays)

**Use case:** Semi-autonomous agents with on-chain guardrails.

### 3. Local Key (Dev)

Plain private key for development and testing. **Never use in production with real funds.**

```toml
# roko.toml
[wallet]
custody = "local_key"

[wallet.local_key]
# Key from environment variable (preferred)
env_var = "ROKO_WALLET_KEY"
# Or hardcoded for tests (DO NOT commit)
# key = "0x..."
```

**Security properties:**
- Minimal — key is in process memory
- No external custody, no hardware protection
- Suitable only for mirage-rs simulation and Anvil testnet

**Use case:** Local development with mirage-rs or Anvil.

---

## WalletHandle Abstraction

All three custody modes are normalized behind `WalletHandle`:

```rust
pub struct WalletHandle {
    /// The custody mode this handle wraps.
    pub mode: CustodyMode,
    /// The signing interface (Alloy Signer trait).
    signer: Arc<dyn Signer>,
    /// Chain-specific configuration.
    chain_config: HashMap<u64, ChainWalletConfig>,
}

#[derive(Debug, Clone)]
pub enum CustodyMode {
    /// Keys in secure hardware (HSM/TEE/enclave).
    Delegation {
        session_key: SessionKey,
        enclave_url: String,
    },
    /// ERC-4337 account abstraction.
    Embedded {
        provider: EmbeddedProvider,
        policy_contract: Address,
    },
    /// Plain private key (dev only).
    LocalKey {
        address: Address,
    },
}

impl WalletHandle {
    /// Sign a transaction. Dispatches to the appropriate custody backend.
    pub async fn sign_transaction(&self, tx: &TransactionRequest) -> Result<SignedTransaction> {
        // All modes normalize to Alloy's Signer trait
        self.signer.sign_transaction(tx).await
    }

    /// Get the wallet's address.
    pub fn address(&self) -> Address {
        match &self.mode {
            CustodyMode::Delegation { session_key, .. } => session_key.address,
            CustodyMode::Embedded { .. } => self.signer.address(),
            CustodyMode::LocalKey { address } => *address,
        }
    }

    /// Check if the wallet can execute a write operation of the given value.
    pub async fn can_execute(&self, value_usd: f64) -> Result<bool> {
        match &self.mode {
            CustodyMode::Delegation { session_key, .. } => {
                Ok(session_key.remaining_value() >= value_usd && !session_key.is_expired())
            }
            CustodyMode::Embedded { policy_contract, .. } => {
                // Check on-chain PolicyCage
                self.check_policy_cage(*policy_contract, value_usd).await
            }
            CustodyMode::LocalKey { .. } => Ok(true), // No limits in dev
        }
    }
}
```

---

## Wallet Providers

Seven wallet providers are supported, each mapping to Alloy's `Signer` trait:

| Provider | Custody Mode | Integration | Notes |
|---|---|---|---|
| **MetaMask** | Delegation | Browser extension / Snap | For user-supervised sessions |
| **Local key** | LocalKey | In-process | Dev/testing only |
| **Privy** | Embedded | HTTP API | Hosted MPC wallet |
| **Safe** | Embedded | Smart contract | Multi-sig support |
| **ZeroDev** | Embedded | HTTP API | ERC-4337 kernel |
| **Lit/Vincent** | Delegation | PKP + MPC | Programmable key pairs |
| **Generic Alloy** | Any | Alloy Signer trait | Custom signer integration |

All providers normalize to the same `Signer` trait interface. The agent code doesn't
distinguish between providers — it calls `wallet.sign_transaction()` regardless of the
underlying custody mechanism.

---

## Session Key Management (ERC-7715)

For Delegation mode, session keys provide time-limited, value-bounded authorization:

```rust
pub struct SessionKey {
    /// The session key address (derived from session keypair).
    pub address: Address,
    /// The delegator's address (the owner's main wallet).
    pub delegator: Address,
    /// Maximum total value this session can authorize (USD).
    pub max_value: f64,
    /// Value already consumed in this session.
    pub consumed_value: f64,
    /// Session expiry timestamp.
    pub expires_at: u64,
    /// Allowed operations (whitelist of tool categories).
    pub allowed_categories: HashSet<Category>,
}

impl SessionKey {
    pub fn remaining_value(&self) -> f64 {
        self.max_value - self.consumed_value
    }

    pub fn is_expired(&self) -> bool {
        chrono::Utc::now().timestamp() as u64 > self.expires_at
    }

    pub fn can_execute(&self, category: &Category, value_usd: f64) -> bool {
        !self.is_expired()
            && self.remaining_value() >= value_usd
            && self.allowed_categories.contains(category)
    }
}
```

Session key lifecycle:
1. **Creation**: Owner delegates a session key with value limit and TTL
2. **Usage**: Agent consumes value against the session key's budget
3. **Expiry**: Session key expires after TTL or when value is exhausted
4. **Renewal**: Agent requests a new session key from the owner

---

## Wallet Management Tools (Chain Domain Plugin)

Four tools manage wallet state:

| Tool | Category | Trust Tier | Description |
|---|---|---|---|
| `wallet_get_status` | wallet | Read | Current balances, session key status, spending limits |
| `wallet_request_session` | wallet | Privileged | Request new session key from owner |
| `wallet_fund` | wallet | Write | Transfer tokens to agent wallet (from authorized source) |
| `wallet_rotate_key` | wallet | Privileged | Rotate session key (emergency) |

---

## Identity NFT Custody

Agent identity is represented on-chain via an ERC-721 soulbound NFT (Korai Passport). The
wallet manages this identity token:

```rust
pub struct AgentIdentity {
    /// ERC-721 token ID on the Korai chain.
    pub token_id: U256,
    /// The agent's on-chain address (wallet address).
    pub address: Address,
    /// Reputation score (accumulated from gate verdicts, mesh attestations).
    pub reputation: f64,
    /// Knowledge contributions (Engrams posted to collective).
    pub contribution_count: u64,
}
```

Identity tools (`identity_verify_agent`, `identity_get_reputation`, etc.) use the wallet's
signer to authenticate on-chain identity queries. The identity NFT is soulbound — it cannot be
transferred, ensuring that reputation is permanently bound to the agent.

---

## Credential Lifecycle

Credentials (API keys, session tokens, wallet keys) follow a managed lifecycle:

```
Creation → Storage → Usage → Rotation → Revocation
    |          |        |         |           |
    v          v        v         v           v
  Mint     TaintedString  Auth   New key    Zeroize
  key      (zeroized      chain  minted,    old key
           on drop)              old revoked
```

All credentials are wrapped in `TaintedString` (see `04-safety-hooks.md`) which ensures
automatic zeroization on drop and prevents sensitive data from flowing to unauthorized sinks
(LLM context, event bus, collective mesh).

---

## Configuration Examples

### Production Chain Agent (Delegation)

```toml
[agent]
domain = "chain"
model = "claude-sonnet-4-20250514"

[tools]
profile = "active"

[wallet]
custody = "delegation"

[wallet.delegation]
provider = "enclave"
session_key_ttl = "24h"
max_value_per_session = "50000 USD"

[wallet.safety]
max_per_tick_usd = 10000.0
max_per_day_usd = 100000.0
require_simulation = true
```

### Development (Local Key + mirage-rs)

```toml
[agent]
domain = "chain"
model = "claude-haiku-4-5-20251001"

[tools]
profile = "dev"

[wallet]
custody = "local_key"

[wallet.local_key]
env_var = "ROKO_WALLET_KEY"

[tools.safety]
require_simulation = false  # mirage-rs handles this
```

### Read-Only Analytics

```toml
[agent]
domain = "chain"
model = "claude-haiku-4-5-20251001"

[tools]
profile = "data"

# No wallet section — read-only agents don't need one
```
